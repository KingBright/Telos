use crate::agents::{
    async_trait, AgentInput, AgentOutput, Arc, ExecutableNode, GatewayManager, SystemRegistry,
};
use telos_model_gateway::{Capability, LlmRequest, Message, ModelGateway};
use tracing::info;

pub struct ReviewAgent {
    pub gateway: Arc<GatewayManager>,
}

impl ReviewAgent {
    pub fn new(gateway: Arc<GatewayManager>) -> Self {
        Self { gateway }
    }
}

use telos_core::agent_traits::WorkerAgent;

#[async_trait]
impl WorkerAgent for ReviewAgent {
    fn worker_type(&self) -> &'static str {
        "reviewer"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content_to_review": { "type": "string" },
                "review_criteria": { "type": "string" }
            },
            "required": ["content_to_review", "review_criteria"]
        })
    }

    async fn execute_worker(&self, input: AgentInput, _registry: &dyn SystemRegistry) -> AgentOutput {
        info!(
            "[ReviewAgent] 🔍 Reviewing output for task: \"{}\"",
            input.node_id
        );

        // Collect the content to review from dependencies
        let content_to_review = input
            .dependencies
            .values()
            .filter_map(|dep| {
                dep.output
                    .as_ref()
                    .and_then(|v| v.get("text").and_then(|t| t.as_str()))
                    .or_else(|| dep.output.as_ref().and_then(|v| v.get("code").and_then(|t| t.as_str())))
            })
            .collect::<Vec<&str>>()
            .join("\n\n---\n\n");

        if content_to_review.is_empty() {
            // No content from dependencies — just pass through
            return AgentOutput::success(serde_json::json!({
                "text": "Review skipped: no upstream content to review."
            }));
        }

        let system_prompt = r#"You are a code reviewer. Review the following code/content for:
1. Correctness — does it solve the stated problem?
2. Quality — is it clean, idiomatic, and well-structured?
3. Edge cases — are there obvious edge cases not handled?

You MUST use the provided `submit_code_review` tool to submit your evaluation. Do not output raw text or explanations.

Keep it concise. If the content is already good, approve it and pass it through."#;

        let mut messages = vec![
            Message { role: "system".to_string(), content: system_prompt.to_string() },
        ];
        for msg in &input.conversation_history {
            messages.push(Message {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
        }
        messages.push(Message {
            role: "user".to_string(),
            content: format!("Task: {}\n\nContent to review:\n{}", input.task, content_to_review),
        });

        let review_tool = telos_model_gateway::ToolDefinition {
            name: "submit_code_review".to_string(),
            description: "Submit the code review results.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "approved": { "type": "boolean" },
                    "review": { "type": "string", "description": "Brief summary of your findings" },
                    "improved_content": { "type": "string", "description": "If you made improvements, put the improved version here. If already good, copy original." }
                },
                "required": ["approved", "review", "improved_content"]
            })
        };

        let req = LlmRequest {
            session_id: format!("reviewer_{}", input.node_id),
            messages,
            required_capabilities: Capability {
                requires_vision: false,
                strong_reasoning: false,
            },
            budget_limit: 4000,
            tools: Some(vec![review_tool]),
        };

        match self.gateway.generate(req).await {
            Ok(res) => {
                let raw_content = if let Some(tc) = res.tool_calls.first() {
                    tc.arguments.clone()
                } else {
                    res.content.clone()
                };

                let content = raw_content.trim()
                    .trim_start_matches("```json")
                    .trim_start_matches("```")
                    .trim_end_matches("```")
                    .trim();

                if let Ok(json) = serde_json::from_str::<serde_json::Value>(content) {
                    // Use improved_content if available, otherwise original
                    let final_text = json.get("improved_content")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or(&content_to_review);
                    return AgentOutput::success(serde_json::json!({"text": final_text}));
                }
                // Couldn't parse — pass through the original content
                AgentOutput::success(serde_json::json!({"text": content_to_review}))
            }
            Err(_) => {
                // LLM failed — pass through original content
                AgentOutput::success(serde_json::json!({"text": content_to_review}))
            }
        }
    }
}

#[async_trait]
impl ExecutableNode for ReviewAgent {
    async fn execute(&self, input: AgentInput, registry: &dyn SystemRegistry) -> AgentOutput {
        self.execute_worker(input, registry).await
    }
}
