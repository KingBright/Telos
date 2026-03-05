use crate::types::{MemoryEntry, MemoryType};
use std::time::{SystemTime, UNIX_EPOCH};

/// Applies Ebbinghaus forgetting curve to a given memory entry.
/// Returns a boolean indicating if the memory should be pruned (forgotten)
/// and updates the strength of the memory in-place.
pub fn apply_decay(entry: &mut MemoryEntry, current_ts: u64, min_strength: f32) -> bool {
    // Only episodic memories decay. Semantic and procedural are long-term.
    if entry.memory_type != MemoryType::Episodic {
        return false;
    }

    if current_ts <= entry.last_accessed {
        return false;
    }

    let elapsed_seconds = current_ts - entry.last_accessed;
    let elapsed_hours = (elapsed_seconds as f32) / 3600.0;

    // Ebbinghaus inspired formula: R = e^(-t/S)
    // Here we reduce strength logarithmically over time.
    // If it hasn't been accessed in hours, strength decreases.

    // Decay factor logic:
    // Ebbinghaus forgetting curve roughly follows a logarithmic decay.
    let decay_factor = (-elapsed_hours / 24.0).exp(); // Slow decay over days

    // The current strength drops based on the time since last access and the base strength established.
    entry.current_strength = entry.base_strength * decay_factor;

    // We only forget episodic memories that drop below minimum strength
    entry.current_strength < min_strength
}

pub fn get_current_timestamp() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

// In a real implementation, a background worker would periodically fetch
// all episodic memories, run `apply_decay`, and remove those that return true.
