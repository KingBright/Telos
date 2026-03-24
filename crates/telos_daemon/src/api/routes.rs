use axum::{
    routing::{get, post}, Router,
};
use tower_http::services::ServeDir;

// Telemetry Metrics

// Core Traits and Primitives

// 1. Adapter to convert Context OpenAiProvider to Gateway ModelProvider for the Gateway Manager

use crate::api::handlers::*;
use crate::api::ws::*;
use crate::core::state::*;

pub fn build_router(state: AppState) -> Router {
    // Serve dashboard static files from ~/.telos/web (deployed) or crates/telos_web/static (dev)
    let home_web_dir = dirs::home_dir().map(|h| h.join(".telos/web"));
    let serve_dir = if let Some(p) = &home_web_dir {
        if p.exists() {
            ServeDir::new(p)
        } else {
            ServeDir::new("crates/telos_web/static")
        }
    } else {
        ServeDir::new("crates/telos_web/static")
    };

    Router::new()
        // API Endpoints
        .route("/api/v1/run", post(handle_run))
        .route("/api/v1/run_sync", post(handle_run_sync))
        .route("/api/v1/traces", get(get_traces))
        .route("/api/v1/tasks/active", get(get_active_tasks))
        .route("/api/v1/metrics", get(handle_metrics))
        .route("/api/v1/metrics/history", get(metrics_history))
        .route("/api/v1/metrics/by-agent", get(metrics_by_agent))
        .route("/api/v1/metrics/by-task", get(metrics_by_task))
        .route("/api/v1/metrics/by-tool", get(metrics_by_tool))
        .route("/api/v1/metrics/performance", get(metrics_performance))
        .route("/api/v1/tools/summary", get(tools_summary))
        .route("/api/v1/workflows/summary", get(workflows_summary))
        .route("/api/v1/approve", post(handle_approve))
        .route("/api/v1/intervention", post(handle_intervention))
        .route("/api/v1/clarify", post(handle_clarify))
        .route("/api/v1/tasks/:id/cancel", post(handle_cancel))
        .route("/api/v1/schedules", get(get_schedules))
        .route("/api/v1/schedules/metrics", get(schedules_metrics))
        .route("/api/v1/schedules/:id", axum::routing::delete(cancel_schedule))
        .route("/api/v1/log-level", get(get_log_level).post(set_log_level))
        .route("/api/v1/stream", get(ws_handler))
        // Dashboard static files (index.html, js/, css/)
        .fallback_service(serve_dir)
        .with_state(state)
}
