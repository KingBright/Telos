use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Debug;

fn default_confidence() -> f32 { 1.0 }
fn default_version() -> u32 { 1 }
fn default_true() -> bool { true }

/// Memory types — split UserProfile into Static (long-term) and Dynamic (recent context).
/// Legacy `UserProfile` variant maps to `UserProfileStatic` for backward compat.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MemoryType {
    Episodic,
    Semantic,
    Procedural,
    /// Legacy alias: deserialized as UserProfileStatic
    #[serde(alias = "UserProfile")]
    UserProfileStatic,
    /// Recent/temporary user context (project work, debugging, etc.)
    UserProfileDynamic,
    InteractionEvent,
    
    // === Meta-Graph Types (Project Harness) ===
    MetaFeature,   // Maps to core::meta_graph::ProductFeature
    MetaModule,    // Maps to core::meta_graph::TechModule
    MetaContract,  // Maps to core::meta_graph::Contract
    MetaTask,      // Maps to core::meta_graph::DevTask
}

/// Relationship types between memories (inspired by Supermemory's graph model).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MemoryRelation {
    /// New fact supersedes/contradicts old fact ("I drive Tesla" updates "I drive BYD")
    Updates,
    /// New fact enriches old fact without replacing it ("Focuses on payments" extends "PM at Stripe")
    Extends,
    /// System inferred a new fact from patterns (reconsolidation)
    Derives,
    
    // === Strict Structural Edges (Project Harness) ===
    /// Module Implements Feature, or Module Implements Contract
    Implements,
    /// Task DependsOn Contract, Module DependsOn Module
    DependsOn,
    /// Code TestedBy Contract Mock
    TestedBy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EdgeDirection {
    Incoming,
    Outgoing,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MemoryQuery {
    VectorSearch { query: Vec<f32>, top_k: usize },
    /// Text-based semantic search: the engine embeds the query internally
    SemanticSearch { query: String, top_k: usize },
    EntityLookup { entity: String },
    TimeRange { start: u64, end: u64 },
    /// Like VectorSearch but also returns the version history chain
    VectorSearchWithHistory { query: Vec<f32>, top_k: usize },
    /// Graph traversal query to find related nodes
    RelatedTo { 
        target_id: String, 
        relation: Option<MemoryRelation>,
        direction: EdgeDirection,
    },
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
    /// Cosine similarity score from the most recent vector search (transient, not persisted meaningfully)
    #[serde(default)]
    pub similarity_score: Option<f32>,

    // === Version Control (Supermemory-inspired) ===
    /// Version number in its chain (starts at 1, increments on each update)
    #[serde(default = "default_version")]
    pub version: u32,
    /// Whether this is the latest version in its chain; retrieve() defaults to only returning latest
    #[serde(default = "default_true")]
    pub is_latest: bool,
    /// ID of the memory this entry supersedes (forms a version chain)
    #[serde(default)]
    pub parent_memory_id: Option<String>,
    /// ID of the root/original memory in the version chain
    #[serde(default)]
    pub root_memory_id: Option<String>,

    // === Memory Relations ===
    /// Relationship map: related_memory_id -> relation_type
    #[serde(default)]
    pub memory_relations: HashMap<String, MemoryRelation>,

    // === Temporal Forgetting ===
    /// If set, this memory should be auto-forgotten after this UNIX timestamp (e.g. "meeting tomorrow")
    #[serde(default)]
    pub forget_after: Option<u64>,
    /// Whether this memory has been forgotten (temporal expiry, contradicted, or user-requested)
    #[serde(default)]
    pub is_forgotten: bool,
    /// Reason for forgetting (for audit/debugging)
    #[serde(default)]
    pub forget_reason: Option<String>,
    /// Whether this is a stable/permanent fact that should never decay
    #[serde(default)]
    pub is_static: bool,
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
            similarity_score: None,
            // Version control defaults
            version: 1,
            is_latest: true,
            parent_memory_id: None,
            root_memory_id: None,
            // Relations
            memory_relations: HashMap::new(),
            // Temporal forgetting
            forget_after: None,
            is_forgotten: false,
            forget_reason: None,
            is_static: false,
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

    /// Check if this memory should be included in retrieval results.
    /// Returns false for forgotten, non-latest, or low-confidence entries.
    pub fn is_retrievable(&self) -> bool {
        !self.is_forgotten && self.is_latest && self.confidence >= 0.3
    }
}

/// Aggregated user profile with static facts and dynamic context.
#[derive(Debug, Clone, Default)]
pub struct UserProfile {
    /// Long-term stable facts about the user
    pub static_facts: Vec<String>,
    /// Recent context and temporary states
    pub dynamic_context: Vec<String>,
}
