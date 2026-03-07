use crate::engine::MemoryOS;
use crate::types::{MemoryEntry, MemoryQuery, MemoryType};
use async_trait::async_trait;
use telos_core::NodeResult;

/// Provides a standard interface for other modules (like DAG Engine and Context Compression)
/// to interact with the Memory OS.
#[async_trait]
pub trait MemoryIntegration: Send + Sync {
    /// Designed for the DAG Engine (Module 2).
    /// Once a node completes execution, its result is ingested directly as an episodic memory.
    async fn ingest_node_result(&self, node_id: String, result: &NodeResult) -> Result<(), String>;

    /// Designed for the Context Compression (Module 3).
    /// Retrieves highly relevant semantic facts from long-term storage to supplement `ScopedContext`.
    async fn retrieve_semantic_facts(&self, query_string: String) -> Result<Vec<String>, String>;

    /// Designed for the HCI Event Bus (Module 1).
    /// Direct, high-priority user feedback triggers immediate write operations to Semantic Memory with maximum strength, ensuring user preferences override default behaviors.
    async fn ingest_user_feedback(&self, feedback: &str, strength: f32) -> Result<(), String>;
}

// Implement this trait for any system that implements `MemoryOS` (like RedbGraphStore).
#[async_trait]
impl<T: MemoryOS + ?Sized> MemoryIntegration for T {
    async fn ingest_node_result(&self, node_id: String, result: &NodeResult) -> Result<(), String> {
        let content = String::from_utf8(result.output_data.clone())
            .unwrap_or_else(|_| "Binary Output".to_string());

        // We store every DAG result as an Episodic Memory.
        // The ID can be the node_id mixed with a timestamp.
        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let entry_id = format!("{}_{}", node_id, timestamp);

        let entry = MemoryEntry::new(
            entry_id,
            MemoryType::Episodic,
            content,
            timestamp,
            None, // In a real setup, we would embed the content here before storing.
        );

        self.store(entry).await
    }

    async fn retrieve_semantic_facts(&self, query_string: String) -> Result<Vec<String>, String> {
        // Query the memory OS specifically for Semantic memories relating to the query.
        let query = MemoryQuery::EntityLookup { entity: query_string };

        let results = self.retrieve(query).await?;

        // Extract the content from the semantic memories.
        Ok(results.into_iter().map(|e| e.content).collect())
    }

    async fn ingest_user_feedback(&self, feedback: &str, strength: f32) -> Result<(), String> {
        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let entry_id = format!("feedback_{}", timestamp);

        let mut entry = MemoryEntry::new(
            entry_id,
            MemoryType::Semantic, // User feedback is directly promoted to Semantic to override behavior
            feedback.to_string(),
            timestamp,
            None,
        );

        // Force the specified strength to ensure high priority overrides
        entry.base_strength = strength;
        entry.current_strength = strength;

        self.store(entry).await
    }
}
