use crate::agents::{
    async_trait, AgentInput, AgentOutput, Arc, ExecutableNode, GatewayManager, SystemRegistry,
};
use telos_model_gateway::{Capability, LlmRequest, Message, ModelGateway};

pub struct RouterAgent {
    pub gateway: Arc<GatewayManager>,
}

impl RouterAgent {
    pub fn new(gateway: Arc<GatewayManager>) -> Self {
        Self { gateway }
    }

    pub async fn evaluate(&self, original_task: &str, expert_output: &str, _registry: &dyn SystemRegistry) -> AgentOutput {
        let env_context = if let Some(ctx) = _registry.get_system_context() {
            format!("[ENVIRONMENT CONTEXT]\nLocal Time: {}\nPhysical Location: {}\n\n", ctx.current_time, ctx.location)
        } else {
            String::new()
        };

        let system_prompt = format!("{}{}", env_context, r#"You are the Telos Master Router acting as a strict Quality Assurance supervisor.
Your job is to evaluate whether the Expert Agent's final output fully satisfies the User's original request.

CRITICAL RULES:
1. You MUST output a strictly valid JSON object.
2. DO NOT INCLUDE ANY CONVERSATIONAL TEXT.
3. If the output satisfies the request, set "is_acceptable" to true and "critique" to "".
4. If the output DOES NOT satisfy the request (e.g. it failed, partially answered, or asked the user for information instead of using tools), set "is_acceptable" to false and provide a clear, constructive "critique" on what the expert needs to do differently.

--- EXAMPLES ---

User: "What is the weather?"
Expert: "I found the weather in New York is sunny."
{
  "is_acceptable": true,
  "critique": ""
}

User: "Summarize the latest SpaceX launch."
Expert: "I need to know which launch you mean."
{
  "is_acceptable": false,
  "critique": "You asked the user for information. As an autonomous agent, you must use your search tools to find the latest SpaceX launch yourself and summarize it."
}

Output EXACTLY two keys: "is_acceptable" and "critique"."#);

        let request = LlmRequest {
            session_id: "router_eval".to_string(),
            messages: vec![
                Message { role: "system".to_string(), content: system_prompt.to_string() },
                Message { role: "user".to_string(), content: format!("User Request:\n{}\n\nExpert Output:\n{}", original_task, expert_output) },
            ],
            required_capabilities: Capability {
                requires_vision: false,
                strong_reasoning: true,
            },
            budget_limit: 1000,
        };

        match self.gateway.generate(request.clone()).await {
            Ok(res) => {
                let trace = telos_core::TraceLog::LlmCall {
                    request: serde_json::to_value(&request).unwrap_or_else(|_| serde_json::json!({})),
                    response: serde_json::to_value(&res).unwrap_or_else(|_| serde_json::json!({})),
                };
                let content = res.content.trim();
                let json_str = if let (Some(s), Some(e)) = (content.find('{'), content.rfind('}')) {
                    if e > s { &content[s..=e] } else { content }
                } else {
                    content
                };
                let clean_reply = json_str.trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();

                if let Ok(json) = serde_json::from_str::<serde_json::Value>(clean_reply) {
                    if json.get("is_acceptable").is_some() || json.get("route").is_some() {
                        return AgentOutput::success(json).with_trace(trace);
                    }
                }
                
                AgentOutput::failure("EvalError", &format!("Failed to parse router evaluation as JSON: {}", content)).with_trace(trace)
            }
            Err(e) => AgentOutput::failure("LLMError", &format!("Router failed to evaluate: {:?}", e)),
        }
    }
}

#[async_trait]
impl ExecutableNode for RouterAgent {
    async fn execute(&self, input: AgentInput, _registry: &dyn SystemRegistry) -> AgentOutput {
        let env_context = if let Some(ctx) = _registry.get_system_context() {
            format!("[ENVIRONMENT CONTEXT]\nLocal Time: {}\nPhysical Location: {}\n\n", ctx.current_time, ctx.location)
        } else {
            String::new()
        };
        let mem_context = input.memory_context.clone().unwrap_or_default();

        let system_prompt = format!("{}{}{}", env_context, mem_context, r#"You are the Telos Master Router. Your job is to analyze a user's task and route it to the most specialized expert agent available.

Available Experts:
- "software_expert": For tasks requiring writing, modifying, or executing programming code, or software architecture.
- "research_expert": For tasks requiring deep, iterative information gathering via search engines (e.g., current events, fact-checking, real-time data).
- "qa_expert": For tasks heavily focused on writing tests, finding edge cases, or breaking code.
- "general_expert": For all other tasks requiring step-by-step reasoning or general tool use.

CRITICAL RULES:
1. You MUST output a strictly valid JSON object.
2. DO NOT INCLUDE ANY CONVERSATIONAL TEXT. DO NOT BE "HELPFUL" BY REFUSING OR ASKING QUESTIONS. 
3. YOUR ONLY ROLE IS TO PICK THE EXPERT. 
4. CHOOSE "research_expert" FOR ANY QUERY REQUIRING REAL-TIME OR EXTERNAL DATA.
5. IF THE REQUEST IS UNCLEAR OR BROAD, PICK "general_expert" AND LET THEM HANDLE IT. NEVER REFUSE.

--- EXAMPLES ---

User: "Write a python script to parse CSV"
{
  "route": "software_expert",
  "reason": "Request involves writing programming code."
}

User: "What were the major news events yesterday?"
{
  "route": "research_expert",
  "reason": "Request involves retrieving recent current events requiring search tools."
}

User: "Help me plan a generic schedule."
{
  "route": "general_expert",
  "reason": "General reasoning query."
}

Output EXACTLY two keys: "route" and "reason"."#);

        // Build context from dependencies
        let deps_context = if !input.dependencies.is_empty() {
            let deps_str = input
                .dependencies
                .iter()
                .map(|(id, out)| {
                    let output_str = out
                        .output
                        .as_ref()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "No output".to_string());
                    format!("- {}: {}", id, output_str)
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!("\n\nContext from previous steps:\n{}", deps_str)
        } else {
            String::new()
        };

        let full_task = format!("{}{}", input.task, deps_context);

        let request = LlmRequest {
            session_id: format!("router_{}", input.node_id),
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: format!("Task: {}", full_task),
                },
            ],
            required_capabilities: Capability {
                requires_vision: false,
                strong_reasoning: true, // We need good reasoning for routing
            },
            budget_limit: 1000,
        };

        match self.gateway.generate(request.clone()).await {
            Ok(res) => {
                let trace = telos_core::TraceLog::LlmCall {
                    request: serde_json::to_value(&request).unwrap_or_else(|_| serde_json::json!({})),
                    response: serde_json::to_value(&res).unwrap_or_else(|_| serde_json::json!({})),
                };
                let content = res.content.trim();
                
                // Try to find the first '{' and last '}' to extract a JSON block
                let json_str = if let (Some(s), Some(e)) = (content.find('{'), content.rfind('}')) {
                    if e > s {
                        &content[s..=e]
                    } else {
                        content
                    }
                } else {
                    content
                };

                let clean_reply = json_str
                    .trim_start_matches("```json")
                    .trim_start_matches("```")
                    .trim_end_matches("```")
                    .trim();

                if let Ok(json) = serde_json::from_str::<serde_json::Value>(clean_reply) {
                    if json.get("route").is_some() {
                        return AgentOutput::success(json).with_trace(trace);
                    }
                }

                // If content exists but parsing failed, try to fallback if it looks like a refusal
                if !content.is_empty() {
                    // If it's a direct response and we couldn't get JSON, 
                    // route to general_expert to handle the conversation or error gracefully.
                    return AgentOutput::success(serde_json::json!({
                        "route": "general_expert",
                        "reason": format!("LLM provided non-JSON response, falling back: {}", content)
                    })).with_trace(trace);
                }

                AgentOutput::failure(
                    "RoutingError",
                    &format!(
                        "Failed to parse router output as valid JSON: {}",
                        res.content
                    ),
                ).with_trace(trace)
            }
            Err(e) => AgentOutput::failure(
                "LLMError",
                &format!("Router failed to communicate with model gateway: {:?}", e),
            ),
        }
    }
}
