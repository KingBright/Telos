//! Lightweight metrics primitives shared across all Telos crates.
//!
//! The `MetricEvent` enum and `record()` function live here (in `telos_core`)
//! so that any crate — `telos_dag`, `telos_memory`, `telos_daemon` — can emit
//! metric events without circular dependencies.
//!
//! The concrete storage backend (`MetricsStore` backed by redb) stays in
//! `telos_daemon` and is injected at startup via `install_sink()`.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

// ────────────────────────── Sink Trait ──────────────────────────

/// Trait implemented by the concrete metrics storage backend.
/// `telos_daemon::MetricsStore` implements this; other crates only see the trait.
pub trait MetricsSink: Send + Sync {
    fn record_event(&self, event: &MetricEvent);
}

/// Global sink — set once at daemon startup.
static SINK: OnceLock<Arc<dyn MetricsSink>> = OnceLock::new();

/// Install the concrete sink (called once in daemon main).
pub fn install_sink(sink: Arc<dyn MetricsSink>) {
    let _ = SINK.set(sink);
}

/// Record a metric event. No-op if no sink is installed (library/test usage).
pub fn record(event: MetricEvent) {
    if let Some(sink) = SINK.get() {
        sink.record_event(&event);
    }
}

// ────────────────────────── Timestamp Helper ──────────────────────────

/// Current time in milliseconds since UNIX epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ────────────────────────── MetricEvent ──────────────────────────

/// All metric event types that get persisted.
///
/// Each variant carries a `timestamp_ms` for time-range queries.
/// The `#[serde(tag = "type")]` attribute lets us deserialise old events
/// (pre-performance-fields) that were written without the new variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MetricEvent {
    // ── Existing events (unchanged wire format) ──

    LlmCall {
        timestamp_ms: u64,
        agent_name: String,
        task_id: String,
        model: String,
        tokens: usize,
        estimated_cost: f64,
        /// LLM round-trip latency in milliseconds (new field, defaults to 0
        /// for events serialised before this field existed).
        #[serde(default)]
        elapsed_ms: u64,
    },
    LlmError {
        timestamp_ms: u64,
        error_type: String, // "429", "network", "other"
        model: String,
    },
    ToolExec {
        timestamp_ms: u64,
        tool_name: String,
        success: bool,
        task_id: String,
        agent_name: String,
    },
    ToolCreation {
        timestamp_ms: u64,
        tool_name: String,
        success: bool,
        is_iteration: bool,
    },
    TaskResult {
        timestamp_ms: u64,
        task_id: String,
        fulfilled: bool,
        total_time_ms: u64,
    },
    QaResult {
        timestamp_ms: u64,
        task_id: String,
        passed: bool,
    },
    SemanticLoop {
        timestamp_ms: u64,
        task_id: String,
        loop_count: usize,
    },
    ProactiveHCI {
        timestamp_ms: u64,
        task_id: String,
    },
    WorkflowStore {
        timestamp_ms: u64,
        workflow_id: String,
        description: String,
    },
    WorkflowReuse {
        timestamp_ms: u64,
        workflow_id: String,
        task_id: String,
        success: bool,
    },

    // ── New performance events ──

    /// Emitted by the DAG engine when a node finishes execution.
    NodeExecution {
        timestamp_ms: u64,
        task_id: String,
        node_id: String,
        node_type: String,
        elapsed_ms: u64,
        success: bool,
    },
    /// Emitted by Memory OS on every retrieval operation.
    MemoryRetrieval {
        timestamp_ms: u64,
        query_type: String, // "semantic_entity", "procedural_semantic", "procedural_keyword"
        result_count: usize,
        elapsed_ms: u64,
    },
    /// Emitted by the router after deciding which expert to dispatch to.
    RouteDecision {
        timestamp_ms: u64,
        task_id: String,
        route: String, // "direct_reply", "general_expert", etc.
        reason: String,
    },
    /// Emitted after context compression for a node.
    ContextCompression {
        timestamp_ms: u64,
        task_id: String,
        elapsed_ms: u64,
        facts_count: usize,
        summary_count: usize,
    },
}

impl MetricEvent {
    /// Extract the timestamp from any variant.
    pub fn timestamp_ms(&self) -> u64 {
        match self {
            MetricEvent::LlmCall { timestamp_ms, .. } => *timestamp_ms,
            MetricEvent::LlmError { timestamp_ms, .. } => *timestamp_ms,
            MetricEvent::ToolExec { timestamp_ms, .. } => *timestamp_ms,
            MetricEvent::ToolCreation { timestamp_ms, .. } => *timestamp_ms,
            MetricEvent::TaskResult { timestamp_ms, .. } => *timestamp_ms,
            MetricEvent::QaResult { timestamp_ms, .. } => *timestamp_ms,
            MetricEvent::SemanticLoop { timestamp_ms, .. } => *timestamp_ms,
            MetricEvent::ProactiveHCI { timestamp_ms, .. } => *timestamp_ms,
            MetricEvent::WorkflowStore { timestamp_ms, .. } => *timestamp_ms,
            MetricEvent::WorkflowReuse { timestamp_ms, .. } => *timestamp_ms,
            MetricEvent::NodeExecution { timestamp_ms, .. } => *timestamp_ms,
            MetricEvent::MemoryRetrieval { timestamp_ms, .. } => *timestamp_ms,
            MetricEvent::RouteDecision { timestamp_ms, .. } => *timestamp_ms,
            MetricEvent::ContextCompression { timestamp_ms, .. } => *timestamp_ms,
        }
    }
}
