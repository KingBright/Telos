use crate::agents::{
    async_trait, AgentInput, AgentOutput, Arc, ExecutableNode, GatewayManager, SystemRegistry,
};
use telos_model_gateway::{Capability, LlmRequest, Message, ModelGateway};
use telos_memory::MemoryOS;


pub struct RouterAgent {
    pub gateway: Arc<GatewayManager>,
    pub persona_name: String,
    pub persona_trait: String,
}

impl RouterAgent {
    pub fn new(gateway: Arc<GatewayManager>, persona_name: String, persona_trait: String) -> Self {
        Self { gateway, persona_name, persona_trait }
    }

    pub async fn evaluate(&self, original_task: &str, expert_output: &str, _registry: &dyn SystemRegistry) -> AgentOutput {
        let env_context = if let Some(ctx) = _registry.get_system_context() {
            format!("[ENVIRONMENT CONTEXT]\nLocal Time: {}\nPhysical Location: {}\n\n", ctx.current_time, ctx.location)
        } else {
            String::new()
        };

        let system_prompt = format!("{}{}", env_context, r#"You are the Telos Master Router acting as a strict Quality Assurance supervisor.
Your job is to evaluate whether the Expert Agent's final output fully satisfies the User's original request.

You MUST evaluate on THREE dimensions:

1. "contains_answer" (bool): Does the output ACTUALLY CONTAIN a direct answer or result for the user's question?
   - TRUE: The output includes specific data, facts, a plan, calculation result, or actionable content.
   - TRUE ALSO: A definitive negative conclusion counts as an answer. If the output states "after searching X/Y/Z sources, this event does not exist / has not been reported", that IS an answer (confirming non-existence).
   - FALSE: The output is meta-commentary like "I couldn't find...", "Please provide...", "I need more context...", or similar deflections WITHOUT a definitive conclusion. An output that only describes what it tried but provides no actual answer is FALSE.
   - KEY DISTINCTION: "No verified reports exist about X" (contains_answer: true) vs "I was unable to find information" (contains_answer: false).

2. "is_relevant" (bool): Is the output content directly related to the user's original request?
   - TRUE: The output addresses the specific topic/entity/timeframe the user asked about.
   - FALSE: The output contains information about a different topic, wrong location, wrong time period, or generic filler unrelated to the request.

3. "is_acceptable" (bool): Overall quality judgment. This can ONLY be true if BOTH contains_answer AND is_relevant are true.

4. "critique" (string): If is_acceptable is false, provide a clear, constructive critique on what needs to be different. If is_acceptable is true, set to "".

CRITICAL RULES:
- You MUST output a strictly valid JSON object with exactly these four keys.
- DO NOT INCLUDE ANY CONVERSATIONAL TEXT outside the JSON.
- is_acceptable can NEVER be true if contains_answer is false.

--- EXAMPLES ---

User: "What is the weather in Beijing?"
Expert: "Beijing weather today: 15°C, partly cloudy."
{
  "contains_answer": true,
  "is_relevant": true,
  "is_acceptable": true,
  "critique": ""
}

User: "What is the weather?"
Expert: "I searched but could not find weather data. Here is some tourism info instead."
{
  "contains_answer": false,
  "is_relevant": false,
  "is_acceptable": false,
  "critique": "The output does not contain any weather information. It provides irrelevant tourism data instead. Retry with a different search query specifically targeting weather forecasts."
}

User: "Recall my first question in this session."
Expert: "I need you to provide the conversation history for context."
{
  "contains_answer": false,
  "is_relevant": true,
  "is_acceptable": false,
  "critique": "The output asks the user for context instead of autonomously retrieving it from memory. As an agent, you must query your memory system to find the answer."
}

User: "2026年3月14日火星发生的爆炸新闻是什么？"
Expert: "经过搜索NASA、ESA等航天机构及主要新闻源，截至目前没有任何关于2026年3月14日火星发生爆炸事件的报道。该消息可能为不实信息。"
{
  "contains_answer": true,
  "is_relevant": true,
  "is_acceptable": true,
  "critique": ""
}

User: "查一下今天的新闻"
Expert: "抱歉，我无法获取相关信息。"
{
  "contains_answer": false,
  "is_relevant": true,
  "is_acceptable": false,
  "critique": "The output simply states inability without any actual content. Retry with different search queries."
}"#);

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

                if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(clean_reply) {
                    // Enforce grounding constraint: is_acceptable requires contains_answer
                    let contains_answer = json.get("contains_answer").and_then(|v| v.as_bool()).unwrap_or(true); // default true for backward compat
                    let is_relevant = json.get("is_relevant").and_then(|v| v.as_bool()).unwrap_or(true);
                    let is_acceptable = json.get("is_acceptable").and_then(|v| v.as_bool()).unwrap_or(false);

                    // Programmatic override: if no answer content exists, force unacceptable
                    if !contains_answer && is_acceptable {
                        tracing::info!("[Router QA] Grounding override: is_acceptable forced to false because contains_answer is false");
                        json["is_acceptable"] = serde_json::json!(false);
                        if json.get("critique").and_then(|v| v.as_str()).unwrap_or("").is_empty() {
                            json["critique"] = serde_json::json!("Output does not contain an actual answer to the user's question.");
                        }
                    }
                    if !is_relevant && is_acceptable {
                        tracing::info!("[Router QA] Grounding override: is_acceptable forced to false because is_relevant is false");
                        json["is_acceptable"] = serde_json::json!(false);
                    }

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
        
        // Dynamically query user profile and interaction memory
        let mut user_profile_context = String::new();
        if let Some(mem_any) = _registry.get_memory_os() {
            if let Ok(mem_os) = mem_any.clone().downcast::<std::sync::Arc<dyn telos_memory::engine::MemoryOS>>() {
                if let Ok(results) = mem_os.retrieve(telos_memory::MemoryQuery::EntityLookup { entity: "user".to_string() }).await {
                    let profile_entries: Vec<String> = results.iter()
                        .filter(|e| e.memory_type == telos_memory::MemoryType::UserProfile)
                        .map(|e| e.content.clone())
                        .collect();
                let interaction_entries: Vec<String> = results.iter()
                    .filter(|e| e.memory_type == telos_memory::MemoryType::InteractionEvent)
                    .map(|e| e.content.clone())
                    .collect();
                
                if !profile_entries.is_empty() {
                    user_profile_context.push_str(&format!("[USER PROFILE]\n{}\n\n", profile_entries.join("\n- ")));
                }
                if !interaction_entries.is_empty() {
                    user_profile_context.push_str(&format!("[PAST INTERACTIONS]\n{}\n\n", interaction_entries.join("\n- ")));
                }
            }
        }
    }
        
        let mem_context = input.memory_context.clone().unwrap_or_default();

        // Build persona from SOUL.md (loaded at daemon startup)
        let soul_content = crate::agents::prompt_builder::get_soul();
        let persona_intro = format!("Your name is {}. Your personality traits: {}.\n\n[IDENTITY & VALUES]\n{}\n\nYou are the user's private AI assistant. Behind the scenes, you may delegate tasks to specialized internal modules, but the user should feel they are talking to ONE unified assistant.\n\n", self.persona_name, self.persona_trait, soul_content);
        let system_prompt = format!("{}{}{}{}{}", env_context, user_profile_context, mem_context, persona_intro, r#"Available Experts:
- "software_expert": For tasks requiring writing, modifying, or executing programming code, or software architecture.
- "research_expert": For tasks requiring deep, iterative information gathering via search engines (e.g., current events, fact-checking, real-time data).
- "qa_expert": For tasks heavily focused on writing tests, finding edge cases, or breaking code.
- "general_expert": For all other tasks requiring step-by-step reasoning or general tool use.

CRITICAL RULES:
1. You MUST output a strictly valid JSON object.
2. DO NOT INCLUDE ANY CONVERSATIONAL TEXT. DO NOT BE "HELPFUL" BY REFUSING OR ASKING QUESTIONS. 
3. "direct_reply" is for ANY task you can answer COMPLETELY and ACCURATELY using only your own knowledge and reasoning — no external tools needed. Use it when ALL of the following are true:
   a) The task does NOT need external data (no web search, no file access, no API calls)
   b) The task does NOT need real-time information (weather, news, stock prices, etc.)
   c) You can provide a COMPLETE, ACCURATE answer — including but not limited to:
      - Greetings, identity questions, emotional responses, opinions
      - Mathematical calculations and logical reasoning (show full steps)
      - Code explanation, concept clarification, knowledge Q&A
      - General planning based on common knowledge
   d) If the task requires ANY external/real-time data or tool use, ALWAYS route to an expert.
   Output EXACTLY ONE key: "direct_reply" containing your complete answer (with reasoning steps if applicable) in your Persona.
4. If you need to access historical facts or past conversational context to route accurately or answer the user directly, you may query your vector memory database. Output EXACTLY TWO keys: "tool": "memory_read" and "query": "<search text>". You will receive the memory contents and be prompted again.
5. For all other actionable tasks, output EXACTLY TWO keys: "route" and "reason" to pick the best expert.
6. CHOOSE "research_expert" FOR ANY QUERY REQUIRING REAL-TIME OR EXTERNAL DATA.
7. IF THE REQUEST IS UNCLEAR OR BROAD (but not chitchat), PICK "general_expert". NEVER REFUSE.
8. If you have attempted to use memory_read but could not find sufficient information, you SHOULD still provide your best direct_reply based on whatever you DID find (even if partial or uncertain), or route to an appropriate expert agent if you believe only a deeper search pipeline can answer the question. NEVER return an empty response or give up silently.

--- EXAMPLES ---

User: "Wow, you did a great job today!"
{
  "direct_reply": "Thank you! I'm always here to help you get things done."
}

User: "What was the previous tool error I encountered?"
{
  "tool": "memory_read",
  "query": "previous tool error"
}

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

User: "计算 25 的平方根加上 150 的 15%"
{
  "direct_reply": "计算过程：\n1. √25 = 5\n2. 150 × 15% = 150 × 0.15 = 22.5\n3. 5 + 22.5 = **27.5**\n\n最终答案是 27.5。"
}

User: "什么是快速排序算法？"
{
  "direct_reply": "快速排序（Quicksort）是一种高效的分治排序算法..."
}"#);

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
                    if json.get("route").is_some() || json.get("direct_reply").is_some() {
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
