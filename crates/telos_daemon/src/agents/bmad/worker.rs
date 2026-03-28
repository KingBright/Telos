use async_trait::async_trait;
use std::sync::Arc;
use telos_core::{AgentInput, AgentOutput, SystemRegistry};
use telos_dag::ExecutableNode;
use telos_model_gateway::gateway::GatewayManager;
use telos_model_gateway::{LlmRequest, Message, Capability, ModelGateway};
use serde_json::Value;

pub struct WorkerAgent {
    pub gateway: Arc<GatewayManager>,
}

#[async_trait]
impl ExecutableNode for WorkerAgent {
    async fn execute(
        &self,
        input: AgentInput,
        _registry: &dyn SystemRegistry,
    ) -> AgentOutput {
        // The MVC (Minimal Viable Context) is injected through `input.task` and `input.schema_payload`
        // Harness QA feedbck is injected via `input.correction` or appended to `input.task`
        
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string" },
                "file_content": { "type": "string" },
                "explanation": { "type": "string", "description": "Short internal reasoning about contract adherence" }
            },
            "required": ["file_path", "file_content", "explanation"]
        });

        let mut instructions = "You are the WorkerAgent. You are a pure function.\n\
            Your job is to read the exact DevTask constraints, adhere STRICTLY to the attached Contracts, \
            and output the final target file content.\n\
            If there are compile errors or test failures provided in the context, you MUST fix them.\n\
            DO NOT hallucinate functionality outside the contract.\n\
            You MUST use the provided `submit_code_file` tool to submit the result. Do not output anything else.".to_string();
        
        if let Some(ref corr) = input.correction {
            instructions.push_str(&format!(
                "\n\n[QA FEEDBACK - CRITICAL]: The Harness rejected your previous attempt.\nReason: {}\nFix instructions: {:?}",
                corr.diagnosis, corr.correction_instructions
            ));
        }

        let mut messages = vec![
            Message { role: "system".to_string(), content: instructions },
            Message { role: "user".to_string(), content: input.task.clone() },
        ];
        
        // Include schema payload representing the MVC (Contracts & DevTask)
        if let Some(payload) = &input.schema_payload {
            messages.push(Message {
                role: "user".to_string(),
                content: format!("MVC (Minimal Viable Context):\n{}", payload)
            });
        }

        let tool_def = telos_model_gateway::ToolDefinition {
            name: "submit_code_file".to_string(),
            description: "Submit the generated code file and path".to_string(),
            parameters: schema,
        };

        let req = LlmRequest {
            session_id: format!("worker_{}", input.node_id),
            messages,
            required_capabilities: Capability { requires_vision: false, strong_reasoning: true },
            budget_limit: 50_000, // Coder may generate large content
            tools: Some(vec![tool_def]),
        };

        match self.gateway.generate(req).await {
            Ok(res) => {
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
                
                tracing::error!("[DEBUG] WorkerAgent raw extract content: {}", content);
                
                if let Ok(parsed) = serde_json::from_str::<Value>(content) {
                    tracing::error!("[DEBUG] WorkerAgent parsed AST successfully: {}", parsed);
                    AgentOutput::success(parsed)
                } else {
                    crate::agents::parse_failure("JsonParseError", "WorkerAgent", content)
                }
            }
            Err(e) => crate::agents::from_gateway_error(e, "WorkerAgent"),
        }
    }
}
