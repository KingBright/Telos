use crate::types::{MemoryEntry, MemoryType};
use std::time::{SystemTime, UNIX_EPOCH};

/// Applies Ebbinghaus forgetting curve to a given memory entry.
/// Returns a boolean indicating if the memory should be pruned (forgotten)
/// and updates the strength of the memory in-place.
///
/// Decay rates:
/// - Episodic memories: 24h half-life (fast decay, temporary experiences)
/// - InteractionEvent: 48h half-life (slower decay, conversation records persist longer)
/// - Semantic/Procedural/UserProfile: never decay (long-term knowledge)
pub fn apply_decay(entry: &mut MemoryEntry, current_ts: u64, min_strength: f32) -> bool {
    // Determine decay rate based on memory type
    let half_life_hours: f32 = match entry.memory_type {
        MemoryType::Episodic => 24.0,           // Fast decay
        MemoryType::InteractionEvent => 48.0,    // Slower decay for conversation records
        // Semantic, Procedural, and UserProfile never decay
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

// In a real implementation, a background worker would periodically fetch
// all episodic memories, run `apply_decay`, and remove those that return true.
