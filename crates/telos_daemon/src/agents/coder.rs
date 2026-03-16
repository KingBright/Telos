use crate::agents::{
    async_trait, AgentInput, AgentOutput, Arc, ExecutableNode, GatewayManager, SystemRegistry,
};
use crate::agents::react_loop::{ReactLoop, ReactConfig};
use tracing::{info, warn};
use telos_model_gateway::ModelGateway;
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

You have access to tools for reading files, editing files, and executing shell commands.
Use them to implement the requested changes step by step.

WORKFLOW:
1. First, READ the relevant files to understand the existing code
2. PLAN your changes (think about what needs to be modified)
3. EDIT the files using the file_edit or fs_write tools
4. VERIFY your changes by running appropriate commands (e.g., cargo check, npm test)
5. If there are errors, READ the error output and FIX them iteratively

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
            let core_names = ["file_edit", "fs_read", "fs_write", "shell_exec", "lsp_tool", "glob"];
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
                max_consecutive_errors: 3,
                max_duplicate_calls: 3,
                session_id: format!("coder_{}", input.node_id),
                budget_limit: 128_000,
            },
        );

        let result = react.run(
            system_prompt,
            format!("Implementation Task:\n{}", input.task),
            available_tools,
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
