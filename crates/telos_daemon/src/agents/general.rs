use crate::agents::{
    async_trait, AgentInput, AgentOutput, Arc, ExecutableNode, GatewayManager, SystemRegistry,
};
use crate::agents::prompt_builder::PromptBuilder;
use telos_tooling::ToolRegistry;
use telos_model_gateway::ModelGateway;
use tracing::info;

pub struct GeneralAgent {
    pub gateway: Arc<GatewayManager>,
    pub tool_registry:
        std::sync::Arc<tokio::sync::RwLock<telos_tooling::retrieval::VectorToolRegistry>>,
    pub tools_dir: String,
}

impl GeneralAgent {
    pub fn new(
        gateway: Arc<GatewayManager>,
        tool_registry: std::sync::Arc<
            tokio::sync::RwLock<telos_tooling::retrieval::VectorToolRegistry>,
        >,
        tools_dir: String,
    ) -> Self {
        Self {
            gateway,
            tool_registry,
            tools_dir,
        }
    }
}

use telos_core::agent_traits::ExpertAgent;

#[async_trait]
impl ExpertAgent for GeneralAgent {
    async fn plan(&self, input: &AgentInput, _registry: &dyn SystemRegistry) -> AgentOutput {
        info!("[GeneralAgent] 🤖 Planning for task: \"{}\"", input.task);

        // 1. Discover available tools (use Router's auto-discovered tools if passed via schema_payload)
        let tools = {
            let mut extracted_tools = None;
            if let Some(ref payload) = input.schema_payload {
                if let Ok(schemas) = serde_json::from_str::<Vec<telos_tooling::ToolSchema>>(payload) {
                    extracted_tools = Some(schemas);
                }
            }
            if let Some(t) = extracted_tools {
                t
            } else {
                let guard = self.tool_registry.read().await;
                guard.discover_tools(&input.task, 5)
            }
        };
        info!("[GeneralAgent] Discovered {} tools: {:?}", tools.len(), tools.iter().map(|t| &t.name).collect::<Vec<_>>());

        // 2. Build system prompt using PromptBuilder (lazy tool loading)
        let soul_content = crate::agents::prompt_builder::get_soul();
        let persona_prefix = format!("[IDENTITY & VALUES]\n{}\n\nYou MUST respond as the persona described above. NEVER call yourself 'GeneralAgent' or reveal internal agent names.\n\n", soul_content);

        let role_instructions = format!(r#"{}You are an internal planning engine for the Telos system. Your job is to solve the user's task by planning a sequence of tool calls.

SPECIAL AGENT TYPES:
- agent_type "search_worker": An intelligent search agent with built-in web scraping. It supports two modes via schema_payload:
  • mode "direct": Single search + auto-scrapes top URLs if snippets are insufficient. Use for: weather, stock prices, specific news.
  • mode "deep": Full pipeline — keyword engineering, multi-query, URL dedup, auto-scrape top 3 pages. Use for: complex research, browsing specific sites.
  YOU decide the mode based on task complexity. Use AT MOST 1-2 search_worker nodes total.
  NOTE: search_worker already includes web scraping internally. DO NOT plan separate web_scrape nodes.
  KEYWORD HINTS: Include suggested search keywords in the "task" field: "Find Apple stock price — keywords: AAPL stock price today, Apple share price"

- agent_type "coder": A ReAct agent that can use tools iteratively, including `create_rhai_tool`. Use this when:
  • The task requires CREATING A NEW TOOL via `create_rhai_tool` (the user says "制作/创建工具", "make/create/build a tool")
  • The task requires multi-step tool usage with reasoning between steps
  • Complex file operations (read → analyze → edit)
  • The task involves ANY native/system tool: shell_exec, schedule_mission, list_scheduled_missions, cancel_mission, fs_read, fs_write, file_edit, web_search

- agent_type "tool": Directly executes a CUSTOM Rhai tool by name. Use schema_payload to pass the tool name and parameters.
  IMPORTANT: This can ONLY execute custom tools created via create_rhai_tool. It CANNOT access native system tools.
  If your plan needs schedule_mission, list_scheduled_missions, cancel_mission, shell_exec, or any native tool, you MUST use agent_type "coder" instead.

TOOL-FIRST PRINCIPLE:
Check the "Available Tools" section above. If a previously-created custom tool (e.g., weather_tool, currency_tool) already exists that matches the user's intent, prefer using it via agent_type "tool" — it's more efficient than searching because the tool was specifically built for this purpose.

PLANNING PHILOSOPHY — Think like a senior engineer:
When you plan a DAG, think about each node's responsibility. A well-designed plan separates concerns:
- A "coder" node should focus on CREATING code/tools, not on performing business calculations.
- A "tool" node should focus on EXECUTING a registered tool and returning its result.
- A "search_worker" node should focus on FINDING information from the web.
- A "general" (summarize) node should focus on SYNTHESIZING results for the user.

Consider these heuristics when planning:
1. **Reusability**: Is this a one-off query, or something the user might ask again? If the user asks "查苏州天气", you could use search_worker directly. But if you sense the user values efficiency or might ask again, consider creating a lightweight tool first and then executing it — this builds long-term capability.
2. **Decomposition**: When a task involves both "making something" and "using it", prefer splitting into separate nodes connected by edges. This gives the user clear visibility into each step and makes failures easier to diagnose.
3. **Implicit intent**: The user may not explicitly say "create a tool". If you judge that a reusable tool would serve them better than a one-off search, you're empowered to plan tool creation proactively. Good judgment here is what separates a great planner from a mediocre one.
4. **DAG guarantees sequential execution**: When Node B depends on Node A via an edge, Node A is fully completed before Node B starts. This means a "tool" node CAN safely reference a tool that was created by an earlier "coder" node — the tool will exist by the time it's needed.

PROTOCOL RULES (MANDATORY — these are structural contracts, not suggestions):
1. You ARE capable of any task. Do not refuse.
2. Your plan MUST be a valid DAG of nodes with edges.
3. You MUST end with a `summarize` node (agent_type: "general").
4. You MUST output strictly valid JSON. No conversational text outside the JSON.
5. If a previous tool attempt failed, construct an altered DAG with different/simpler queries.

--- EXAMPLE: Using an existing custom tool ---
Available Tools include: weather_tool (Fetches weather for a given city)
User Task: "苏州今天天气怎么样"
{{
  "nodes": [
    {{ "id": "tool_1", "agent_type": "tool", "task": "Execute tool: weather_tool", "schema_payload": "{{\"tool\": \"weather_tool\", \"params\": {{\"city\": \"Suzhou\"}}}}" }},
    {{ "id": "summary_1", "agent_type": "general", "task": "summarize", "schema_payload": "" }}
  ],
  "edges": [
    {{ "from": "tool_1", "to": "summary_1", "dep_type": "Data" }}
  ]
}}

--- EXAMPLE: Creating a new tool (with immediate usage) ---
User Task: "帮我制作一个查天气的工具，然后查苏州天气"
{{
  "nodes": [
    {{ "id": "create_tool_1", "agent_type": "coder", "task": "Create a weather query tool using create_rhai_tool. The tool should use http_get_with_fallback to fetch weather data from wttr.in API. Name it weather_oracle.", "schema_payload": "" }},
    {{ "id": "use_tool_1", "agent_type": "tool", "task": "Execute weather_oracle", "schema_payload": "{{\"tool\": \"weather_oracle\", \"params\": {{\"city\": \"Suzhou\"}}}}" }},
    {{ "id": "summary_1", "agent_type": "general", "task": "summarize", "schema_payload": "" }}
  ],
  "edges": [
    {{ "from": "create_tool_1", "to": "use_tool_1", "dep_type": "Execution" }},
    {{ "from": "use_tool_1", "to": "summary_1", "dep_type": "Data" }}
  ]
}}
NOTE: Node `use_tool_1` correctly waits for `create_tool_1` to finish before executing.

--- EXAMPLE: Search (no custom tool available) ---
User Task: "What's the current stock price of Apple?"
{{
  "nodes": [
    {{ "id": "search_1", "agent_type": "search_worker", "task": "Find the current stock price of Apple — keywords: AAPL stock price today, Apple share price", "schema_payload": "{{\"mode\":\"direct\"}}" }},
    {{ "id": "summary_1", "agent_type": "general", "task": "summarize", "schema_payload": "" }}
  ],
  "edges": [
    {{ "from": "search_1", "to": "summary_1", "dep_type": "Data" }}
  ]
}}

REQUIRED JSON STRUCTURE:
{{
  "nodes": [ {{ "id": "node_1", "agent_type": "search_worker|tool|coder|general", "task": "descriptive task", "schema_payload": "" }} ],
  "edges": [ {{ "from": "node_1", "to": "node_2", "dep_type": "Data" }} ]
}}"#, persona_prefix);

        let all_tools = {
            let guard = self.tool_registry.read().await;
            guard.list_all_tools()
        };

        let system_prompt = PromptBuilder::new()
            .with_identity()
            .with_environment(_registry)
            .with_memory(&input.memory_context)
            .with_default_core_tools(&all_tools, &tools)
            .with_role_instructions(&role_instructions)
            .build();

        let mut messages = vec![
            Message { role: "system".to_string(), content: system_prompt },
        ];
        for msg in &input.conversation_history {
            messages.push(Message {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
        }
        messages.push(Message { role: "user".to_string(), content: format!("Task: {}\n\nCRITICAL CONSTRAINT: You MUST output ONLY a valid JSON object matching the requested schema. DO NOT output any other conversational text or formatting. If you cannot provide a JSON plan, still output a valid JSON containing a tool or summary node.", input.task) });

        let req = LlmRequest {
            session_id: format!("general_{}", input.node_id),
            messages,
            required_capabilities: Capability {
                requires_vision: false,
                strong_reasoning: true,
            },
            budget_limit: 4000,
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

                let clean_reply = json_str
                    .trim_start_matches("```json")
                    .trim_start_matches("```")
                    .trim_end_matches("```")
                    .trim();

                match serde_json::from_str::<telos_core::AgentSubGraph>(clean_reply) {
                    Ok(sub_graph) => AgentOutput::with_subgraph(
                        serde_json::json!({ "text": "Plan generated by GeneralAgent" }),
                        sub_graph
                    ).with_trace(trace),
                    Err(e) => {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(clean_reply) {
                            if let Some(tool) = val.get("tool").and_then(|t| t.as_str()) {
                                let sub_node = telos_core::SubGraphNode {
                                    id: format!("direct_call_{}", tool),
                                    agent_type: "tool".to_string(),
                                    task: format!("Execute tool: {}", tool),
                                    schema_payload: serde_json::to_string(&val).unwrap_or_default(),
                                    loop_config: None,
                                    is_critic: false,
                                };
                                let sub_graph = telos_core::AgentSubGraph {
                                    nodes: vec![sub_node],
                                    edges: vec![],
                                };
                                return AgentOutput::with_subgraph(
                                    serde_json::json!({ "text": "Plan generated by GeneralAgent" }),
                                    sub_graph
                                ).with_trace(trace);
                            }
                        }
                        AgentOutput::failure("PlanParseError", &format!("Failed to parse plan: {} (Raw: {})", e, clean_reply)).with_trace(trace)
                    }
                }
            }
            Err(e) => AgentOutput::failure("LLMError", &format!("{:?}", e)),
        }
    }

    async fn summarize(&self, input: &AgentInput, _registry: &dyn SystemRegistry) -> AgentOutput {
        info!("[GeneralAgent] 📝 Synthesizing results for: \"{}\"", input.task);
        
        // --- RELEVANCE GATE: filter out irrelevant dependency results before synthesis ---
        // Smart extraction: convert structured JSON outputs into clean, readable text
        // for the summarizer LLM. Supports multiple output formats from different agents.
        let raw_results: Vec<(String, String)> = input.dependencies.iter()
            .map(|(id, out)| {
                let content = out.output.as_ref().map(|v| {
                    Self::extract_readable_output(v)
                }).unwrap_or_else(|| {
                    // No output value — check if there's an error message
                    if let Some(ref err) = out.error {
                        let mut s = format!("[Error] {}: {}", err.error_type, err.message);
                        if let Some(ref td) = err.technical_detail {
                            s.push_str(&format!("\nTechnical Detail: {}", td));
                        }
                        s
                    } else {
                        "[No output]".to_string()
                    }
                });
                (id.clone(), content)
            })
            .collect();

        let mut filtered_results = Vec::new();
        let mut irrelevant_notes = Vec::new();

        if raw_results.len() > 1 {
            // Use a single lightweight LLM call to classify all results at once
            let classify_items: Vec<String> = raw_results.iter().enumerate()
                .map(|(i, (id, content))| {
                    let truncated: String = content.chars().take(300).collect();
                    format!("[{}] {}: {}", i, id, truncated)
                })
                .collect();
            let classify_prompt = format!(
                "Task: \"{}\"\n\nFor each item below, respond with ONLY a JSON array of booleans indicating if the item is relevant to the task.\n\n{}\n\nOutput: [true/false, ...]",
                input.task, classify_items.join("\n")
            );
            let classify_req = telos_model_gateway::LlmRequest {
                session_id: format!("relevance_gate_{}", input.node_id),
                messages: vec![
                    telos_model_gateway::Message { role: "system".to_string(), content: "You are a relevance classifier. Output ONLY a JSON array of booleans.".to_string() },
                    telos_model_gateway::Message { role: "user".to_string(), content: classify_prompt },
                ],
                required_capabilities: telos_model_gateway::Capability { requires_vision: false, strong_reasoning: false },
                budget_limit: 300,
                tools: None,
            };

            let relevance_flags: Vec<bool> = match self.gateway.generate(classify_req).await {
                Ok(res) => {
                    let content = res.content.trim();
                    // Extract JSON array
                    if let (Some(s), Some(e)) = (content.find('['), content.rfind(']')) {
                        serde_json::from_str(&content[s..=e]).unwrap_or_else(|_| vec![true; raw_results.len()])
                    } else {
                        vec![true; raw_results.len()] // default: all relevant
                    }
                }
                Err(_) => vec![true; raw_results.len()], // default: all relevant on error
            };

            for (i, (id, content)) in raw_results.iter().enumerate() {
                let is_relevant = relevance_flags.get(i).copied().unwrap_or(true);
                if is_relevant {
                    filtered_results.push(format!("{}: {}", id, content));
                } else {
                    info!("[GeneralAgent] Relevance Gate filtered out result from '{}' as irrelevant", id);
                    irrelevant_notes.push(format!("[Note: Result from '{}' was filtered as irrelevant to the task]", id));
                }
            }
        } else {
            // Single result: skip classification overhead
            for (id, content) in &raw_results {
                filtered_results.push(format!("{}: {}", id, content));
            }
        }

        let results = if filtered_results.is_empty() && !irrelevant_notes.is_empty() {
            format!("[ALL search results were irrelevant to the task. No useful data retrieved.]\n{}", irrelevant_notes.join("\n"))
        } else {
            let mut r = filtered_results.join("\n");
            if !irrelevant_notes.is_empty() {
                r.push_str(&format!("\n{}", irrelevant_notes.join("\n")));
            }
            r
        };
            
        let prompt = format!(
            "You are the GeneralAgent. You have completed a task by executing several tools. \
            Original Task: \"{}\"\n\n\
            Tool Results:\n{}\n\n\
            Please synthesize a helpful, COMPLETE final answer for the user based on these results. \
            IMPORTANT: Preserve the full detail and structure of the results. \
            If the original task asks for a plan, itinerary, list, or report, include ALL items with their details — do NOT compress them into one sentence. \
            Only remove truly redundant or repetitive information. \
            [TOOL OUTPUT INTERPRETATION]: Tool results may be in various formats (JSON, plain text, raw API response). \
            If a tool returned raw text or unstructured data, EXTRACT and INTERPRET the relevant information intelligently. \
            If a tool was successfully created or updated, report that clearly (e.g., 'Tool get_weather has been successfully created/updated'). \
            [ERROR TRANSPARENCY — CRITICAL]: If ANY tool execution failed, encountered errors, or returned error messages, \
            you MUST include the FULL error details in your response. Do NOT hide, minimize, or vaguely summarize errors. \
            Specifically: (1) State which tool/step failed, (2) Include the exact error message, (3) Describe what the error means. \
            If a tool creation (create_rhai_tool) failed with syntax errors, include the specific syntax error message and line number. \
            The user NEEDS to see these details to understand and fix problems. NEVER say just 'an error occurred' — be specific. \
            [ABSOLUTE PROHIBITION]: Your final answer MUST be substantive text content, NOT a bare URL or list of URLs. If the tool results contain scraped web page content with actual data (weather, prices, news articles, etc.), EXTRACT and PRESENT that data as readable text. \
            [CRITICAL CONSTRAINT]: Filter the Tool Results STRICTLY against the Original Task constraints (especially time/date/location). \
            If the retrieved data is macroscopic, irrelevant SEO garbage, or completely empty, EXPLICITLY state the specific data deficiency directly (e.g., 'No specific data found for this context') INSTEAD of hallucinating misaligned fluff or summarizing generic information. \
            If the results contain weather information, report it clearly. \
            Output ONLY the final answer text.",
            input.task, results
        );

        let env_context = if let Some(ctx) = _registry.get_system_context() {
            format!("[ENVIRONMENT CONTEXT]\nLocal Time: {}\nPhysical Location: {}\n\n", ctx.current_time, ctx.location)
        } else {
            String::new()
        };
        let mem_context = input.memory_context.clone().unwrap_or_default();
        let soul_content2 = crate::agents::prompt_builder::get_soul();
        let system_prompt = format!("{}{}{}[IDENTITY & VALUES]\n{}\n\nYou MUST respond as the persona described above. NEVER call yourself 'GeneralAgent' or reveal internal agent names. Synthesize results into a natural, persona-consistent response.", env_context, mem_context, if mem_context.is_empty() {""} else {"\n\n"}, soul_content2);

        let mut messages = vec![
            Message { role: "system".to_string(), content: system_prompt },
        ];
        for msg in &input.conversation_history {
            messages.push(Message {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
        }
        messages.push(Message { role: "user".to_string(), content: prompt });

        let req = LlmRequest {
            session_id: format!("summarize_{}", input.node_id),
            messages,
            required_capabilities: Capability {
                requires_vision: false,
                strong_reasoning: false,
            },
            budget_limit: 4000,
            tools: None,
        };
        match self.gateway.generate(req.clone()).await {
            Ok(res) => {
                let trace = telos_core::TraceLog::LlmCall {
                    request: serde_json::to_value(&req).unwrap_or_else(|_| serde_json::json!({})),
                    response: serde_json::to_value(&res).unwrap_or_else(|_| serde_json::json!({})),
                };
                AgentOutput::success(serde_json::json!({ "text": res.content })).with_trace(trace)
            },
            Err(e) => AgentOutput::failure("SynthesisError", &format!("{:?}", e)),
        }
    }
}

impl GeneralAgent {
    /// Intelligently extract readable text from a dependency's JSON output.
    ///
    /// Priority chain:
    /// 1. If the value has a "text" key → use that (standard agent output format)
    /// 2. If the value is a JSON object → flatten all human-readable string fields,
    ///    skipping internal metadata keys (react_meta, trace_logs, etc.)
    /// 3. If the value is a plain string/number/bool → use it directly
    /// 4. Fallback → pretty-print the JSON
    fn extract_readable_output(value: &serde_json::Value) -> String {
        use serde_json::Value;

        match value {
            // Case 1: Standard structured output with "text" field
            Value::Object(map) if map.contains_key("text") => {
                let text = map["text"].as_str().unwrap_or("").to_string();
                // Also include status hints if available
                let mut parts = vec![text];
                if let Some(meta) = map.get("react_meta") {
                    if let Some(completed) = meta.get("completed_normally").and_then(|v| v.as_bool()) {
                        parts.push(format!("[Status: {}]",
                            if completed { "completed successfully" } else { "completed with limitations" }
                        ));
                    }
                }
                parts.join("\n")
            }
            // Case 2: JSON object without "text" — flatten readable fields
            Value::Object(map) => {
                let skip_keys = ["react_meta", "trace_logs", "trace", "metadata", "internal"];
                let mut readable_parts = Vec::new();
                for (key, val) in map {
                    if skip_keys.contains(&key.as_str()) {
                        continue;
                    }
                    match val {
                        Value::String(s) => readable_parts.push(format!("{}: {}", key, s)),
                        Value::Number(n) => readable_parts.push(format!("{}: {}", key, n)),
                        Value::Bool(b) => readable_parts.push(format!("{}: {}", key, b)),
                        Value::Array(arr) if arr.len() <= 5 => {
                            let items: Vec<String> = arr.iter().map(|v| {
                                v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string())
                            }).collect();
                            readable_parts.push(format!("{}: [{}]", key, items.join(", ")));
                        }
                        _ => readable_parts.push(format!("{}: {}", key, val)),
                    }
                }
                if readable_parts.is_empty() {
                    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
                } else {
                    readable_parts.join("\n")
                }
            }
            // Case 3: Plain string
            Value::String(s) => s.clone(),
            // Case 4: Scalar
            _ => value.to_string(),
        }
    }
}

use telos_model_gateway::{Capability, LlmRequest, Message};

#[async_trait]
impl ExecutableNode for GeneralAgent {
    async fn execute(&self, input: AgentInput, registry: &dyn SystemRegistry) -> AgentOutput {
        if input.task.to_lowercase().contains("summarize") {
            self.summarize(&input, registry).await
        } else {
            self.plan(&input, registry).await
        }
    }
}
