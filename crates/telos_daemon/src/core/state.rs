use std::sync::Arc;

// Telemetry Metrics

// Core Traits and Primitives
use telos_hci::TokioEventBroker;
use telos_memory::engine::RedbGraphStore;
use crate::core::metrics_store::MetricsStore;

// 1. Adapter to convert Context OpenAiProvider to Gateway ModelProvider for the Gateway Manager

pub struct SessionState {
    pub logs: std::collections::VecDeque<telos_context::LogEntry>,
    pub evicted_buffer: Vec<telos_context::LogEntry>,
    pub rolling_summary: String,
}

impl SessionState {
    pub fn new() -> Self {
        Self {
            logs: std::collections::VecDeque::with_capacity(20),
            evicted_buffer: Vec::new(),
            rolling_summary: String::new(),
        }
    }
}

// --- Helper: Summarize evicted session logs ---

#[derive(Clone)]
pub struct AppState {
    pub broker: Arc<TokioEventBroker>,
    pub recent_traces: Arc<tokio::sync::RwLock<std::collections::VecDeque<telos_hci::AgentFeedback>>>,
    pub active_tasks: telos_dag::engine::ActiveTaskRegistry,
    pub memory_os: Arc<RedbGraphStore>,
    pub metrics_store: Arc<MetricsStore>,
}
