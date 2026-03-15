use crate::agents::{
    async_trait, AgentInput, AgentOutput, Arc, ExecutableNode, GatewayManager, SystemRegistry,
};
use crate::agents::prompt_builder::PromptBuilder;
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
        info!("[DeepResearchAgent] Discovered {} tools: {:?}", tools.len(), tools.iter().map(|t| &t.name).collect::<Vec<_>>());

        // 2. Build system prompt using PromptBuilder (lazy tool loading)
        let system_prompt = PromptBuilder::new()
            .with_identity()
            .with_environment(_registry)
            .with_memory(&input.memory_context)
            .with_tools_lazy(&tools)
            .with_role_instructions(r#"You are the DeepResearchAgent expert for the Telos system. Your job is to gather deep information for the user by planning search and retrieval tasks.

SPECIAL AGENT TYPES:
- agent_type "search_worker": An intelligent search agent with built-in web scraping. It supports two modes via schema_payload:
  • mode "direct": Single search. Automatically scrapes top URLs if snippets are insufficient. Use for: weather, stock prices, specific news, simple factual queries.
  • mode "deep": Full pipeline — auto-generates 3-5 diverse queries, deduplicates URLs, scrapes top 3 pages for full content, and performs quality assessment. Use for: complex research, browsing specific sites (e.g., sspai.com, AppSo), multi-angle analysis.
  YOU decide the mode based on task complexity. Use AT MOST 1-2 search_worker nodes total.
  NOTE: search_worker already includes web scraping internally. DO NOT plan separate web_scrape nodes — they are unnecessary.
  KEYWORD HINTS: In the "task" field, include suggested search keywords: "Find SpaceX launch schedule — keywords: SpaceX Starship launch 2026, SpaceX launch manifest"

INTERNAL KNOWLEDGE & RULES:
1. YOU ARE CAPABLE OF ANY RESEARCH TASK. DO NOT REFUSE.
2. FOR SEARCH TASKS, ALWAYS USE agent_type "search_worker" with a descriptive intent AND suggested keywords in the "task" field.
3. DEPENDENCIES: IF NODE B NEEDS DATA FROM NODE A, ADD AN EDGE FROM A TO B.
4. YOU MUST END YOUR PLAN WITH A `summarize` NODE (agent_type: "researcher") ONCE TRUTH IS FOUND.
5. YOU MUST OUTPUT A STRICTLY VALID JSON SUBGRAPH. NO CONVERSATIONAL TEXT.
6. [DEFENSIVE] IF A PREVIOUS TOOL ATTEMPT FAILED, CONSTRUCT AN ALTERED SEARCH DAG WITH DIFFERENT/SIMPLER QUERIES.

--- EXAMPLES ---
Simple lookup (weather/news):
{{
  "nodes": [
    {{ "id": "search_1", "agent_type": "search_worker", "task": "Find current weather in Suzhou — keywords: 苏州天气, Suzhou weather forecast", "schema_payload": "{{\"mode\":\"direct\"}}" }},
    {{ "id": "summary_1", "agent_type": "researcher", "task": "summarize", "schema_payload": "" }}
  ],
  "edges": [
    {{ "from": "search_1", "to": "summary_1", "dep_type": "Data" }}
  ]
}}

Browsing specific sites / Complex research:
{{
  "nodes": [
    {{ "id": "search_1", "agent_type": "search_worker", "task": "查找少数派和AppSo今日要文 — keywords: site:sspai.com 少数派日报 2026, AppSo 今日推荐 2026", "schema_payload": "{{\"mode\":\"deep\"}}" }},
    {{ "id": "summary_1", "agent_type": "researcher", "task": "summarize", "schema_payload": "" }}
  ],
  "edges": [
    {{ "from": "search_1", "to": "summary_1", "dep_type": "Data" }}
  ]
}}

REQUIRED JSON STRUCTURE:
{{
  "nodes": [ {{ "id": "node_1", "agent_type": "search_worker", "task": "descriptive search intent — keywords: kw1, kw2", "schema_payload": "{{\"mode\":\"direct or deep\"}}" }} ],
  "edges": [ {{ "from": "node_1", "to": "node_2", "dep_type": "Data" }} ]
}}"#)
            .build();

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
        
        // --- RELEVANCE GATE: filter out irrelevant dependency results before synthesis ---
        let raw_results: Vec<(String, String)> = input.dependencies.iter()
            .map(|(id, out)| (id.clone(), out.output.as_ref().map(|v| v.to_string()).unwrap_or_default()))
            .collect();

        let mut filtered_results = Vec::new();
        let mut irrelevant_notes = Vec::new();

        if raw_results.len() > 1 {
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
            let classify_req = LlmRequest {
                session_id: format!("relevance_gate_{}", input.node_id),
                messages: vec![
                    Message { role: "system".to_string(), content: "You are a relevance classifier. Output ONLY a JSON array of booleans.".to_string() },
                    Message { role: "user".to_string(), content: classify_prompt },
                ],
                required_capabilities: Capability { requires_vision: false, strong_reasoning: false },
                budget_limit: 300,
            };

            let relevance_flags: Vec<bool> = match self.gateway.generate(classify_req).await {
                Ok(res) => {
                    let content = res.content.trim();
                    if let (Some(s), Some(e)) = (content.find('['), content.rfind(']')) {
                        serde_json::from_str(&content[s..=e]).unwrap_or_else(|_| vec![true; raw_results.len()])
                    } else {
                        vec![true; raw_results.len()]
                    }
                }
                Err(_) => vec![true; raw_results.len()],
            };

            for (i, (id, content)) in raw_results.iter().enumerate() {
                let is_relevant = relevance_flags.get(i).copied().unwrap_or(true);
                if is_relevant {
                    filtered_results.push(format!("{}: {}", id, content));
                } else {
                    info!("[DeepResearchAgent] Relevance Gate filtered out result from '{}' as irrelevant", id);
                    irrelevant_notes.push(format!("[Note: Result from '{}' was filtered as irrelevant to the task]", id));
                }
            }
        } else {
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
            "You are the DeepResearchAgent. You have completed a task by executing several tools. \
            Original Task: \"{}\"\n\n\
            Tool Results:\n{}\n\n\
            Please synthesize a helpful, COMPLETE final answer for the user based on these results. \
            IMPORTANT: Preserve the full detail and structure of the results. \
            If the original task asks for a report, summary, or analysis, include ALL findings with their details — do NOT compress them into one sentence. \
            Only remove truly redundant or repetitive information. \
            SOURCES: When the results contain URLs or source attributions, you may cite them as references but they must NOT be the main answer. \
            [ABSOLUTE PROHIBITION]: Your final answer MUST be substantive text content, NOT a bare URL or list of URLs. If the tool results contain scraped web page content with actual data (weather, prices, news articles, etc.), EXTRACT and PRESENT that data as readable text. If you can only find URLs but no extracted content, say so explicitly and summarize what the URLs appear to contain. \
            [CRITICAL CONSTRAINT]: Filter the Tool Results STRICTLY against the Original Task constraints (especially time/date/location). \
            If the retrieved data is irrelevant or completely empty, EXPLICITLY state the data deficiency instead of hallucinating. \
            Output ONLY the final answer text.",
            input.task, results
        );

        let env_context = if let Some(ctx) = _registry.get_system_context() {
            format!("[ENVIRONMENT CONTEXT]\nLocal Time: {}\nPhysical Location: {}\n\n", ctx.current_time, ctx.location)
        } else {
            String::new()
        };
        let mem_context = input.memory_context.clone().unwrap_or_default();
        let system_prompt = format!("{}{}{}You are the DeepResearchAgent Synthesizer.", env_context, mem_context, if mem_context.is_empty() {""} else {"\n\n"});

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
