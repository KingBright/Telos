use serde::{Deserialize, Serialize};
use std::fmt::Debug;

fn default_confidence() -> f32 { 1.0 }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MemoryType {
    Episodic,
    Semantic,
    Procedural,
    UserProfile,
    InteractionEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MemoryQuery {
    VectorSearch { query: Vec<f32>, top_k: usize },
    /// Text-based semantic search: the engine embeds the query internally
    SemanticSearch { query: String, top_k: usize },
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
    /// How many times this memory has been retrieved (influences strength growth)
    #[serde(default)]
    pub access_count: u32,
    /// Confidence score: 1.0 = fully trusted, lower = contested by conflicting facts
    #[serde(default = "default_confidence")]
    pub confidence: f32,
}

impl MemoryEntry {
    pub fn new(id: String, memory_type: MemoryType, content: String, created_at: u64, embedding: Option<Vec<f32>>) -> Self {
        Self {
            id,
            memory_type,
            content,
            base_strength: 1.0,
            current_strength: 1.0,
            created_at,
            last_accessed: created_at,
            embedding,
            access_count: 0,
            confidence: 1.0,
        }
    }

    pub fn access(&mut self, timestamp: u64) {
        self.last_accessed = timestamp;
        self.access_count += 1;
        // Spaced repetition: diminishing returns on strength gain
        let boost = 0.5 / (1.0 + self.access_count as f32 * 0.1);
        self.base_strength = (self.base_strength + boost).min(5.0); // Cap strength
        self.current_strength = self.base_strength; // Reset decay curve on access
    }
}
