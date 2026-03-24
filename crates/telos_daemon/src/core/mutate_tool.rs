use telos_tooling::{JsonSchema, ToolError, ToolExecutor, ToolRegistry, ToolSchema};
use telos_model_gateway::{gateway::GatewayManager, ModelGateway, LlmRequest, Message, Capability};
use telos_core::RiskLevel;
use std::sync::Arc;
use serde_json::Value;
use tracing::{info, warn};
use async_trait::async_trait;
use telos_tooling::script_sandbox::{ScriptExecutor, ScriptSandbox};
use std::path::Path;

#[derive(Clone)]
pub struct MutateTool {
    registry: Arc<dyn ToolRegistry>,
    gateway: Arc<GatewayManager>,
    tools_dir: String,
}

impl MutateTool {
    pub fn new(registry: Arc<dyn ToolRegistry>, gateway: Arc<GatewayManager>, tools_dir: String) -> Self {
        Self { registry, gateway, tools_dir }
    }

    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "mutate_tool".into(),
            description: "Automated tool evolution. Rewrites a Rhai script tool based on feedback/errors, saving a backup of the old code. \
            Only use this if you know EXACTLY what is wrong with the current tool, otherwise investigate the error first.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "tool_name": { "type": "string", "description": "The exact name of the tool to mutate" },
                        "feedback": { "type": "string", "description": "Specific feedback or errors from the last run to guide the rewrite" },
                        "test_params": { "type": "string", "description": "JSON string of realistic parameters to test the NEW mutated script with" }
                    },
                    "required": ["tool_name", "feedback", "test_params"]
                }),
            },
            risk_level: RiskLevel::HighRisk,
            ..Default::default()
        }
    }
}

