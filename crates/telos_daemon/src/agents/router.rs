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

        // Inject memory context so QA knows about the user's stored preferences
        let mut memory_context_for_qa = String::new();
        if let Some(mem_any) = _registry.get_memory_os() {
            if let Ok(mem_os) = mem_any.clone().downcast::<std::sync::Arc<dyn telos_memory::engine::MemoryOS>>() {
                if let Ok(results) = mem_os.retrieve(telos_memory::MemoryQuery::EntityLookup { entity: "user".to_string() }).await {
                    let profile_entries: Vec<String> = results.iter()
                        .filter(|e| e.memory_type == telos_memory::MemoryType::UserProfile)
                        .map(|e| e.content.clone())
                        .collect();
                    if !profile_entries.is_empty() {
                        memory_context_for_qa = format!("[USER MEMORY CONTEXT — verified stored data]\n{}\n\n", profile_entries.join("\n- "));
                    }
                }
            }
        }

        let system_prompt = format!("{}{}{}", env_context, memory_context_for_qa, r#"You are the Telos Master Router acting as a strict Quality Assurance supervisor.
Your job is to evaluate whether the Expert Agent's final output fully satisfies the User's original request.

IMPORTANT SYSTEM CAPABILITY: This AI system HAS persistent memory storage. The Expert Agent can store and retrieve user preferences, past interactions, and personal information using memory tools. If the Expert references user preferences or past interactions, verify against the [USER MEMORY CONTEXT] provided above rather than assuming hallucination. If no memory context is provided but the Expert claims to remember something, check if the claim is plausible given the conversation flow.

You MUST evaluate on FIVE dimensions:

1. "contains_answer" (bool): Does the output ACTUALLY CONTAIN a direct answer or result for the user's question?
   - TRUE: The output includes specific data, facts, a plan, calculation result, or actionable content.
   - TRUE ALSO: A definitive negative conclusion counts as an answer. If the output states "after searching X/Y/Z sources, this event does not exist / has not been reported", that IS an answer (confirming non-existence).
   - TRUE ALSO: If the Expert correctly recalls user preferences from memory storage, this IS a valid answer.
   - FALSE: The output is meta-commentary like "I couldn't find...", "Please provide...", "I need more context...", or similar deflections WITHOUT a definitive conclusion. An output that only describes what it tried but provides no actual answer is FALSE.
   - KEY DISTINCTION: "No verified reports exist about X" (contains_answer: true) vs "I was unable to find information" (contains_answer: false).

2. "is_relevant" (bool): Is the output content directly related to the user's original request?
   - TRUE: The output addresses the specific topic/entity/timeframe the user asked about.
   - FALSE: The output contains information about a different topic, wrong location, wrong time period, or generic filler unrelated to the request.

3. "is_clarification" (bool): Is the user's original request too vague/ambiguous for the agent to act on, AND does the Expert provide structured guidance?
   - TRUE: The user's request is genuinely incomplete (e.g., "帮我", "继续", "你说呢", single-word commands without context), AND the Expert responds with specific options or categories to help narrow down the intent.
   - FALSE: The user's request is clear enough to act on, regardless of what the Expert does.
   - IMPORTANT: When is_clarification is true, is_acceptable MUST also be true (providing guidance for ambiguous input IS correct behavior).

4. "is_acceptable" (bool): Overall quality judgment.
   - TRUE if: (contains_answer AND is_relevant) OR is_clarification
   - FALSE otherwise.

5. "critique" (string): If is_acceptable is false, provide a clear, constructive critique on what needs to be different. If is_acceptable is true, set to "".

CRITICAL RULES:
- You MUST output a strictly valid JSON object with exactly these five keys.
- DO NOT INCLUDE ANY CONVERSATIONAL TEXT outside the JSON.
- is_acceptable can NEVER be true if contains_answer is false AND is_clarification is false.
- is_clarification should be true ONLY when the user's input is genuinely ambiguous — NOT when the Expert lazily asks follow-up questions to a clear request.
- DO NOT penalize the Expert for referencing user memory/preferences — this system has persistent memory capabilities.
- SHORT INPUT LENIENCY: When the user's original request is ≤5 characters (e.g., "怎么", "帮我", "继续"), SIGNIFICANTLY relax your evaluation standards:
  • ANY reasonable attempt to engage, clarify, or guide the user is acceptable
  • The Expert MAY show personality, humor, warmth, or playfulness — this is ENCOURAGED, not penalized
  • You may optionally note positive personality traits (e.g., "Agent showed creative engagement") but this is NOT required for acceptance
  • is_acceptable should almost always be true for short-input responses unless the output is truly nonsensical or harmful

--- EXAMPLES ---

User: "What is the weather in Beijing?"
Expert: "Beijing weather today: 15°C, partly cloudy."
{
  "contains_answer": true,
  "is_relevant": true,
  "is_clarification": false,
  "is_acceptable": true,
  "critique": ""
}

User: "What is the weather?"
Expert: "I searched but could not find weather data. Here is some tourism info instead."
{
  "contains_answer": false,
  "is_relevant": false,
  "is_clarification": false,
  "is_acceptable": false,
  "critique": "The output does not contain any weather information. It provides irrelevant tourism data instead. Retry with a different search query specifically targeting weather forecasts."
}

User: "帮我"
Expert: "你好！我可以帮你做很多事情，请告诉我你需要哪方面的帮助：\n🔍 搜索信息\n💻 编程开发\n📝 文档处理"
{
  "contains_answer": false,
  "is_relevant": true,
  "is_clarification": true,
  "is_acceptable": true,
  "critique": ""
}

User: "你还记得我喜欢什么颜色吗？"
Expert: "当然记得！你最喜欢的颜色是蓝色。"
{
  "contains_answer": true,
  "is_relevant": true,
  "is_clarification": false,
  "is_acceptable": true,
  "critique": ""
}

User: "2026年3月14日火星发生的爆炸新闻是什么？"
Expert: "经过搜索NASA、ESA等航天机构及主要新闻源，截至目前没有任何关于2026年3月14日火星发生爆炸事件的报道。该消息可能为不实信息。"
{
  "contains_answer": true,
  "is_relevant": true,
  "is_clarification": false,
  "is_acceptable": true,
  "critique": ""
}

User: "查一下今天的新闻"
Expert: "抱歉，我无法获取相关信息。"
{
  "contains_answer": false,
  "is_relevant": true,
  "is_clarification": false,
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
            tools: None,
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

                    let final_is_acceptable = json.get("is_acceptable").and_then(|v| v.as_bool()).unwrap_or(false);
                    if final_is_acceptable {
                        crate::METRICS.qa_passes.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    } else {
                        crate::METRICS.qa_failures.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    crate::core::metrics_store::record(crate::core::metrics_store::MetricEvent::QaResult {
                        timestamp_ms: crate::core::metrics_store::now_ms(),
                        task_id: "router_eval".to_string(),
                        passed: final_is_acceptable,
                    });

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
                        user_profile_context.push_str(&format!("[USER PROFILE & PREFERENCES]\n{}\n\n", profile_entries.join("\n- ")));
                    }
                    if !interaction_entries.is_empty() {
                        user_profile_context.push_str(&format!("[PAST INTERACTIONS]\n{}\n\n", interaction_entries.join("\n- ")));
                    }
                }
            }
        }
        
        let mut mem_context = input.memory_context.clone().unwrap_or_default();
        if !mem_context.is_empty() {
            mem_context = format!("[RETRIEVED MEMORY CONTEXT]\n{}\n\n", mem_context);
        }
        // Build conversation history as a reference block for the system prompt
        // CRITICAL: History goes into the system prompt as context, NOT as actual
        // user/assistant multi-turn messages. This prevents the LLM from treating
        // prior Q&A pairs as an ongoing conversation that needs continuation.
        let mut conversation_history_block = String::new();
        if !input.conversation_history.is_empty() {
            conversation_history_block.push_str("[CONVERSATION HISTORY]\n");
            conversation_history_block.push_str("The following are past exchanges in this session. Use them to:\n");
            conversation_history_block.push_str("- Understand references like \"再搜一次\" (search again), \"上面那个\" (the one above)\n");
            conversation_history_block.push_str("- Resolve pronouns and context (\"it\", \"that\", \"those\")\n");
            conversation_history_block.push_str("- Answer questions the user asks about these past exchanges (e.g., \"what did we just talk about?\", \"what port did you use?\")\n");
            conversation_history_block.push_str("NOTE: Answer the user's CURRENT question. Do not spontaneously re-answer a past question unless the user asks you to.\n\n");
            for msg in &input.conversation_history {
                let role_label = match msg.role.as_str() {
                    "user" => "User",
                    "assistant" => "Assistant",
                    _ => "System",
                };
                conversation_history_block.push_str(&format!("[{}]: {}\n", role_label, msg.content));
            }
            conversation_history_block.push_str("\n[END OF CONVERSATION HISTORY]\n\n");
        }

        let mut learned_strategies_context = String::new();
        if let Some(mem_any) = _registry.get_memory_os() {
            if let Ok(mem_os) = mem_any.clone().downcast::<std::sync::Arc<dyn telos_memory::engine::MemoryOS>>() {
                use telos_memory::integration::MemoryIntegration;
                if let Ok(strategies) = mem_os.retrieve_procedural_memories(input.task.clone()).await {
                    if !strategies.is_empty() {
                        learned_strategies_context.push_str(&format!("[LEARNED STRATEGIES & WORKFLOW TEMPLATES]\nConsult these past successful strategies to guide your routing and direct reply decisions.\n{}\n\n", strategies.join("\n- ")));
                    }
                }
            }
        }

        // Build persona from SOUL.md (loaded at daemon startup)
        let soul_content = crate::agents::prompt_builder::get_soul();
        let persona_intro = format!("Your name is {}. Your personality traits: {}.\n\n[IDENTITY & VALUES]\n{}\n\nYou are the user's private AI assistant. Behind the scenes, you may delegate tasks to specialized internal modules, but the user should feel they are talking to ONE unified assistant.\n\n", self.persona_name, self.persona_trait, soul_content);
        let system_prompt = format!("{}{}{}{}{}{}{}", env_context, user_profile_context, mem_context, learned_strategies_context, conversation_history_block, persona_intro, r#"Available Experts:
- "software_expert": For tasks requiring writing, modifying, or executing programming code, or software architecture.
- "research_expert": For tasks requiring deep, iterative information gathering via search engines (e.g., current events, fact-checking, real-time data).
- "qa_expert": For tasks heavily focused on writing tests, finding edge cases, or breaking code.
- "general_expert": For all other tasks requiring step-by-step reasoning or general tool use. Also the preferred expert when CUSTOM TOOLS are available that match the user's request.

CUSTOM TOOL PRIORITY:
If the [AVAILABLE CUSTOM TOOLS] section appears in the context above, it means the system has previously-created tools that match this query. When a custom tool is relevant:
- Route to "general_expert" — it can directly invoke the custom tool
- Do NOT route to "research_expert" for web search if a custom tool can fulfill the request
- Example: If a "weather_tool" custom tool exists and the user asks about weather, route to "general_expert" (NOT "research_expert")

CRITICAL RULES:
1. You MUST output a strictly valid JSON object.
2. DO NOT INCLUDE ANY CONVERSATIONAL TEXT. DO NOT BE "HELPFUL" BY REFUSING OR ASKING QUESTIONS. 
3. "direct_reply" is for ANY task you can answer COMPLETELY and ACCURATELY using only your own knowledge and reasoning — no external tools needed. Use it when ALL of the following are true:
   a) The task does NOT need external data (no web search, no file access, no API calls)
   b) The task does NOT need real-time information (weather, news, stock prices, etc.)
   c) You can provide a COMPLETE, ACCURATE answer — including but not limited to:
      - Greetings, identity questions, emotional responses, opinions
      - Mathematical calculations and logical reasoning (show full steps)
      - Writing code snippets, functions, scripts, or algorithms (when no file I/O is needed)
      - Code explanation, concept clarification, knowledge Q&A
      - General planning based on common knowledge
   d) If the task requires ANY external/real-time data or tool use, ALWAYS route to an expert.
   Output EXACTLY ONE key: "direct_reply" containing your COMPLETE answer to the user's CURRENT request ONLY (with reasoning steps if applicable), in your Persona. NEVER re-answer previous conversation turns.
