use crate::types::{MemoryEntry, MemoryType};
use std::time::{SystemTime, UNIX_EPOCH};

/// Applies Ebbinghaus forgetting curve to a given memory entry.
/// Returns a boolean indicating if the memory should be pruned (forgotten)
/// and updates the strength of the memory in-place.
///
/// Decay rates:
/// - Episodic memories: 24h half-life (fast decay, temporary experiences)
/// - InteractionEvent: 48h half-life (slower decay, conversation records persist longer)
/// - UserProfileDynamic: 72h half-life (recent context, decays slower than events)
/// - Semantic/Procedural/UserProfileStatic: never decay (long-term knowledge)
/// - Any memory with is_static=true: never decays
///
/// Temporal forgetting:
/// - If forget_after is set and current_ts exceeds it, memory is marked as forgotten
pub fn apply_decay(entry: &mut MemoryEntry, current_ts: u64, min_strength: f32) -> bool {
    // === Temporal Forgetting: precise expiration ===
    if let Some(forget_at) = entry.forget_after {
        if current_ts > forget_at {
            entry.is_forgotten = true;
            entry.forget_reason = Some("temporal_expiry".to_string());
            return true;
        }
    }

    // Already explicitly forgotten
    if entry.is_forgotten {
        return true;
    }

    // Static memories never decay regardless of type
    if entry.is_static {
        return false;
    }

    // Determine decay rate based on memory type
    let half_life_hours: f32 = match entry.memory_type {
        MemoryType::Episodic => 24.0,              // Fast decay
        MemoryType::InteractionEvent => 48.0,       // Slower decay for conversation records
        MemoryType::UserProfileDynamic => 72.0,     // Recent context decays slowly
        // Semantic, Procedural, and UserProfileStatic never decay
        _ => return false,
    };

    if current_ts <= entry.last_accessed {
        return false;
    }

    let elapsed_seconds = current_ts - entry.last_accessed;
    let elapsed_hours = (elapsed_seconds as f32) / 3600.0;

    // Ebbinghaus inspired formula: R = e^(-t/S)
    // half_life_hours controls how quickly memories fade
    let decay_factor = (-elapsed_hours / half_life_hours).exp();

    // The current strength drops based on the time since last access and the base strength established.
    entry.current_strength = entry.base_strength * decay_factor;

    // We only forget memories that drop below minimum strength
    entry.current_strength < min_strength
}

pub fn get_current_timestamp() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

