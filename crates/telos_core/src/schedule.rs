use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MissionStatus {
    Active,
    Paused,
    Completed,
    Failed,
}

impl Default for MissionStatus {
    fn default() -> Self {
        Self::Active
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduledMission {
    pub id: String,
    pub project_id: Option<String>,
    pub cron_expr: String,
    pub instruction: String,
    pub origin_channel: String,
    pub status: MissionStatus,
    pub last_run_at: Option<i64>,
    pub next_run_at: Option<i64>,
    pub execute_count: u32,
    pub failure_count: u32,
}

impl ScheduledMission {
    pub fn new(
        id: String,
        project_id: Option<String>,
        cron_expr: String,
        instruction: String,
        origin_channel: String,
    ) -> Self {
        Self {
            id,
            project_id,
            cron_expr,
            instruction,
            origin_channel,
            status: MissionStatus::Active,
            last_run_at: None,
            next_run_at: None,
            execute_count: 0,
            failure_count: 0,
        }
    }
}
