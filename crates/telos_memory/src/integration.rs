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
    async fn store_workflow_template(&self, description: String, template_json: String, required_tools: Vec<String>) -> Result<(), String>;

    /// Upgrades an existing workflow template if a similar one exists (cosine similarity > 0.8).
    /// If no similar template is found, stores as new. Returns true if upgraded, false if new.
    async fn upgrade_workflow_template(&self, description: String, template_json: String, required_tools: Vec<String>) -> Result<bool, String>;

    /// Attaches a failure note to the best-matching workflow template.
    /// The note describes WHY the template failed, enabling the Architect to see
    /// known limitations when the template is later retrieved.
    /// Also increments a [FailureCount] metadata field on the template.
    /// Returns the new failure count so callers can trigger species divergence.
    async fn attach_failure_note(&self, template_description: String, failure_note: String) -> Result<u32, String>;

    /// Applies a mild strength penalty (-0.3, floored at 1.0) to the best-matching
    /// workflow template. Called when an adopted template leads to a failed execution.
    async fn penalize_workflow_template(&self, template_description: String) -> Result<(), String>;
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
        let start = std::time::Instant::now();
        // Query the memory OS specifically for Semantic memories relating to the query.
        let query = MemoryQuery::EntityLookup { entity: query_string };

        let results = self.retrieve(query).await?;
        let elapsed_ms = start.elapsed().as_millis() as u64;
        telos_core::metrics::record(telos_core::metrics::MetricEvent::MemoryRetrieval {
            timestamp_ms: telos_core::metrics::now_ms(),
            query_type: "semantic_entity".to_string(),
            result_count: results.len(),
            elapsed_ms,
        });

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
        let overall_start = std::time::Instant::now();
        // Dual-strategy retrieval for maximum recall:
        // Strategy 1: SemanticSearch (embedding-based similarity) for fuzzy matching
        // Strategy 2: EntityLookup (keyword) as fallback
        let mut seen_ids = std::collections::HashSet::new();
        let mut procedural_results = Vec::new();
        
        // Similarity threshold: only return results above this cosine similarity
        // Below this threshold, templates are likely irrelevant ("programmer architect designing a house")
        const SIMILARITY_THRESHOLD: f32 = 0.65;
        
        // Strategy 1: Vector similarity search (primary)
        let sem_start = std::time::Instant::now();
        let semantic_query = MemoryQuery::SemanticSearch { query: query_string.clone(), top_k: 10 };
        if let Ok(results) = self.retrieve(semantic_query).await {
            let sem_elapsed = sem_start.elapsed().as_millis() as u64;
            let sem_count = results.len();
            for e in results {
                if e.memory_type == MemoryType::Procedural && seen_ids.insert(e.id.clone()) {
                    // Gate: only include if similarity is above threshold
                    if let Some(score) = e.similarity_score {
                        if score >= SIMILARITY_THRESHOLD {
                            tracing::debug!(
                                "[MemoryOS] Procedural template matched (score={:.3}): {}",
                                score,
                                &e.content[..e.content.len().min(80)]
                            );
                            procedural_results.push(e.content);
                        } else {
                            tracing::debug!(
                                "[MemoryOS] Filtered out low-relevance template (score={:.3} < {:.2}): {}",
                                score, SIMILARITY_THRESHOLD,
                                &e.content[..e.content.len().min(80)]
                            );
                        }
                    }
                }
            }
            telos_core::metrics::record(telos_core::metrics::MetricEvent::MemoryRetrieval {
                timestamp_ms: telos_core::metrics::now_ms(),
                query_type: "procedural_semantic".to_string(),
                result_count: sem_count,
                elapsed_ms: sem_elapsed,
            });
        }
        
        // Strategy 2: Keyword fallback (catches exact term matches that vector might miss)
        // Keyword matches don't have similarity scores — they matched on exact terms, so they're relevant
        let kw_start = std::time::Instant::now();
        let keyword_query = MemoryQuery::EntityLookup { entity: query_string };
        if let Ok(results) = self.retrieve(keyword_query).await {
            let kw_elapsed = kw_start.elapsed().as_millis() as u64;
            let kw_count = results.len();
            for e in results {
                if e.memory_type == MemoryType::Procedural && seen_ids.insert(e.id.clone()) {
                    procedural_results.push(e.content);
                }
            }
            telos_core::metrics::record(telos_core::metrics::MetricEvent::MemoryRetrieval {
                timestamp_ms: telos_core::metrics::now_ms(),
                query_type: "procedural_keyword".to_string(),
                result_count: kw_count,
                elapsed_ms: kw_elapsed,
            });
        }
        let _total_elapsed = overall_start.elapsed().as_millis() as u64;
        
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

    async fn store_workflow_template(&self, description: String, template_json: String, required_tools: Vec<String>) -> Result<(), String> {
        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let entry_id = format!("workflow_template_{}", timestamp);
        let tools_line = if required_tools.is_empty() { String::new() } else { format!("[RequiredTools] {}\n", required_tools.join(", ")) };
        let content = format!("[Description] {}\n[Version] 1\n{}[TemplateJSON]\n{}", description, tools_line, template_json);

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

    async fn upgrade_workflow_template(&self, description: String, template_json: String, required_tools: Vec<String>) -> Result<bool, String> {
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
            
            let tools_line = if required_tools.is_empty() { String::new() } else { format!("[RequiredTools] {}\n", required_tools.join(", ")) };
            // Build upgraded content with new version
            let upgraded_content = format!(
                "[Description] {}\n[Version] {}\n{}[TemplateJSON]\n{}",
                description, new_version, tools_line, template_json
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
            self.store_workflow_template(description, template_json, required_tools).await?;
            Ok(false) // Stored as new
        }
    }

    async fn attach_failure_note(&self, template_description: String, failure_note: String) -> Result<u32, String> {
        // Find the best-matching workflow template
        let query = MemoryQuery::SemanticSearch { query: template_description.clone(), top_k: 5 };
        let results = self.retrieve(query).await.unwrap_or_default();
        
        let existing_template = results.into_iter()
            .find(|e| e.memory_type == MemoryType::Procedural && e.content.contains("[TemplateJSON]"));
        
        if let Some(existing) = existing_template {
            // Increment failure count
            let current_failure_count: u32 = existing.content
                .lines()
                .find(|l| l.starts_with("[FailureCount] "))
                .and_then(|l| l.trim_start_matches("[FailureCount] ").parse().ok())
                .unwrap_or(0);
            let new_failure_count = current_failure_count + 1;
            
            // Build updated content with failure note appended
            let mut updated_content = existing.content.clone();
            
            // Update or add FailureCount
            if updated_content.contains("[FailureCount] ") {
                // Replace existing FailureCount line
                let lines: Vec<&str> = updated_content.lines().collect();
                updated_content = lines.iter()
                    .map(|l| {
                        if l.starts_with("[FailureCount] ") {
                            format!("[FailureCount] {}", new_failure_count)
                        } else {
                            l.to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
            } else {
                // Insert before [TemplateJSON]
                updated_content = updated_content.replace(
                    "[TemplateJSON]",
                    &format!("[FailureCount] {}\n[TemplateJSON]", new_failure_count)
                );
            }
            
            // Append the failure note (keep only the most recent 3 to avoid bloat)
            let existing_notes: Vec<&str> = updated_content.lines()
                .filter(|l| l.starts_with("[FailureNote] "))
                .collect();
                
            if existing_notes.len() >= 3 {
                // Remove oldest note (first one found)
                if let Some(oldest) = existing_notes.first() {
                    updated_content = updated_content.replace(&format!("{}\n", oldest), "");
                }
            }
            
            // Append new failure note before [TemplateJSON]
            updated_content = updated_content.replace(
                "[TemplateJSON]",
                &format!("[FailureNote] {}\n[TemplateJSON]", failure_note)
            );
            
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
            let mut updated_entry = MemoryEntry::new(
                existing.id.clone(),
                MemoryType::Procedural,
                updated_content,
                timestamp,
                None,
            );
            updated_entry.base_strength = existing.base_strength;
            updated_entry.current_strength = existing.current_strength;
            
            self.store(updated_entry).await?;
            tracing::info!(
                "[MemoryOS] 📝 Attached failure note to workflow template (failures: {}): {}",
                new_failure_count,
                &template_description[..template_description.len().min(60)]
            );
            Ok(new_failure_count)
        } else {
            tracing::debug!(
                "[MemoryOS] No matching template found for failure note: {}",
                &template_description[..template_description.len().min(60)]
            );
            Ok(0) // No template found — nothing to annotate
        }
    }

    async fn penalize_workflow_template(&self, template_description: String) -> Result<(), String> {
        let query = MemoryQuery::SemanticSearch { query: template_description.clone(), top_k: 5 };
        let results = self.retrieve(query).await.unwrap_or_default();
        
        let existing_template = results.into_iter()
            .find(|e| e.memory_type == MemoryType::Procedural && e.content.contains("[TemplateJSON]"));
        
        if let Some(existing) = existing_template {
            let new_strength = (existing.base_strength - 0.3).max(1.0);
            
            if (new_strength - existing.base_strength).abs() < f32::EPSILON {
                tracing::debug!("[MemoryOS] Template already at minimum strength 1.0, skipping penalty.");
                return Ok(());
            }
            
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
            let mut penalized_entry = MemoryEntry::new(
                existing.id.clone(),
                MemoryType::Procedural,
                existing.content.clone(),
                timestamp,
                None,
            );
            penalized_entry.base_strength = new_strength;
            penalized_entry.current_strength = new_strength;
            
            self.store(penalized_entry).await?;
            tracing::info!(
                "[MemoryOS] 📉 Penalized workflow template strength {:.1} → {:.1}: {}",
                existing.base_strength, new_strength,
                &template_description[..template_description.len().min(60)]
            );
            Ok(())
        } else {
            Ok(()) // No template found — nothing to penalize
        }
    }
}
