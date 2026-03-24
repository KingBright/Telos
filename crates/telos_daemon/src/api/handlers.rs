use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use futures::stream::Stream;
use tracing::error;
use uuid::Uuid;

// Telemetry Metrics
use std::sync::atomic::Ordering;

// Core Traits and Primitives
use telos_core::config::TelosConfig;
use telos_hci::{
    global_log_level, AgentEvent, AgentFeedback, EventBroker, LogLevel,
};

// 1. Adapter to convert Context OpenAiProvider to Gateway ModelProvider for the Gateway Manager

use crate::core::state::*;
use crate::api::models::*;
use crate::core::metrics::*;
    pub async fn get_traces(State(state): State<AppState>) -> Json<serde_json::Value> {
    let q = state.recent_traces.read().await;
    let traces: Vec<_> = q.iter().cloned().collect();
    Json(serde_json::json!({
        "traces": traces
    }))
}
    pub async fn get_active_tasks(State(state): State<AppState>) -> Json<serde_json::Value> {
    let w = state.active_tasks.read().await;
    let tasks: Vec<_> = w.values().cloned().collect();
    Json(serde_json::json!({
        "active_tasks": tasks
    }))
}

    pub async fn handle_metrics(
    State(state): State<AppState>,
) -> Json<MetricsResponse> {
    let m = &METRICS;
    let active_tasks = state.active_tasks.read().await.len();
    let uptime = m.launch_time.get().map(|t| t.elapsed().as_secs()).unwrap_or(0);
    
    // Query real memory node counts from RedbGraphStore
    let (mut episodic, mut semantic, mut procedural) = (0usize, 0usize, 0usize);
    if let Ok(entries) = telos_memory::engine::MemoryOS::retrieve_all(&*state.memory_os).await {
        for entry in &entries {
            match entry.memory_type {
                telos_memory::MemoryType::Episodic | telos_memory::MemoryType::InteractionEvent | telos_memory::MemoryType::UserProfileStatic | telos_memory::MemoryType::UserProfileDynamic => { episodic += 1; }
                telos_memory::MemoryType::Semantic => { semantic += 1; }
                telos_memory::MemoryType::Procedural => { procedural += 1; }
            }
        }
    }
    
    // Paused tasks count (from managed paused_tasks map is in event_loop, approximate via active − running)
    // For now, we track paused via the global counter or 0 if not yet instrumented
    let paused = 0usize; // TODO: Wire paused_tasks HashMap len
    
    Json(MetricsResponse {
        memory_os: MemoryMetrics {
            episodic_nodes: episodic,
            semantic_nodes: semantic,
            procedural_nodes: procedural,
            distillation_count: procedural, // Procedural nodes = distilled skills
        },
        dynamic_tooling: ToolingMetrics {
            creation_success: m.tool_creation_success.load(Ordering::Relaxed),
            creation_failure: m.tool_creation_failure.load(Ordering::Relaxed),
            iteration_success: m.tool_iteration_success.load(Ordering::Relaxed),
            iteration_failure: m.tool_iteration_failure.load(Ordering::Relaxed),
            execution_success: m.tool_execution_success.load(Ordering::Relaxed),
            execution_failure: m.tool_execution_failure.load(Ordering::Relaxed),
        },
        task_flow: TaskMetrics {
            total_success: m.task_total_success.load(Ordering::Relaxed),
            total_failures: m.task_total_failures.load(Ordering::Relaxed),
            active_concurrent_tasks: active_tasks,
            paused_tasks: paused,
            semantic_loop_interventions: m.semantic_loop_interventions.load(Ordering::Relaxed),
        },
        agent: AgentMetrics {
            proactive_interactions: m.proactive_interactions.load(Ordering::Relaxed),
            qa_passes: m.qa_passes.load(Ordering::Relaxed),
            qa_failures: m.qa_failures.load(Ordering::Relaxed),
        },
        llm: LlmMetrics {
            total_requests: m.llm_total_requests.load(Ordering::Relaxed),
            http_429_errors: m.llm_http_429_errors.load(Ordering::Relaxed),
            other_api_errors: m.llm_other_api_errors.load(Ordering::Relaxed),
            cumulative_tokens: m.llm_cumulative_tokens.load(Ordering::Relaxed),
            estimated_cost_usd: m.llm_estimated_cost_x10000.load(Ordering::Relaxed) as f64 / 10_000.0,
        },
        uptime_seconds: uptime,
    })
}
    pub async fn handle_run(
    State(state): State<AppState>,
    Json(req): Json<RunRequest>,
) -> Json<RunResponse> {
    let trace_id = req.trace_id.and_then(|id| Uuid::parse_str(&id).ok()).unwrap_or_else(Uuid::new_v4);
    let _ = state
        .broker
        .publish_event(AgentEvent::UserInput {
            session_id: "default".into(),
            payload: req.payload,
            trace_id,
            project_id: req.project_id,
        })
        .await;

    Json(RunResponse {
        status: "accepted".into(),
        trace_id: trace_id.to_string(),
    })
}
    pub async fn handle_run_sync(
    State(state): State<AppState>,
    Json(req): Json<RunRequest>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let trace_id = req.trace_id.clone().and_then(|id| Uuid::parse_str(&id).ok()).unwrap_or_else(Uuid::new_v4);
    let trace_id_str = trace_id.to_string();
    
    // Subscribe *before* dispatching to avoid race conditions
    let mut rx = state.broker.subscribe_feedback();
    
    let _ = state
        .broker
        .publish_event(AgentEvent::UserInput {
            session_id: "default".into(),
            payload: req.payload,
            trace_id,
            project_id: req.project_id,
        })
        .await;

    let stream = async_stream::stream! {
        // Send initial acknowledgment
        yield Ok(Event::default().event("started").data(
            serde_json::json!({"trace_id": trace_id_str}).to_string()
        ));

        // Idle timeout: if no event arrives for 300s, the task is likely stalled.
        // Complex tasks that keep producing heartbeats will never trigger this.
        let idle_timeout = tokio::time::Duration::from_secs(300);

        loop {
            match tokio::time::timeout(idle_timeout, rx.recv()).await {
                Ok(Ok(feedback)) => {
                    match feedback {
                        AgentFeedback::TaskCompleted { task_id, summary } if task_id == trace_id_str => {
                            let summary_json = serde_json::to_string(&summary).unwrap_or_default();
                            yield Ok(Event::default().event("completed").data(summary_json));
                            break;
                        }
                        AgentFeedback::Output { task_id, content, is_final, .. } if task_id == trace_id_str => {
                            if is_final {
                                yield Ok(Event::default().event("output").data(content));
                            } else {
                                yield Ok(Event::default().event("heartbeat").data(content));
                            }
                        }
                        AgentFeedback::ClarificationNeeded { task_id, prompt, options, .. } if task_id == trace_id_str => {
                            let data = serde_json::json!({
                                "prompt": prompt,
                                "options": options,
                            });
                            yield Ok(Event::default().event("clarification").data(data.to_string()));
                        }
                        AgentFeedback::ProgressUpdate { task_id, progress } if task_id == trace_id_str => {
                            let data = serde_json::json!({
                                "type": "progress",
                                "completed": progress.completed,
                                "total": progress.total,
                            });
                            yield Ok(Event::default().event("heartbeat").data(data.to_string()));
                        }
                        AgentFeedback::NodeStarted { task_id, node_id, detail } if task_id == trace_id_str => {
                            let data = serde_json::json!({
                                "type": "node_started",
                                "node_id": node_id,
                                "task_type": detail.task_type,
                            });
                            yield Ok(Event::default().event("heartbeat").data(data.to_string()));
                        }
                        AgentFeedback::StateChanged { task_id, current_node, status } if task_id == trace_id_str => {
                            let data = serde_json::json!({
                                "type": "state_changed",
                                "node": current_node,
                                "status": format!("{:?}", status),
                            });
                            yield Ok(Event::default().event("heartbeat").data(data.to_string()));
                        }
                        _ => {}
                    }
                }
                Ok(Err(_)) => {
                    // Channel closed
                    break;
                }
                Err(_) => {
                    // Idle timeout: no event for 300s — task is likely stalled
                    yield Ok(Event::default().event("error").data(
                        "Task stalled: no activity for 300s"
                    ));
                    break;
                }
            }
        }
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}
    pub async fn handle_approve(
    State(state): State<AppState>,
    Json(req): Json<ApproveRequest>,
) -> Json<ApproveResponse> {
    let trace_id = Uuid::new_v4();
    let _ = state
        .broker
        .publish_event(AgentEvent::UserApproval {
            task_id: req.task_id,
            node_id: None,
            approved: req.approved,
            supplement_info: None,
            trace_id,
        })
        .await;

    Json(ApproveResponse {
        status: "approval received".into(),
    })
}

    pub async fn handle_intervention(
    State(state): State<AppState>,
    Json(req): Json<InterventionRequest>,
) -> Json<InterventionResponse> {
    let trace_id = Uuid::new_v4();
    let _ = state
        .broker
        .publish_event(AgentEvent::UserIntervention {
            task_id: req.task_id,
            node_id: req.node_id,
            instruction: req.instruction,
            trace_id,
        })
        .await;

    Json(InterventionResponse {
        status: "intervention received".into(),
    })
}

    pub async fn handle_clarify(
    State(state): State<AppState>,
    Json(req): Json<ClarifyRequest>,
) -> Json<ClarifyResponse> {
    let trace_id = Uuid::new_v4();
    let _ = state
        .broker
        .publish_event(AgentEvent::ClarificationResponse {
            task_id: req.task_id,
            selected_option_id: req.selected_option_id,
            free_text: req.free_text,
            trace_id,
        })
        .await;

    Json(ClarifyResponse {
        status: "clarification received".into(),
    })
}

pub async fn handle_cancel(
    State(state): State<AppState>,
    axum::extract::Path(task_id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    let mut w = state.active_tasks.write().await;
    if w.remove(&task_id).is_some() {
        Json(serde_json::json!({
            "status": "success",
            "message": format!("Task {} cancellation requested", task_id)
        }))
    } else {
        Json(serde_json::json!({
            "status": "error",
            "message": "Task not found or already completed"
        }))
    }
}

    pub async fn get_log_level() -> Json<GetLogLevelResponse> {
    let level = global_log_level().get();
    Json(GetLogLevelResponse {
        level: format!("{:?}", level).to_lowercase(),
    })
}
    pub async fn set_log_level(
    State(state): State<AppState>,
    Json(req): Json<SetLogLevelRequest>,
) -> Json<SetLogLevelResponse> {
    let old_level = global_log_level().get();
    let new_level = LogLevel::from_string(&req.level);

    global_log_level().set(new_level);

    // Persist to config file
    if let Ok(mut config) = TelosConfig::load() {
        config.log_level = format!("{:?}", new_level).to_lowercase();
        if let Err(e) = config.save() {
            error!("Failed to persist log level to config: {}", e);
        }
    }

    // Publish LogLevelChanged feedback via broker
    state
        .broker
        .publish_feedback(AgentFeedback::LogLevelChanged {
            old_level,
            new_level,
        });

    Json(SetLogLevelResponse {
        status: "ok".into(),
        old_level: format!("{:?}", old_level).to_lowercase(),
        new_level: format!("{:?}", new_level).to_lowercase(),
    })
}

/// Helper: convert range string → (from_ms, to_ms)
fn range_to_bounds(range: &str) -> (u64, u64) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let from = match range {
        "hour" => now.saturating_sub(3_600_000),
        "day" => now.saturating_sub(86_400_000),
        "week" => now.saturating_sub(7 * 86_400_000),
        "month" => now.saturating_sub(30 * 86_400_000),
        "year" => now.saturating_sub(365 * 86_400_000),
        "all" => 0,
        _ => now.saturating_sub(86_400_000), // default: 1 day
    };
    (from, now)
}

/// GET /api/v1/metrics/history?range=day
/// Returns aggregated metrics + time-bucketed breakdown
pub async fn metrics_history(
    axum::extract::Query(params): axum::extract::Query<HistoryQuery>,
) -> Json<serde_json::Value> {
    let (from, to) = range_to_bounds(&params.range);
    
    if let Some(store) = crate::core::metrics_store::METRICS_STORE.get() {
        let agg = store.aggregate(from, to);
        // Auto-scale bucket size based on range
        let bucket_ms: u64 = match params.range.as_str() {
            "hour" => 5 * 60_000,          // 5-minute buckets
            "day" => 3_600_000,             // 1-hour buckets
            "week" => 24 * 3_600_000,       // 1-day buckets
            "month" => 24 * 3_600_000,      // 1-day buckets
            "year" => 7 * 24 * 3_600_000,   // 1-week buckets
            "all" => 30 * 24 * 3_600_000,   // 1-month buckets
            _ => 3_600_000,                 // default: 1-hour buckets
        };
        let buckets = store.aggregate_buckets(from, to, bucket_ms);
        let bucket_data: Vec<serde_json::Value> = buckets.iter().map(|b| {
            serde_json::json!({
                "bucket_start_ms": b.bucket_start_ms,
                "bucket_end_ms": b.bucket_end_ms,
                "llm_calls": b.metrics.llm_total_requests,
                "tokens": b.metrics.llm_total_tokens,
                "cost": b.metrics.llm_total_cost,
                "tool_success": b.metrics.tool_exec_success,
                "tool_failure": b.metrics.tool_exec_failure,
                "task_success": b.metrics.task_success,
                "task_failure": b.metrics.task_failure,
                "qa_pass": b.metrics.qa_passes,
                "qa_fail": b.metrics.qa_failures,
            })
        }).collect();

        Json(serde_json::json!({
            "range": params.range,
            "from_ms": from,
            "to_ms": to,
            "aggregate": {
                "total_llm_calls": agg.llm_total_requests,
                "total_tokens": agg.llm_total_tokens,
                "total_cost": agg.llm_total_cost,
                "total_429_errors": agg.llm_429_errors,
                "total_other_errors": agg.llm_other_errors,
                "tool_exec_success": agg.tool_exec_success,
                "tool_exec_failure": agg.tool_exec_failure,
                "task_success": agg.task_success,
                "task_failure": agg.task_failure,
                "qa_passes": agg.qa_passes,
                "qa_failures": agg.qa_failures,
            },
            "buckets": bucket_data,
        }))
    } else {
        Json(serde_json::json!({"error": "metrics store not initialized"}))
    }
}

/// GET /api/v1/metrics/by-agent?range=day
/// Returns per-agent token/cost breakdown
pub async fn metrics_by_agent(
    axum::extract::Query(params): axum::extract::Query<HistoryQuery>,
) -> Json<serde_json::Value> {
    let (from, to) = range_to_bounds(&params.range);
    
    if let Some(store) = crate::core::metrics_store::METRICS_STORE.get() {
        let agents = store.by_agent(from, to);
        let data: Vec<serde_json::Value> = agents.iter().map(|a| {
            serde_json::json!({
                "agent_name": a.agent_name,
                "total_tokens": a.total_tokens,
                "total_cost": a.total_cost,
                "call_count": a.total_calls,
            })
        }).collect();
        Json(serde_json::json!({
            "range": params.range,
            "agents": data,
        }))
    } else {
        Json(serde_json::json!({"error": "metrics store not initialized"}))
    }
}

/// GET /api/v1/metrics/by-task?range=day
/// Returns per-task detail breakdown
pub async fn metrics_by_task(
    axum::extract::Query(params): axum::extract::Query<HistoryQuery>,
) -> Json<serde_json::Value> {
    let (from, to) = range_to_bounds(&params.range);
    
    if let Some(store) = crate::core::metrics_store::METRICS_STORE.get() {
        let tasks = store.by_task(from, to, 100);
        let data: Vec<serde_json::Value> = tasks.iter().map(|t| {
            serde_json::json!({
                "task_id": t.task_id,
                "total_tokens": t.tokens,
                "total_cost": t.cost,
                "llm_calls": t.llm_calls,
                "tools_called": t.tool_calls,
                "fulfilled": t.fulfilled,
                "total_time_ms": t.total_time_ms,
                "timestamp_ms": t.timestamp_ms,
            })
        }).collect();
        Json(serde_json::json!({
            "range": params.range,
            "tasks": data,
        }))
    } else {
        Json(serde_json::json!({"error": "metrics store not initialized"}))
    }
}

/// GET /api/v1/metrics/by-tool?range=day
/// Returns per-tool usage breakdown (calls, success, failure)
pub async fn metrics_by_tool(
    axum::extract::Query(params): axum::extract::Query<HistoryQuery>,
) -> Json<serde_json::Value> {
    let (from, to) = range_to_bounds(&params.range);
    
    if let Some(store) = crate::core::metrics_store::METRICS_STORE.get() {
        let tools = store.by_tool(from, to);
        let data: Vec<serde_json::Value> = tools.iter().map(|t| {
            serde_json::json!({
                "tool_name": t.tool_name,
                "total_calls": t.total_calls,
                "success_count": t.success_count,
                "failure_count": t.failure_count,
                "success_rate": if t.total_calls > 0 { format!("{}%", (t.success_count * 100) / t.total_calls) } else { "—".to_string() },
            })
        }).collect();
        Json(serde_json::json!({
            "range": params.range,
            "tools": data,
        }))
    } else {
        Json(serde_json::json!({"error": "metrics store not initialized"}))
    }
}

/// Native (built-in) tool names
const NATIVE_TOOL_NAMES: &[&str] = &[
    "fs_read", "fs_write", "shell_exec", "calculator", "tool_register",
    "memory_recall", "memory_store", "file_edit", "glob", "grep",
    "http", "web_search", "web_scrape", "get_time", "lsp",
    "create_rhai_tool", "list_rhai_tools",
];

/// GET /api/v1/tools/summary?range=day
/// Returns tool inventory (native vs custom) + per-tool usage with metadata
pub async fn tools_summary(
    axum::extract::Query(params): axum::extract::Query<HistoryQuery>,
) -> Json<serde_json::Value> {
    let (from, to) = range_to_bounds(&params.range);
    
    // Scan plugins directory for custom tools and their metadata
    let plugins_dir = dirs::home_dir()
        .map(|h| h.join(".telos").join("plugins"))
        .unwrap_or_default();
    let mut custom_tool_meta: std::collections::HashMap<String, serde_json::Value> = std::collections::HashMap::new();
    if plugins_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&plugins_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        let mut meta = serde_json::json!({"tool_type": "custom"});
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                                meta["version"] = parsed.get("version").cloned().unwrap_or(serde_json::json!("—"));
                                meta["iteration"] = parsed.get("iteration").cloned().unwrap_or(serde_json::json!(0));
                                meta["change_reason"] = parsed.get("change_reason").cloned().unwrap_or(serde_json::json!("—"));
                            }
                        }
                        // Get file modification time as "last_updated_ms"
                        if let Ok(file_meta) = std::fs::metadata(&path) {
                            if let Ok(modified) = file_meta.modified() {
                                if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
                                    meta["last_updated_ms"] = serde_json::json!(duration.as_millis() as u64);
                                }
                            }
                        }
                        custom_tool_meta.insert(stem.to_string(), meta);
                    }
                }
            }
        }
    }

    let native_count = NATIVE_TOOL_NAMES.len();
    let custom_count = custom_tool_meta.len();
    let total_count = native_count + custom_count;
    
    // Get per-tool usage from metrics
    let tool_usage = if let Some(store) = crate::core::metrics_store::METRICS_STORE.get() {
        let tools = store.by_tool(from, to);
        tools.iter().map(|t| {
            let is_native = NATIVE_TOOL_NAMES.contains(&t.tool_name.as_str());
            let mut entry = serde_json::json!({
                "tool_name": t.tool_name,
                "total_calls": t.total_calls,
                "success_count": t.success_count,
                "failure_count": t.failure_count,
                "success_rate": if t.total_calls > 0 { format!("{}%", (t.success_count * 100) / t.total_calls) } else { "—".to_string() },
                "tool_type": if is_native { "native" } else { "custom" },
            });
            // Merge metadata from JSON schema for custom tools
            if let Some(meta) = custom_tool_meta.get(&t.tool_name) {
                if let Some(obj) = entry.as_object_mut() {
                    for (k, v) in meta.as_object().unwrap_or(&serde_json::Map::new()) {
                        if k != "tool_type" { obj.insert(k.clone(), v.clone()); }
                    }
                }
            }
            entry
        }).collect::<Vec<_>>()
    } else {
        vec![]
    };

    let total_calls: usize = tool_usage.iter()
        .filter_map(|t| t.get("total_calls").and_then(|v| v.as_u64()))
        .map(|v| v as usize).sum();
    let total_success: usize = tool_usage.iter()
        .filter_map(|t| t.get("success_count").and_then(|v| v.as_u64()))
        .map(|v| v as usize).sum();
    
    Json(serde_json::json!({
        "range": params.range,
        "inventory": {
            "total": total_count,
            "native": native_count,
            "custom": custom_count,
        },
        "usage_summary": {
            "total_calls": total_calls,
            "total_success": total_success,
            "total_failure": total_calls - total_success,
            "success_rate": if total_calls > 0 { format!("{}%", (total_success * 100) / total_calls) } else { "—".to_string() },
        },
        "tools": tool_usage,
    }))
}

