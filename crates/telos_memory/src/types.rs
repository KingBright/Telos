use serde::{Deserialize, Serialize};
use std::fmt::Debug;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MemoryType {
    Episodic,
    Semantic,
    Procedural,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MemoryQuery {
    VectorSearch { query: Vec<f32>, top_k: usize },
    EntityLookup { entity: String },
    TimeRange { start: u64, end: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub memory_type: MemoryType,
    pub content: String,
    pub base_strength: f32,
    pub current_strength: f32,
    pub created_at: u64,
    pub last_accessed: u64,
    pub embedding: Option<Vec<f32>>,
}

impl MemoryEntry {
    pub fn new(id: String, memory_type: MemoryType, content: String, created_at: u64, embedding: Option<Vec<f32>>) -> Self {
        Self {
            id,
            memory_type,
            content,
            base_strength: 1.0, // Initial memory strength
            current_strength: 1.0,
            created_at,
            last_accessed: created_at,
            embedding,
        }
    }

    pub fn access(&mut self, timestamp: u64) {
        self.last_accessed = timestamp;
        self.base_strength = (self.base_strength + 0.5).min(5.0); // Cap strength
        self.current_strength = self.base_strength; // Reset decay curve on access
    }
}
