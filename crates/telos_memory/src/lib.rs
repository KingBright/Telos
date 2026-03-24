pub mod types;
pub mod engine;
pub mod decay;
pub mod reconsolidation;
pub mod conflict;
pub mod integration;
pub mod profile;

#[cfg(test)]
mod tests;

// Re-exports
pub use types::{MemoryType, MemoryQuery, MemoryEntry, MemoryRelation, UserProfile};
pub use engine::{MemoryOS, RedbGraphStore, MissionStore};
pub use integration::MemoryIntegration;
pub use profile::{build_user_profile, format_profile_for_prompt, build_and_format_profile};
