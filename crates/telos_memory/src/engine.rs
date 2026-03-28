use crate::types::{MemoryEntry, MemoryQuery, MemoryRelation, MemoryType};
use crate::decay;
use crate::reconsolidation;
use async_trait::async_trait;
use redb::{Database, ReadableTable, TableDefinition};
use std::sync::Arc;
use telos_core::schedule::{ScheduledMission, MissionStatus};

const MEMORY_TABLE: TableDefinition<&str, &str> = TableDefinition::new("memories");
const MISSION_TABLE: TableDefinition<&str, &str> = TableDefinition::new("missions");

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
    /// Expand a set of retrieved memories by looking up their related memories.
    /// Returns the original entries plus any related entries not already present.
    async fn expand_relations(&self, entries: &[MemoryEntry]) -> Result<Vec<MemoryEntry>, String> {
        Ok(entries.to_vec())
    }
}

#[async_trait]
pub trait MissionStore: Send + Sync {
    async fn store_mission(&self, mission: ScheduledMission) -> Result<(), String>;
    async fn retrieve_missions(&self) -> Result<Vec<ScheduledMission>, String>;
    async fn retrieve_mission(&self, id: &str) -> Result<Option<ScheduledMission>, String>;
    async fn delete_mission(&self, id: &str) -> Result<(), String>;
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
            let _ = write_txn.open_table(MISSION_TABLE).map_err(|e| e.to_string())?;
        }
        write_txn.commit().map_err(|e| e.to_string())?;
        
        // Initialize Embedding Model
        let model = std::panic::catch_unwind(|| {
            fastembed::TextEmbedding::try_new(fastembed::InitOptions::new(
                fastembed::EmbeddingModel::MultilingualE5Small,
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
        // Automatically inject embedding for knowledge-bearing memory types
        let needs_embedding = matches!(
            entry.memory_type,
            MemoryType::Semantic | MemoryType::UserProfileStatic | MemoryType::UserProfileDynamic
            | MemoryType::InteractionEvent | MemoryType::Procedural
        );
        if needs_embedding && entry.embedding.is_none() {
            if let Some(ref model_arc) = self.model {
                let mut m = model_arc.write().await;
                if let Ok(embeddings) = m.embed(vec![entry.content.clone()], None) {
                    if let Some(vec) = embeddings.into_iter().next() {
                        entry.embedding = Some(vec);
                    }
                }
            }
        }

        // --- Conflict Detection & Version Chain Creation ---
        // For Semantic and UserProfile types: check for contradicting existing facts
        let conflict_types = matches!(
            entry.memory_type,
            MemoryType::Semantic | MemoryType::UserProfileStatic | MemoryType::UserProfileDynamic
        );
        if conflict_types && entry.embedding.is_some() {
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

                    // === VERSION CHAIN CREATION ===
                    // If old confidence is very low, this is a superseding update
                    let mut updated_existing = conflict.existing.clone();
                    updated_existing.confidence = old_conf;

                    if old_conf < 0.4 {
                        // Old memory is superseded → mark as non-latest
                        updated_existing.is_latest = false;

                        // Set up version chain on the new entry
                        entry.parent_memory_id = Some(conflict.existing.id.clone());
                        entry.root_memory_id = Some(
                            conflict.existing.root_memory_id.clone()
                                .unwrap_or_else(|| conflict.existing.id.clone())
                        );
                        entry.version = conflict.existing.version + 1;

                        // Add Updates relation
                        entry.memory_relations.insert(
                            conflict.existing.id.clone(),
                            MemoryRelation::Updates,
                        );

                        tracing::info!(
                            "[MemoryOS] 🔗 Version chain: v{} '{}' updates v{} '{}'",
                            entry.version,
                            entry.content.chars().take(30).collect::<String>(),
                            conflict.existing.version,
                            conflict.existing.content.chars().take(30).collect::<String>(),
                        );
                    } else if old_conf >= 0.4 && new_conf >= 0.4 {
                        // Both facts are valid → complementary/extending relationship
                        // Bidirectional: new extends existing, existing extends new
                        entry.memory_relations.insert(
                            conflict.existing.id.clone(),
                            MemoryRelation::Extends,
                        );
                        updated_existing.memory_relations.insert(
                            entry.id.clone(),
                            MemoryRelation::Extends,
                        );

                        tracing::info!(
                            "[MemoryOS] 🔗 Extends: '{}' enriches '{}'",
                            entry.content.chars().take(30).collect::<String>(),
                            conflict.existing.content.chars().take(30).collect::<String>(),
                        );
                    }

                    // Persist updated existing entry
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
                        entry.content.chars().take(40).collect::<String>(),
                        conflict.existing.content.chars().take(40).collect::<String>(),
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
        // Determine if this is a history-aware query
        let include_history = matches!(query, MemoryQuery::VectorSearchWithHistory { .. });

        // Normalize query: SemanticSearch → VectorSearch, VectorSearchWithHistory → VectorSearch
        let effective_query = match query {
            MemoryQuery::SemanticSearch { ref query, top_k } => {
                if let Some(embedding) = self.embed_text(query).await {
                    MemoryQuery::VectorSearch { query: embedding, top_k }
                } else {
                    MemoryQuery::EntityLookup { entity: query.clone() }
                }
            }
            MemoryQuery::VectorSearchWithHistory { query, top_k } => {
                MemoryQuery::VectorSearch { query, top_k }
            }
            other => other,
        };

        let read_txn = self.db.begin_read().map_err(|e| e.to_string())?;
        let table = read_txn.open_table(MEMORY_TABLE).map_err(|e| e.to_string())?;

        let now_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // === SMART FAST-PATH FOR RELATED-TO (OUTGOING) ===
        use crate::types::EdgeDirection;
        if let MemoryQuery::RelatedTo { target_id, relation, direction } = &effective_query {
            if *direction == EdgeDirection::Outgoing {
                let mut fast_results = Vec::new();
                if let Some(value) = table.get(target_id.as_str()).map_err(|e| e.to_string())? {
                    let target_entry: MemoryEntry = serde_json::from_str(value.value()).map_err(|e| e.to_string())?;
                    for (rel_id, rel_type) in target_entry.memory_relations {
                        if relation.as_ref().map_or(true, |r| r == &rel_type) {
                            if let Some(rel_value) = table.get(rel_id.as_str()).map_err(|e| e.to_string())? {
                                let mut rel_entry: MemoryEntry = serde_json::from_str(rel_value.value()).map_err(|e| e.to_string())?;
                                if rel_entry.is_retrievable() {
                                    fast_results.push(rel_entry);
                                }
                            }
                        }
                    }
                }
                if !fast_results.is_empty() {
                    self.touch_entries(&fast_results).await;
                }
                return Ok(fast_results);
            }
        }

        let mut results = Vec::new();

        let iter = table.iter().map_err(|e| e.to_string())?;
        for result in iter {
            let (_key, value) = result.map_err(|e| e.to_string())?;
            let mut entry: MemoryEntry = serde_json::from_str(value.value()).map_err(|e| e.to_string())?;

            // === SMART FILTERING ===
            // Skip forgotten, non-latest, and low-confidence entries (unless querying history)
            if !include_history && !entry.is_retrievable() {
                continue;
            }

            // Check temporal expiry at retrieval time (lazy check)
            if let Some(forget_at) = entry.forget_after {
                if now_ts > forget_at && !entry.is_forgotten {
                    entry.is_forgotten = true;
                    entry.forget_reason = Some("temporal_expiry".to_string());
                    // Fire-and-forget: mark as forgotten in DB
                    // (will be persisted on next consolidation sweep)
                    if !include_history {
                        continue;
                    }
                }
            }

            let matches = match &effective_query {
                MemoryQuery::EntityLookup { entity } => {
                    matches!(
                        entry.memory_type,
                        MemoryType::Semantic | MemoryType::UserProfileStatic
                        | MemoryType::UserProfileDynamic | MemoryType::InteractionEvent
                        | MemoryType::Procedural
                    ) && entry.content.contains(entity)
                },
                MemoryQuery::TimeRange { start, end } => {
                    entry.created_at >= *start && entry.created_at <= *end
                },
                MemoryQuery::VectorSearch { query: search_vec, top_k: _ } => {
                    if let Some(ref doc_vec) = entry.embedding {
                         let sim = Self::cosine_similarity(doc_vec, search_vec);
                         if sim > 0.5 {
                             entry.similarity_score = Some(sim);
                             true
                         } else {
                             false
                         }
                    } else {
                        false
                    }
                },
                MemoryQuery::RelatedTo { target_id, relation, direction } => {
                    use crate::types::EdgeDirection;
                    match direction {
                        EdgeDirection::Outgoing => false, // Handled strictly by fast-path earlier
                        EdgeDirection::Incoming | EdgeDirection::Both => {
                            // Because fast-path returns early for Outgoing, 
                            // Both will currently only scan for Incoming. We can refine Both later if needed.
                            if let Some(rel) = entry.memory_relations.get(target_id) {
                                relation.as_ref().map_or(true, |r| r == rel)
                            } else {
                                false
                            }
                        }
                    }
                },
                _ => false, // SemanticSearch and VectorSearchWithHistory already converted
            };

            if matches {
                results.push(entry);
            }
        }

        if let MemoryQuery::VectorSearch { top_k, .. } = effective_query {
             // Time-weighted scoring: final_score = similarity * 0.7 + recency * 0.3
             const ONE_DAY_SECS: f64 = 86400.0;
             for entry in &mut results {
                 let sim = entry.similarity_score.unwrap_or(0.0) as f64;
                 let age_days = (now_ts.saturating_sub(entry.created_at) as f64) / ONE_DAY_SECS;
                 let recency = 1.0 / (1.0 + age_days);
                 let final_score = sim * 0.7 + recency * 0.3;
                 entry.similarity_score = Some(final_score as f32);
             }

             results.sort_by(|a, b| {
                let sim_a = a.similarity_score.unwrap_or(0.0);
                let sim_b = b.similarity_score.unwrap_or(0.0);
                sim_b.partial_cmp(&sim_a).unwrap_or(std::cmp::Ordering::Equal)
             });
             results.truncate(top_k);
        }

        // Touch retrieved entries to update last_accessed and access_count
        if !results.is_empty() {
            self.touch_entries(&results).await;
        }

        Ok(results)
    }

    async fn consolidate(&self) -> Result<(), String> {
        let all_entries = self.retrieve_all().await?;

        let mut episodic_entries: Vec<MemoryEntry> = all_entries
            .into_iter()
            .filter(|e| e.memory_type == MemoryType::Episodic)
            .collect();

        if episodic_entries.is_empty() {
            return Ok(());
        }

        let gateway = {
            let guard = self.gateway.read().await;
            guard.clone()
        };

        let threshold = 3.0;
        let newly_consolidated = reconsolidation::consolidate_memories(
            &mut episodic_entries,
            threshold,
            gateway,
        ).await;

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
        let min_strength = 0.1;

        let mut pruned_count = 0;
        let mut forgotten_count = 0;
        let mut updated_count = 0;

        for mut entry in all_entries {
            let should_prune = decay::apply_decay(&mut entry, current_ts, min_strength);

            if should_prune {
                if entry.is_forgotten {
                    // Temporally expired → keep in DB but marked forgotten (preserves history)
                    self.store(entry).await?;
                    forgotten_count += 1;
                } else {
                    // Strength-decayed → delete
                    self.delete(&entry.id).await?;
                    pruned_count += 1;
                }
            } else if matches!(
                entry.memory_type,
                MemoryType::Episodic | MemoryType::InteractionEvent | MemoryType::UserProfileDynamic
            ) {
                // Update the decayed strength in-place
                self.store(entry).await?;
                updated_count += 1;
            }
        }

        if pruned_count > 0 || forgotten_count > 0 || updated_count > 0 {
            tracing::info!(
                "[MemoryOS] 🧹 Fade sweep: pruned={}, forgotten={}, updated={}",
                pruned_count, forgotten_count, updated_count
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

    async fn expand_relations(&self, entries: &[MemoryEntry]) -> Result<Vec<MemoryEntry>, String> {
        let mut result: Vec<MemoryEntry> = entries.to_vec();
        let existing_ids: std::collections::HashSet<String> = result.iter().map(|e| e.id.clone()).collect();
        let mut ids_to_fetch: Vec<String> = Vec::new();

        // Collect all related IDs not already in the result set
        for entry in entries {
            for related_id in entry.memory_relations.keys() {
                if !existing_ids.contains(related_id) && !ids_to_fetch.contains(related_id) {
                    ids_to_fetch.push(related_id.clone());
                }
            }
        }

        if ids_to_fetch.is_empty() {
            return Ok(result);
        }

        // Look up related entries from DB
        let read_txn = self.db.begin_read().map_err(|e| e.to_string())?;
        let table = read_txn.open_table(MEMORY_TABLE).map_err(|e| e.to_string())?;

        for id in &ids_to_fetch {
            if let Some(value) = table.get(id.as_str()).map_err(|e| e.to_string())? {
                let entry: MemoryEntry = serde_json::from_str(value.value()).map_err(|e| e.to_string())?;
                if entry.is_retrievable() {
                    result.push(entry);
                }
            }
        }

        Ok(result)
    }
}

#[async_trait]
impl MissionStore for RedbGraphStore {
    async fn store_mission(&self, mission: ScheduledMission) -> Result<(), String> {
        let write_txn = self.db.begin_write().map_err(|e| e.to_string())?;
        {
            let mut table = write_txn.open_table(MISSION_TABLE).map_err(|e| e.to_string())?;
            let serialized = serde_json::to_string(&mission).map_err(|e| e.to_string())?;
            table.insert(mission.id.as_str(), serialized.as_str()).map_err(|e| e.to_string())?;
        }
        write_txn.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn retrieve_missions(&self) -> Result<Vec<ScheduledMission>, String> {
        let read_txn = self.db.begin_read().map_err(|e| e.to_string())?;
        let table = read_txn.open_table(MISSION_TABLE).map_err(|e| e.to_string())?;
        let mut results = Vec::new();
        let iter = table.iter().map_err(|e| e.to_string())?;
        for result in iter {
            let (_key, value) = result.map_err(|e| e.to_string())?;
            let mission: ScheduledMission = serde_json::from_str(value.value()).map_err(|e| e.to_string())?;
            results.push(mission);
        }
        Ok(results)
    }
    
    async fn retrieve_mission(&self, id: &str) -> Result<Option<ScheduledMission>, String> {
        let read_txn = self.db.begin_read().map_err(|e| e.to_string())?;
        let table = read_txn.open_table(MISSION_TABLE).map_err(|e| e.to_string())?;
        if let Some(value) = table.get(id).map_err(|e| e.to_string())? {
            let mission: ScheduledMission = serde_json::from_str(value.value()).map_err(|e| e.to_string())?;
            Ok(Some(mission))
        } else {
            Ok(None)
        }
    }

    async fn delete_mission(&self, id: &str) -> Result<(), String> {
        let write_txn = self.db.begin_write().map_err(|e| e.to_string())?;
        {
            let mut table = write_txn.open_table(MISSION_TABLE).map_err(|e| e.to_string())?;
            table.remove(id).map_err(|e| e.to_string())?;
        }
        write_txn.commit().map_err(|e| e.to_string())?;
        Ok(())
    }
}