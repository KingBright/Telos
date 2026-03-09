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

        Ok(Self {
            db: Arc::new(db),
        })
    }
}

#[async_trait]
impl MemoryOS for RedbGraphStore {
    async fn store(&self, entry: MemoryEntry) -> Result<(), String> {
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

            // Filter logic based on query type (very basic mock implementation for V1)
            let matches = match &query {
                MemoryQuery::EntityLookup { entity } => {
                    // Search content or semantic type
                    entry.memory_type == MemoryType::Semantic && entry.content.contains(entity)
                },
                MemoryQuery::TimeRange { start, end } => {
                    entry.created_at >= *start && entry.created_at <= *end
                },
                MemoryQuery::VectorSearch { query: search_vec, top_k: _ } => {
                    if let Some(ref doc_vec) = entry.embedding {
                         // Basic cosine simmock
                         let sim = doc_vec.iter().zip(search_vec).map(|(a, b)| a * b).sum::<f32>();
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
                let sim_a = a.embedding.as_ref().map(|v| v.iter().zip(&search_vec).map(|(x,y)| x*y).sum::<f32>()).unwrap_or(0.0);
                let sim_b = b.embedding.as_ref().map(|v| v.iter().zip(&search_vec).map(|(x,y)| x*y).sum::<f32>()).unwrap_or(0.0);
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