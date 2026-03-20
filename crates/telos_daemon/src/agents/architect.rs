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

        let mut template_context = String::new();
        let mut reused_template_count = 0usize;
        if let Some(mem_any) = _registry.get_memory_os() {
            if let Ok(mem_os) = mem_any.clone().downcast::<std::sync::Arc<dyn telos_memory::engine::MemoryOS>>() {
                use telos_memory::integration::MemoryIntegration;
                if let Ok(templates) = mem_os.retrieve_procedural_memories(input.task.clone()).await {
                    if !templates.is_empty() {
                        reused_template_count = templates.len();
                        template_context = format!("[LEARNED WORKFLOW TEMPLATES & STRATEGIES]\nIf any of the following procedural templates match the user's current goal, you MUST reuse its node topology (agent types, edges). You must instantiate the template by replacing specific placeholder arguments (like filenames or test targets) with the parameters from the current task.\n\n{}\n\n", templates.join("\n---\n"));
                    }
                }
            }
        }

        let system_prompt = format!("{}{}{}{}", env_context, mem_context, template_context, r#"You are the ArchitectAgent, a master system planner.
Your goal is to decompose the user's complex task into a Directed Acyclic Graph (DAG) of micro-tasks.
You must use Maximal Agentic Decomposition (MAD) to assign precise, scoped tasks to specialized expert agents.

EXTREMELY IMPORTANT TOOL REFLECTION RULE:
If the user's task requires connecting to an API, fetching data, or performing a specific action, you MUST FIRST consider if the built-in tools can achieve this.
ONLY if no combination of existing tools can solve the problem are you allowed to instruct the `coder` to write a new "Dynamic Tool" (`.rhai` script).
If you do instruct the creation of a new tool, your task description MUST start with the phrase: "[TOOL_REFLECTION] Existing tools cannot solve this because... Therefore I am creating a new tool."

Available Expert Agents (`agent_type`):
- "coder": A pure executor that writes code. If instructed to build a tool, it will write a Rhai script.
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
CRITICAL FORMAT REQUIREMENT: 
Your total response MUST be a single valid JSON object exactly matching the schema. DO NOT output any markdown (no ```json ... ```), NO conversational text, NO apologies, NO thinking process. ONLY JSON."#);

        let mut messages = vec![
            Message { role: "system".to_string(), content: system_prompt },
        ];
        for msg in &input.conversation_history {
            messages.push(Message {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
        }
        messages.push(Message {
            role: "user".to_string(),
            content: format!("User Task:\n{}{}\n\nPlease generate the SubGraph plan.", input.task, deps_context),
        });

        let req = LlmRequest {
            session_id: format!("architect_{}", input.node_id),
            messages,
            required_capabilities: Capability {
                requires_vision: false,
                strong_reasoning: true, // Architect prefers strong reasoning for planning MAD
            },
            budget_limit: 128_000,
            tools: None,
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
                        // Fortify agent types against LLM hallucinations
                        for node in &mut sub_graph.nodes {
                            let valid_types = ["coder", "reviewer", "tester", "researcher", "general"];
                            if !valid_types.contains(&node.agent_type.as_str()) {
                                // Fallback mapping based on task context
                                let task_lower = node.task.to_lowercase();
                                if task_lower.contains("tool") || task_lower.contains("script") || task_lower.contains("code") {
                                    tracing::warn!("[ArchitectAgent] Rewriting hallucinated agent_type '{}' to 'coder'", node.agent_type);
                                    node.agent_type = "coder".to_string();
                                } else {
                                    tracing::warn!("[ArchitectAgent] Rewriting hallucinated agent_type '{}' to 'general'", node.agent_type);
                                    node.agent_type = "general".to_string();
                                }
                            }
                        }
                        
                        for (i, node) in sub_graph.nodes.iter().enumerate() {
                            info!("  └─ Node {}: [{}] {}", i + 1, node.agent_type, node.id);
                        }
                        AgentOutput::with_subgraph(
                            serde_json::json!({
                                "text": format!("SubGraph decomposition complete with {} nodes", sub_graph.nodes.len()),
                                "reused_workflow_count": reused_template_count,
                            }),
                            sub_graph,
                        ).with_trace(trace)
                    }
                    Err(e) => {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(clean_json) {
                            if let Some(tool) = val.get("tool").and_then(|t| t.as_str()) {
                                let sub_node = telos_core::SubGraphNode {
                                    id: format!("direct_call_{}", tool),
                                    agent_type: "tool".to_string(),
                                    task: tool.to_string(), // ToolNode expects task to be EXACTLY the tool name
                                    schema_payload: serde_json::to_string(&val).unwrap_or_default(),
                                    loop_config: None,
                                    is_critic: false,
                                };
                                let sub_graph = telos_core::AgentSubGraph {
                                    nodes: vec![sub_node],
                                    edges: vec![],
                                };
                                return AgentOutput::with_subgraph(
                                    serde_json::json!({ "text": "SubGraph decomposition complete with 1 node" }),
                                    sub_graph
                                ).with_trace(trace);
                            }
                        }
                        parse_failure(
                            "ArchitectParseError",
                            &format!("无法解析规划输出: {}", e),
                            &res.content,
                        ).with_trace(trace)
                    }
                }
            }
            Err(e) => from_gateway_error(e, "规划生成失败"),
        }
    }

    async fn summarize(&self, input: &AgentInput, _registry: &dyn SystemRegistry) -> AgentOutput {
        // 1. Collect raw results from dependencies (injected by main.rs from DAG terminal nodes)
        let mut raw_results = Vec::new();
        for (node_id, output) in &input.dependencies {
            if let Some(val) = &output.output {
                let text = val.get("text").and_then(|v| v.as_str())
                    .or_else(|| val.get("code").and_then(|v| v.as_str()))
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| val.to_string());
                if !text.is_empty() {
                    raw_results.push((node_id.clone(), text));
                }
            }
        }

        if raw_results.is_empty() {
            return AgentOutput::success(serde_json::json!({
                "text": "任务已执行，但子节点未产生文本输出。"
            }));
        }

        // 2. If there's only one result and it's already substantial, pass through directly
        if raw_results.len() == 1 {
            let (_, text) = &raw_results[0];
            // Strip JSON wrapping like [node_id] prefix if present
            let clean = text.trim_start_matches('[')
                .find(']')
                .map(|i| text[i+1..].trim())
                .unwrap_or(text.as_str());
            // If it looks like already formatted content, pass through
            if clean.len() > 50 {
                return AgentOutput::success(serde_json::json!({"text": clean}));
            }
        }

        // 3. Use LLM to synthesize multiple results into a user-facing response
        let env_context = if let Some(ctx) = _registry.get_system_context() {
            format!("[ENVIRONMENT CONTEXT]\nLocal Time: {}\nPhysical Location: {}\n\n", ctx.current_time, ctx.location)
        } else {
            String::new()
        };

        let results_text = raw_results.iter()
            .map(|(id, text)| format!("[{}]:\n{}", id, text))
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");

        let system_prompt = format!("{}You are a helpful AI assistant. Your task is to synthesize execution results into a clear, well-formatted response for the user.\n\nRules:\n- Output the final content DIRECTLY — do not say \"based on the results\" or \"the execution produced\"\n- Preserve code blocks, tables, and formatting from the source content\n- If the content is code, present it properly with explanations\n- Use the user's language (Chinese if the request is in Chinese)\n- Be concise but complete", env_context);

        let mut messages = vec![
            Message { role: "system".to_string(), content: system_prompt },
        ];
        for msg in &input.conversation_history {
            messages.push(Message {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
        }
        messages.push(Message {
            role: "user".to_string(),
            content: format!(
                "用户请求: {}\n\n执行节点输出:\n{}\n\n请整合为面向用户的完整回复。",
                input.task, results_text
            ),
        });

        let req = LlmRequest {
            session_id: format!("architect_summary_{}", input.node_id),
            messages,
            required_capabilities: Capability {
                requires_vision: false,
                strong_reasoning: false,
            },
            budget_limit: 4000,
            tools: None,
        };

        match self.gateway.generate(req).await {
            Ok(res) => {
                let content = res.content.trim().to_string();
                if content.is_empty() {
                    // Fallback: return raw results
                    let fallback = raw_results.iter()
                        .map(|(_, text)| text.as_str())
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    AgentOutput::success(serde_json::json!({"text": fallback}))
                } else {
                    AgentOutput::success(serde_json::json!({"text": content}))
                }
            }
            Err(_) => {
                // LLM failed, fall back to raw concatenation
                let fallback = raw_results.iter()
                    .map(|(_, text)| text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                AgentOutput::success(serde_json::json!({"text": fallback}))
            }
        }
    }
}

#[async_trait]
impl ExecutableNode for ArchitectAgent {
    async fn execute(&self, input: AgentInput, _registry: &dyn SystemRegistry) -> AgentOutput {
        self.plan(&input, _registry).await
    }
}