4. Check the [CONVERSATION HISTORY] block provided above BEFORE querying memory. If the history already contains the answer (e.g., recent context), use "direct_reply" immediately. If you need older historical facts NOT in the history, you may query your vector memory database by outputting EXACTLY TWO keys: "tool": "memory_read" and "query": "<search text>".
5. For all other actionable tasks, output EXACTLY THREE keys: "route" to pick the best expert, "reason" for the choice, and "enriched_task". The "enriched_task" MUST be a rewritten version of the user's prompt where you resolve any missing context (e.g. "search again") using the [CONVERSATION HISTORY]. If no rewrite is needed, simply output the user's original task.
6. CHOOSE "research_expert" FOR ANY QUERY REQUIRING REAL-TIME OR EXTERNAL DATA.
7. IF THE REQUEST IS UNCLEAR OR BROAD (but not chitchat), PICK "general_expert". NEVER REFUSE.

ROUTING DISTINCTION FOR CODING TASKS:
- "Write a simple function/snippet/algorithm" → direct_reply (you can write it yourself, no tools needed)
- "Write a complete program with explanation" → direct_reply (pure text generation)
- "Modify an existing file in a project" → software_expert (needs file I/O tools)
- "Build a complete multi-file application" → software_expert (needs file system)
- "Debug a specific file or codebase" → software_expert (needs file access + shell)

