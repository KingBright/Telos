use crate::types::{MemoryEntry, MemoryQuery, MemoryType};
use crate::decay;
use crate::reconsolidation;
use async_trait::async_trait;
use redb::{Database, ReadableTable, TableDefinition};
use std::sync::Arc;

const MEMORY_TABLE: TableDefinition<&str, &str> = TableDefinition::new("memories");

#[async_trait]
pub trait MemoryOS: Send + Sync {
    async fn store(&self, entry: MemoryEntry) -> Result<(), String>;
    async fn retrieve(&self, query: MemoryQuery) -> Result<Vec<MemoryEntry>, String>;
    async fn consolidate(&self) -> Result<(), String>;
    async fn trigger_fade_consolidation(&self) -> Result<(), String>;
    /// Delete a memory entry by ID
    async fn delete(&self, id: &str) -> Result<(), String>;
    /// Retrieve all entries (for maintenance operations)
    async fn retrieve_all(&self) -> Result<Vec<MemoryEntry>, String>;
}

pub struct RedbGraphStore {
    db: Arc<Database>,
    model: Option<Arc<tokio::sync::RwLock<fastembed::TextEmbedding>>>,
    /// Optional LLM gateway for reconsolidation (set via `set_gateway`)
    gateway: tokio::sync::RwLock<Option<Arc<dyn telos_model_gateway::ModelGateway>>>,
}

impl RedbGraphStore {
    pub fn new(path: &str) -> Result<Self, String> {
        let mut attempts = 0;
        let mut base_delay_ms = 100;

        let db = loop {
            match Database::create(path) {
                Ok(db) => break db,
                Err(e) => {
                    attempts += 1;
                    eprintln!(
                        "[telos_memory] Database lock at `{}` busy (attempt {}). Retrying in {}ms... (Error: {})",
                        path, attempts, base_delay_ms, e
                    );
                    std::thread::sleep(std::time::Duration::from_millis(base_delay_ms));
                    base_delay_ms = std::cmp::min(base_delay_ms * 2, 5000); // Exponential backoff capped at 5s
                }
            }
        };

        let write_txn = db.begin_write().map_err(|e| e.to_string())?;
        {
            let _ = write_txn.open_table(MEMORY_TABLE).map_err(|e| e.to_string())?;
        }
        write_txn.commit().map_err(|e| e.to_string())?;
        
        // Initialize Embedding Model
        let model = std::panic::catch_unwind(|| {
            fastembed::TextEmbedding::try_new(fastembed::InitOptions::new(
                fastembed::EmbeddingModel::AllMiniLML6V2,
            ).with_cache_dir(
                dirs::home_dir().map(|h| h.join(".telos").join("models")).unwrap_or_else(|| std::path::PathBuf::from(".fastembed_cache"))
            ))
        });
        
        let model_opt = match model {
            Ok(Ok(m)) => {
                tracing::info!("[MemoryOS] Fastembed local model initialized for vector search.");
                Some(Arc::new(tokio::sync::RwLock::new(m)))
            }
            _ => {
                tracing::warn!("[MemoryOS] Fastembed init failed. Vector search will fallback or fail.");
                None
            }
        };

        Ok(Self {
            db: Arc::new(db),
            model: model_opt,
            gateway: tokio::sync::RwLock::new(None),
        })
    }

    /// Inject an LLM gateway for reconsolidation (called once at daemon startup)
    pub fn set_gateway(&self, gw: Arc<dyn telos_model_gateway::ModelGateway>) {
        // We use try_write to avoid blocking; if it fails we just skip
        if let Ok(mut guard) = self.gateway.try_write() {
            *guard = Some(gw);
        }
    }

    /// Embed a text query using the local fastembed model
    async fn embed_text(&self, text: &str) -> Option<Vec<f32>> {
        if let Some(ref model_arc) = self.model {
            let mut m = model_arc.write().await;
            if let Ok(embeddings) = m.embed(vec![text.to_string()], None) {
                return embeddings.into_iter().next();
            }
        }
        None
    }

    fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        let mut dot_product = 0.0;
        let mut norm_a = 0.0;
        let mut norm_b = 0.0;
        for (x, y) in a.iter().zip(b.iter()) {
            dot_product += x * y;
            norm_a += x * x;
            norm_b += y * y;
        }
        if norm_a == 0.0 || norm_b == 0.0 {
            0.0
        } else {
            dot_product / (norm_a.sqrt() * norm_b.sqrt())
        }
    }

    /// Update specified entries' last_accessed and access_count in the database
    async fn touch_entries(&self, entries: &[MemoryEntry]) {
        let now_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if let Ok(write_txn) = self.db.begin_write() {
            let update_result: Result<(), String> = (|| {
                let mut table = write_txn.open_table(MEMORY_TABLE).map_err(|e| e.to_string())?;
                for entry in entries {
                    let mut updated = entry.clone();
                    updated.access(now_ts);
                    let serialized = serde_json::to_string(&updated).map_err(|e| e.to_string())?;
                    table.insert(updated.id.as_str(), serialized.as_str()).map_err(|e| e.to_string())?;
                }
                Ok(())
            })();

            if update_result.is_ok() {
                let _ = write_txn.commit();
            }
        }
    }
}