#[async_trait]
impl ToolExecutor for MutateTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let tool_name = params.get("tool_name").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'tool_name'".into()))?;
        let feedback = params.get("feedback").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'feedback'".into()))?;
        let test_params_str = params.get("test_params").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'test_params'".into()))?;

        let old_schema = match self.registry.get_schema(tool_name) {
            Some(s) => s,
            None => return Err(ToolError::ExecutionFailed(format!("Tool '{}' not found", tool_name))),
        };

        let old_executor = match self.registry.get_executor(tool_name) {
            Some(e) => e,
            None => return Err(ToolError::ExecutionFailed(format!("Executor for tool '{}' not found", tool_name))),
        };

        let old_code = match old_executor.source_code() {
            Some(c) => c,
            None => return Err(ToolError::ExecutionFailed(format!("Tool '{}' does not have source code (might be a native tool). Only Rhai scripts can be mutated.", tool_name))),
        };

        info!("[MutateTool] 🧬 Initiating mutation for '{}', iteration {}", tool_name, old_schema.iteration);

        let prompt = format!(
            r#"You are a SoftwareExpert Agent tasked with mutating (rewriting) a Rhai script tool to fix errors or improve behavior.
Tool Name: {}
Description: {}

CURRENT RHAI CODE:
```rhai
{}
```

CURRENT PARAMETER SCHEMA:
{}

FEEDBACK/ERRORS:
{}

Your task:
Write the NEW Rhai code and NEW parameter schema that fixes the issues.
RULES:
1. Keep the data-fetch logic (http_get / try_parse_json) simple.
2. Ensure you return valid data (not empty).
3. Output EXACTLY in this format:
[SchemaJSON]
<the raw JSON object for parameters>
[RhaiCode]
<the new rhai script without markdown backticks unless strictly necessary>
"#,
            tool_name, old_schema.description, old_code, old_schema.parameters_schema.raw_schema, feedback
        );

        let req = LlmRequest {
            session_id: format!("mutate_{}", tool_name),
            messages: vec![
                Message { role: "system".into(), content: "You are the Tool Mutator. Output only the requested format.".into() },
                Message { role: "user".into(), content: prompt },
            ],
            required_capabilities: Capability { requires_vision: false, strong_reasoning: true },
            budget_limit: 4000,
            tools: None,
        };

        let response = match self.gateway.generate(req).await {
            Ok(r) => r.content,
            Err(e) => return Err(ToolError::ExecutionFailed(format!("LLM call failed: {}", e.to_user_message()))),
        };

        let (new_schema_str, new_code) = Self::parse_llm_output(&response)?;
        
        let raw_schema: Value = match serde_json::from_str(&new_schema_str) {
            Ok(s) => s,
            Err(e) => return Err(ToolError::ExecutionFailed(format!("LLM generated invalid JSON schema: {}", e))),
        };

        let test_params: Value = match serde_json::from_str(test_params_str) {
            Ok(s) => s,
            Err(e) => return Err(ToolError::ExecutionFailed(format!("Your test_params are invalid JSON: {}", e))),
        };

        let sandbox = Arc::new(ScriptSandbox::new());
        match sandbox.execute(&new_code, test_params) {
            Ok(test_result) => {
                let test_output_str = serde_json::to_string_pretty(&test_result).unwrap_or_else(|_| test_result.to_string());
                let is_empty_output = test_result.is_null() || test_output_str.trim() == "{}" || test_output_str.trim() == "\"\"" || test_output_str.trim() == "\"()\"";
                
                if is_empty_output {
                    return Err(ToolError::ExecutionFailed(format!("Mutated script ran but returned empty data: {}", test_output_str)));
                }

                // 1. Backup old file
                let base_path = Path::new(&self.tools_dir);
                let old_rhai_path = base_path.join(format!("{}.rhai", tool_name));
                if old_rhai_path.exists() {
                    let backup_path = base_path.join(format!("{}.v{}.rhai.bak", tool_name, old_schema.version));
                    if let Err(e) = std::fs::copy(&old_rhai_path, &backup_path) {
                        warn!("Failed to backup old code: {}", e);
                    } else {
                        info!("Backed up old code to {:?}", backup_path);
                    }
                }

                // 2. Register new tool
                let new_version = Self::bump_minor_version(&old_schema.version);
                let change_reason = format!("Mutated from feedback: {}", feedback);
                
                let mutated_schema = ToolSchema {
                    name: tool_name.to_string(),
                    description: old_schema.description.clone(),
                    parameters_schema: JsonSchema { raw_schema },
                    risk_level: RiskLevel::Normal,
                    version: new_version.clone(),
                    iteration: old_schema.iteration + 1,
                    parent_tool: Some(tool_name.to_string()),
                    change_reason: Some(change_reason.clone()),
                    experience_notes: old_schema.experience_notes.clone(),
                    // Preserve health stats, reset health to active since mutation is a fix
                    success_count: old_schema.success_count,
                    failure_count: old_schema.failure_count,
                    last_success_at: old_schema.last_success_at,
                    last_failure_at: old_schema.last_failure_at,
                    health_status: "active".to_string(),
                };

                let executor = Arc::new(ScriptExecutor::new(new_code.clone(), sandbox));
                if let Err(e) = self.registry.register_dynamic_tool(mutated_schema, executor) {
                    return Err(ToolError::ExecutionFailed(format!("Failed to register mutated tool: {}", e)));
                }

                let out = serde_json::json!({
                    "status": "success",
                    "message": format!("Tool '{}' mutated from v{} to v{}", tool_name, old_schema.version, new_version),
                    "test_output": test_result,
                    "mutated_code": new_code,
                });
                
                Ok(serde_json::to_vec_pretty(&out).unwrap())
            }
            Err(e) => {
                Err(ToolError::ExecutionFailed(format!("Mutated script failed tests: {:?}. Try mutating again with this new error info.", e)))
            }
        }
    }
}

impl MutateTool {
    fn parse_llm_output(content: &str) -> Result<(String, String), ToolError> {
        let content = content.trim();
        let schema_marker = "[SchemaJSON]";
        let code_marker = "[RhaiCode]";
        
        let schema_start = content.find(schema_marker)
            .ok_or_else(|| ToolError::ExecutionFailed("LLM output missing [SchemaJSON] block".into()))?;
        let code_start = content.find(code_marker)
            .ok_or_else(|| ToolError::ExecutionFailed("LLM output missing [RhaiCode] block".into()))?;
        
        if schema_start >= code_start {
            return Err(ToolError::ExecutionFailed("[SchemaJSON] must appear before [RhaiCode]".into()));
        }
        
        let schema_section = content[schema_start + schema_marker.len()..code_start].trim();
        let mut code_section = content[code_start + code_marker.len()..].trim();
        
        if code_section.starts_with("```rhai") {
            code_section = &code_section[7..];
        } else if code_section.starts_with("```") {
            code_section = &code_section[3..];
        }
        if code_section.ends_with("```") {
            code_section = &code_section[..code_section.len()-3];
        }
        
        Ok((schema_section.to_string(), code_section.trim().to_string()))
    }

    fn bump_minor_version(version: &str) -> String {
        let parts: Vec<&str> = version.split('.').collect();
        if parts.len() == 3 {
            let major = parts[0];
            let minor: u32 = parts[1].parse().unwrap_or(0);
            format!("{}.{}.0", major, minor + 1)
        } else {
            "2.0.0".to_string()
        }
    }
}