/// GET /api/v1/workflows/summary?range=day
/// Returns workflow inventory + per-workflow reuse stats
pub async fn workflows_summary(
    axum::extract::Query(params): axum::extract::Query<HistoryQuery>,
) -> Json<serde_json::Value> {
    let (from, to) = range_to_bounds(&params.range);
    
    if let Some(store) = crate::core::metrics_store::METRICS_STORE.get() {
        let workflows = store.by_workflow(from, to);
        let total_stored = workflows.len();
        let total_reused: usize = workflows.iter().map(|w| w.reuse_count).sum();
        let total_reuse_success: usize = workflows.iter().map(|w| w.reuse_success).sum();
        
        let data: Vec<serde_json::Value> = workflows.iter().map(|w| {
            serde_json::json!({
                "workflow_id": w.workflow_id,
                "description": w.description,
                "stored_at_ms": w.stored_at_ms,
                "reuse_count": w.reuse_count,
                "reuse_success": w.reuse_success,
                "reuse_failure": w.reuse_failure,
                "success_rate": if w.reuse_count > 0 { format!("{}%", (w.reuse_success * 100) / w.reuse_count) } else { "—".to_string() },
                "version": w.version,
                "type": if w.is_variant { "variant" } else { "original" },
                "failure_count": w.reuse_failure,
            })
        }).collect();
        
        Json(serde_json::json!({
            "range": params.range,
            "summary": {
                "total_stored": total_stored,
                "total_reused": total_reused,
                "total_reuse_success": total_reuse_success,
                "total_reuse_failure": total_reused.saturating_sub(total_reuse_success),
                "reuse_success_rate": if total_reused > 0 { format!("{}%", (total_reuse_success * 100) / total_reused) } else { "—".to_string() },
            },
            "workflows": data,
        }))
    } else {
        Json(serde_json::json!({"error": "metrics store not initialized"}))
    }
}