#[async_trait]
impl MemoryOS for RedbGraphStore {
    async fn store(&self, mut entry: MemoryEntry) -> Result<(), String> {
        // Automatically inject embedding if this is a Semantic, UserProfile, InteractionEvent, or Procedural memory without one
        if (entry.memory_type == MemoryType::Semantic || entry.memory_type == MemoryType::UserProfile || entry.memory_type == MemoryType::InteractionEvent || entry.memory_type == MemoryType::Procedural) && entry.embedding.is_none() {
            if let Some(ref model_arc) = self.model {
                let mut m = model_arc.write().await;
                if let Ok(embeddings) = m.embed(vec![entry.content.clone()], None) {
                    if let Some(vec) = embeddings.into_iter().next() {
                        entry.embedding = Some(vec);
                    }
                }
            }
        }

        // --- Conflict Detection (GraphRAG-inspired) ---
        // Only for Semantic and UserProfile: check for contradicting existing facts
        if (entry.memory_type == MemoryType::Semantic || entry.memory_type == MemoryType::UserProfile) 
            && entry.embedding.is_some() 
        {
            let existing = self.retrieve_all().await.unwrap_or_default();
            let conflicts = crate::conflict::detect_conflicts(&entry, &existing, 0.8);

            if !conflicts.is_empty() {
                let gateway_opt = {
                    let guard = self.gateway.read().await;
                    guard.clone()
                };

                for conflict in &conflicts {
                    let (new_conf, old_conf) = if let Some(ref gw) = gateway_opt {
                        crate::conflict::resolve_conflict_with_llm(
                            &entry.content,
                            &conflict.existing.content,
                            gw.as_ref(),
                        ).await
                    } else {
                        // Without LLM: new fact takes precedence, old slightly reduced
                        (1.0, 0.5)
                    };

                    // Update the existing entry's confidence
                    let mut updated_existing = conflict.existing.clone();
                    updated_existing.confidence = old_conf;
                    
                    let write_txn = self.db.begin_write().map_err(|e| e.to_string())?;
                    {
                        let mut table = write_txn.open_table(MEMORY_TABLE).map_err(|e| e.to_string())?;
                        let serialized = serde_json::to_string(&updated_existing).map_err(|e| e.to_string())?;
                        table.insert(updated_existing.id.as_str(), serialized.as_str()).map_err(|e| e.to_string())?;
                    }
                    write_txn.commit().map_err(|e| e.to_string())?;

                    entry.confidence = new_conf;

                    tracing::info!(
                        "[MemoryOS] ⚡ Conflict detected: '{}' vs '{}' (sim={:.2}). New={:.1}, Old={:.1}",
                        &entry.content[..entry.content.len().min(40)],
                        &conflict.existing.content[..conflict.existing.content.len().min(40)],
                        conflict.similarity,
                        new_conf,
                        old_conf,
                    );
                }
            }
        }

        // --- Write the entry ---
        let write_txn = self.db.begin_write().map_err(|e| e.to_string())?;
        {
            let mut table = write_txn.open_table(MEMORY_TABLE).map_err(|e| e.to_string())?;
            let serialized = serde_json::to_string(&entry).map_err(|e| e.to_string())?;
            table.insert(entry.id.as_str(), serialized.as_str()).map_err(|e| e.to_string())?;
        }
        write_txn.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn retrieve(&self, query: MemoryQuery) -> Result<Vec<MemoryEntry>, String> {
        // If SemanticSearch, embed the text query first, then delegate to VectorSearch logic
        let effective_query = match query {
            MemoryQuery::SemanticSearch { ref query, top_k } => {
                if let Some(embedding) = self.embed_text(query).await {
                    MemoryQuery::VectorSearch { query: embedding, top_k }
                } else {
                    // Fallback to EntityLookup if embedding fails
                    MemoryQuery::EntityLookup { entity: query.clone() }
                }
            }
            other => other,
        };

        let read_txn = self.db.begin_read().map_err(|e| e.to_string())?;
        let table = read_txn.open_table(MEMORY_TABLE).map_err(|e| e.to_string())?;

        let mut results = Vec::new();

        let iter = table.iter().map_err(|e| e.to_string())?;
        for result in iter {
            let (_key, value) = result.map_err(|e| e.to_string())?;
            let entry: MemoryEntry = serde_json::from_str(value.value()).map_err(|e| e.to_string())?;

            let matches = match &effective_query {
                MemoryQuery::EntityLookup { entity } => {
                    // Search content or semantic type
                    (entry.memory_type == MemoryType::Semantic || entry.memory_type == MemoryType::UserProfile || entry.memory_type == MemoryType::InteractionEvent || entry.memory_type == MemoryType::Procedural) && entry.content.contains(entity)
                },
                MemoryQuery::TimeRange { start, end } => {
                    entry.created_at >= *start && entry.created_at <= *end
                },
                MemoryQuery::VectorSearch { query: search_vec, top_k: _ } => {
                    if let Some(ref doc_vec) = entry.embedding {
                         let sim = Self::cosine_similarity(doc_vec, search_vec);
                         sim > 0.5 // Arbitrary threshold
                    } else {
                        false
                    }
                },
                MemoryQuery::SemanticSearch { .. } => {
                    // Already converted to VectorSearch or EntityLookup above
                    false
                }
            };

            if matches {
                results.push(entry);
            }
        }

        if let MemoryQuery::VectorSearch { top_k, query: ref search_vec } = effective_query {
             results.sort_by(|a, b| {
                let sim_a = a.embedding.as_ref().map(|v| Self::cosine_similarity(v, search_vec)).unwrap_or(0.0);
                let sim_b = b.embedding.as_ref().map(|v| Self::cosine_similarity(v, search_vec)).unwrap_or(0.0);
                sim_b.partial_cmp(&sim_a).unwrap_or(std::cmp::Ordering::Equal)
             });
             results.truncate(top_k);
        }

        // Touch retrieved entries to update last_accessed and access_count
        // This is fire-and-forget to avoid blocking the caller
        if !results.is_empty() {
            self.touch_entries(&results).await;
        }

        Ok(results)
    }

    async fn consolidate(&self) -> Result<(), String> {
        // Fetch all entries
        let all_entries = self.retrieve_all().await?;

        // Filter only Episodic memories
        let mut episodic_entries: Vec<MemoryEntry> = all_entries
            .into_iter()
            .filter(|e| e.memory_type == MemoryType::Episodic)
            .collect();

        if episodic_entries.is_empty() {
            return Ok(());
        }

        // Get gateway if available
        let gateway = {
            let guard = self.gateway.read().await;
            guard.clone()
        };

        // Run reconsolidation: promote strong Episodic → Semantic
        let threshold = 3.0; // base_strength threshold for promotion
        let newly_consolidated = reconsolidation::consolidate_memories(
            &mut episodic_entries,
            threshold,
            gateway,
        ).await;

        // Store the newly promoted Semantic entries
        let promoted_count = newly_consolidated.len();
        for entry in newly_consolidated {
            self.store(entry).await?;
        }

        if promoted_count > 0 {
            tracing::info!(
                "[MemoryOS] 🧠 Consolidated {} episodic memories → semantic knowledge",
                promoted_count
            );
        }

        Ok(())
    }

    async fn trigger_fade_consolidation(&self) -> Result<(), String> {
        let all_entries = self.retrieve_all().await?;
        let current_ts = decay::get_current_timestamp();
        let min_strength = 0.1; // Below this = forgotten

        let mut pruned_count = 0;
        let mut updated_count = 0;

        for mut entry in all_entries {
            let should_prune = decay::apply_decay(&mut entry, current_ts, min_strength);

            if should_prune {
                // Delete the forgotten memory
                self.delete(&entry.id).await?;
                pruned_count += 1;
            } else if entry.memory_type == MemoryType::Episodic || entry.memory_type == MemoryType::InteractionEvent {
                // Update the decayed strength in-place (only for decay-eligible types)
                self.store(entry).await?;
                updated_count += 1;
            }
        }

        if pruned_count > 0 || updated_count > 0 {
            tracing::info!(
                "[MemoryOS] 🧹 Fade sweep: pruned {} forgotten memories, updated {} decay strengths",
                pruned_count, updated_count
            );
        }

        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<(), String> {
        let write_txn = self.db.begin_write().map_err(|e| e.to_string())?;
        {
            let mut table = write_txn.open_table(MEMORY_TABLE).map_err(|e| e.to_string())?;
            table.remove(id).map_err(|e| e.to_string())?;
        }
        write_txn.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn retrieve_all(&self) -> Result<Vec<MemoryEntry>, String> {
        let read_txn = self.db.begin_read().map_err(|e| e.to_string())?;
        let table = read_txn.open_table(MEMORY_TABLE).map_err(|e| e.to_string())?;
        let mut results = Vec::new();
        let iter = table.iter().map_err(|e| e.to_string())?;
        for result in iter {
            let (_key, value) = result.map_err(|e| e.to_string())?;
            let entry: MemoryEntry = serde_json::from_str(value.value()).map_err(|e| e.to_string())?;
            results.push(entry);
        }
        Ok(results)
    }
}