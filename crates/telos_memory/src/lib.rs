use async_trait::async_trait;

pub struct EventTrace {
    pub events: Vec<String>,
}

pub struct GraphNode {
    pub label: String,
    pub properties: std::collections::HashMap<String, String>,
}

pub struct SkillTemplate {
    pub code_recipe: String,
}

pub enum MemoryType {
    Episodic(EventTrace),
    Semantic(GraphNode),
    Procedural(SkillTemplate),
}

pub struct TimeRange {
    pub start_ts: u64,
    pub end_ts: u64,
}

pub struct MemoryQuery {
    pub semantic_vector: Vec<f32>,
    pub time_range: TimeRange,
    pub tags: Vec<String>,
}

#[async_trait]
pub trait MemoryOS: Send + Sync {
    async fn store(&self, mem_type: MemoryType);
    async fn retrieve(&self, query: &MemoryQuery, limit: usize) -> Vec<MemoryType>;
    fn trigger_fade_consolidation(&self);
}
