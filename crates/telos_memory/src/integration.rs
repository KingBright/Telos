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

    /// Designed for explicit memory storage via tools.
    /// Allows the agent to explicitly store important facts or insights as semantic memories.
    async fn store_semantic_fact(&self, content: String) -> Result<(), String>;

    /// Retrieves procedural memories (distilled skills or workflow templates) based on a query.
    async fn retrieve_procedural_memories(&self, query_string: String) -> Result<Vec<String>, String>;

    /// Designed for the Evolution module (Module 6).
    /// Stores distilled skills/strategies as Procedural Memory (never decays).
    async fn store_procedural_skill(&self, trigger: String, procedure: String) -> Result<(), String>;

    /// Stores the topology of a successful DAG execution as a JSON Workflow Template.
    async fn store_workflow_template(&self, description: String, template_json: String) -> Result<(), String>;

    /// Upgrades an existing workflow template if a similar one exists (cosine similarity > 0.8).
    /// If no similar template is found, stores as new. Returns true if upgraded, false if new.
    async fn upgrade_workflow_template(&self, description: String, template_json: String) -> Result<bool, String>;
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

    async fn store_semantic_fact(&self, content: String) -> Result<(), String> {
        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let entry_id = format!("semantic_{}", timestamp);

        let entry = MemoryEntry::new(
            entry_id,
            MemoryType::Semantic,
            content,
            timestamp,
            None,
        );

        self.store(entry).await
    }

    async fn retrieve_procedural_memories(&self, query_string: String) -> Result<Vec<String>, String> {
        // Dual-strategy retrieval for maximum recall:
        // Strategy 1: SemanticSearch (embedding-based similarity) for fuzzy matching
        // Strategy 2: EntityLookup (keyword) as fallback
        let mut seen_ids = std::collections::HashSet::new();
        let mut procedural_results = Vec::new();
        
        // Strategy 1: Vector similarity search (primary)
        let semantic_query = MemoryQuery::SemanticSearch { query: query_string.clone(), top_k: 10 };
        if let Ok(results) = self.retrieve(semantic_query).await {
            for e in results {
                if e.memory_type == MemoryType::Procedural && seen_ids.insert(e.id.clone()) {
                    procedural_results.push(e.content);
                }
            }
        }
        
        // Strategy 2: Keyword fallback (catches exact term matches that vector might miss)
        let keyword_query = MemoryQuery::EntityLookup { entity: query_string };
        if let Ok(results) = self.retrieve(keyword_query).await {
            for e in results {
                if e.memory_type == MemoryType::Procedural && seen_ids.insert(e.id.clone()) {
                    procedural_results.push(e.content);
                }
            }
        }
        
        Ok(procedural_results)
    }

    async fn store_procedural_skill(&self, trigger: String, procedure: String) -> Result<(), String> {
        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let entry_id = format!("proc_skill_{}", timestamp);
        let content = format!("[Trigger] {}\n[Procedure] {}", trigger, procedure);

        let mut entry = MemoryEntry::new(
            entry_id,
            MemoryType::Procedural,
            content,
            timestamp,
            None,
        );
        // High initial strength — procedural skills are valuable long-term knowledge
        entry.base_strength = 5.0;
        entry.current_strength = 5.0;

        self.store(entry).await
    }

    async fn store_workflow_template(&self, description: String, template_json: String) -> Result<(), String> {
        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let entry_id = format!("workflow_template_{}", timestamp);
        let content = format!("[Description] {}\n[Version] 1\n[TemplateJSON]\n{}", description, template_json);

        let mut entry = MemoryEntry::new(
            entry_id,
            MemoryType::Procedural, // Workflow templates are procedural
            content,
            timestamp,
            None,
        );
        entry.base_strength = 5.0;
        entry.current_strength = 5.0;

        self.store(entry).await
    }

    async fn upgrade_workflow_template(&self, description: String, template_json: String) -> Result<bool, String> {
        // Search for existing similar templates via vector similarity
        let query = MemoryQuery::SemanticSearch { query: description.clone(), top_k: 5 };
        let results = self.retrieve(query).await.unwrap_or_default();
        
        // Find the best matching existing workflow template
        let existing_template = results.into_iter()
            .find(|e| e.memory_type == MemoryType::Procedural && e.content.contains("[TemplateJSON]"));
        
        if let Some(existing) = existing_template {
            // Extract current version number
            let current_version: u32 = existing.content
                .lines()
                .find(|l| l.starts_with("[Version] "))
                .and_then(|l| l.trim_start_matches("[Version] ").parse().ok())
                .unwrap_or(1);
            
            let new_version = current_version + 1;
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
            
            // Build upgraded content with new version
            let upgraded_content = format!(
                "[Description] {}\n[Version] {}\n[TemplateJSON]\n{}",
                description, new_version, template_json
            );
            
            // Create a replacement entry with the same ID (overwrites in redb)
            let mut upgraded_entry = MemoryEntry::new(
                existing.id.clone(),
                MemoryType::Procedural,
                upgraded_content,
                timestamp,
                None, // Embedding will be auto-generated
            );
            // Boost strength on successful upgrade (successful reuse = proven value)
            upgraded_entry.base_strength = (existing.base_strength + 0.5).min(10.0);
            upgraded_entry.current_strength = upgraded_entry.base_strength;
            
            self.store(upgraded_entry).await?;
            tracing::info!(
                "[MemoryOS] 🔄 Upgraded workflow template '{}' to version {} (strength: {:.1})",
                &description[..description.len().min(60)], new_version, existing.base_strength + 0.5
            );
            Ok(true) // Upgraded existing
        } else {
            // No similar template found — store as new
            self.store_workflow_template(description, template_json).await?;
            Ok(false) // Stored as new
        }
    }
}
