use async_trait::async_trait;
use tracing::info;
use std::sync::Arc;
use telos_core::{AgentInput, AgentOutput, AgentSubGraph, SystemRegistry, agent_traits::ExpertAgent};
use telos_dag::ExecutableNode;
use telos_model_gateway::gateway::GatewayManager;
use telos_model_gateway::{Capability, LlmRequest, Message, ModelGateway};
use crate::agents::{from_gateway_error, parse_failure};

pub struct ArchitectAgent {
    gateway: Arc<GatewayManager>,
}

impl ArchitectAgent {
    pub fn new(gateway: Arc<GatewayManager>) -> Self {
        Self { gateway }
    }
}

#[async_trait]
impl ExpertAgent for ArchitectAgent {
    async fn plan(&self, input: &AgentInput, _registry: &dyn SystemRegistry) -> AgentOutput {
        info!(
            "[ArchitectAgent] 🏗️  Architect is planning for task: \"{}\"",
            input.task
        );

        // Build context from dependencies if the architect is replanning or invoked deep in a graph
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

        let env_context = if let Some(sys_ctx) = _registry.get_system_context() {
            format!("[ENVIRONMENT CONTEXT]\nLocal Time: {}\nPhysical Location: {}\n\n", sys_ctx.current_time, sys_ctx.location)
        } else {
            String::new()
        };
        let mem_context = input.memory_context.clone().unwrap_or_default();

        let system_prompt = format!("{}{}{}", env_context, mem_context, r#"You are the ArchitectAgent, a master system planner.
Your goal is to decompose the user's complex task into a Directed Acyclic Graph (DAG) of micro-tasks.
You must use Maximal Agentic Decomposition (MAD) to assign precise, scoped tasks to specialized expert agents.

Available Expert Agents (`agent_type`):
- "coder": A pure executor that writes code or bash scripts. Expects `instructions` and `file_path`.
- "reviewer": Reviews code/text against rules. Expects `content_to_review` and `review_criteria`.
- "tester": Writes and executes adversarial tests.
- "researcher": Finds information and synthesizes it.
- "general": A general-purpose step-by-step assistant.

Rules for planning:
1. Break down the task into sequential or parallel nodes.
2. Give each node a unique short string `id` (e.g., "node_1").
3. Assign an `agent_type` from the list above.
4. Provide a clear, detailed `task` description for each node. This will be the human-readable description.
5. Provide `schema_payload`, which must be a string containing a valid JSON object with the strict parameters required by that specific worker type. (e.g. "{\"instructions\": \"...\"}")
6. Define `edges` where `from` node must complete before `to` node starts. Use `dep_type` as "Data" or "Control".

Output exactly a JSON object matching this schema:
{
  "nodes": [ { "id": "...", "agent_type": "...", "task": "...", "schema_payload": "..." } ],
  "edges": [ { "from": "...", "to": "...", "dep_type": "Data" } ]
}
Do not include markdown wrappers if possible, just the raw JSON."#);

        let prompt = format!(
            "System: {}\n\nUser Task:\n{}{}\n\nPlease generate the SubGraph plan.",
            system_prompt, input.task, deps_context
        );

        let req = LlmRequest {
            session_id: format!("architect_{}", input.node_id),
            messages: vec![Message {
                role: "user".to_string(),
                content: prompt,
            }],
            required_capabilities: Capability {
                requires_vision: false,
                strong_reasoning: true, // Architect prefers strong reasoning for planning MAD
            },
            budget_limit: 128_000,
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

                let clean_json = json_str
                    .trim_start_matches("```json")
                    .trim_start_matches("```")
                    .trim_end_matches("```")
                    .trim();

                match serde_json::from_str::<AgentSubGraph>(clean_json) {
                    Ok(mut sub_graph) => {
                        info!(
                            "[ArchitectAgent] ✅ Decomposition complete. Generated {} nodes.",
                            sub_graph.nodes.len()
                        );
                        // Modify agent types to match Tier 3 Worker types if needed
                        for node in &mut sub_graph.nodes {
                            if node.agent_type == "coder" {
                                node.agent_type = "coder".to_string(); 
                            }
                        }
                        
                        for (i, node) in sub_graph.nodes.iter().enumerate() {
                            info!("  └─ Node {}: [{}] {}", i + 1, node.agent_type, node.id);
                        }
                        AgentOutput::with_subgraph(
                            serde_json::json!({
                                "text": format!("SubGraph decomposition complete with {} nodes", sub_graph.nodes.len())
                            }),
                            sub_graph,
                        ).with_trace(trace)
                    }
                    Err(e) => parse_failure(
                        "ArchitectParseError",
                        &format!("无法解析规划输出: {}", e),
                        &res.content,
                    ).with_trace(trace),
                }
            }
            Err(e) => from_gateway_error(e, "规划生成失败"),
        }
    }

    async fn summarize(&self, _input: &AgentInput, _registry: &dyn SystemRegistry) -> AgentOutput {
        let mut final_text = String::new();
        
        for (node_id, output) in &_input.dependencies {
            if let Some(val) = &output.output {
                if let Some(text) = val.get("text").and_then(|v| v.as_str()) {
                    final_text.push_str(&format!("--- Output from {} ---\n{}\n\n", node_id, text));
                } else if let Some(code) = val.get("code").and_then(|v| v.as_str()) {
                    final_text.push_str(&format!("--- Code from {} ---\n{}\n\n", node_id, code));
                } else {
                    final_text.push_str(&format!("--- Result from {} ---\n{}\n\n", node_id, val));
                }
            }
        }
        
        let mut final_text = final_text.trim().to_string();
        if final_text.is_empty() {
             final_text = "Architect successfully orchestrated the workflow, but no explicit text output was returned by sub-nodes.".to_string();
        }

        AgentOutput::success(serde_json::json!({
            "text": final_text
        }))
    }
}

#[async_trait]
impl ExecutableNode for ArchitectAgent {
    async fn execute(&self, input: AgentInput, _registry: &dyn SystemRegistry) -> AgentOutput {
        self.plan(&input, _registry).await
    }
}
