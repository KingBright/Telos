use crate::types::{MemoryEntry, MemoryType};

/// Memory reconsolidation mimics the brain's process of turning short-term
/// episodic experiences into long-term semantic facts or procedural skills.
/// In this system, we take highly accessed episodic memories and
/// optionally merge them or promote their type.
use std::sync::Arc;
use telos_model_gateway::{ModelGateway, LlmRequest, Message, Capability};

pub async fn consolidate_memories(
    memories: &mut [MemoryEntry],
    threshold_strength: f32,
    gateway: Option<Arc<dyn ModelGateway>>,
) -> Vec<MemoryEntry> {
    let mut newly_consolidated = Vec::new();

    for entry in memories.iter_mut() {
        // We use base_strength, since current_strength fluctuates based on recent access.
        if entry.memory_type == MemoryType::Episodic && entry.base_strength >= threshold_strength {
            // Memory is strong enough to become Semantic
            // In a real system, an LLM might summarize or extract facts here.

            let mut content = entry.content.clone();

            if let Some(gw) = &gateway {
                let req = LlmRequest {
                    session_id: "memory_reconsolidation".to_string(),
                    messages: vec![
                        Message {
                            role: "system".to_string(),
                            content: "You are a memory consolidation worker. Extract the core semantic fact from this episodic memory. Be extremely concise. Reply with the fact only.".to_string(),
                        },
                        Message {
                            role: "user".to_string(),
                            content: content.clone(),
                        }
                    ],
                    required_capabilities: Capability { requires_vision: false, strong_reasoning: false },
                    budget_limit: 100,
                    tools: None,
                };

                if let Ok(res) = gw.generate(req).await {
                    content = res.content;
                }
            }

            // We clone it to add to the semantic graph
            let semantic_entry = MemoryEntry {
                id: format!("sem_{}", entry.id),
                memory_type: MemoryType::Semantic,
                content, // Promoted content
                base_strength: 5.0, // Fixed high strength
                current_strength: 5.0,
                created_at: entry.created_at,
                last_accessed: entry.last_accessed,
                embedding: entry.embedding.clone(),
                access_count: 0,
                confidence: 1.0, // Freshly consolidated = full confidence
                similarity_score: None,
            };

            newly_consolidated.push(semantic_entry);
            // We intentionally do NOT change the original episodic memory type to Semantic.
            // It remains episodic and will naturally decay over time.
        }
    }

    newly_consolidated
}
