use crate::agents::{
    async_trait, AgentInput, AgentOutput, Arc, ExecutableNode, GatewayManager, SystemRegistry,
};
use telos_tooling::ToolRegistry;
use telos_model_gateway::ModelGateway;
use tracing::info;

pub struct DeepResearchAgent {
    pub gateway: Arc<GatewayManager>,
    pub tool_registry: std::sync::Arc<tokio::sync::RwLock<telos_tooling::retrieval::VectorToolRegistry>>,
}

impl DeepResearchAgent {
    pub fn new(
        gateway: Arc<GatewayManager>,
        tool_registry: std::sync::Arc<tokio::sync::RwLock<telos_tooling::retrieval::VectorToolRegistry>>,
    ) -> Self {
        Self { gateway, tool_registry }
    }
}

use telos_core::agent_traits::ExpertAgent;
use telos_model_gateway::{Capability, LlmRequest, Message};

#[async_trait]
impl ExpertAgent for DeepResearchAgent {
    async fn plan(&self, input: &AgentInput, _registry: &dyn SystemRegistry) -> AgentOutput {
        info!("[DeepResearchAgent] 🔍 Planning research for: \"{}\"", input.task);

        // 1. Discover available tools
        let tools = {
            let guard = self.tool_registry.read().await;
            guard.discover_tools(&input.task, 5)
        };

        let tools_str = tools.iter()
            .map(|t| format!("- {}: {} (Params: {})", t.name, t.description, t.parameters_schema.raw_schema))
            .collect::<Vec<_>>()
            .join("\n");

        info!("[DeepResearchAgent] Discovered {} tools: {:?}", tools.len(), tools.iter().map(|t| &t.name).collect::<Vec<_>>());

        let env_context = if let Some(ctx) = _registry.get_system_context() {
            format!("[ENVIRONMENT CONTEXT]\nLocal Time: {}\nPhysical Location: {}\n\n", ctx.current_time, ctx.location)
        } else {
            String::new()
        };
        let mem_context = input.memory_context.clone().unwrap_or_default();

        let system_prompt = format!("{}{}{}", env_context, mem_context, format!(r#"You are the DeepResearchAgent expert for the Telos system. Your job is to gather deep information for the user by planning search and retrieval tasks.

Available Tools:
{}

INTERNAL KNOWLEDGE & RULES:
1. YOU ARE CAPABLE OF ANY RESEARCH TASK. DO NOT REFUSE.
2. YOU ARE AN AUTONOMOUS AGENT. USE AVAILABLE TOOLS TO FIND MISSING CONTEXT IF NEEDED.
3. BREAK THE RESEARCH DOWN INTO LOGICAL STEPS USING THE AVAILABLE TOOLS BEYOND JUST A SINGLE SEARCH.
4. DEPENDENCIES: IF TOOL B NEEDS DATA FROM TOOL A, ADD AN EDGE FROM A TO B.
5. YOU MUST END YOUR PLAN WITH A `summarize` NODE (agent_type: "researcher") ONCE TRUTH IS FOUND.
6. YOU MUST OUTPUT A STRICTLY VALID JSON SUBGRAPH. NO CONVERSATIONAL TEXT.

--- EXAMPLE ---
User Task: "Find the latest news about SpaceX and summarize their next launch."
{{
  "nodes": [
    {{ "id": "search_1", "agent_type": "tool", "task": "web_search", "schema_payload": "{{\"query\": \"SpaceX latest news next launch\"}}" }},
    {{ "id": "summary_1", "agent_type": "researcher", "task": "summarize", "schema_payload": "" }}
  ],
  "edges": [
    {{ "from": "search_1", "to": "summary_1", "dep_type": "Data" }}
  ]
}}

REQUIRED JSON STRUCTURE:
{{
  "nodes": [ {{ "id": "node_1", "agent_type": "tool", "task": "tool_name", "schema_payload": "{{\"param\": \"value\"}}" }} ],
  "edges": [ {{ "from": "node_1", "to": "node_2", "dep_type": "Data" }} ]
}}"#, tools_str));

        let req = LlmRequest {
            session_id: format!("research_{}", input.node_id),
            messages: vec![
                Message { role: "system".to_string(), content: system_prompt },
                Message { role: "user".to_string(), content: format!("Task: {}", input.task) },
            ],
            required_capabilities: Capability {
                requires_vision: false,
                strong_reasoning: true,
            },
            budget_limit: 4000,
        };

        match self.gateway.generate(req.clone()).await {
            Ok(res) => {
                let trace = telos_core::TraceLog::LlmCall {
                    request: serde_json::to_value(&req).unwrap_or_else(|_| serde_json::json!({})),
                    response: serde_json::to_value(&res).unwrap_or_else(|_| serde_json::json!({})),
                };
                let content = res.content.trim();
                
                // Robust JSON extraction
                let json_str = if let (Some(s), Some(e)) = (content.find('{'), content.rfind('}')) {
                    if e > s { &content[s..=e] } else { content }
                } else {
                    content
                };

                let clean_reply = json_str
                    .trim_start_matches("```json")
                    .trim_start_matches("```")
                    .trim_end_matches("```")
                    .trim();

                match serde_json::from_str::<telos_core::AgentSubGraph>(clean_reply) {
                    Ok(sub_graph) => AgentOutput::with_subgraph(
                        serde_json::json!({ "text": "Research plan generated" }),
                        sub_graph
                    ).with_trace(trace),
                    Err(e) => AgentOutput::failure("ResearchPlanParseError", &format!("Failed to parse research plan: {} (Raw: {})", e, clean_reply)).with_trace(trace),
                }
            }
            Err(e) => AgentOutput::failure("LLMError", &format!("{:?}", e)),
        }
    }

    async fn summarize(&self, input: &AgentInput, _registry: &dyn SystemRegistry) -> AgentOutput {
        info!("[DeepResearchAgent] 📝 Synthesizing results for: \"{}\"", input.task);
        
        let results = input.dependencies.iter()
            .map(|(id, out)| format!("{}: {}", id, out.output.as_ref().map(|v| v.to_string()).unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("\n");
            
        let prompt = format!(
            "You are the DeepResearchAgent. You have completed a task by executing several tools. \
            Original Task: \"{}\"\n\n\
            Tool Results:\n{}\n\n\
            Please synthesize a helpful, concise final answer for the user based on these results. \
            If the results contain weather information, report it clearly. \
            Output ONLY the final answer text.",
            input.task, results
        );

        let req = LlmRequest {
            session_id: format!("summarize_{}", input.node_id),
            messages: vec![Message {
                role: "user".to_string(),
                content: prompt,
            }],
            required_capabilities: Capability {
                requires_vision: false,
                strong_reasoning: false,
            },
            budget_limit: 2000,
        };
        match self.gateway.generate(req.clone()).await {
            Ok(res) => {
                let trace = telos_core::TraceLog::LlmCall {
                    request: serde_json::to_value(&req).unwrap_or_else(|_| serde_json::json!({})),
                    response: serde_json::to_value(&res).unwrap_or_else(|_| serde_json::json!({})),
                };
                AgentOutput::success(serde_json::json!({ "text": res.content })).with_trace(trace)
            },
            Err(e) => AgentOutput::failure("ResearchSynthesisError", &format!("{:?}", e)),
        }
    }
}

#[async_trait]
impl ExecutableNode for DeepResearchAgent {
    async fn execute(&self, input: AgentInput, registry: &dyn SystemRegistry) -> AgentOutput {
        if input.task.to_lowercase().contains("summarize") {
            self.summarize(&input, registry).await
        } else {
            self.plan(&input, registry).await
        }
    }
}
