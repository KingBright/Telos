use serde::{Deserialize, Serialize};

// Telemetry Metrics

// Core Traits and Primitives

// 1. Adapter to convert Context OpenAiProvider to Gateway ModelProvider for the Gateway Manager


#[derive(serde::Deserialize)]
pub struct RunRequest {
    pub payload: String,
    pub project_id: Option<String>,
    pub trace_id: Option<String>,
}

#[derive(Serialize)]
pub struct RunResponse {
    pub status: String,
    pub trace_id: String,
}

#[derive(Serialize)]
pub struct RunSyncResponse {
    pub status: String,
    pub trace_id: String,
    pub task_summary: Option<telos_hci::TaskSummary>,
    pub final_output: Option<String>,
}

#[derive(Deserialize)]
pub struct ApproveRequest {
    pub task_id: String,
    pub approved: bool,
}

#[derive(Serialize)]
pub struct ApproveResponse {
    pub status: String,
}

#[derive(Deserialize)]
pub struct SetLogLevelRequest {
    pub level: String,
}

#[derive(Serialize)]
pub struct GetLogLevelResponse {
    pub level: String,
}

#[derive(Serialize)]
pub struct SetLogLevelResponse {
    pub status: String,
    pub old_level: String,
    pub new_level: String,
}

#[derive(serde::Deserialize)]
pub struct InterventionRequest {
    pub task_id: String,
    pub node_id: Option<String>,
    pub instruction: String,
}

#[derive(serde::Serialize)]
pub struct InterventionResponse {
    pub status: String,
}


#[derive(serde::Deserialize)]
pub struct ClarifyRequest {
    pub task_id: String,
    pub selected_option_id: Option<String>,
    pub free_text: Option<String>,
}

#[derive(serde::Serialize)]
pub struct ClarifyResponse {
    pub status: String,
}


#[derive(serde::Deserialize)]
pub struct WsQuery {
    pub trace_id: Option<String>,
}

/// Query parameters for time-series metrics endpoints.
/// `range` can be: "hour", "day", "week", "month", "year", "all" (default: "day")
#[derive(serde::Deserialize)]
pub struct HistoryQuery {
    #[serde(default = "default_range")]
    pub range: String,
}

fn default_range() -> String {
    "day".to_string()
}
