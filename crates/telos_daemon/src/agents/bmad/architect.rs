use async_trait::async_trait;
use std::sync::Arc;
use telos_core::{AgentInput, AgentOutput, SystemRegistry};
use telos_dag::ExecutableNode;
use telos_model_gateway::gateway::GatewayManager;
use telos_model_gateway::{LlmRequest, Message, Capability, ModelGateway};
use serde_json;

pub struct BmadArchitectAgent {
    pub gateway: Arc<GatewayManager>,
}

impl BmadArchitectAgent {
    /// Extract the approved L1 features from upstream dependencies.
    /// The SmartApprovalNode wraps the ProductAgent's output in
    /// `{ "original_plan": { "features": [...] }, "status": "approved", ... }`.
    fn extract_upstream_features(input: &AgentInput) -> (Option<serde_json::Value>, Option<String>) {
        for dep_out in input.dependencies.values() {
            if let Some(ref val) = dep_out.output {
                // SmartApprovalNode wraps upstream in "original_plan"
                let plan = if let Some(p) = val.get("original_plan") { p } else { val };
                if plan.get("features").is_some() {
                    let proj_name = plan.get("project_name").and_then(|v| v.as_str()).map(|s| s.to_string());
                    return (Some(plan.clone()), proj_name);
                }
            }
        }
        (None, None)
    }
}

#[async_trait]
impl ExecutableNode for BmadArchitectAgent {
    async fn execute(
        &self,
        input: AgentInput,
        _registry: &dyn SystemRegistry,
    ) -> AgentOutput {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "modules": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "mapped_feature_id": { "type": "string" },
                            "name": { "type": "string" },
                            "directory_path": { "type": "string" }
                        },
                        "required": ["id", "mapped_feature_id", "name", "directory_path"]
                    }
                },
                "contracts": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "name": { "type": "string" },
                            "description": { "type": "string" },
                            "provider_module_id": { "type": "string" },
                            "consumer_module_ids": { "type": "array", "items": { "type": "string" } },
                            "schema_definition": { "type": "object", "description": "OpenAPI or equivalent schema" }
                        },
                        "required": ["id", "name", "description", "provider_module_id", "consumer_module_ids", "schema_definition"]
                    }
                },
                "integration_commands": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "A list of terminal commands to sequentially verify the integration and build success of the project (e.g. ['cargo check'], ['npm run test'], ['python -m pytest']). If this is a non-code project without compilation steps, leave this array extremely strict and empty."
                }
            },
            "required": ["modules", "contracts", "integration_commands"]
        });

        // Build enriched prompt with upstream L1 features
        let (upstream_opt, project_name) = Self::extract_upstream_features(&input);
        
        let upstream_context = if let Some(features_data) = upstream_opt {
            let features_json = serde_json::to_string_pretty(&features_data).unwrap_or_default();
            format!(
                "## Approved L1 Product Features (from ProductAgent)\n{}\n\n## Original User Requirement\n{}",
                features_json, input.task
            )
        } else {
            input.task.clone()
        };

        let system_prompt = "You are the ArchitectAgent (L2 Meta-Graph). \
            Your job is to read L1 ProductFeatures and output the physical software architecture: TechModules and rigid Contracts. \
            Contracts define strict schema boundaries between modules. \
            You MUST use the provided `generate_architecture_blueprint` tool to submit the result. Do not output anything else.".to_string();

        let messages = vec![
            Message { role: "system".to_string(), content: system_prompt },
            Message { role: "user".to_string(), content: upstream_context },
        ];

        let tool_def = telos_model_gateway::ToolDefinition {
            name: "generate_architecture_blueprint".to_string(),
            description: "Submit the decomposed TechModules and Contracts".to_string(),
            parameters: schema,
        };

        let req = LlmRequest {
            session_id: format!("bmad_architect_{}", input.node_id),
            messages,
            required_capabilities: Capability { requires_vision: false, strong_reasoning: true },
            budget_limit: 15_000,
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
                    if let Some(name) = project_name {
                        if let Some(obj) = parsed.as_object_mut() {
                            obj.insert("project_name".to_string(), serde_json::json!(name));
                        }
                    }
                    AgentOutput::success(parsed)
                } else {
                    crate::agents::parse_failure("JsonParseError", "BmadArchitectAgent", content)
                }
            }
            Err(e) => crate::agents::from_gateway_error(e, "BmadArchitectAgent"),
        }
    }
}
