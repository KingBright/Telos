use serde::{Serialize, Deserialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct LlmMetrics {
    pub total_requests: u64,
    pub http_429_errors: u64,
    pub other_api_errors: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub estimated_cost_usd: f64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TaskFlowMetrics {
    pub total_success: u64,
    pub total_failures: u64,
    pub active_concurrent_tasks: u64,
    pub paused_tasks: u64,
    pub semantic_loop_interventions: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AgentMetrics {
    pub qa_passes: u64,
    pub qa_failures: u64,
    pub proactive_interactions: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DynamicToolingMetrics {
    pub creation_success: u64,
    pub creation_failure: u64,
    pub iteration_success: u64,
    pub iteration_failure: u64,
    pub execution_success: u64,
    pub execution_failure: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct MemoryOsMetrics {
    pub episodic_count: u64,
    pub semantic_count: u64,
    pub procedural_count: u64,
    pub procedural_distillations: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct GlobalTelemetryMetrics {
    pub llm: LlmMetrics,
    pub task_flow: TaskFlowMetrics,
    pub agent: AgentMetrics,
    pub dynamic_tooling: DynamicToolingMetrics,
    pub memory_os: MemoryOsMetrics,
}

impl GlobalTelemetryMetrics {
    pub fn new() -> Self {
        Self::default()
    }
}
