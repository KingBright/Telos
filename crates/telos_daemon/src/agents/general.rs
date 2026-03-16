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

        // 1. Discover available tools
        let tools = {
            let guard = self.tool_registry.read().await;
            guard.discover_tools(&input.task, 5)
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

INTERNAL KNOWLEDGE & RULES:
1. YOU ARE CAPABLE OF ANY TASK. DO NOT REFUSE.
2. ANALYZE THE AVAILABLE TOOLS AND SELECT THE BEST ONES TO ACCOMPLISH THE TASK.
3. FOR SEARCH/RETRIEVAL TASKS, PREFER agent_type "search_worker" over raw "tool" with "web_search".
4. YOUR PLAN MUST BE A DIRECTED ACYCLIC GRAPH (DAG) OF TOOL NODES.
5. YOU MUST END YOUR PLAN WITH A `summarize` NODE (agent_type: "general") ONCE ALL DATA IS GATHERED.
6. YOU MUST OUTPUT A STRICTLY VALID JSON SUBGRAPH. NO CONVERSATIONAL TEXT.
7. [DEFENSIVE] IF A PREVIOUS TOOL ATTEMPT FAILED, CONSTRUCT AN ALTERED SEARCH DAG WITH DIFFERENT/SIMPLER QUERIES.

--- EXAMPLE ---
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
  "nodes": [ {{ "id": "node_1", "agent_type": "search_worker", "task": "descriptive search intent — keywords: kw1, kw2", "schema_payload": "{{\"mode\":\"direct or deep\"}}" }} ],
  "edges": [ {{ "from": "node_1", "to": "node_2", "dep_type": "Data" }} ]
}}"#, persona_prefix);

        let system_prompt = PromptBuilder::new()
            .with_identity()
            .with_environment(_registry)
            .with_memory(&input.memory_context)
            .with_tools_lazy(&tools)
            .with_role_instructions(&role_instructions)
            .build();

        let req = LlmRequest {
            session_id: format!("general_{}", input.node_id),
            messages: vec![
                Message { role: "system".to_string(), content: system_prompt },
                Message { role: "user".to_string(), content: format!("Task: {}", input.task) },
            ],
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
                    Err(e) => AgentOutput::failure("PlanParseError", &format!("Failed to parse plan: {} (Raw: {})", e, clean_reply)).with_trace(trace),
                }
            }
            Err(e) => AgentOutput::failure("LLMError", &format!("{:?}", e)),
        }
    }

    async fn summarize(&self, input: &AgentInput, _registry: &dyn SystemRegistry) -> AgentOutput {
        info!("[GeneralAgent] 📝 Synthesizing results for: \"{}\"", input.task);
        
        // --- RELEVANCE GATE: filter out irrelevant dependency results before synthesis ---
        let raw_results: Vec<(String, String)> = input.dependencies.iter()
            .map(|(id, out)| (id.clone(), out.output.as_ref().map(|v| v.to_string()).unwrap_or_default()))
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

        let req = LlmRequest {
            session_id: format!("summarize_{}", input.node_id),
            messages: vec![
                Message { role: "system".to_string(), content: system_prompt },
                Message { role: "user".to_string(), content: prompt },
            ],
            required_capabilities: Capability {
                requires_vision: false,
                strong_reasoning: false,
            },
            budget_limit: 2000,
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
