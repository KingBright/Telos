//! Persistent time-series metrics storage using redb.
//!
//! Each metric event is stored with a timestamp key for time-range queries.
//! This enables the dashboard to show historical trends by day/week/month/year
//! and survive daemon restarts without losing data.

use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tracing::{debug, error, warn};

/// Global metrics store — set once at startup, accessible from any module
pub static METRICS_STORE: std::sync::OnceLock<Arc<MetricsStore>> = std::sync::OnceLock::new();

/// Helper to record a metric event (no-op if store not initialized)
pub fn record(event: MetricEvent) {
    if let Some(store) = METRICS_STORE.get() {
        store.record_event(&event);
    }
}

/// redb table: key = "{timestamp_ms}:{seq}" (string for lexicographic ordering), value = JSON
const METRICS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("metric_events");

/// All metric event types that get persisted
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MetricEvent {
    LlmCall {
        timestamp_ms: u64,
        agent_name: String,
        task_id: String,
        model: String,
        tokens: usize,
        estimated_cost: f64,
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
}

impl MetricEvent {
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
        }
    }
}

/// Aggregated metrics for a time range, returned by the API
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AggregatedMetrics {
    pub from_ms: u64,
    pub to_ms: u64,
    pub llm_total_requests: usize,
    pub llm_total_tokens: usize,
    pub llm_total_cost: f64,
    pub llm_429_errors: usize,
    pub llm_other_errors: usize,
    pub tool_exec_success: usize,
    pub tool_exec_failure: usize,
    pub tool_creation_success: usize,
    pub tool_creation_failure: usize,
    pub tool_iteration_success: usize,
    pub tool_iteration_failure: usize,
    pub task_success: usize,
    pub task_failure: usize,
    pub qa_passes: usize,
    pub qa_failures: usize,
    pub semantic_loops: usize,
    pub proactive_hci: usize,
    pub workflow_stored: usize,
    pub workflow_reused: usize,
    pub workflow_reuse_success: usize,
}

/// Per-agent metrics aggregation
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentTokenMetrics {
    pub agent_name: String,
    pub total_tokens: usize,
    pub total_cost: f64,
    pub total_calls: usize,
}

/// Per-tool usage metrics aggregation
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolUsageMetrics {
    pub tool_name: String,
    pub total_calls: usize,
    pub success_count: usize,
    pub failure_count: usize,
}

/// Per-workflow metrics aggregation
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkflowMetrics {
    pub workflow_id: String,
    pub description: String,
    pub stored_at_ms: u64,
    pub reuse_count: usize,
    pub reuse_success: usize,
    pub reuse_failure: usize,
    pub version: usize,
    pub is_variant: bool,
    pub last_failure_note: Option<String>,
}

/// Per-task metrics aggregation
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskMetricsDetail {
    pub task_id: String,
    pub tokens: usize,
    pub cost: f64,
    pub llm_calls: usize,
    pub tool_calls: usize,
    pub fulfilled: Option<bool>,
    pub total_time_ms: Option<u64>,
    pub timestamp_ms: u64,
}

/// Time bucket for historical charts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeBucket {
    pub bucket_start_ms: u64,
    pub bucket_end_ms: u64,
    pub metrics: AggregatedMetrics,
}

pub struct MetricsStore {
    db: Database,
    seq: std::sync::atomic::AtomicU32,
}

impl MetricsStore {
    pub fn new(path: &str) -> Result<Self, String> {
        let p = Path::new(path);
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let db = Database::create(path).map_err(|e| format!("Failed to create metrics DB: {}", e))?;
        
        // Ensure table exists
        let txn = db.begin_write().map_err(|e| format!("Failed to begin write: {}", e))?;
        { let _ = txn.open_table(METRICS_TABLE); }
        txn.commit().map_err(|e| format!("Failed to commit: {}", e))?;
        
        debug!("[MetricsStore] Initialized at {}", path);
        Ok(Self {
            db,
            seq: std::sync::atomic::AtomicU32::new(0),
        })
    }

    /// Record a metric event to persistent storage
    pub fn record_event(&self, event: &MetricEvent) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let key = format!("{}:{:06}", event.timestamp_ms(), seq);
        let json = match serde_json::to_string(event) {
            Ok(j) => j,
            Err(e) => { error!("[MetricsStore] Failed to serialize event: {}", e); return; }
        };
        