use telos_core::schedule::MissionStatus;
use telos_memory::engine::MissionStore;

/// GET /api/v1/schedules
pub async fn get_schedules(State(state): State<AppState>) -> Json<serde_json::Value> {
    let mut missions = state.memory_os.retrieve_missions().await.unwrap_or_default();
    // sort by next_run_at ascending
    missions.sort_by_key(|m| m.next_run_at);
    Json(serde_json::json!({
        "schedules": missions
    }))
}

/// DELETE /api/v1/schedules/:id
pub async fn cancel_schedule(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    match state.memory_os.delete_mission(&id).await {
        Ok(_) => Json(serde_json::json!({ "status": "success" })),
        Err(e) => Json(serde_json::json!({ "status": "error", "message": e.to_string() })),
    }
}

/// GET /api/v1/schedules/metrics
pub async fn schedules_metrics(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HistoryQuery>,
) -> Json<serde_json::Value> {
    let mut total_active = 0;
    let mut total_completed = 0;
    let mut total_failed = 0;
    let mut total_executions = 0;

    if let Ok(missions) = state.memory_os.retrieve_missions().await {
         for m in missions {
             match m.status {
                MissionStatus::Active => total_active += 1,
                MissionStatus::Completed => total_completed += 1,
                MissionStatus::Failed => total_failed += 1,
                _ => {}
             }
             total_executions += m.execute_count;
         }
    }
    Json(serde_json::json!({
        "range": params.range,
        "metrics": {
            "total_active": total_active,
            "total_completed": total_completed,
            "total_failed": total_failed,
            "total_executions": total_executions,
        }
    }))
}

