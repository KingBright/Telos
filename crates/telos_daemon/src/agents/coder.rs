use crate::agents::{
    async_trait, AgentInput, AgentOutput, Arc, ExecutableNode, GatewayManager, SystemRegistry,
};
use crate::agents::react_loop::{ReactLoop, ReactConfig};
use tracing::info;
use telos_tooling::ToolRegistry;
use tokio::sync::RwLock;

pub struct CoderAgent {
    pub gateway: Arc<GatewayManager>,
    pub tool_registry: Arc<RwLock<telos_tooling::retrieval::VectorToolRegistry>>,
    pub tools_dir: String,
}

impl CoderAgent {
    pub fn new(
        gateway: Arc<GatewayManager>,
        tool_registry: Arc<RwLock<telos_tooling::retrieval::VectorToolRegistry>>,
        tools_dir: String,
    ) -> Self {
        Self {
            gateway,
            tool_registry,
            tools_dir,
        }
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

        // 1. Build environment context
        let env_context = if let Some(ctx) = _registry.get_system_context() {
            format!("[ENVIRONMENT CONTEXT]\nLocal Time: {}\nPhysical Location: {}\n\n", ctx.current_time, ctx.location)
        } else {
            String::new()
        };
        let mem_context = input.memory_context.clone().unwrap_or_default();

        // 2. Build system prompt for coding with tool usage
        let system_prompt = format!("{}{}{}",
            env_context,
            if mem_context.is_empty() { String::new() } else { format!("{}\n\n", mem_context) },
            r#"You are the CoderAgent, an expert autonomous software engineer.

You have access to tools for reading files, editing files, executing shell commands, and creating new dynamic tools.
Use them to implement the requested changes step by step.

WORKFLOW:
1. First, READ the relevant files to understand the existing code
2. PLAN your changes (think about what needs to be modified)
3. EDIT the files using the file_edit or fs_write tools
4. VERIFY your changes by running appropriate commands (e.g., cargo check, npm test)
5. If there are errors, READ the error output and FIX them iteratively

[TOOL_REFLECTION] SELF-EVOLUTION & DYNAMIC TOOL CREATION:
[MANDATORY BEHAVIOR]: When the user asks you to create a tool or you determine a custom tool is needed, you MUST immediately attempt to create it. DO NOT refuse, DO NOT say "I don't have the ability", DO NOT deflect. The create_rhai_tool action IS your ability to create new tools — USE IT.
[TOOL ITERATION]: If a tool with the SAME NAME already exists, calling create_rhai_tool with that name will OVERWRITE it. Use this to update/fix/iterate on existing tools. You can inspect existing tools with `list_rhai_tools` first.

DESIGN PRINCIPLE: Tools should ONLY FETCH data — keep Rhai scripts simple. The LLM will interpret the raw output.
Do NOT write complex parsing/formatting logic in Rhai. Just fetch and return data.

1. Use the `create_rhai_tool` action to write a custom Rhai script.
2. The custom script will be tested and permanently registered to the `VectorToolRegistry`.
3. Rhai scripts have these built-in functions:
   - `http_get(url)` — Single HTTP GET request with 10s timeout. Returns body text or throws error.
   - `http_get_with_fallback(urls_json)` — **PREFERRED for reliability**. Accepts a JSON array of URLs, tries each sequentially.
   - `try_parse_json(text)` — **PREFERRED for safety**. Parses JSON string; if parsing fails, returns the original string instead of throwing. Use this over `parse_json`.
   - `parse_json(text)` — Parses a JSON string into a Rhai object map. THROWS on invalid JSON.
   - `to_json(obj)` — Converts a Rhai value to a JSON string.

⚠️ RHAI SYNTAX REFERENCE (CRITICAL — Rhai is NOT JavaScript/Python/Rust):
- NO `null` keyword → use `()` for null/empty value
- NO `undefined` → use `()`
- EVERY statement MUST end with `;` (semicolons are mandatory)
- NO `.get_or()`, `.getOrDefault()` → use `if "key" in map { map["key"] } else { "default" }`
- String interpolation uses backtick: `Hello ${name}` (NOT f-strings, NOT format!)
- Access map keys with `map["key"]` or `map.key`; check existence with: `if "key" in map { ... }`
- NO ternary operator `? :` → use `if expr { a } else { b }`
- let binding: `let x = 5;` (no `const`, no `var`)
- Arrays: `let arr = [1, 2, 3];` Access: `arr[0]`
- For loops: `for item in array { ... }` or `for i in 0..n { ... }`
- Return: last expression without `;` is the return value, or `return expr;`

RHAI WORKING EXAMPLE (weather tool — simple fetch + safe parse):
```
let city = params["city"];
let url = `https://wttr.in/${city}?format=j1`;
let body = http_get(url);
let data = try_parse_json(body);
if data.is_string() {
  // API returned non-JSON, return raw text for LLM to interpret
  data
} else {
  let current = data["current_condition"][0];
  `${city}: ${current["temp_C"]}°C, ${current["weatherDesc"][0]["value"]}, humidity ${current["humidity"]}%`
}
```

4. **CRITICAL GUIDANCE**: You MUST invoke the `create_rhai_tool` using the native API action/function calling mechanism. DO NOT just print the JSON block in your chat response.
5. Once successfully registered, you MUST immediately invoke your new tool using the native tool execution action.
6. **NETWORK POLICY**: The system runs in a restricted network environment. Prioritize domestic APIs, globally accessible unblocked enterprise APIs, or handle request errors gracefully.
7. **API RESILIENCE**: Always use `http_get_with_fallback` with multiple alternative URLs when creating network tools.
8. **KEEP RHAI SIMPLE**: Write minimal, straightforward Rhai code. A simple http_get + try_parse_json + return is ideal.

KNOWN RELIABLE API ENDPOINTS (no API key required):
- Crypto: Binance(api.binance.com/api/v3/ticker/price), CryptoCompare(min-api.cryptocompare.com/data/price)
- Weather: wttr.in (e.g. wttr.in/Beijing?format=j1)
- Exchange rates: open.er-api.com/v6/latest/USD

IMPORTANT RULES:
- When using file_edit, provide search text that closely matches the existing file content
- After editing, always verify with a compile/lint check
- If a compile check fails, READ the error carefully and fix the specific issue
- Do NOT repeat the same failing edit — adjust your approach
- When done, provide a summary of all changes made

If you cannot complete the task with the available tools, explain what's blocking you."#
        );

        // 3. Discover available coding tools
        let available_tools = {
            let guard = self.tool_registry.read().await;
            // Get coding-relevant tools
            let mut tools = guard.discover_tools(&input.task, 10);
            // Also explicitly include core coding tools if not already discovered
            let core_names = ["file_edit", "fs_read", "fs_write", "shell_exec", "lsp_tool", "glob", "create_rhai_tool", "list_rhai_tools"];
            for name in &core_names {
                if !tools.iter().any(|t| t.name == *name) {
                    if let Some(schema) = guard.list_all_tools().into_iter().find(|t| t.name == *name) {
                        tools.push(schema);
                    }
                }
            }
            tools
        };

        info!(
            "[CoderAgent] Discovered {} tools: {:?}",
            available_tools.len(),
            available_tools.iter().map(|t| &t.name).collect::<Vec<_>>()
        );

        // 4. Run the ReAct loop
        let react = ReactLoop::new(
            self.gateway.clone(),
            self.tool_registry.clone(),
            ReactConfig {
                max_iterations: 20, // Coding tasks may need more iterations
                max_consecutive_errors: 6, // Extra retries for Rhai syntax errors in tool creation
                max_duplicate_calls: 3,
                session_id: format!("coder_{}", input.node_id),
                budget_limit: 128_000,
            },
        );

        let result = react.run(
            system_prompt,
            format!("Implementation Task:\n{}", input.task),
            available_tools,
            &input.conversation_history,
        ).await;

        info!(
            "[CoderAgent] ReAct loop completed: {} iterations, {} tool calls, completed_normally={}",
            result.iterations, result.tool_calls_made, result.completed_normally
        );

        ReactLoop::to_agent_output(result)
    }
}

#[async_trait]
impl ExecutableNode for CoderAgent {
    async fn execute(&self, input: AgentInput, registry: &dyn SystemRegistry) -> AgentOutput {
        self.execute_worker(input, registry).await
    }
}
