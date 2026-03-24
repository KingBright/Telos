//! Memory conflict detection and resolution.
//!
//! Implements the GraphRAG-inspired conflict detection described in the design document:
//! When writing a new Semantic/UserProfile fact, we search existing memories for
//! semantically similar entries. If found, an LLM arbitrator reassigns confidence
//! scores to resolve contradictions.

use crate::types::{MemoryEntry, MemoryType};
use std::sync::Arc;
use telos_model_gateway::{ModelGateway, LlmRequest, Message, Capability};

/// Result of conflict detection
#[derive(Debug)]
pub struct ConflictResult {
    /// The conflicting existing entry
    pub existing: MemoryEntry,
    /// Cosine similarity between new and existing
    pub similarity: f32,
}

/// Search existing memories for potential conflicts with a new entry.
/// Returns entries that are semantically similar (cosine > threshold) but potentially contradictory.
/// Only checks Semantic and UserProfile types.
pub fn detect_conflicts(
    new_entry: &MemoryEntry,
    existing_entries: &[MemoryEntry],
    similarity_threshold: f32,
) -> Vec<ConflictResult> {
    let new_embedding = match &new_entry.embedding {
        Some(emb) => emb,
        None => return Vec::new(), // Can't detect conflicts without embeddings
    };

    let mut conflicts = Vec::new();

    for existing in existing_entries {
        // Skip non-latest and forgotten entries (they've been superseded)
        if !existing.is_latest || existing.is_forgotten {
            continue;
        }

        // Only check knowledge-bearing types for conflicts
        let is_conflict_type = matches!(
            existing.memory_type,
            MemoryType::Semantic | MemoryType::UserProfileStatic | MemoryType::UserProfileDynamic
        );
        if !is_conflict_type {
            continue;
        }

        // Skip self-comparison
        if existing.id == new_entry.id {
            continue;
        }

        if let Some(ref existing_emb) = existing.embedding {
            let similarity = cosine_similarity(new_embedding, existing_emb);
            
            // High similarity but different content = potential conflict
            if similarity > similarity_threshold && existing.content != new_entry.content {
                conflicts.push(ConflictResult {
                    existing: existing.clone(),
                    similarity,
                });
            }
        }
    }

    // Sort by similarity descending (most relevant conflicts first)
    conflicts.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap_or(std::cmp::Ordering::Equal));
    conflicts
}

/// Use an LLM to arbitrate between conflicting memories and assign confidence scores.
/// Returns (new_confidence, old_confidence) — the confidence for the new fact and the existing fact.
pub async fn resolve_conflict_with_llm(
    new_content: &str,
    existing_content: &str,
    gateway: &dyn ModelGateway,
) -> (f32, f32) {
    let prompt = format!(
        "You are a memory conflict resolver. Two facts from the same user may contradict each other.\n\n\
        EXISTING FACT: \"{}\"\n\
        NEW FACT: \"{}\"\n\n\
        Determine how to resolve this conflict. Consider:\n\
        - The NEW fact is likely more current and should usually take precedence for things that change (preferences, phone numbers, addresses)\n\
        - The EXISTING fact may still be valid if the new fact is about a different context\n\
        - If they're about the exact same thing, the new fact likely supersedes the old one\n\n\
        Respond with ONLY a JSON object: {{\"new_confidence\": 0.0-1.0, \"old_confidence\": 0.0-1.0}}\n\
        Example: {{\"new_confidence\": 1.0, \"old_confidence\": 0.2}}",
        existing_content, new_content
    );

    let req = LlmRequest {
        session_id: "memory_conflict_resolution".to_string(),
        messages: vec![
            Message { role: "user".to_string(), content: prompt },
        ],
        required_capabilities: Capability { requires_vision: false, strong_reasoning: false },
        budget_limit: 50,
        tools: None,
    };

    match gateway.generate(req).await {
        Ok(res) => {
            // Parse JSON response
            parse_confidence_response(&res.content)
        }
        Err(_) => {
            // Default: new fact gets full confidence, old fact is slightly reduced
            (1.0, 0.7)
        }
    }
}

/// Parse the LLM's JSON response for confidence scores
fn parse_confidence_response(response: &str) -> (f32, f32) {
    // Try to find JSON in the response
    if let Some(start) = response.find('{') {
        if let Some(end) = response[start..].find('}') {
            let json_str = &response[start..start + end + 1];
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
                let new_conf = parsed.get("new_confidence")
                    .and_then(|v| v.as_f64())
                    .map(|v| v as f32)
                    .unwrap_or(1.0)
                    .clamp(0.0, 1.0);
                let old_conf = parsed.get("old_confidence")
                    .and_then(|v| v.as_f64())
                    .map(|v| v as f32)
                    .unwrap_or(0.7)
                    .clamp(0.0, 1.0);
                return (new_conf, old_conf);
            }
        }
    }
    // Default fallback
    (1.0, 0.7)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_confidence_response() {
        let response = r#"{"new_confidence": 0.95, "old_confidence": 0.3}"#;
        let (new_c, old_c) = parse_confidence_response(response);
        assert!((new_c - 0.95).abs() < 0.01);
        assert!((old_c - 0.3).abs() < 0.01);
    }

    #[test]
    fn test_parse_confidence_with_extra_text() {
        let response = r#"Based on analysis, {"new_confidence": 1.0, "old_confidence": 0.2} is the right split."#;
        let (new_c, old_c) = parse_confidence_response(response);
        assert!((new_c - 1.0).abs() < 0.01);
        assert!((old_c - 0.2).abs() < 0.01);
    }

    #[test]
    fn test_parse_confidence_fallback() {
        let response = "I don't know what to say";
        let (new_c, old_c) = parse_confidence_response(response);
        assert!((new_c - 1.0).abs() < 0.01);
        assert!((old_c - 0.7).abs() < 0.01);
    }

    #[test]
    fn test_detect_conflicts_basic() {
        let new_entry = MemoryEntry::new(
            "new_1".to_string(),
            MemoryType::UserProfileStatic,
            "User likes blue".to_string(),
            100,
            Some(vec![1.0, 0.0, 0.0]),
        );

        let existing = vec![
            MemoryEntry::new(
                "old_1".to_string(),
                MemoryType::UserProfileStatic,
                "User likes red".to_string(),
                50,
                Some(vec![0.95, 0.05, 0.0]), // very similar vector
            ),
            MemoryEntry::new(
                "old_2".to_string(),
                MemoryType::Semantic,
                "Rust is fast".to_string(),
                50,
                Some(vec![0.0, 1.0, 0.0]), // different vector
            ),
        ];

        let conflicts = detect_conflicts(&new_entry, &existing, 0.8);
        // old_1 has similar vector but is UserProfileStatic (same type as new_entry) -> conflict
        // old_2 is Semantic (different content space), but still knowledge-bearing -> checked but low similarity
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].existing.id, "old_1");
    }
}