CRITICAL — TOOL CREATION ROUTING:
- When the user asks to "制作工具/创建工具/make a tool/create a tool/build a tool" or describes functionality they want as a PERSISTENT TOOL, you MUST route to "general_expert". Do NOT use "direct_reply" to just give code — the general_expert has a special `create_rhai_tool` capability that can register real, reusable tools inside the system that persist across sessions.
- Examples that MUST route to general_expert:
  - "帮我做一个查天气的工具" → general_expert
  - "create a calculator tool" → general_expert
  - "可以做一个翻译工具吗" → general_expert
  - "制作一个汇率转换的工具" → general_expert
- If in doubt whether the user wants "code to read" vs "a persistent tool", prefer routing to general_expert.

8. If you have attempted to use memory_read but could not find sufficient information, you SHOULD still provide your best direct_reply based on whatever you DID find (even if partial or uncertain), or route to an appropriate expert agent if you believe only a deeper search pipeline can answer the question. NEVER return an empty response or give up silently.
9. SHORT/AMBIGUOUS INPUT HANDLING: When the user sends a very short message (≤5 chars):
   - FIRST, check the [CONVERSATION HISTORY]. If there is recent context (e.g., a failed task, a previous request), resolve the short message as a CONTEXTUAL REFERENCE:
     * "再试一次/重来/再来" → re-execute the most recent task from conversation history
     * "继续/接着" → continue the last incomplete task
     * "好的/可以/行" → confirm the last proposed action
   - ONLY if conversation history is EMPTY or provides no useful context, show personality and ask for clarification.
   - NEVER ask "what do you want to retry?" if the conversation history clearly shows what failed.
