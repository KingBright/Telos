use crate::{JsonSchema, ToolError, ToolExecutor, ToolSchema};
use tracing::{info, debug, warn, error};
use async_trait::async_trait;
use serde_json::Value;
use telos_core::RiskLevel;
use tokio::process::Command;

// 3. Shell Execution Tool
#[derive(Clone)]
pub struct ShellExecTool;

#[async_trait]
impl ToolExecutor for ShellExecTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'command' parameter".into()))?;

        let output = Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to execute shell command: {}", e))
            })?;

        let result = if output.status.success() {
            String::from_utf8_lossy(&output.stdout).to_string()
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ToolError::ExecutionFailed(format!(
                "Command failed with error: {}",
                stderr
            )));
        };

        Ok(result.into_bytes())
    }
}

impl ShellExecTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "shell_exec".into(),
            description: "Executes a shell command on the host OS. Useful for compiling code. Requires a 'command' parameter.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "command": { "type": "string" }
                    },
                    "required": ["command"]
                }),
            },
            risk_level: RiskLevel::HighRisk,
            ..Default::default()
        }
    }
}


// 15. Get Location Tool
#[derive(Clone)]
pub struct GetLocationTool;

#[async_trait]
impl ToolExecutor for GetLocationTool {
    async fn call(&self, _params: Value) -> Result<Vec<u8>, ToolError> {
        let response = reqwest::get("http://ip-api.com/json/")
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to fetch location: {}", e)))?
            .json::<serde_json::Value>()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to parse location JSON: {}", e)))?;

        let location = serde_json::json!({
            "lat": response.get("lat"),
            "lon": response.get("lon"),
            "country": response.get("country"),
            "province": response.get("regionName"),
            "city": response.get("city")
        });

        Ok(serde_json::to_vec(&location).unwrap_or_default())
    }
}

impl GetLocationTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "get_location".into(),
            description: "Gets the current geographical location based on IP. Keywords: location, geolocation, lat, lon, country, province, city, get_location.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}

// 16. Get Time Tool
#[derive(Clone)]
pub struct GetTimeTool;

#[async_trait]
impl ToolExecutor for GetTimeTool {
    async fn call(&self, _params: Value) -> Result<Vec<u8>, ToolError> {
        let now = std::time::SystemTime::now();
        let timestamp_ms = now.duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let formatted_time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S.%3f").to_string();

        let result = serde_json::json!({
            "formatted_time": formatted_time,
            "timestamp_ms": timestamp_ms
        });

        Ok(serde_json::to_vec(&result).unwrap_or_default())
    }
}

impl GetTimeTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "get_time".into(),
            description: "Gets the current local time. Keywords: time, get_time, get_current_time, clock, current, date, timestamp.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}

