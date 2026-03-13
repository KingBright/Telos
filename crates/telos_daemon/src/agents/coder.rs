use crate::agents::{
    async_trait, AgentInput, AgentOutput, Arc, ExecutableNode, GatewayManager, SystemRegistry,
};
use tracing::{info, warn, error};
use telos_model_gateway::ModelGateway;

pub struct CoderAgent {
    pub gateway: Arc<GatewayManager>,
    pub tools_dir: String,
    // Add additional fields as needed for TDD-style execution loop
}

impl CoderAgent {
    pub fn new(gateway: Arc<GatewayManager>, tools_dir: String) -> Self {
        Self { gateway, tools_dir }
    }
}

use telos_core::agent_traits::WorkerAgent;

#[async_trait]
impl WorkerAgent for CoderAgent {
    fn worker_type(&self) -> &'static str {
        "coder"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string" },
                "instructions": { "type": "string" }
            },
            "required": ["instructions"]
        })
    }

    async fn execute_worker(&self, input: AgentInput, _registry: &dyn SystemRegistry) -> AgentOutput {
        info!(
            "[CoderAgent] 💻 Executing implementation task: \"{}\"",
            input.node_id
        );

        // As a strict worker in the Sub-DAG, the CoderAgent receives an exact implementation prompt.
        let env_context = if let Some(ctx) = _registry.get_system_context() {
            format!("[ENVIRONMENT CONTEXT]\nLocal Time: {}\nPhysical Location: {}\n\n", ctx.current_time, ctx.location)
        } else {
            String::new()
        };
        let mem_context = input.memory_context.clone().unwrap_or_default();

        let system_prompt = format!("{}{}{}", env_context, mem_context, r#"You are the CoderAgent, an expert software engineer.
You are a worker node inside a larger task graph. You have received precise implementation instructions from the Architect.

If you are asked to create a new Telos Dynamic Tool or fetch data from a new API, you MUST write a `rhai` script.
Rhai is a Rust-like scripting language. By default, the sandbox provides a `http_get(url)` function which returns a JSON string or text.
Example Rhai tool script:
```rhai
let response = http_get("https://api.example.com/data");
// return a JSON object (or Map) structure
#{ status: "success", data: response }
```
Return your implementation details or the raw script."#);

        let mut attempts = 0;
        let mut current_task_payload = input.task.clone();
        let max_attempts = 3;

        loop {
            let prompt = format!("System: {}\n\nTask:\n{}", system_prompt, current_task_payload);
    
            let req = telos_model_gateway::LlmRequest {
                session_id: format!("coder_{}_{}", input.node_id, attempts),
                messages: vec![telos_model_gateway::Message {
                    role: "user".to_string(),
                    content: prompt,
                }],
                required_capabilities: telos_model_gateway::Capability {
                    requires_vision: false,
                    strong_reasoning: false, // Coding can often manage without strong reasoning if instructions are precise
                },
                budget_limit: 128_000,
            };
    
            match self.gateway.generate(req.clone()).await {
                Ok(res) => {
                    let trace = telos_core::TraceLog::LlmCall {
                        request: serde_json::to_value(&req).unwrap_or_else(|_| serde_json::json!({})),
                        response: serde_json::to_value(&res).unwrap_or_else(|_| serde_json::json!({})),
                    };
                    let text = res.content.to_lowercase();
                    // Determine if we need to escalate based on content (mocking validation failure)
                    // In reality, if tests fail repeatedly we would escalate.
                    if text.contains("i don't know") || text.contains("cannot complete") || text.contains("need help") || text.contains("stuck") {
                        attempts += 1;
                        if attempts < max_attempts {
                            warn!("[CoderAgent] ⚠️ Worker stuck. Consulting Expert (Attempt {}/{})", attempts, max_attempts);
                            let expert_prompt = format!(
                                "You are a Senior Software Architect overseeing a junior CoderAgent.\n\
                                The Coder is stuck on this task:\n{}\n\n\
                                The Coder's last output/issue was:\n{}\n\n\
                                Please provide concrete guidance, hints, or revised step-by-step instructions to help the Coder succeed. Be concise.",
                                input.task, res.content
                            );

                            let expert_req = telos_model_gateway::LlmRequest {
                                session_id: format!("expert_help_{}_{}", input.node_id, attempts),
                                messages: vec![telos_model_gateway::Message {
                                    role: "user".to_string(),
                                    content: expert_prompt,
                                }],
                                required_capabilities: telos_model_gateway::Capability {
                                    requires_vision: false,
                                    strong_reasoning: true, // Expert needs to reason
                                },
                                budget_limit: 128_000,
                            };

                            if let Ok(expert_res) = self.gateway.generate(expert_req.clone()).await {
                                info!("[CoderAgent] 🧠 Expert provided guidance. Retrying...");
                                current_task_payload = format!(
                                    "Original Task:\n{}\n\nExpert Guidance (Follow this carefully):\n{}",
                                    input.task, expert_res.content
                                );
                                continue;
                            }
                        }
                        
                        error!("[CoderAgent] 🆘 Worker completely stuck after {} attempts. Escalating to Router/User.", attempts);
                        return AgentOutput::help(
                            "ImplementationBlock",
                            &format!("Escalated after {} attempts. The instructions are ambiguous or I lack the necessary tool. Last output: {}", attempts, res.content),
                            vec![
                                "Clarify the precise architectural pattern to use".to_string(),
                                "Provide the missing dependency or file path".to_string()
                            ]
                        ).with_trace(trace);
                    } else {
                        info!(
                            "[CoderAgent] ✅ Implementation finished. ({} bytes)",
                            res.content.len()
                        );
                        return AgentOutput::success(serde_json::json!({
                            "text": res.content
                        })).with_trace(trace);
                    }
                }
                Err(e) => return AgentOutput::failure("CoderLLMError", &format!("LLM failed: {:?}", e)),
            }
        }
    }
}

#[async_trait]
impl ExecutableNode for CoderAgent {
    async fn execute(&self, input: AgentInput, registry: &dyn SystemRegistry) -> AgentOutput {
        self.execute_worker(input, registry).await
    }
}