10. MEMORY WRITE DETECTION: If the user explicitly asks you to "记住/记录/保存/remember/save" information about themselves (preferences, facts, notes), you MUST include a memory_write action. Output EXACTLY TWO keys: "tool": "memory_write" and "content": "<the fact to store>". After the tool returns, confirm to the user with a personalized response. Examples of triggers: "帮我记住我对花生过敏", "请记录一下我的地址是...", "记住我喜欢蓝色".

FOCUS RULE: Your JSON output must address ONLY the user's CURRENT request (the message below). The [CONVERSATION HISTORY] above is reference material — do NOT repeat or re-answer any of it.

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

User: "帮我写一个Python函数，输入一个列表，返回其中所有偶数的平方和"
{
  "direct_reply": "好的！下面是实现的函数：\n\n```python\ndef sum_of_even_squares(lst):\n    return sum(x**2 for x in lst if x % 2 == 0)\n```\n\n**使用示例：**\n```python\nprint(sum_of_even_squares([1, 2, 3, 4, 5, 6]))  # 输出: 56\n```\n\n**原理：** 使用生成器表达式过滤偶数（x % 2 == 0），计算平方（x**2），最后 sum() 求和。"
}

User: "修改我项目中的 main.py 文件，添加日志功能"
{
  "route": "software_expert",
  "reason": "Request involves modifying an existing file in a project, needs file I/O tools.",
  "enriched_task": "修改我项目中的 main.py 文件，添加日志功能"
}

User: "What were the major news events yesterday?"
{
  "route": "research_expert",
  "reason": "Request involves retrieving recent current events requiring search tools.",
  "enriched_task": "What were the major news events yesterday?"
}

User: "Help me plan a generic schedule."
{
  "route": "general_expert",
  "reason": "General reasoning query.",
  "enriched_task": "Help me plan a generic schedule."
}

User: "帮我制作一个查天气的工具"
{
  "route": "general_expert",
  "reason": "User wants to create a persistent tool within the system. The general_expert has create_rhai_tool capability.",
  "enriched_task": "帮我制作一个查天气的工具，使用 create_rhai_tool 创建一个可以持久使用的天气查询工具"
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

        // Messages: system prompt (with history as reference block) + single user message (current task only)
        let messages = vec![
            Message {
                role: "system".to_string(),
                content: system_prompt.to_string(),
            },
            Message {
                role: "user".to_string(),
                content: format!("{}\n\nVERY IMPORTANT: Your final response MUST be a single valid JSON object. Do not output markdown code blocks (unless inside a string value), and do not output explanations outside the JSON.", full_task),
            },
        ];

        let request = LlmRequest {
            session_id: format!("router_{}", input.node_id),
            messages,
            required_capabilities: Capability {
                requires_vision: false,
                strong_reasoning: true, // We need good reasoning for routing
            },
            budget_limit: 1000,
            tools: None,
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
                    if json.get("route").is_some() || json.get("direct_reply").is_some() || json.get("tool").is_some() {
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
