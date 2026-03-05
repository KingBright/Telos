use crate::types::{MemoryEntry, MemoryType};

/// Memory reconsolidation mimics the brain's process of turning short-term
/// episodic experiences into long-term semantic facts or procedural skills.
/// In this system, we take highly accessed episodic memories and
/// optionally merge them or promote their type.
pub fn consolidate_memories(memories: &mut Vec<MemoryEntry>, threshold_strength: f32) -> Vec<MemoryEntry> {
    let mut newly_consolidated = Vec::new();

    for entry in memories.iter_mut() {
        // We use base_strength, since current_strength fluctuates based on recent access.
        if entry.memory_type == MemoryType::Episodic && entry.base_strength >= threshold_strength {
            // Memory is strong enough to become Semantic
            // In a real system, an LLM might summarize or extract facts here.

            // We clone it to add to the semantic graph
            let semantic_entry = MemoryEntry {
                id: format!("sem_{}", entry.id),
                memory_type: MemoryType::Semantic,
                content: entry.content.clone(), // Promoted content
                base_strength: 5.0, // Fixed high strength
                current_strength: 5.0,
                created_at: entry.created_at,
                last_accessed: entry.last_accessed,
                embedding: entry.embedding.clone(),
            };

            newly_consolidated.push(semantic_entry);
            // We intentionally do NOT change the original episodic memory type to Semantic.
            // It remains episodic and will naturally decay over time.
        }
    }

    newly_consolidated
}
