use telos_context::ContextManager;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex as TokioMutex;
use uuid::Uuid;

// Core Traits and Primitives
use async_trait::async_trait;
use telos_context::providers::OpenAiProvider;
use telos_context::{RaptorContextManager, ScopedContext};
use telos_core::config::TelosConfig;
use telos_core::{NodeError, NodeResult, SystemRegistry};
use telos_dag::{ExecutionEngine, NodeMetadata};
use telos_dag::{ExecutableNode, GraphState, TaskGraph};
use telos_hci::{
    global_log_level, AgentEvent, AgentFeedback, EventBroker, LogLevel, PlanInfo, PlanNodeInfo,
    TaskSummary, TokioEventBroker,
};
use telos_memory::engine::RedbGraphStore;
use telos_memory::integration::MemoryIntegration;
use telos_model_gateway::gateway::{GatewayManager, ModelProvider};
use telos_model_gateway::{Capability, GatewayError, LlmRequest, LlmResponse, ModelGateway};
use telos_tooling::ToolRegistry;

// 1. Adapter to convert Context OpenAiProvider to Gateway ModelProvider for the Gateway Manager
struct GatewayAdapter {
    inner: OpenAiProvider,
}

#[async_trait]
impl ModelProvider for GatewayAdapter {
    async fn generate(&self, req: &LlmRequest) -> Result<LlmResponse, GatewayError> {
        let prompt = req
            .messages
            .last()
            .map(|m| m.content.as_str())
            .unwrap_or("");

        match self.inner.chat_completion(prompt).await {
            Ok(content) => Ok(LlmResponse {
                content,
                tokens_used: req.budget_limit,
            }),
            Err(e) => {
                let error_msg = e.0.to_lowercase();
                // Detect network-related errors for retry
                if error_msg.contains("error sending request")
                    || error_msg.contains("connection")
                    || error_msg.contains("timeout")
                    || error_msg.contains("dns")
                    || error_msg.contains("network")
                    || error_msg.contains("socket")
                    || error_msg.contains("http error")
                {
                    Err(GatewayError::NetworkError(e.0))
                } else if error_msg.contains("429") || error_msg.contains("rate limit") {
                    Err(GatewayError::TooManyRequests)
                } else if error_msg.contains("503") || error_msg.contains("service unavailable") {
                    Err(GatewayError::ServiceUnavailable)
                } else {
                    Err(GatewayError::Other(e.0))
                }
            }
        }
    }
}

// 2. System Registry
struct DaemonRegistry {
    gateway: Arc<GatewayManager>,
    memory_os: Arc<RedbGraphStore>,
}

impl SystemRegistry for DaemonRegistry {
    fn get_memory_os(&self) -> Option<std::sync::Arc<dyn std::any::Any + Send + Sync>> {
        Some(self.memory_os.clone() as Arc<dyn std::any::Any + Send + Sync>)
    }

    fn get_model_gateway(&self) -> Option<std::sync::Arc<dyn std::any::Any + Send + Sync>> {
        Some(self.gateway.clone() as Arc<dyn std::any::Any + Send + Sync>)
    }
}

// 3. Real Executable Node that calls the LLM dynamically

// --- Dynamic DAG Deserialization structs ---
#[derive(serde::Deserialize, Debug)]
struct DagEdge {
    from: String,
    to: String,
}

#[derive(serde::Deserialize, Debug)]
struct DagNode {
    id: String,
    task_type: String, // "LLM" or "TOOL"
    prompt: String,
}

#[derive(serde::Deserialize, Debug)]
struct DagPlan {
    tier: Option<String>,
    reply: Option<String>,
    nodes: Vec<DagNode>,
    edges: Vec<DagEdge>,
}

/// Helper to truncate strings for preview
fn truncate_for_preview(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}...", &s[..max_len])
    } else {
        s.to_string()
    }
}

struct LlmPromptNode {
    prompt: String,
    gateway: Arc<GatewayManager>,
}

#[async_trait]
impl ExecutableNode for LlmPromptNode {
    async fn execute(
        &self,
        _ctx: &ScopedContext,
        _registry: &dyn SystemRegistry,
    ) -> Result<NodeResult, NodeError> {
        let gateway = self.gateway.clone();

        let request = LlmRequest {
            session_id: "daemon_session".to_string(),
            messages: vec![telos_model_gateway::Message {
                role: "user".to_string(),
                content: self.prompt.clone(),
            }],
            required_capabilities: Capability {
                requires_vision: false,
                strong_reasoning: false,
            },
            budget_limit: 1000,
        };

        match gateway.generate(request).await {
            Ok(res) => Ok(NodeResult {
                output_data: res.content.into_bytes(),
                extracted_knowledge: None,
                next_routing_hint: None,
            }),
            Err(e) => Err(NodeError::ExecutionFailed(format!("{:?}", e))),
        }
    }
}


struct ReactNode {
    prompt: String,
    gateway: Arc<GatewayManager>,
    tool_registry: std::sync::Arc<tokio::sync::RwLock<telos_tooling::retrieval::VectorToolRegistry>>,
    _tools_dir: String,
}

