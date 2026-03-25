use std::sync::Arc;
use tracing::{debug, info};

// Core Traits and Primitives
use telos_memory::engine::MemoryOS;
use telos_model_gateway::gateway::GatewayManager;
use telos_model_gateway::ModelGateway;

/// Structured fact extracted by the LLM.
#[derive(serde::Deserialize, Debug)]
pub struct ExtractedFact {
    pub content: String,
    /// "static" for permanent traits, "dynamic" for temporary context
    #[serde(default = "default_fact_type")]
    pub fact_type: String,
    /// Optional ISO 8601 timestamp after which this fact should be forgotten
    #[serde(default)]
    pub forget_after: Option<String>,
}

fn default_fact_type() -> String { "static".to_string() }

/// Response wrapper for the structured extraction prompt.
#[derive(serde::Deserialize, Debug)]
struct ExtractionResponse {
    facts: Vec<serde_json::Value>,
}

/// Parse a single fact value — supports both structured objects and legacy flat strings.
fn parse_fact(val: &serde_json::Value) -> Option<ExtractedFact> {
    // Try structured object first
    if let Ok(fact) = serde_json::from_value::<ExtractedFact>(val.clone()) {
        if !fact.content.trim().is_empty() {
            return Some(fact);
        }
    }
    // Fallback: plain string → static fact
    if let Some(s) = val.as_str() {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(ExtractedFact {
                content: trimmed.to_string(),
                fact_type: "static".to_string(),
                forget_after: None,
            });
        }
    }
    None
}

/// Parse an ISO 8601 timestamp string into a UNIX epoch (seconds).
fn parse_iso_to_epoch(iso: &str) -> Option<u64> {
    // Try common ISO 8601 formats
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(iso) {
        return Some(dt.timestamp() as u64);
    }
    // Try without timezone (assume UTC)
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%S") {
        return Some(dt.and_utc().timestamp() as u64);
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(iso, "%Y-%m-%d %H:%M:%S") {
        return Some(dt.and_utc().timestamp() as u64);
    }
    // Date only
    if let Ok(d) = chrono::NaiveDate::parse_from_str(iso, "%Y-%m-%d") {
        if let Some(dt) = d.and_hms_opt(23, 59, 59) {
            return Some(dt.and_utc().timestamp() as u64);
        }
    }
    None
}

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
- Each fact must include a "content" field with a concise statement prefixed by its category in brackets
- Classify each fact with "fact_type":
  * "static" — permanent user traits, preferences, identity, expertise (e.g., name, language, skills)
  * "dynamic" — temporary context, current projects, ongoing work, time-bound states
- If a fact is time-bound (e.g., "meeting tomorrow", "deadline on Friday"), set "forget_after" to an ISO 8601 timestamp
- DO extract persistent traits and patterns
- Do NOT extract transient requests (e.g., "user asked about weather")
- QUESTION/RECALL GUARD: Do NOT extract facts from questions or recall requests. If the user is ASKING whether the system remembers something (e.g., "你还记得我喜欢什么颜色吗？", "我之前说过什么？", "你知道我的名字吗？"), this is a QUERY not a DECLARATION. Do NOT store the content of such questions as facts. Only extract facts from DECLARATIVE statements where the user explicitly provides new information.
- If NO new user information is found, return an empty array

Output ONLY a valid JSON object:
{{"facts": [
  {{"content": "[Communication] User prefers Chinese language responses", "fact_type": "static"}},
  {{"content": "[Technical] User is working on Telos memory upgrade", "fact_type": "dynamic"}},
  {{"content": "[Environment] User has a meeting at 3pm tomorrow", "fact_type": "dynamic", "forget_after": "2026-03-25T15:00:00+08:00"}}
]}}

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
            debug!("[UserProfile] Failed to parse LLM extraction output: {}", response_text.chars().take(200).collect::<String>());
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
            .filter(|e| matches!(e.memory_type, telos_memory::MemoryType::UserProfileStatic | telos_memory::MemoryType::UserProfileDynamic))
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
    let mut static_count = 0;
    let mut dynamic_count = 0;

    for (i, fact_val) in facts.iter().enumerate() {
        let extracted = match parse_fact(fact_val) {
            Some(f) => f,
            None => continue,
        };

        // Deduplication: skip if a similar fact already exists
        let fact_lower = extracted.content.to_lowercase();
        if existing_profiles.iter().any(|existing| {
            existing.contains(&fact_lower) || fact_lower.contains(existing.as_str())
        }) {
            debug!("[UserProfile] Skipping duplicate fact: {}", extracted.content);
            continue;
        }

        // Determine memory type from fact_type
        let memory_type = if extracted.fact_type == "dynamic" {
            dynamic_count += 1;
            telos_memory::MemoryType::UserProfileDynamic
        } else {
            static_count += 1;
            telos_memory::MemoryType::UserProfileStatic
        };

        let mut entry = telos_memory::MemoryEntry::new(
            format!("profile_{}_{}", timestamp, i),
            memory_type.clone(),
            extracted.content.clone(),
            timestamp,
            None, // Embedding will be auto-generated by engine.rs store()
        );

        // Set is_static for permanent facts
        if matches!(memory_type, telos_memory::MemoryType::UserProfileStatic) {
            entry.is_static = true;
        }

        // Parse temporal forgetting
        if let Some(ref forget_str) = extracted.forget_after {
            if let Some(epoch) = parse_iso_to_epoch(forget_str) {
                entry.forget_after = Some(epoch);
                debug!("[UserProfile] Fact '{}' set to expire at {}", &extracted.content[..extracted.content.len().min(40)], forget_str);
            }
        }

        if let Err(e) = memory_os.store(entry).await {
            debug!("[UserProfile] Failed to store fact: {:?}", e);
        } else {
            stored_count += 1;
        }
    }

    if stored_count > 0 {
        info!("[UserProfile] ✅ Extracted and stored {} new user profile facts ({} static, {} dynamic)", stored_count, static_count, dynamic_count);
    }
}

