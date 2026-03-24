//! User Profile Assembly — centralizes dual-layer profile construction.
//!
//! Instead of ad-hoc filtering in every injection site (event_loop, router, spawner),
//! this module provides a single `build_user_profile()` entry point that produces a
//! structured `UserProfile` with separated static facts and dynamic context.

use crate::engine::MemoryOS;
use crate::types::{MemoryEntry, MemoryQuery, MemoryType, UserProfile};

/// How far back (in seconds) to include dynamic context entries.
/// Default: 72 hours.
const DYNAMIC_WINDOW_SECS: u64 = 72 * 3600;

/// Maximum number of dynamic context entries to include.
const MAX_DYNAMIC_ENTRIES: usize = 20;

/// Build a structured `UserProfile` from the memory store.
///
/// - **Static facts**: All `UserProfileStatic` entries that are retrievable
///   (latest, not forgotten, confidence ≥ 0.3).
/// - **Dynamic context**: Recent `UserProfileDynamic` and `InteractionEvent`
///   entries within the last 72 hours, capped at `MAX_DYNAMIC_ENTRIES`.
pub async fn build_user_profile(memory_os: &dyn MemoryOS) -> UserProfile {
    let all_entries = match memory_os.retrieve(MemoryQuery::TimeRange {
        start: 0,
        end: u64::MAX,
    }).await {
        Ok(entries) => entries,
        Err(_) => return UserProfile::default(),
    };

    let now_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let cutoff = now_ts.saturating_sub(DYNAMIC_WINDOW_SECS);

    let mut static_facts = Vec::new();
    let mut dynamic_context = Vec::new();

    for entry in &all_entries {
        // Only include retrievable entries (latest, not forgotten, confidence ≥ 0.3)
        if !entry.is_retrievable() {
            continue;
        }

        match entry.memory_type {
            MemoryType::UserProfileStatic => {
                static_facts.push(entry.content.clone());
            }
            MemoryType::UserProfileDynamic | MemoryType::InteractionEvent => {
                // Only include recent dynamic entries
                if entry.created_at >= cutoff {
                    dynamic_context.push(entry.clone());
                }
            }
            _ => {}
        }
    }

    // Sort dynamic entries by recency (newest first) and cap
    dynamic_context.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    dynamic_context.truncate(MAX_DYNAMIC_ENTRIES);

    UserProfile {
        static_facts,
        dynamic_context: dynamic_context.into_iter().map(|e| e.content).collect(),
    }
}

/// Format a `UserProfile` into a structured prompt block for LLM injection.
///
/// Produces a dual-layer format:
/// ```text
/// [USER BACKGROUND — PERSISTENT KNOWLEDGE]
/// • Senior engineer specializing in Rust
/// • Prefers CLI tools over GUIs
///
/// [CURRENT CONTEXT — RECENT ACTIVITY]
/// • Working on Telos memory system upgrade
/// • Recently debugging authentication issues
/// ```
///
/// Returns an empty string if the profile has no data.
pub fn format_profile_for_prompt(profile: &UserProfile) -> String {
    if profile.static_facts.is_empty() && profile.dynamic_context.is_empty() {
        return String::new();
    }

    let mut output = String::new();

    if !profile.static_facts.is_empty() {
        output.push_str("[USER BACKGROUND — PERSISTENT KNOWLEDGE ABOUT YOUR OWNER]\nThe following are stable facts, preferences, and personal information you have learned about the user (your 主人) through past interactions. Use this to personalize your responses:\n");
        for fact in &profile.static_facts {
            output.push_str(&format!("• {}\n", fact));
        }
        output.push('\n');
    }

    if !profile.dynamic_context.is_empty() {
        output.push_str("[CURRENT CONTEXT — RECENT ACTIVITY]\nThe following are recent contextual facts about what the user is currently working on or recently discussed:\n");
        for ctx in &profile.dynamic_context {
            output.push_str(&format!("• {}\n", ctx));
        }
        output.push('\n');
    }

    output
}

/// Convenience: build + format in one call.
pub async fn build_and_format_profile(memory_os: &dyn MemoryOS) -> String {
    let profile = build_user_profile(memory_os).await;
    format_profile_for_prompt(&profile)
}
