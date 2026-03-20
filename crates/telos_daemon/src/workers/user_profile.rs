use std::sync::Arc;
use tracing::{debug, info};

// Telemetry Metrics

// Core Traits and Primitives
use telos_memory::engine::MemoryOS;
use telos_model_gateway::gateway::{GatewayManager, ModelProvider};
use telos_model_gateway::ModelGateway;

// 1. Adapter to convert Context OpenAiProvider to Gateway ModelProvider for the Gateway Manager

    pub async fn extract_and_store_user_profile(
    conversation: &str,
    gateway: Arc<GatewayManager>,
    memory_os: std::sync::Arc<telos_memory::RedbGraphStore>,
) {
    use telos_model_gateway::{Capability, LlmRequest, Message, ModelGateway};

    let extraction_prompt = format!(
        r#"Analyze the following conversation between a user and an AI assistant.
Extract ANY new information about the user across the following dimensions:

1. **Personal Identity**: name, location, timezone, native language
2. **Communication Style**: preferred response language, formality level, verbosity preference (concise vs detailed), emoji usage preference
3. **Technical Preferences**: preferred programming languages, frameworks, tools, coding style (e.g. "prefers functional style"), editor/IDE
4. **Workflow Patterns**: typical task types they request, how they iterate (quick feedback vs batch review), preferred review depth
5. **Emotional Signals**: frustration patterns, excitement triggers, patience level, trust signals toward the AI
6. **Domain Expertise**: areas of deep knowledge, areas where they need more guidance, learning goals
7. **Time & Environment**: working hours patterns, project names, team context, deployment targets
8. **Relationship Dynamics**: how they give feedback, correction style, delegation patterns

RULES:
- Extract ONLY facts about the USER (not the assistant)
- Each fact should be a single, concise statement prefixed with its category in brackets
  Example: "[Communication] User prefers Chinese language responses"
  Example: "[Technical] User's project Telos is a Rust-based autonomous agent system"
  Example: "[Workflow] User prefers to review plans before implementation"
  Example: "[Emotional] User shows frustration when AI makes assumptions"
- DO extract persistent traits and patterns
- Do NOT extract transient requests (e.g., "user asked about weather")
- If NO new user information is found, return an empty array

Output ONLY a valid JSON object:
{{"facts": ["[Category] fact1", "[Category] fact2", ...]}}

Conversation:
{}
"#,
        conversation
    );

    let request = LlmRequest {
        session_id: "profile_extraction".to_string(),
        messages: vec![
            Message { role: "system".into(), content: "You are a precise information extraction system. Output only valid JSON.".into() },
            Message { role: "user".into(), content: extraction_prompt },
        ],
        required_capabilities: Capability { requires_vision: false, strong_reasoning: false },
        budget_limit: 1000,
        tools: None,
    };

    let llm_result = gateway.generate(request).await;
    let response_text = match llm_result {
        Ok(r) => r.content,
        Err(e) => {
            debug!("[UserProfile] LLM extraction failed: {:?}", e);
            return;
        }
    };

    // Parse JSON response
    let cleaned = response_text.trim().trim_start_matches("```json").trim_end_matches("```").trim();
    let parsed: serde_json::Value = match serde_json::from_str(cleaned) {
        Ok(v) => v,
        Err(_) => {
            debug!("[UserProfile] Failed to parse LLM extraction output: {}", &response_text[..response_text.len().min(200)]);
            return;
        }
    };

    let facts = match parsed.get("facts").and_then(|f| f.as_array()) {
        Some(arr) => arr,
        None => {
            debug!("[UserProfile] No 'facts' array in extraction output");
            return;
        }
    };

    if facts.is_empty() {
        debug!("[UserProfile] No new user facts extracted from conversation");
        return;
    }

    // Load existing UserProfile entries for deduplication
    let existing_profiles: Vec<String> = if let Ok(results) = memory_os.retrieve(
        telos_memory::MemoryQuery::TimeRange { start: 0, end: u64::MAX }
    ).await {
        results.iter()
            .filter(|e| e.memory_type == telos_memory::MemoryType::UserProfile)
            .map(|e| e.content.to_lowercase())
            .collect()
    } else {
        Vec::new()
    };

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    let mut stored_count = 0;
    for (i, fact_val) in facts.iter().enumerate() {
        if let Some(fact) = fact_val.as_str() {
            let fact_trimmed = fact.trim();
            if fact_trimmed.is_empty() {
                continue;
            }
            // Deduplication: skip if a similar fact already exists
            let fact_lower = fact_trimmed.to_lowercase();
            if existing_profiles.iter().any(|existing| {
                existing.contains(&fact_lower) || fact_lower.contains(existing.as_str())
            }) {
                debug!("[UserProfile] Skipping duplicate fact: {}", fact_trimmed);
                continue;
            }

            let entry = telos_memory::MemoryEntry::new(
                format!("profile_{}_{}", timestamp, i),
                telos_memory::MemoryType::UserProfile,
                fact_trimmed.to_string(),
                timestamp,
                None, // Embedding will be auto-generated by engine.rs store()
            );

            if let Err(e) = memory_os.store(entry).await {
                debug!("[UserProfile] Failed to store fact: {:?}", e);
            } else {
                stored_count += 1;
            }
        }
    }

    if stored_count > 0 {
        info!("[UserProfile] ✅ Extracted and stored {} new user profile facts", stored_count);
    }
}