        match self.db.begin_write() {
            Ok(txn) => {
                match txn.open_table(METRICS_TABLE) {
                    Ok(mut table) => {
                        if let Err(e) = table.insert(key.as_str(), json.as_str()) {
                            error!("[MetricsStore] Failed to insert event: {}", e);
                        }
                    }
                    Err(e) => error!("[MetricsStore] Failed to open table: {}", e),
                }
                if let Err(e) = txn.commit() {
                    error!("[MetricsStore] Failed to commit: {}", e);
                }
            }
            Err(e) => error!("[MetricsStore] Failed to begin write: {}", e),
        }
    }

    /// Query events in a time range
    pub fn query_events(&self, from_ms: u64, to_ms: u64) -> Vec<MetricEvent> {
        let mut events = Vec::new();
        let from_key = format!("{}:", from_ms);
        let to_key = format!("{}:", to_ms + 1);
        
        let txn = match self.db.begin_read() {
            Ok(t) => t,
            Err(e) => { warn!("[MetricsStore] Read failed: {}", e); return events; }
        };
        let table = match txn.open_table(METRICS_TABLE) {
            Ok(t) => t,
            Err(e) => { warn!("[MetricsStore] Table open failed: {}", e); return events; }
        };
        
        let range = table.range(from_key.as_str()..to_key.as_str());
        match range {
            Ok(iter) => {
                for entry in iter {
                    if let Ok(kv) = entry {
                        let json_str = kv.1.value();
                        if let Ok(event) = serde_json::from_str::<MetricEvent>(json_str) {
                            events.push(event);
                        }
                    }
                }
            }
            Err(e) => warn!("[MetricsStore] Range query failed: {}", e),
        }
        events
    }

    /// Aggregate metrics for a time range
    pub fn aggregate(&self, from_ms: u64, to_ms: u64) -> AggregatedMetrics {
        let events = self.query_events(from_ms, to_ms);
        let mut agg = AggregatedMetrics { from_ms, to_ms, ..Default::default() };
        
        for event in &events {
            match event {
                MetricEvent::LlmCall { tokens, estimated_cost, .. } => {
                    agg.llm_total_requests += 1;
                    agg.llm_total_tokens += tokens;
                    agg.llm_total_cost += estimated_cost;
                }
                MetricEvent::LlmError { error_type, .. } => {
                    if error_type == "429" { agg.llm_429_errors += 1; }
                    else { agg.llm_other_errors += 1; }
                }
                MetricEvent::ToolExec { success, .. } => {
                    if *success { agg.tool_exec_success += 1; } else { agg.tool_exec_failure += 1; }
                }
                MetricEvent::ToolCreation { success, is_iteration, .. } => {
                    if *is_iteration {
                        if *success { agg.tool_iteration_success += 1; } else { agg.tool_iteration_failure += 1; }
                    } else {
                        if *success { agg.tool_creation_success += 1; } else { agg.tool_creation_failure += 1; }
                    }
                }
                MetricEvent::TaskResult { fulfilled, .. } => {
                    if *fulfilled { agg.task_success += 1; } else { agg.task_failure += 1; }
                }
                MetricEvent::QaResult { passed, .. } => {
                    if *passed { agg.qa_passes += 1; } else { agg.qa_failures += 1; }
                }
                MetricEvent::SemanticLoop { .. } => { agg.semantic_loops += 1; }
                MetricEvent::ProactiveHCI { .. } => { agg.proactive_hci += 1; }
                MetricEvent::WorkflowStore { .. } => { agg.workflow_stored += 1; }
                MetricEvent::WorkflowReuse { success, .. } => {
                    agg.workflow_reused += 1;
                    if *success { agg.workflow_reuse_success += 1; }
                }
            }
        }
        agg
    }

    /// Aggregate metrics grouped into time buckets for charts (max 200 buckets)
    pub fn aggregate_buckets(&self, from_ms: u64, to_ms: u64, bucket_size_ms: u64) -> Vec<TimeBucket> {
        let events = self.query_events(from_ms, to_ms);
        let mut buckets: std::collections::BTreeMap<u64, AggregatedMetrics> = std::collections::BTreeMap::new();
        
        // Cap bucket count at 200 to prevent massive responses
        const MAX_BUCKETS: u64 = 200;
        let effective_from = if bucket_size_ms > 0 && (to_ms - from_ms) / bucket_size_ms > MAX_BUCKETS {
            to_ms - MAX_BUCKETS * bucket_size_ms
        } else {
            from_ms
        };
        
        // Initialize buckets
        let mut t = effective_from;
        while t < to_ms {
            buckets.insert(t, AggregatedMetrics { from_ms: t, to_ms: t + bucket_size_ms, ..Default::default() });
            t += bucket_size_ms;
        }
        
        // Distribute events into buckets
        for event in &events {
            let ts = event.timestamp_ms();
            if ts < effective_from { continue; }
            let bucket_start = effective_from + ((ts - effective_from) / bucket_size_ms) * bucket_size_ms;
            if let Some(agg) = buckets.get_mut(&bucket_start) {
                match event {
                    MetricEvent::LlmCall { tokens, estimated_cost, .. } => {
                        agg.llm_total_requests += 1;
                        agg.llm_total_tokens += tokens;
                        agg.llm_total_cost += estimated_cost;
                    }
                    MetricEvent::LlmError { error_type, .. } => {
                        if error_type == "429" { agg.llm_429_errors += 1; } else { agg.llm_other_errors += 1; }
                    }
                    MetricEvent::ToolExec { success, .. } => {
                        if *success { agg.tool_exec_success += 1; } else { agg.tool_exec_failure += 1; }
                    }
                    MetricEvent::ToolCreation { success, is_iteration, .. } => {
                        if *is_iteration {
                            if *success { agg.tool_iteration_success += 1; } else { agg.tool_iteration_failure += 1; }
                        } else {
                            if *success { agg.tool_creation_success += 1; } else { agg.tool_creation_failure += 1; }
                        }
                    }
                    MetricEvent::TaskResult { fulfilled, .. } => {
                        if *fulfilled { agg.task_success += 1; } else { agg.task_failure += 1; }
                    }
                    MetricEvent::QaResult { passed, .. } => {
                        if *passed { agg.qa_passes += 1; } else { agg.qa_failures += 1; }
                    }
                    MetricEvent::SemanticLoop { .. } => { agg.semantic_loops += 1; }
                    MetricEvent::ProactiveHCI { .. } => { agg.proactive_hci += 1; }
                    MetricEvent::WorkflowStore { .. } => { agg.workflow_stored += 1; }
                    MetricEvent::WorkflowReuse { success, .. } => {
                        agg.workflow_reused += 1;
                        if *success { agg.workflow_reuse_success += 1; }
                    }
                }
            }
        }
        
        buckets.into_values().map(|agg| TimeBucket {
            bucket_start_ms: agg.from_ms,
            bucket_end_ms: agg.to_ms,
            metrics: agg,
        }).collect()
    }

    /// Get per-agent token breakdown
    pub fn by_agent(&self, from_ms: u64, to_ms: u64) -> Vec<AgentTokenMetrics> {
        let events = self.query_events(from_ms, to_ms);
        let mut map: std::collections::HashMap<String, AgentTokenMetrics> = std::collections::HashMap::new();
        
        for event in &events {
            if let MetricEvent::LlmCall { agent_name, tokens, estimated_cost, .. } = event {
                let entry = map.entry(agent_name.clone()).or_insert_with(|| AgentTokenMetrics {
                    agent_name: agent_name.clone(),
                    ..Default::default()
                });
                entry.total_tokens += tokens;
                entry.total_cost += estimated_cost;
                entry.total_calls += 1;
            }
        }
        
        let mut result: Vec<_> = map.into_values().collect();
        result.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
        result
    }

    /// Get per-tool usage breakdown
    pub fn by_tool(&self, from_ms: u64, to_ms: u64) -> Vec<ToolUsageMetrics> {
        let events = self.query_events(from_ms, to_ms);
        let mut map: std::collections::HashMap<String, ToolUsageMetrics> = std::collections::HashMap::new();
        
        for event in &events {
            if let MetricEvent::ToolExec { tool_name, success, .. } = event {
                let entry = map.entry(tool_name.clone()).or_insert_with(|| ToolUsageMetrics {
                    tool_name: tool_name.clone(),
                    ..Default::default()
                });
                entry.total_calls += 1;
                if *success { entry.success_count += 1; } else { entry.failure_count += 1; }
            }
        }
        
        let mut result: Vec<_> = map.into_values().collect();
        result.sort_by(|a, b| b.total_calls.cmp(&a.total_calls));
        result
    }

    /// Get per-workflow metrics breakdown
    pub fn by_workflow(&self, from_ms: u64, to_ms: u64) -> Vec<WorkflowMetrics> {
        let events = self.query_events(from_ms, to_ms);
        let mut map: std::collections::HashMap<String, WorkflowMetrics> = std::collections::HashMap::new();
        
        for event in &events {
            match event {
                MetricEvent::WorkflowStore { workflow_id, description, timestamp_ms, .. } => {
                    let entry = map.entry(workflow_id.clone()).or_insert_with(|| WorkflowMetrics {
                        workflow_id: workflow_id.clone(),
                        description: description.clone(),
                        stored_at_ms: *timestamp_ms,
                        ..Default::default()
                    });
                    // Each WorkflowStore event bumps the version
                    entry.version += 1;
                    if *timestamp_ms > entry.stored_at_ms {
                        entry.description = description.clone();
                        entry.stored_at_ms = *timestamp_ms;
                    }
                    // Detect variant templates from description prefix
                    if description.starts_with("[Variant]") {
                        entry.is_variant = true;
                    }
                }
                MetricEvent::WorkflowReuse { workflow_id, success, .. } => {
                    let entry = map.entry(workflow_id.clone()).or_insert_with(|| WorkflowMetrics {
                        workflow_id: workflow_id.clone(),
                        ..Default::default()
                    });
                    entry.reuse_count += 1;
                    if *success { entry.reuse_success += 1; } else { entry.reuse_failure += 1; }
                }
                _ => {}
            }
        }
        
        let mut result: Vec<_> = map.into_values().collect();
        result.sort_by(|a, b| b.reuse_count.cmp(&a.reuse_count));
        result
    }

    /// Get per-task metrics detail
    pub fn by_task(&self, from_ms: u64, to_ms: u64, limit: usize) -> Vec<TaskMetricsDetail> {
        let events = self.query_events(from_ms, to_ms);
        let mut map: std::collections::HashMap<String, TaskMetricsDetail> = std::collections::HashMap::new();
        
        for event in &events {
            match event {
                MetricEvent::LlmCall { task_id, tokens, estimated_cost, timestamp_ms, .. } => {
                    let entry = map.entry(task_id.clone()).or_insert_with(|| TaskMetricsDetail {
                        task_id: task_id.clone(),
                        timestamp_ms: *timestamp_ms,
                        ..Default::default()
                    });
                    entry.tokens += tokens;
                    entry.cost += estimated_cost;
                    entry.llm_calls += 1;
                }
                MetricEvent::ToolExec { task_id, timestamp_ms, .. } => {
                    let entry = map.entry(task_id.clone()).or_insert_with(|| TaskMetricsDetail {
                        task_id: task_id.clone(),
                        timestamp_ms: *timestamp_ms,
                        ..Default::default()
                    });
                    entry.tool_calls += 1;
                }
                MetricEvent::TaskResult { task_id, fulfilled, total_time_ms, timestamp_ms, .. } => {
                    let entry = map.entry(task_id.clone()).or_insert_with(|| TaskMetricsDetail {
                        task_id: task_id.clone(),
                        timestamp_ms: *timestamp_ms,
                        ..Default::default()
                    });
                    entry.fulfilled = Some(*fulfilled);
                    entry.total_time_ms = Some(*total_time_ms);
                }
                _ => {}
            }
        }
        
        let mut result: Vec<_> = map.into_values().collect();
        result.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms));
        result.truncate(limit);
        result
    }

    /// Restore cumulative counters from persisted data (called at startup)
    pub fn restore_counters(&self) {
        let m = &super::metrics::METRICS;
        // Query ALL events from epoch
        let all = self.aggregate(0, now_ms());
        
        m.llm_total_requests.store(all.llm_total_requests, Ordering::Relaxed);
        m.llm_cumulative_tokens.store(all.llm_total_tokens, Ordering::Relaxed);
        m.llm_estimated_cost_x10000.store((all.llm_total_cost * 10_000.0) as usize, Ordering::Relaxed);
        m.llm_http_429_errors.store(all.llm_429_errors, Ordering::Relaxed);
        m.llm_other_api_errors.store(all.llm_other_errors, Ordering::Relaxed);
        m.tool_execution_success.store(all.tool_exec_success, Ordering::Relaxed);
        m.tool_execution_failure.store(all.tool_exec_failure, Ordering::Relaxed);
        m.tool_creation_success.store(all.tool_creation_success, Ordering::Relaxed);
        m.tool_creation_failure.store(all.tool_creation_failure, Ordering::Relaxed);
        m.tool_iteration_success.store(all.tool_iteration_success, Ordering::Relaxed);
        m.tool_iteration_failure.store(all.tool_iteration_failure, Ordering::Relaxed);
        m.task_total_success.store(all.task_success, Ordering::Relaxed);
        m.task_total_failures.store(all.task_failure, Ordering::Relaxed);
        m.qa_passes.store(all.qa_passes, Ordering::Relaxed);
        m.qa_failures.store(all.qa_failures, Ordering::Relaxed);
        m.semantic_loop_interventions.store(all.semantic_loops, Ordering::Relaxed);
        m.proactive_interactions.store(all.proactive_hci, Ordering::Relaxed);
        
        debug!("[MetricsStore] Restored counters: {} LLM calls, {} tokens, {} tasks", 
            all.llm_total_requests, all.llm_total_tokens, all.task_success + all.task_failure);
    }
}

/// Current time in milliseconds
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
