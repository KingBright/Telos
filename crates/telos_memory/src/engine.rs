use crate::types::{MemoryEntry, MemoryQuery, MemoryType};
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
}

pub struct RedbGraphStore {
    db: Arc<Database>,
    model: Option<Arc<tokio::sync::RwLock<fastembed::TextEmbedding>>>,
}

impl RedbGraphStore {
    pub fn new(path: &str) -> Result<Self, String> {
        let mut attempts = 0;
        let max_attempts = 5;
        let mut base_delay_ms = 100;

        let db = loop {
            match Database::create(path) {
                Ok(db) => break db,
                Err(e) => {
                    attempts += 1;
                    if attempts >= max_attempts {
                        return Err(format!(
                            "Failed to initialize MemoryOS database at {} after {} attempts: {}",
                            path, max_attempts, e
                        ));
                    }
                    eprintln!(
                        "[telos_memory] Database lock at `{}` busy (attempt {}/{}). Retrying in {}ms...",
                        path, attempts, max_attempts, base_delay_ms
                    );
                    std::thread::sleep(std::time::Duration::from_millis(base_delay_ms));
                    base_delay_ms *= 2; // Exponential backoff
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
        })
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
}

#[async_trait]
impl MemoryOS for RedbGraphStore {
    async fn store(&self, mut entry: MemoryEntry) -> Result<(), String> {
        // Automatically inject embedding if this is a Semantic, UserProfile, or InteractionEvent memory without one
        if (entry.memory_type == MemoryType::Semantic || entry.memory_type == MemoryType::UserProfile || entry.memory_type == MemoryType::InteractionEvent) && entry.embedding.is_none() {
            if let Some(ref model_arc) = self.model {
                let mut m = model_arc.write().await;
                if let Ok(embeddings) = m.embed(vec![entry.content.clone()], None) {
                    if let Some(vec) = embeddings.into_iter().next() {
                        entry.embedding = Some(vec);
                    }
                }
            }
        }
        
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
        let read_txn = self.db.begin_read().map_err(|e| e.to_string())?;
        let table = read_txn.open_table(MEMORY_TABLE).map_err(|e| e.to_string())?;

        let mut results = Vec::new();

        let iter = table.iter().map_err(|e| e.to_string())?;
        for result in iter {
            let (_key, value) = result.map_err(|e| e.to_string())?;
            let entry: MemoryEntry = serde_json::from_str(value.value()).map_err(|e| e.to_string())?;

            let matches = match &query {
                MemoryQuery::EntityLookup { entity } => {
                    // Search content or semantic type
                    (entry.memory_type == MemoryType::Semantic || entry.memory_type == MemoryType::UserProfile || entry.memory_type == MemoryType::InteractionEvent) && entry.content.contains(entity)
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
                }
            };

            if matches {
                results.push(entry);
            }
        }

        if let MemoryQuery::VectorSearch { top_k, query: search_vec } = query {
             // In a real system, compute sim for all and sort
             results.sort_by(|a, b| {
                let sim_a = a.embedding.as_ref().map(|v| Self::cosine_similarity(v, &search_vec)).unwrap_or(0.0);
                let sim_b = b.embedding.as_ref().map(|v| Self::cosine_similarity(v, &search_vec)).unwrap_or(0.0);
                sim_b.partial_cmp(&sim_a).unwrap_or(std::cmp::Ordering::Equal)
             });
             results.truncate(top_k);
        }

        Ok(results)
    }

    async fn consolidate(&self) -> Result<(), String> {
         // Placeholder for reconsolidation triggered by engine.
         // Actually implementation might move memories from Episodic to Semantic
         Ok(())
    }

    async fn trigger_fade_consolidation(&self) -> Result<(), String> {
         Ok(())
    }
}