#[async_trait]
impl ExecutableNode for ReactNode {
    async fn execute(
        &self,
        _ctx: &ScopedContext,
        registry: &dyn SystemRegistry,
    ) -> Result<NodeResult, NodeError> {
        let mut loop_count = 0;
        let mut session_messages = vec![telos_model_gateway::Message {
            role: "user".to_string(),
            content: format!("Use ReAct to solve this task. You must output a JSON with either {{ \"action\": \"TOOL_NAME\", \"params\": {{...}} }} to call a tool, or {{ \"final_answer\": \"YOUR_ANSWER\" }}. Task: {}", self.prompt),
        }];

        while loop_count < 5 {
            loop_count += 1;

            let req = telos_model_gateway::LlmRequest {
                session_id: "daemon_react_session".to_string(),
                messages: session_messages.clone(),
                required_capabilities: telos_model_gateway::Capability {
                    requires_vision: false,
                    strong_reasoning: false,
                },
                budget_limit: 1000,
            };

            let llm_reply = match self.gateway.generate(req).await {
                Ok(res) => res.content.trim().to_string(),
                Err(e) => return Err(NodeError::ExecutionFailed(format!("LLM Error: {:?}", e))),
            };

            let clean_reply = llm_reply.trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```");

            if let Ok(json) = serde_json::from_str::<serde_json::Value>(clean_reply) {
                if let Some(final_ans) = json.get("final_answer").and_then(|v| v.as_str()) {
                    return Ok(NodeResult {
                        output_data: final_ans.as_bytes().to_vec(),
                        extracted_knowledge: None,
                        next_routing_hint: None,
                    });
                } else if let (Some(action), Some(params)) = (json.get("action").and_then(|v| v.as_str()), json.get("params")) {
                    session_messages.push(telos_model_gateway::Message {
                        role: "assistant".to_string(),
                        content: llm_reply.clone(),
                    });

                    let registry_guard = self.tool_registry.read().await;
                    let executor = match registry_guard.get_executor(action) {
                        Some(e) => e,
                        None => {
                            session_messages.push(telos_model_gateway::Message {
                                role: "user".to_string(),
                                content: format!("Tool {} not found.", action),
                            });
                            continue;
                        }
                    };

                    match executor.call(params.clone()).await {
                        Ok(tool_output) => {
                            // Handle memory tool macros
                            let tool_output_str = String::from_utf8_lossy(&tool_output).to_string();
                            let response_content = if let Ok(json_res) = serde_json::from_str::<serde_json::Value>(&tool_output_str) {
                                if let Some(macro_type) = json_res.get("__macro__").and_then(|m| m.as_str()) {
                                    match macro_type {
                                        "memory_recall" => {
                                            let query = json_res.get("query").and_then(|q| q.as_str()).unwrap_or("");
                                            if let Some(memory_os_any) = registry.get_memory_os() {
                                                if let Ok(memory_integration) = memory_os_any.clone().downcast::<telos_memory::engine::RedbGraphStore>() {
                                                    match memory_integration.retrieve_semantic_facts(query.to_string()).await {
                                                        Ok(facts) => {
                                                            if facts.is_empty() {
                                                                format!("Memory recall: No relevant memories found for '{}'", query)
                                                            } else {
                                                                format!("Memory recall for '{}':\n{}", query, facts.join("\n"))
                                                            }
                                                        }
                                                        Err(e) => format!("Memory recall failed: {:?}", e),
                                                    }
                                                } else {
                                                    "Memory system downcast failed".to_string()
                                                }
                                            } else {
                                                "Memory system not available".to_string()
                                            }
                                        }
                                        "memory_store" => {
                                            let content = json_res.get("content").and_then(|c| c.as_str()).unwrap_or("");
                                            if let Some(memory_os_any) = registry.get_memory_os() {
                                                if let Ok(memory_integration) = memory_os_any.clone().downcast::<telos_memory::engine::RedbGraphStore>() {
                                                    match memory_integration.store_semantic_fact(content.to_string()).await {
                                                        Ok(_) => format!("Successfully stored in memory: {}", content),
                                                        Err(e) => format!("Failed to store in memory: {:?}", e),
                                                    }
                                                } else {
                                                    "Memory system downcast failed".to_string()
                                                }
                                            } else {
                                                "Memory system not available".to_string()
                                            }
                                        }
                                        _ => tool_output_str.clone(),
                                    }
                                } else {
                                    tool_output_str.clone()
                                }
                            } else {
                                tool_output_str.clone()
                            };

                            session_messages.push(telos_model_gateway::Message {
                                role: "user".to_string(),
                                content: format!("Tool output: {}", response_content),
                            });
                        }
                        Err(e) => {
                            session_messages.push(telos_model_gateway::Message {
                                role: "user".to_string(),
                                content: format!("Tool execution failed: {:?}", e),
                            });
                        }
                    }
                } else {
                    return Ok(NodeResult {
                        output_data: clean_reply.as_bytes().to_vec(),
                        extracted_knowledge: None,
                        next_routing_hint: None,
                    });
                }
            } else {
                return Ok(NodeResult {
                    output_data: clean_reply.as_bytes().to_vec(),
                    extracted_knowledge: None,
                    next_routing_hint: None,
                });
            }
        }

        Ok(NodeResult {
            output_data: b"Max ReAct loop reached".to_vec(),
            extracted_knowledge: None,
            next_routing_hint: None,
        })
    }
}

struct WasmToolNode {
    tool_name: String,
    tool_registry:
        std::sync::Arc<tokio::sync::RwLock<telos_tooling::retrieval::VectorToolRegistry>>,
    _tools_dir: String,
}

#[async_trait]
impl ExecutableNode for WasmToolNode {
    async fn execute(
        &self,
        _ctx: &ScopedContext,
        _registry: &dyn SystemRegistry,
    ) -> Result<NodeResult, NodeError> {
        let registry_guard = self.tool_registry.read().await;

        let discovered_tools = registry_guard.discover_tools(&self.tool_name, 1);
        if discovered_tools.is_empty() {
            return Err(NodeError::ExecutionFailed(format!(
                "No suitable tool found for intent: {}",
                self.tool_name
            )));
        }

        let tool_schema = &discovered_tools[0];
        let tool_name = &tool_schema.name;
        println!("[Daemon] Selected tool: {}", tool_name);

        let executor = registry_guard.get_executor(tool_name).ok_or_else(|| {
            NodeError::ExecutionFailed(format!("Tool executor not found for {}", tool_name))
        })?;

        // Extract parameters from LLM dynamically based on the tool schema and the node's prompt
        let system_registry = _registry;
        let gateway = system_registry
            .get_model_gateway()
            .and_then(|g| {
                g.downcast::<telos_model_gateway::gateway::GatewayManager>()
                    .ok()
            })
            .ok_or_else(|| {
                NodeError::ExecutionFailed(
                    "Failed to get GatewayManager for tool parameter extraction".into(),
                )
            })?;

        let prompt = format!(
            "You are a tool execution planner. Extract the necessary parameters for the tool '{}' based on the following task.\n\
            Task: {}\n\
            Tool Schema: {}\n\n\
            Respond strictly with a JSON object containing the parameters. Do not include markdown blocks.",
            tool_name,
            self.tool_name,
            serde_json::to_string_pretty(&tool_schema.parameters_schema.raw_schema).unwrap()
        );

        let req = telos_model_gateway::LlmRequest {
            session_id: "daemon_tool_param_extractor".to_string(),
            messages: vec![telos_model_gateway::Message {
                role: "user".to_string(),
                content: prompt,
            }],
            required_capabilities: telos_model_gateway::Capability {
                requires_vision: false,
                strong_reasoning: false,
            },
            budget_limit: 1000,
        };

        let params_json_str = match gateway.generate(req).await {
            Ok(res) => res
                .content
                .trim()
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .to_string(),
            Err(e) => {
                return Err(NodeError::ExecutionFailed(format!(
                    "Failed to extract tool parameters via LLM: {:?}",
                    e
                )))
            }
        };

        let params: serde_json::Value = serde_json::from_str(&params_json_str).map_err(|e| {
            NodeError::ExecutionFailed(format!(
                "LLM returned invalid JSON for tool parameters: {}",
                e
            ))
        })?;

        println!("[Daemon] Extracted Tool Parameters: {}", params);

        match executor.call(params).await {
            Ok(output_data) => {
                // Intercept __macro__: register_tool to fully support self-upgrading
                if let Ok(json_res) = serde_json::from_slice::<serde_json::Value>(&output_data) {
                    if json_res.get("__macro__").and_then(|m| m.as_str()) == Some("register_tool") {
                        if let (Some(wasm_path), Some(schema_val)) = (
                            json_res.get("wasm_path").and_then(|p| p.as_str()),
                            json_res.get("schema"),
                        ) {
                            if let Ok(wasm_bytes) = std::fs::read(wasm_path) {
                                let config_sb = telos_tooling::sandbox::SandboxConfig::default();
                                // We spawn blocking to compile the new Wasm safely
                                let wasm_bytes_clone = wasm_bytes.clone();
                                let executor_res = tokio::task::spawn_blocking(move || {
                                    telos_tooling::sandbox::WasmExecutor::new(
                                        wasm_bytes_clone,
                                        config_sb,
                                    )
                                })
                                .await
                                .unwrap();

                                match executor_res {
                                    Ok(wasm_executor) => {
                                        if let Ok(schema) =
                                            serde_json::from_value::<telos_tooling::ToolSchema>(
                                                schema_val.clone(),
                                            )
                                        {
                                            // Persist the tool to disk for future runs
                                            let target_dir = std::path::Path::new(&self._tools_dir);
                                            if !target_dir.exists() {
                                                let _ = std::fs::create_dir_all(target_dir);
                                            }
                                            let safe_name = schema
                                                .name
                                                .replace(|c: char| !c.is_alphanumeric(), "_");
                                            let dest_wasm =
                                                target_dir.join(format!("{}.wasm", safe_name));
                                            let dest_json =
                                                target_dir.join(format!("{}.json", safe_name));

                                            if let Err(e) = std::fs::copy(wasm_path, &dest_wasm) {
                                                println!("[Daemon] Warning: Failed to persist Wasm binary: {}", e);
                                            }
                                            if let Ok(schema_str) =
                                                serde_json::to_string_pretty(&schema)
                                            {
                                                if let Err(e) =
                                                    std::fs::write(&dest_json, schema_str)
                                                {
                                                    println!("[Daemon] Warning: Failed to persist Wasm Schema JSON: {}", e);
                                                }
                                            }

                                            // Drop read lock
                                            drop(registry_guard);
                                            let mut write_guard = self.tool_registry.write().await;
                                            write_guard.register_tool(
                                                schema,
                                                Some(std::sync::Arc::new(wasm_executor)),
                                            );
                                            println!("[Daemon] Successfully registered new Wasm Tool from path: {}", wasm_path);
                                            return Ok(NodeResult {
                                                output_data: format!(
                                                    "Tool Registration Successful: {}",
                                                    wasm_path
                                                )
                                                .into_bytes(),
                                                extracted_knowledge: None,
                                                next_routing_hint: None,
                                            });
                                        }
                                    }
                                    Err(e) => {
                                        return Err(NodeError::ExecutionFailed(format!(
                                            "Failed to compile newly registered Wasm tool: {}",
                                            e
                                        )))
                                    }
                                }
                            } else {
                                return Err(NodeError::ExecutionFailed(format!(
                                    "Failed to read newly generated Wasm file at path: {}",
                                    wasm_path
                                )));
                            }
                        }
                    } else if json_res.get("__macro__").and_then(|m| m.as_str()) == Some("memory_recall") {
                        // Handle memory_recall macro
                        let query = json_res.get("query").and_then(|q| q.as_str()).unwrap_or("");
                        let system_registry = _registry;
                        if let Some(memory_os_any) = system_registry.get_memory_os() {
                            if let Ok(memory_integration) = memory_os_any.clone().downcast::<telos_memory::engine::RedbGraphStore>() {
                                match memory_integration.retrieve_semantic_facts(query.to_string()).await {
                                    Ok(facts) => {
                                        let result = if facts.is_empty() {
                                            format!("No relevant memories found for '{}'", query)
                                        } else {
                                            format!("Memories for '{}':\n{}", query, facts.join("\n"))
                                        };
                                        return Ok(NodeResult {
                                            output_data: result.into_bytes(),
                                            extracted_knowledge: None,
                                            next_routing_hint: None,
                                        });
                                    }
                                    Err(e) => {
                                        return Ok(NodeResult {
                                            output_data: format!("Memory recall failed: {:?}", e).into_bytes(),
                                            extracted_knowledge: None,
                                            next_routing_hint: None,
                                        });
                                    }
                                }
                            } else {
                                return Ok(NodeResult {
                                    output_data: b"Memory system downcast failed".to_vec(),
                                    extracted_knowledge: None,
                                    next_routing_hint: None,
                                });
                            }
                        } else {
                            return Ok(NodeResult {
                                output_data: b"Memory system not available".to_vec(),
                                extracted_knowledge: None,
                                next_routing_hint: None,
                            });
                        }
                    } else if json_res.get("__macro__").and_then(|m| m.as_str()) == Some("memory_store") {
                        // Handle memory_store macro
                        let content = json_res.get("content").and_then(|c| c.as_str()).unwrap_or("");
                        let system_registry = _registry;
                        if let Some(memory_os_any) = system_registry.get_memory_os() {
                            if let Ok(memory_integration) = memory_os_any.clone().downcast::<telos_memory::engine::RedbGraphStore>() {
                                match memory_integration.store_semantic_fact(content.to_string()).await {
                                    Ok(_) => {
                                        return Ok(NodeResult {
                                            output_data: format!("Successfully stored in memory: {}", content).into_bytes(),
                                            extracted_knowledge: None,
                                            next_routing_hint: None,
                                        });
                                    }
                                    Err(e) => {
                                        return Ok(NodeResult {
                                            output_data: format!("Failed to store in memory: {:?}", e).into_bytes(),
                                            extracted_knowledge: None,
                                            next_routing_hint: None,
                                        });
                                    }
                                }
                            } else {
                                return Ok(NodeResult {
                                    output_data: b"Memory system downcast failed".to_vec(),
                                    extracted_knowledge: None,
                                    next_routing_hint: None,
                                });
                            }
                        } else {
                            return Ok(NodeResult {
                                output_data: b"Memory system not available".to_vec(),
                                extracted_knowledge: None,
                                next_routing_hint: None,
                            });
                        }
                    }
                }

                Ok(NodeResult {
                    output_data,
                    extracted_knowledge: None,
                    next_routing_hint: None,
                })
            }
            Err(e) => Err(NodeError::ExecutionFailed(format!(
                "Tool Execution failed: {:?}",
                e
            ))),
        }
    }
}

// 4. Server App State
#[derive(Clone)]
struct AppState {
    broker: Arc<TokioEventBroker>,
}

#[derive(Deserialize)]
struct RunRequest {
    payload: String,
    project_id: Option<String>,
}

#[derive(Serialize)]
struct RunResponse {
    status: String,
    trace_id: String,
}

#[derive(Deserialize)]
struct ApproveRequest {
    task_id: String,
    approved: bool,
}

#[derive(Serialize)]
struct ApproveResponse {
    status: String,
}

#[derive(Deserialize)]
struct SetLogLevelRequest {
    level: String,
}

#[derive(Serialize)]
struct GetLogLevelResponse {
    level: String,
}

#[derive(Serialize)]
struct SetLogLevelResponse {
    status: String,
    old_level: String,
    new_level: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Initializing Telos Daemon...");

    let config = TelosConfig::load().expect(
        "Failed to load configuration. Please run `telos cli` first to complete initialization.",
    );

    // --- WIRING ---
    let (broker, mut event_rx) = TokioEventBroker::new(1000, 1000, 1024);
    let broker = Arc::new(broker);

    let openai_provider = OpenAiProvider::new(
        config.openai_api_key.clone(),
        config.openai_base_url.clone(),
        config.openai_model.clone(),
        config.openai_embedding_model.clone(),
    );
    let gateway_adapter = Arc::new(GatewayAdapter {
        inner: openai_provider.clone(),
    });
    let gateway = Arc::new(GatewayManager::new(gateway_adapter, 10000, 10.0));

    // Initialize VectorToolRegistry with Native Tools
    let mut tool_registry = telos_tooling::retrieval::VectorToolRegistry::new_keyword_only();
    tool_registry.register_tool(
        telos_tooling::native::FsReadTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::FsReadTool)),
    );
    tool_registry.register_tool(
        telos_tooling::native::FsWriteTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::FsWriteTool)),
    );
    tool_registry.register_tool(
        telos_tooling::native::ShellExecTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::ShellExecTool)),
    );
    tool_registry.register_tool(
        telos_tooling::native::CalculatorTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::CalculatorTool)),
    );
    tool_registry.register_tool(
        telos_tooling::native::ToolRegisterTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::ToolRegisterTool)),
    );
    // Register memory tools for explicit memory operations
    tool_registry.register_tool(
        telos_tooling::native::MemoryRecallTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::MemoryRecallTool)),
    );
    tool_registry.register_tool(
        telos_tooling::native::MemoryStoreTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::MemoryStoreTool)),
    );
    // Register code editing tools
    tool_registry.register_tool(
        telos_tooling::native::FileEditTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::FileEditTool)),
    );
    // Register file search tools
    tool_registry.register_tool(
        telos_tooling::native::GlobTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::GlobTool)),
    );
    tool_registry.register_tool(
        telos_tooling::native::GrepTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::GrepTool)),
    );
    // Register web tools
    tool_registry.register_tool(
        telos_tooling::native::HttpTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::HttpTool)),
    );
    tool_registry.register_tool(
        telos_tooling::native::WebSearchTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::WebSearchTool)),
    );
    // Register LSP tool for code navigation
    tool_registry.register_tool(
        telos_tooling::native::LspTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::LspTool)),
    );

    // --- Auto-Load Persisted Tools on Startup ---
    let target_dir = std::path::Path::new(&config.tools_dir);
    if target_dir.exists() && target_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(target_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Ok(json_content) = std::fs::read_to_string(&path) {
                        if let Ok(schema) =
                            serde_json::from_str::<telos_tooling::ToolSchema>(&json_content)
                        {
                            let wasm_path = path.with_extension("wasm");
                            if wasm_path.exists() {
                                if let Ok(wasm_bytes) = std::fs::read(&wasm_path) {
                                    let sb_config =
                                        telos_tooling::sandbox::SandboxConfig::default();
                                    let executor_res = tokio::task::block_in_place(|| {
                                        telos_tooling::sandbox::WasmExecutor::new(
                                            wasm_bytes, sb_config,
                                        )
                                    });

                                    if let Ok(wasm_executor) = executor_res {
                                        tool_registry.register_tool(
                                            schema,
                                            Some(std::sync::Arc::new(wasm_executor)),
                                        );
                                        println!(
                                            "[Daemon] Auto-loaded persisted tool from {:?}",
                                            wasm_path.file_name().unwrap()
                                        );
                                    } else {
                                        eprintln!(
                                            "[Daemon] Failed to compile persisted tool at {:?}",
                                            wasm_path
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let tool_registry = std::sync::Arc::new(tokio::sync::RwLock::new(tool_registry));

    let memory_os_instance = Arc::new(RedbGraphStore::new(&config.db_path).expect("Failed to init MemoryOS database"));
    let registry = Arc::new(DaemonRegistry {
        gateway: gateway.clone(),
        memory_os: memory_os_instance.clone(),
    });
    let _memory =
        Arc::new(RedbGraphStore::new(&config.db_path).expect("Failed to init MemoryOS database"));
    // Using cloud embeddings as configured
    let context_manager = Arc::new(RaptorContextManager::new(
        Arc::new(openai_provider.clone()),
        Arc::new(openai_provider.clone()),
        Some(memory_os_instance.clone() as Arc<dyn telos_memory::integration::MemoryIntegration>),
    ));

    // --- BACKGROUND EVENT LOOP ---
    let broker_bg = Arc::clone(&broker);
    let gateway_clone = gateway.clone();
    let registry_clone = registry.clone();
    let tool_registry = tool_registry.clone();
    let loop_config = config.clone();
    let paused_tasks: Arc<TokioMutex<HashMap<String, String>>> =
        Arc::new(TokioMutex::new(HashMap::new()));
    let paused_tasks_bg = paused_tasks.clone();

    tokio::spawn(async move {
        println!("[Daemon] Event loop started.");
        while let Some(event) = event_rx.recv().await {
            match event {
                AgentEvent::SetLogLevel { level } => {
                    let old_level = global_log_level().get();
                    global_log_level().set(level);
                    broker_bg.publish_feedback(AgentFeedback::LogLevelChanged {
                        old_level,
                        new_level: level,
                    });
                    println!(
                        "[Daemon] Log level changed: {:?} -> {:?}",
                        old_level, level
                    );
                }
                AgentEvent::UserInput {
                    session_id,
                    payload,
                    trace_id,
                    project_id,
                } => {
                    let broker_bg = broker_bg.clone();
                    let gateway_clone = gateway_clone.clone();
                    let registry_clone = registry_clone.clone();
                    let tool_registry = tool_registry.clone();
                    let config = loop_config.clone();
                    let paused_tasks_bg = paused_tasks_bg.clone();
                    let mut execution_engine = telos_dag::engine::TokioExecutionEngine::new();

                    let context_manager_spawn = context_manager.clone();
                    tokio::spawn(async move {
                        let task_start_time = Instant::now();
                        println!(
                            "[Daemon] Received UserInput: {} (trace: {})",
                            payload, trace_id
                        );

                        let mut enriched_payload = payload.clone();

                        if let Some(pid) = &project_id {
                            println!("[Daemon] Active Project ID: {}", pid);
                            if let Ok(Some(project)) =
                                telos_project::manager::ProjectRegistry::new().get_project(pid)
                            {
                                let working_dir = project.path.clone();
                                println!("[Daemon] Project working directory: {:?}", working_dir);

                                // Load custom project instructions
                                let project_config =
                                    telos_core::project::ProjectConfig::load(&working_dir);

                                // Dynamically inject project context into the payload for the agent
                                enriched_payload = format!(
                                    "Context:\n- Active Project: {}\n- Description: {}\n- Working Directory: {:?}\n- Custom Instructions: {}\n\nTask:\n{}",
                                    project.name,
                                    project.description.unwrap_or_else(|| "None".to_string()),
                                    working_dir,
                                    project_config.custom_instructions.unwrap_or_else(|| "None".to_string()),
                                    payload
                                );

                                println!(
                                    "[Daemon] Dynamically injected project context into payload."
                                );
                            }
                        }

                        broker_bg.publish_feedback(AgentFeedback::StateChanged {
                            task_id: trace_id.to_string(),
                            current_node: "planning".into(),
                            status: telos_core::NodeStatus::Running,
                        });

                        if payload.contains("sudo") {
                            broker_bg.publish_feedback(AgentFeedback::RequireHumanIntervention {
                                task_id: trace_id.to_string(),
                                prompt: format!("Task '{}' requires elevated privileges.", payload),
                                risk_level: telos_core::RiskLevel::HighRisk,
                            });
                            paused_tasks_bg
                                .lock()
                                .await
                                .insert(trace_id.to_string(), payload.clone());
                            return;
                        }

                        // --- DYNAMIC DAG GENERATION VIA LLM ---
                        let prompt = format!(
                            r#"You are an expert planner. First classify the task complexity, then break it down appropriately.

## Tier Classification Rules:
1. "Simple" - Single LLM call is enough:
   - Simple questions, explanations, summaries
   - Basic math (2+2), creative writing
   - Code explanation, knowledge retrieval
   → nodes: empty (single LLM call will be used automatically)

2. "Medium" - Requires tool usage in a loop (ReAct pattern):
   - Tasks needing file reads, web searches
   - Multi-step reasoning with tool calls
   - Code tasks requiring multiple file operations
   → nodes: empty (ReAct loop will be used automatically)

3. "Complex" - Requires explicit DAG with multiple parallel/sequential nodes:
   - Multi-stage workflows with dependencies
   - Parallel task execution needed
   - Complex orchestration between different components
   → nodes: explicit DAG nodes required

## Available Tools (for Medium/Complex tiers):
- fs_read, fs_write: File operations
- shell_exec: Shell commands
- calculator: Math calculations
- memory_recall, memory_store: Memory operations
- file_edit: Edit files by search/replace
- glob, grep: File/code search
- http_get, web_search: Web operations
- lsp_symbol_search: Code navigation

## Instructions:
1. Classify the task tier (Simple/Medium/Complex)
2. For Simple/Medium: no nodes needed (automatic handling)
3. For Complex: create DAG nodes with proper edges

Task: {}

Respond strictly with a JSON object matching this schema:
{{
    "tier": "Simple" or "Medium" or "Complex",
    "reply": "string (friendly acknowledgment of the task)",
    "nodes": [ {{ "id": "string", "task_type": "LLM" or "TOOL", "prompt": "Detailed execution instruction" }} ],
    "edges": [ {{ "from": "node_id_1", "to": "node_id_2" }} ]
}}
Do not include markdown blocks, only raw JSON."#,
                            payload
                        );

                        let classification_req = LlmRequest {
                            session_id: "daemon_planner".to_string(),
                            messages: vec![telos_model_gateway::Message {
                                role: "user".to_string(),
                                content: prompt,
                            }],
                            required_capabilities: Capability {
                                requires_vision: false,
                                strong_reasoning: false,
                            },
                            budget_limit: 2000,
                        };

                        let plan_json = match gateway_clone.generate(classification_req).await {
                            Ok(res) => res
                                .content
                                .trim()
                                .trim_start_matches("```json")
                                .trim_start_matches("```")
                                .trim_end_matches("```")
                                .to_string(),
                            Err(e) => {
                                broker_bg.publish_feedback(AgentFeedback::Output {
                                    task_id: trace_id.to_string(),
                                    session_id: session_id.clone(),
                                    content: format!("Planning Failed: {:?}", e),
                                    is_final: true,
                                });
                                return;
                            }
                        };

                        println!("LLM Plan JSON: {}", plan_json);

                        let dag_plan: DagPlan = match serde_json::from_str(&plan_json) {
                            Ok(plan) => plan,
                            Err(e) => {
                                // Fallback if LLM fails to return valid JSON
                                println!(
                                    "Failed to parse DAG plan: {}. Using fallback single node.",
                                    e
                                );
                                DagPlan {
                                    tier: Some("Complex".to_string()),
                                    reply: Some("I will handle that.".to_string()),
                                    nodes: vec![DagNode {
                                        id: "main".to_string(),
                                        task_type: "LLM".to_string(),
                                        prompt: enriched_payload.clone(),
                                    }],
                                    edges: vec![],
                                }
                            }
                        };

                        let mut graph = TaskGraph::new(trace_id.to_string());
                        let mut terminal_nodes = vec![];

                        // Build plan info for PlanCreated feedback
                        let mut plan_node_infos: Vec<PlanNodeInfo> = Vec::new();

                        let tier = dag_plan.tier.clone().unwrap_or("Complex".to_string());

                        let mut _plan_node_infos: Vec<PlanNodeInfo> = Vec::new();

                        if tier == "Simple" {
                            terminal_nodes.push("simple_node".to_string());
                            graph.add_node_with_metadata(
                                "simple_node".to_string(),
                                Box::new(LlmPromptNode {
                                    prompt: enriched_payload.clone(),
                                    gateway: gateway_clone.clone(),
                                }),
                                NodeMetadata {
                                    task_type: "LLM".to_string(),
                                    prompt_preview: truncate_for_preview(&enriched_payload, 100),
                                    tool_name: None,
                                },
                            );
                            graph.current_state = GraphState { is_running: true, completed: false };
                        } else if tier == "Medium" {
                            terminal_nodes.push("react_node".to_string());
                            graph.add_node_with_metadata(
                                "react_node".to_string(),
                                Box::new(ReactNode {
                                    prompt: enriched_payload.clone(),
                                    gateway: gateway_clone.clone(),
                                    tool_registry: tool_registry.clone(),
                                    _tools_dir: config.tools_dir.clone(),
                                }),
                                NodeMetadata {
                                    task_type: "LLM".to_string(),
                                    prompt_preview: truncate_for_preview(&enriched_payload, 100),
                                    tool_name: Some("ReactLoop".to_string()),
                                },
                            );
                            graph.current_state = GraphState { is_running: true, completed: false };
                        } else {
                            // Build plan info for PlanCreated feedback
                            for node in &dag_plan.nodes {
                                terminal_nodes.push(node.id.clone());

                                // Build node metadata
                                let metadata = NodeMetadata {
                                    task_type: node.task_type.clone(),
                                    prompt_preview: truncate_for_preview(&node.prompt, 100),
                                    tool_name: if node.task_type == "TOOL" {
                                        Some(node.prompt.clone())
                                    } else {
                                        None
                                    },
                                };

                                let node_info = PlanNodeInfo {
                                    id: node.id.clone(),
                                    task_type: node.task_type.clone(),
                                    prompt_preview: truncate_for_preview(&node.prompt, 150),
                                    dependencies: vec![],
                                };
                                plan_node_infos.push(node_info);

                                if node.task_type.to_uppercase() == "TOOL" {
                                    graph.add_node_with_metadata(
                                        node.id.clone(),
                                        Box::new(WasmToolNode {
                                            tool_name: node.prompt.clone(),
                                            tool_registry: tool_registry.clone(),
                                            _tools_dir: config.tools_dir.clone(),
                                        }),
                                        metadata,
                                    );
                                } else {
                                    graph.add_node_with_metadata(
                                        node.id.clone(),
                                        Box::new(LlmPromptNode {
                                            prompt: node.prompt.clone(),
                                            gateway: gateway_clone.clone(),
                                        }),
                                        metadata,
                                    );
                                }
                            }

                            for edge in &dag_plan.edges {
                                terminal_nodes.retain(|id| id != &edge.from);
                                let _ = graph.add_edge(&edge.from, &edge.to);

                                // Update dependencies in plan info
                                if let Some(node_info) =
                                    plan_node_infos.iter_mut().find(|n| n.id == edge.to)
                                {
                                    node_info.dependencies.push(edge.from.clone());
                                }
                            }

                            if terminal_nodes.is_empty() && !dag_plan.nodes.is_empty() {
                                terminal_nodes.push(dag_plan.nodes.last().unwrap().id.clone());
                            }

                            // Publish PlanCreated feedback
                            let plan_info = PlanInfo {
                                reply: dag_plan.reply.clone(),
                                nodes: plan_node_infos.clone(),
                                total_steps: dag_plan.nodes.len(),
                                estimated_complexity: if dag_plan.nodes.len() <= 2 {
                                    Some("low".to_string())
                                } else if dag_plan.nodes.len() <= 5 {
                                    Some("medium".to_string())
                                } else {
                                    Some("high".to_string())
                                },
                            };

                            broker_bg.publish_feedback(AgentFeedback::PlanCreated {
                                task_id: trace_id.to_string(),
                                plan: plan_info,
                            });

                            graph.current_state = GraphState {
                                is_running: true,
                                completed: false,
                            };
                        }

                        // Build context using the context manager with memory integration
                        let raw_ctx = telos_context::RawContext {
                            history_logs: vec![],  // Could include session history here
                            retrieved_docs: vec![],
                        };
                        let ctx_req = telos_context::NodeRequirement {
                            required_tokens: 2000,
                            query: enriched_payload.clone(),
                        };
                        let actual_ctx = context_manager_spawn.compress_for_node(&raw_ctx, &ctx_req).await
                            .unwrap_or_else(|e| {
                                println!("[Daemon] Context compression failed: {:?}, using empty context", e);
                                telos_context::ScopedContext {
                                    budget_tokens: 1000,
                                    summary_tree: vec![],
                                    precise_facts: vec![],
                                }
                            });

                        println!("[Daemon] Context prepared with {} summary nodes and {} precise facts",
                            actual_ctx.summary_tree.len(),
                            actual_ctx.precise_facts.len());

                        execution_engine
                            .run_graph(
                                &mut graph,
                                &actual_ctx,
                                registry_clone.as_ref(),
                                broker_bg.as_ref(),
                            )
                            .await;

                        // Calculate task summary
                        let total_time_ms = task_start_time.elapsed().as_millis() as u64;
                        let mut completed_nodes = 0;
                        let mut failed_nodes = 0;
                        let mut failed_node_ids: Vec<String> = Vec::new();

                        for (node_id, status) in &graph.node_statuses {
                            if *status == telos_core::NodeStatus::Completed {
                                completed_nodes += 1;
                            } else if *status == telos_core::NodeStatus::Failed {
                                failed_nodes += 1;
                                failed_node_ids.push(node_id.clone());
                            }
                        }

                        // Fetch the results from the terminal nodes
                        let mut final_results = Vec::new();
                        for node_id in &terminal_nodes {
                            if let Some(Ok(res)) = graph.node_results.get(node_id) {
                                final_results.push(format!(
                                    "[{}] {}",
                                    node_id,
                                    String::from_utf8_lossy(&res.output_data)
                                ));
                            } else if let Some(Err(e)) = graph.node_results.get(node_id) {
                                final_results.push(format!("[{}] Failed: {:?}", node_id, e));
                            }
                        }

                        let combined_result = if final_results.is_empty() {
                            "No result generated by graph".to_string()
                        } else {
                            final_results.join("\n")
                        };

                        let task_success = failed_nodes == 0;

                        // Build summary message
                        let summary = if task_success {
                            format!(
                                "Task completed successfully. {} node(s) executed in {:.1}s.",
                                completed_nodes,
                                total_time_ms as f64 / 1000.0
                            )
                        } else {
                            format!(
                                "Task finished with errors. {} succeeded, {} failed. Node(s) failed: {}",
                                completed_nodes,
                                failed_nodes,
                                failed_node_ids.join(", ")
                            )
                        };

                        // Publish TaskCompleted feedback
                        broker_bg.publish_feedback(AgentFeedback::TaskCompleted {
                            task_id: trace_id.to_string(),
                            summary: TaskSummary {
                                success: task_success,
                                total_nodes: graph.node_statuses.len(),
                                completed_nodes,
                                failed_nodes,
                                total_time_ms,
                                summary: summary.clone(),
                                failed_node_ids: failed_node_ids.clone(),
                            },
                        });

                        // Also send Output for backward compatibility
                        broker_bg.publish_feedback(AgentFeedback::Output {
                            task_id: trace_id.to_string(),
                            session_id,
                            content: format!("{}\n\n{}", summary, combined_result),
                            is_final: true,
                        });
                    });
                }
                AgentEvent::UserApproval {
                    task_id, approved, ..
                } => {
                    let broker_bg = broker_bg.clone();
                    let gateway_clone = gateway_clone.clone();
                    let registry_clone = registry_clone.clone();
                    let paused_tasks_bg = paused_tasks_bg.clone();
                    let mut execution_engine = telos_dag::engine::TokioExecutionEngine::new();

                    let context_manager_approval = context_manager.clone();
                    tokio::spawn(async move {
                        let task_start_time = Instant::now();
                        println!(
                            "[Daemon] Received UserApproval for task {} (approved: {})",
                            task_id, approved
                        );

                        if !approved {
                            broker_bg.publish_feedback(AgentFeedback::Output {
                                task_id: task_id.clone(),
                                session_id: "default".into(),
                                content: "Task Rejected.".into(),
                                is_final: true,
                            });
                            paused_tasks_bg.lock().await.remove(&task_id);

                            // Publish TaskCompleted for rejected task
                            broker_bg.publish_feedback(AgentFeedback::TaskCompleted {
                                task_id: task_id.clone(),
                                summary: TaskSummary {
                                    success: false,
                                    total_nodes: 0,
                                    completed_nodes: 0,
                                    failed_nodes: 0,
                                    total_time_ms: 0,
                                    summary: "Task was rejected by user".to_string(),
                                    failed_node_ids: vec![],
                                },
                            });
                            return;
                        }

                        // User approved. Retrieve the paused task payload and execute it.
                        let payload_opt = paused_tasks_bg.lock().await.remove(&task_id);
                        if let Some(payload) = payload_opt {
                            broker_bg.publish_feedback(AgentFeedback::Output {
                                task_id: task_id.clone(),
                                session_id: "default".into(),
                                content: "Task Approved. Executing...".into(),
                                is_final: false,
                            });

                            let mut graph = TaskGraph::new(task_id.clone());
                            graph.add_node_with_metadata(
                                "llm_node".to_string(),
                                Box::new(LlmPromptNode {
                                    prompt: format!(
                                        "Execute the following elevated user command: {}",
                                        payload
                                    ),
                                    gateway: gateway_clone.clone(),
                                }),
                                NodeMetadata {
                                    task_type: "LLM".to_string(),
                                    prompt_preview: truncate_for_preview(&payload, 100),
                                    tool_name: None,
                                },
                            );
                            graph.current_state = GraphState {
                                is_running: true,
                                completed: false,
                            };

                            let raw_ctx = telos_context::RawContext {
                                history_logs: vec![],
                                retrieved_docs: vec![
                                    telos_context::Document {
                                        doc_id: "user_input".to_string(),
                                        content: payload.clone(),
                                    }
                                ],
                            };
                            let req = telos_context::NodeRequirement {
                                required_tokens: 1000,
                                query: payload.clone(),
                            };
                            let actual_ctx = context_manager_approval.compress_for_node(&raw_ctx, &req).await.unwrap_or_else(|_e| telos_context::ScopedContext {
                                budget_tokens: 1000,
                                summary_tree: vec![],
                                precise_facts: vec![],
                            });

                            execution_engine
                                .run_graph(
                                    &mut graph,
                                    &actual_ctx,
                                    registry_clone.as_ref(),
                                    broker_bg.as_ref(),
                                )
                                .await;

                            let final_result = match graph.node_results.get("llm_node") {
                                Some(Ok(res)) => {
                                    String::from_utf8_lossy(&res.output_data).to_string()
                                }
                                Some(Err(e)) => format!("Error executing node: {:?}", e),
                                None => "No result generated by node".to_string(),
                            };

                            let total_time_ms = task_start_time.elapsed().as_millis() as u64;
                            let task_success = graph
                                .node_statuses
                                .get("llm_node")
                                .map(|s| *s == telos_core::NodeStatus::Completed)
                                .unwrap_or(false);

                            // Publish TaskCompleted
                            broker_bg.publish_feedback(AgentFeedback::TaskCompleted {
                                task_id: task_id.clone(),
                                summary: TaskSummary {
                                    success: task_success,
                                    total_nodes: 1,
                                    completed_nodes: if task_success { 1 } else { 0 },
                                    failed_nodes: if task_success { 0 } else { 1 },
                                    total_time_ms,
                                    summary: if task_success {
                                        "Approved task completed successfully".to_string()
                                    } else {
                                        "Approved task failed during execution".to_string()
                                    },
                                    failed_node_ids: if task_success {
                                        vec![]
                                    } else {
                                        vec!["llm_node".to_string()]
                                    },
                                },
                            });

                            broker_bg.publish_feedback(AgentFeedback::Output {
                                task_id: task_id.clone(),
                                session_id: "default".into(),
                                content: format!(
                                    "Execution Complete. LLM Response: {}",
                                    final_result
                                ),
                                is_final: true,
                            });
                        } else {
                            broker_bg.publish_feedback(AgentFeedback::Output {
                                task_id: task_id.clone(),
                                session_id: "default".into(),
                                content: "Task failed to resume: Payload lost or expired.".into(),
                                is_final: true,
                            });

                            broker_bg.publish_feedback(AgentFeedback::TaskCompleted {
                                task_id: task_id.clone(),
                                summary: TaskSummary {
                                    success: false,
                                    total_nodes: 0,
                                    completed_nodes: 0,
                                    failed_nodes: 0,
                                    total_time_ms: 0,
                                    summary: "Task failed to resume: Payload lost or expired"
                                        .to_string(),
                                    failed_node_ids: vec![],
                                },
                            });
                        }
                    });
                }
                _ => {}
            }
        }
    });

    // Start Bot Provider in background if configured
    if let Some(bot_token) = config.telegram_bot_token.clone() {
        println!("Starting Telegram Bot Provider from Daemon...");
        let daemon_url = "http://127.0.0.1:3000".to_string();
        let daemon_ws_url = "ws://127.0.0.1:3000/api/v1/stream".to_string();
        let send_state_changes = config.bot_send_state_changes;

        tokio::spawn(async move {
            let provider = telos_bot::providers::telegram::TelegramBotProvider::new(
                bot_token,
                daemon_url,
                daemon_ws_url,
                send_state_changes,
            );
            if let Err(e) = telos_bot::traits::ChatBotProvider::start(&provider).await {
                eprintln!("Failed to start bot provider: {}", e);
            }
        });
    }

    // --- API SERVER ---
    let state = AppState {
        broker: broker.clone(),
    };

    let app = Router::new()
        .route("/api/v1/run", post(handle_run))
        .route("/api/v1/approve", post(handle_approve))
        .route("/api/v1/stream", get(ws_handler))
        .route("/api/v1/log-level", get(get_log_level).post(set_log_level))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    println!("Telos Daemon listening on ws://0.0.0.0:3000/api/v1/stream");

    axum::serve(listener, app).await?;

    Ok(())
}