/// GET /api/v1/metrics/performance?range=day
/// Returns performance summary (avg/max latency) + time-bucketed trends
pub async fn metrics_performance(
    axum::extract::Query(params): axum::extract::Query<HistoryQuery>,
) -> Json<serde_json::Value> {
    let (from, to) = range_to_bounds(&params.range);

    if let Some(store) = crate::core::metrics_store::METRICS_STORE.get() {
        let perf = store.by_performance(from, to);

        let llm_avg = if perf.llm_count > 0 { perf.llm_total_ms / perf.llm_count as u64 } else { 0 };
        let node_avg = if perf.node_count > 0 { perf.node_total_ms / perf.node_count as u64 } else { 0 };
        let mem_avg = if perf.memory_count > 0 { perf.memory_total_ms / perf.memory_count as u64 } else { 0 };
        let ctx_avg = if perf.ctx_count > 0 { perf.ctx_total_ms / perf.ctx_count as u64 } else { 0 };

        // Node breakdown by type
        let node_by_type: Vec<serde_json::Value> = perf.node_by_type.iter().map(|(t, (c, ms))| {
            serde_json::json!({
                "type": t,
                "count": c,
                "avg_ms": if *c > 0 { *ms / *c as u64 } else { 0 },
                "total_ms": ms,
            })
        }).collect();

        // Memory breakdown by query type
        let mem_by_type: Vec<serde_json::Value> = perf.memory_by_type.iter().map(|(t, (c, ms))| {
            serde_json::json!({
                "type": t,
                "count": c,
                "avg_ms": if *c > 0 { *ms / *c as u64 } else { 0 },
                "total_ms": ms,
            })
        }).collect();

        // Time-bucketed trends
        let bucket_ms: u64 = match params.range.as_str() {
            "hour" => 5 * 60_000,
            "day" => 3_600_000,
            "week" => 24 * 3_600_000,
            "month" => 24 * 3_600_000,
            "year" => 7 * 24 * 3_600_000,
            "all" => 30 * 24 * 3_600_000,
            _ => 3_600_000,
        };
        let buckets = store.aggregate_buckets(from, to, bucket_ms);
        let trend: Vec<serde_json::Value> = buckets.iter().map(|b| {
            let m = &b.metrics;
            serde_json::json!({
                "bucket_start_ms": b.bucket_start_ms,
                "llm_count": m.llm_total_requests,
                "llm_avg_ms": if m.llm_total_requests > 0 { m.llm_call_total_elapsed_ms / m.llm_total_requests as u64 } else { 0 },
                "node_count": m.node_exec_count,
                "node_avg_ms": if m.node_exec_count > 0 { m.node_exec_total_ms / m.node_exec_count as u64 } else { 0 },
                "memory_count": m.memory_retrieval_count,
                "memory_avg_ms": if m.memory_retrieval_count > 0 { m.memory_retrieval_total_ms / m.memory_retrieval_count as u64 } else { 0 },
                "ctx_count": m.context_compression_count,
                "ctx_avg_ms": if m.context_compression_count > 0 { m.context_compression_total_ms / m.context_compression_count as u64 } else { 0 },
            })
        }).collect();

        Json(serde_json::json!({
            "range": params.range,
            "summary": {
                "llm": { "count": perf.llm_count, "avg_ms": llm_avg, "max_ms": perf.llm_max_ms },
                "node": { "count": perf.node_count, "avg_ms": node_avg, "max_ms": perf.node_max_ms, "by_type": node_by_type },
                "memory": { "count": perf.memory_count, "avg_ms": mem_avg, "max_ms": perf.memory_max_ms, "total_results": perf.memory_total_results, "by_type": mem_by_type },
                "context": { "count": perf.ctx_count, "avg_ms": ctx_avg, "max_ms": perf.ctx_max_ms },
                "routes": { "direct_reply": perf.route_direct, "expert": perf.route_expert },
            },
            "trend_buckets": trend,
        }))
    } else {
        Json(serde_json::json!({"error": "metrics store not initialized"}))
    }
}
