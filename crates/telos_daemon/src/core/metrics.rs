use std::time::Instant;
use std::sync::atomic::AtomicUsize;

/// Global atomic metrics counters — updated from across the daemon.
/// Each field maps to a sub-item in tasks/16_telemetry_dashboard.md §16.3.
pub struct MetricsManager {
    // -- LLM Gateway (16.3a) --
    pub llm_total_requests: AtomicUsize,
    pub llm_http_429_errors: AtomicUsize,
    pub llm_other_api_errors: AtomicUsize,
    pub llm_cumulative_tokens: AtomicUsize,
    /// USD cost × 10_000 (integer cents-of-cents for atomic safety)
    pub llm_estimated_cost_x10000: AtomicUsize,

    // -- Task & Control Flow (16.3b) --
    pub task_total_success: AtomicUsize,
    pub task_total_failures: AtomicUsize,
    pub semantic_loop_interventions: AtomicUsize,

    // -- Agent & Evolution (16.3c) --
    pub proactive_interactions: AtomicUsize,
    pub qa_passes: AtomicUsize,
    pub qa_failures: AtomicUsize,

    // -- Dynamic Tool Sandbox (16.3d) --
    pub tool_creation_success: AtomicUsize,
    pub tool_creation_failure: AtomicUsize,
    pub tool_iteration_success: AtomicUsize,
    pub tool_iteration_failure: AtomicUsize,
    pub tool_execution_success: AtomicUsize,
    pub tool_execution_failure: AtomicUsize,

    // -- Uptime --
    pub launch_time: std::sync::OnceLock<Instant>,
}

impl Default for MetricsManager {
    fn default() -> Self {
        Self {
            llm_total_requests: AtomicUsize::new(0),
            llm_http_429_errors: AtomicUsize::new(0),
            llm_other_api_errors: AtomicUsize::new(0),
            llm_cumulative_tokens: AtomicUsize::new(0),
            llm_estimated_cost_x10000: AtomicUsize::new(0),
            task_total_success: AtomicUsize::new(0),
            task_total_failures: AtomicUsize::new(0),
            semantic_loop_interventions: AtomicUsize::new(0),
            proactive_interactions: AtomicUsize::new(0),
            qa_passes: AtomicUsize::new(0),
            qa_failures: AtomicUsize::new(0),
            tool_creation_success: AtomicUsize::new(0),
            tool_creation_failure: AtomicUsize::new(0),
            tool_iteration_success: AtomicUsize::new(0),
            tool_iteration_failure: AtomicUsize::new(0),
            tool_execution_success: AtomicUsize::new(0),
            tool_execution_failure: AtomicUsize::new(0),
            launch_time: std::sync::OnceLock::new(),
        }
    }
}

pub static METRICS: std::sync::LazyLock<MetricsManager> = std::sync::LazyLock::new(|| {
    let m = MetricsManager::default();
    m.launch_time.set(Instant::now()).unwrap();
    m
});

// ─── JSON Response Structs ───────────────────────────────────────────

#[derive(serde::Serialize)]
pub struct MetricsResponse {
    pub memory_os: MemoryMetrics,
    pub dynamic_tooling: ToolingMetrics,
    pub task_flow: TaskMetrics,
    pub agent: AgentMetrics,
    pub llm: LlmMetrics,
    pub uptime_seconds: u64,
}

/// Memory OS (16.3e): entries by type + distillation count
#[derive(serde::Serialize)]
pub struct MemoryMetrics {
    pub episodic_nodes: usize,
    pub semantic_nodes: usize,
    pub procedural_nodes: usize,
    pub distillation_count: usize,
}

/// Dynamic Tool Sandbox (16.3d): creation/iteration/execution success & failure
#[derive(serde::Serialize)]
pub struct ToolingMetrics {
    pub creation_success: usize,
    pub creation_failure: usize,
    pub iteration_success: usize,
    pub iteration_failure: usize,
    pub execution_success: usize,
    pub execution_failure: usize,
}

/// Task & Control Flow (16.3b): success/failure/active/paused/semantic loops
#[derive(serde::Serialize)]
pub struct TaskMetrics {
    pub total_success: usize,
    pub total_failures: usize,
    pub active_concurrent_tasks: usize,
    pub paused_tasks: usize,
    pub semantic_loop_interventions: usize,
}

/// Agent & Evolution (16.3c)
#[derive(serde::Serialize)]
pub struct AgentMetrics {
    pub proactive_interactions: usize,
    pub qa_passes: usize,
    pub qa_failures: usize,
}

/// LLM Gateway (16.3a): requests, errors, tokens, cost
#[derive(serde::Serialize)]
pub struct LlmMetrics {
    pub total_requests: usize,
    pub http_429_errors: usize,
    pub other_api_errors: usize,
    pub cumulative_tokens: usize,
    /// Estimated USD cost (precision: 4 decimal places, e.g. 0.0023)
    pub estimated_cost_usd: f64,
}