async fn handle_run(
    State(state): State<AppState>,
    Json(req): Json<RunRequest>,
) -> Json<RunResponse> {
    let trace_id = Uuid::new_v4();
    let _ = state
        .broker
        .publish_event(AgentEvent::UserInput {
            session_id: "default".into(),
            payload: req.payload,
            trace_id,
            project_id: req.project_id,
        })
        .await;

    Json(RunResponse {
        status: "accepted".into(),
        trace_id: trace_id.to_string(),
    })
}

async fn handle_approve(
    State(state): State<AppState>,
    Json(req): Json<ApproveRequest>,
) -> Json<ApproveResponse> {
    let trace_id = Uuid::new_v4();
    let _ = state
        .broker
        .publish_event(AgentEvent::UserApproval {
            task_id: req.task_id,
            approved: req.approved,
            supplement_info: None,
            trace_id,
        })
        .await;

    Json(ApproveResponse {
        status: "approval received".into(),
    })
}

async fn get_log_level() -> Json<GetLogLevelResponse> {
    let level = global_log_level().get();
    Json(GetLogLevelResponse {
        level: format!("{:?}", level).to_lowercase(),
    })
}

async fn set_log_level(
    State(state): State<AppState>,
    Json(req): Json<SetLogLevelRequest>,
) -> Json<SetLogLevelResponse> {
    let old_level = global_log_level().get();
    let new_level = LogLevel::from_string(&req.level);

    global_log_level().set(new_level);

    // Publish LogLevelChanged feedback via broker
    state
        .broker
        .publish_feedback(AgentFeedback::LogLevelChanged {
            old_level,
            new_level,
        });

    Json(SetLogLevelResponse {
        status: "ok".into(),
        old_level: format!("{:?}", old_level).to_lowercase(),
        new_level: format!("{:?}", new_level).to_lowercase(),
    })
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let mut rx = state.broker.subscribe_feedback();

    while let Ok(feedback) = rx.recv().await {
        let msg_str = serde_json::to_string(&feedback).unwrap_or_else(|_| "{}".to_string());
        if socket.send(Message::Text(msg_str)).await.is_err() {
            break;
        }
    }
}
