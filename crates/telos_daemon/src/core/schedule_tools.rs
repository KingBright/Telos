use std::sync::Arc;
use std::str::FromStr;
use serde_json::Value;
use tracing::{info, warn};
use async_trait::async_trait;
use uuid::Uuid;

use telos_tooling::{JsonSchema, ToolError, ToolExecutor, ToolSchema};
use telos_core::RiskLevel;
use telos_core::schedule::{ScheduledMission, MissionStatus};
use telos_memory::engine::RedbGraphStore;
use telos_memory::engine::MissionStore;

#[derive(Clone)]
pub struct ScheduleMissionTool {
    memory_os: Arc<RedbGraphStore>,
}

impl ScheduleMissionTool {
    pub fn new(memory_os: Arc<RedbGraphStore>) -> Self {
        Self { memory_os }
    }

    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "schedule_mission".into(),
            description: "Schedules a recurring autonomous Telos/Agent mission in the internal database. Use this WHENEVER the user asks to 'schedule a task', 'create a cron job', or '定时任务'. NEVER use OS-level crontab or launchctl to schedule tasks unless explicitly requested. You MUST verify you have the capability (tools/workflows) to execute the instruction BEFORE scheduling it, ideally by performing a dry run.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "cron_expr": { "type": "string", "description": "Standard cron expression (e.g., '0 8 * * *' for 8 AM every day)" },
                        "instruction": { "type": "string", "description": "The exact natural language instruction to execute autonomously" },
                        "required_tools": { 
                            "type": "array", 
                            "items": { "type": "string" },
                            "description": "List of tools you verified are required and available for this mission"
                        },
                        "dry_run_validation": { 
                            "type": "string", 
                            "description": "Brief summary of how you verified you have the capability to execute this mission (e.g. 'I just ran the weather tool successfully')" 
                        }
                    },
                    "required": ["cron_expr", "instruction", "required_tools", "dry_run_validation"]
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}

#[async_trait]
impl ToolExecutor for ScheduleMissionTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let cron_expr = params.get("cron_expr").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'cron_expr'".into()))?;
        let instruction = params.get("instruction").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'instruction'".into()))?;
        
        // Validate cron expression before persisting
        if let Err(e) = cron::Schedule::from_str(cron_expr) {
            return Err(ToolError::ExecutionFailed(format!(
                "Invalid cron expression '{}': {}. Use standard 7-field cron format (sec min hour day month weekday year), e.g. '0 0 8 * * * *' for 8 AM daily.",
                cron_expr, e
            )));
        }

        let id = uuid::Uuid::new_v4().to_string();
        let mission = ScheduledMission::new(
            id.clone(),
            Some("default_project".to_string()),
            cron_expr.to_string(),
            instruction.to_string(),
            "telos_daemon".to_string(),
        );
        
        match self.memory_os.store_mission(mission).await {
            Ok(_) => {
                info!("[ScheduleMissionTool] Successfully scheduled mission: {}", id);
                let out = serde_json::json!({
                    "status": "success",
                    "mission_id": id,
                    "message": format!("Mission scheduled successfully with cron: {}", cron_expr),
                });
                Ok(serde_json::to_vec_pretty(&out).unwrap())
            }
            Err(e) => {
                Err(ToolError::ExecutionFailed(format!("Failed to persist scheduled mission: {}", e)))
            }
        }
    }
}

#[derive(Clone)]
pub struct ListScheduledMissionsTool {
    memory_os: Arc<RedbGraphStore>,
}

impl ListScheduledMissionsTool {
    pub fn new(memory_os: Arc<RedbGraphStore>) -> Self {
        Self { memory_os }
    }

    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "list_scheduled_missions".into(),
            description: "Lists all currently active scheduled autonomous Agent/Telos missions in the internal database. Use this WHENEVER the user asks to check 'my scheduled tasks', 'cron tasks', or '定时任务'. NEVER use shell execution to scan macOS launchd/crontab or system services unless the user explicitly requests OS-level background processes.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}

#[async_trait]
impl ToolExecutor for ListScheduledMissionsTool {
    async fn call(&self, _params: Value) -> Result<Vec<u8>, ToolError> {
        match self.memory_os.retrieve_missions().await {
            Ok(missions) => {
                let out = serde_json::json!({
                    "status": "success",
                    "total": missions.len(),
                    "missions": missions,
                });
                Ok(serde_json::to_vec_pretty(&out).unwrap())
            }
            Err(e) => {
                Err(ToolError::ExecutionFailed(format!("Failed to retrieve missions: {}", e)))
            }
        }
    }
}

#[derive(Clone)]
pub struct CancelMissionTool {
    memory_os: Arc<RedbGraphStore>,
}

impl CancelMissionTool {
    pub fn new(memory_os: Arc<RedbGraphStore>) -> Self {
        Self { memory_os }
    }

    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "cancel_mission".into(),
            description: "Cancels or deletes a scheduled mission by its ID.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "mission_id": { "type": "string", "description": "The exact ID of the mission to cancel" }
                    },
                    "required": ["mission_id"]
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}

#[async_trait]
impl ToolExecutor for CancelMissionTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let mission_id = params.get("mission_id").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'mission_id'".into()))?;
            
        match self.memory_os.delete_mission(mission_id).await {
            Ok(_) => {
                info!("[CancelMissionTool] Cancelled mission: {}", mission_id);
                let out = serde_json::json!({
                    "status": "success",
                    "message": format!("Mission {} has been cancelled.", mission_id),
                });
                Ok(serde_json::to_vec_pretty(&out).unwrap())
            }
            Err(e) => {
                Err(ToolError::ExecutionFailed(format!("Failed to cancel mission: {}", e)))
            }
        }
    }
}
