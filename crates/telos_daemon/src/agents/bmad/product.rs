use async_trait::async_trait;
use std::sync::Arc;
use telos_core::{AgentInput, AgentOutput, SystemRegistry};
use telos_dag::ExecutableNode;
use telos_model_gateway::gateway::GatewayManager;
use telos_model_gateway::{LlmRequest, Message, Capability, ModelGateway};
use serde_json;

pub struct ProductAgent {
    pub gateway: Arc<GatewayManager>,
}

impl ProductAgent {
    /// Extract a project name from the user prompt using simple heuristics.
    /// Looks for patterns like "项目叫 X", "project called X", "project named X".
    fn extract_project_name(prompt: &str) -> Option<String> {
        // Chinese patterns
        for prefix in &["项目叫", "项目名叫", "项目名为", "项目名称为", "项目叫做"] {
            if let Some(idx) = prompt.find(prefix) {
                let after = &prompt[idx + prefix.len()..];
                let name = after.trim().split(|c: char| c.is_whitespace() || c == '，' || c == ',' || c == '。' || c == '.' || c == '、')
                    .next()
                    .map(|s| s.trim().to_string());
                if let Some(ref n) = name {
                    if !n.is_empty() {
                        return name;
                    }
                }
            }
        }
        // English patterns
        for prefix in &["project called ", "project named ", "Create a new project called ", "new project called "] {
            let lower = prompt.to_lowercase();
            if let Some(idx) = lower.find(&prefix.to_lowercase()) {
                let start = idx + prefix.len();
                let after = &prompt[start..];
                let name = after.split(|c: char| c.is_whitespace() || c == ',' || c == '.')
                    .next()
                    .map(|s| s.trim().trim_matches(|c: char| c == '\'' || c == '"').to_string());
                if let Some(ref n) = name {
                    if !n.is_empty() {
                        return name;
                    }
                }
            }
        }

        None
    }

    /// Create the project directory and record the initial user interaction.
    fn create_project_and_log(project_name: &str, user_prompt: &str) -> Option<String> {
        let registry = telos_project::manager::ProjectRegistry::new();
        
        // Create project under ~/.telos/projects/<name>
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let project_path = format!("{}/.telos/projects/{}", home, project_name);
        
        match registry.create_project(
            project_name.to_string(),
            Some(project_path.clone()),
            Some(format!("BMAD-generated project from user request")),
        ) {
            Ok(project) => {
                tracing::info!("[ProductAgent] 📁 Created project '{}' at {}", project_name, project.path.display());
                
                // --- Project State Machine & Git Version Rhythm ---
                let _ = std::process::Command::new("git").arg("init").current_dir(&project.path).output();
                
                // Write initial state machine registry
                let toml_path = project.path.join(".telos_project.toml");
                if !toml_path.exists() {
                    let toml_content = format!(r#"[project]
name = "{}"
version = "0.1.0"
status = "designing"

[dependencies]
# Empty topological map

[tasks]
# Dynamic Kanban tracker
"#, project_name);
                    let _ = std::fs::write(&toml_path, toml_content);
                }

                // Create initial root commit on main, then branch out to working Draft branch
                let _ = std::process::Command::new("git").arg("add").arg(".telos_project.toml").current_dir(&project.path).output();
                let _ = std::process::Command::new("git").args(["commit", "-m", "chore: initial project state machine"]).current_dir(&project.path).output();
                let _ = std::process::Command::new("git").args(["checkout", "-b", "ai/wip-v0.1.0"]).current_dir(&project.path).output();

                // Write initial user interaction log
                let interactions_path = project.path.join("interactions.jsonl");
                let interaction = serde_json::json!({
                    "timestamp": std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64,
                    "type": "initial_requirement",
                    "source": "user",
                    "content": user_prompt,
                });
                if let Ok(line) = serde_json::to_string(&interaction) {
                    let _ = std::fs::write(&interactions_path, format!("{}\n", line));
                }
                
                Some(project.id.clone())
            }
            Err(e) => {
                tracing::warn!("[ProductAgent] ⚠️ Failed to create project '{}': {}", project_name, e);
                None
            }
        }
    }
}

#[async_trait]
impl ExecutableNode for ProductAgent {
    async fn execute(
        &self,
        input: AgentInput,
        _registry: &dyn SystemRegistry,
    ) -> AgentOutput {
        // Step 1: Extract project name and create project directory
        let project_name = Self::extract_project_name(&input.task);
        let project_id = if let Some(ref name) = project_name {
            Self::create_project_and_log(name, &input.task)
        } else {
            tracing::debug!("[ProductAgent] No project name detected in prompt, skipping project creation");
            None
        };

        // Step 2: Generate product features via LLM
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "features": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "Unique format like FEAT-123" },
                            "title": { "type": "string" },
                            "description": { "type": "string" },
                            "acceptance_criteria": { "type": "array", "items": { "type": "string" } },
                            "user_journey_flows": { "type": "array", "items": { "type": "string" } }
                        },
                        "required": ["id", "title", "description", "acceptance_criteria", "user_journey_flows"]
                    }
                }
            },
            "required": ["features"]
        });

        let system_prompt = "You are the ProductAgent (L1 Meta-Graph). \
            Your job is to deconstruct user requirements into a strict array of `ProductFeature` objects. \
            Focus on business value, user journeys, and deep acceptance criteria. \
            You MUST use the provided `generate_features` tool to submit the result. Do not output anything else.".to_string();

        let messages = vec![
            Message { role: "system".to_string(), content: system_prompt },
            Message { role: "user".to_string(), content: input.task.clone() },
        ];

        let tool_def = telos_model_gateway::ToolDefinition {
            name: "generate_features".to_string(),
            description: "Submit the decomposed product features".to_string(),
            parameters: schema,
        };

        let req = LlmRequest {
            session_id: format!("product_agent_{}", input.node_id),
            messages,
            required_capabilities: Capability { requires_vision: false, strong_reasoning: true },
            budget_limit: 10_000,
            tools: Some(vec![tool_def]),
        };

        match self.gateway.generate(req).await {
            Ok(res) => {
                // Determine if LLM used the tool or output raw text
                let raw_content = if let Some(tc) = res.tool_calls.first() {
                    tc.arguments.clone()
                } else {
                    res.content
                };

                let content = raw_content.trim()
                    .trim_start_matches("```json")
                    .trim_start_matches("```")
                    .trim_end_matches("```")
                    .trim();

                if let Ok(mut parsed) = serde_json::from_str::<serde_json::Value>(content) {
                    // Extract the "features" array from the tool wrapper object
                    let features = if parsed.is_object() && parsed.get("features").is_some() {
                        parsed.get("features").unwrap().clone()
                    } else {
                        parsed.clone()
                    };

                    // Embed project metadata into the output for downstream agents
                    let mut result = serde_json::json!({
                        "features": features,
                    });
                    if let Some(ref name) = project_name {
                        result["project_name"] = serde_json::json!(name);
                    }
                    if let Some(ref id) = project_id {
                        result["project_id"] = serde_json::json!(id);
                    }
                    
                    AgentOutput::success(result)
                } else {
                    tracing::error!("[ProductAgent] Failed to parse JSON. Raw content: {}", content);
                    crate::agents::parse_failure("JsonParseError", "ProductAgent", content)
                }
            }
            Err(e) => crate::agents::from_gateway_error(e, "ProductAgent"),
        }
    }
}
