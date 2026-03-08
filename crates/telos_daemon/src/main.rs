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
use tokio::sync::Mutex as TokioMutex;
use uuid::Uuid;

// Core Traits and Primitives
use async_trait::async_trait;
use telos_context::providers::OpenAiProvider;
use telos_context::{RaptorContextManager, ScopedContext};
use telos_core::config::TelosConfig;
use telos_core::{NodeError, NodeResult, SystemRegistry};
use telos_dag::ExecutionEngine;
use telos_dag::{ExecutableNode, GraphState, TaskGraph};
use telos_hci::{AgentEvent, AgentFeedback, EventBroker, TokioEventBroker};
use telos_memory::engine::RedbGraphStore;
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
            Err(e) => Err(GatewayError::Other(e.0)),
        }
    }
}

// 2. System Registry
struct DaemonRegistry {
    gateway: Arc<GatewayManager>,
}

impl SystemRegistry for DaemonRegistry {
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
    reply: Option<String>,
    nodes: Vec<DagNode>,
    edges: Vec<DagEdge>,
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

struct WasmToolNode {
    tool_name: String,
    tool_registry:
        std::sync::Arc<tokio::sync::RwLock<telos_tooling::retrieval::VectorToolRegistry>>,
    tools_dir: String,
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
                                            let target_dir = std::path::Path::new(&self.tools_dir);
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
        telos_tooling::native::FsListDirTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::FsListDirTool)),
    );
    tool_registry.register_tool(
        telos_tooling::native::CodeSearchTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::CodeSearchTool)),
    );
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
        telos_tooling::native::ToolRegisterTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::ToolRegisterTool)),
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

    let registry = Arc::new(DaemonRegistry {
        gateway: gateway.clone(),
    });
    let _memory =
        Arc::new(RedbGraphStore::new(&config.db_path).expect("Failed to init MemoryOS database"));
    // Using cloud embeddings as configured
    let _context_manager = Arc::new(RaptorContextManager::new(
        Arc::new(openai_provider.clone()),
        Arc::new(openai_provider.clone()),
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

                    tokio::spawn(async move {
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
                            "You are an expert planner. Break down the following task into a directed acyclic graph (DAG) of sub-tasks.\nFirst, generate a friendly, conversational response in the `reply` field acknowledging the user's intent.\nTask: {}\n\nRespond strictly with a JSON object matching this schema:\n{{\n    \"reply\": \"string\",\n    \"nodes\": [ {{ \"id\": \"string\", \"task_type\": \"LLM\" or \"TOOL\", \"prompt\": \"Detailed execution instruction for this node\" }} ],\n    \"edges\": [ {{ \"from\": \"node_id_1\", \"to\": \"node_id_2\" }} ]\n}}\nDo not include markdown blocks, only raw JSON.",
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

                        if let Some(reply) = &dag_plan.reply {
                            broker_bg.publish_feedback(AgentFeedback::Output {
                                task_id: trace_id.to_string(),
                                session_id: session_id.clone(),
                                content: reply.clone(),
                                is_final: false,
                            });
                        }

                        for node in &dag_plan.nodes {
                            terminal_nodes.push(node.id.clone());
                            if node.task_type == "TOOL" {
                                graph.add_node(
                                    node.id.clone(),
                                    Box::new(WasmToolNode {
                                        tool_name: node.prompt.clone(),
                                        tool_registry: tool_registry.clone(),
                                        tools_dir: config.tools_dir.clone(),
                                    }),
                                );
                            } else {
                                graph.add_node(
                                    node.id.clone(),
                                    Box::new(LlmPromptNode {
                                        prompt: node.prompt.clone(),
                                        gateway: gateway_clone.clone(),
                                    }),
                                );
                            }
                        }

                        for edge in &dag_plan.edges {
                            let _ = graph.add_edge(&edge.from, &edge.to);
                            terminal_nodes.retain(|id| id != &edge.from); // Keep only nodes that have no outgoing edges
                        }

                        if terminal_nodes.is_empty() && !dag_plan.nodes.is_empty() {
                            terminal_nodes.push(dag_plan.nodes.last().unwrap().id.clone());
                        }

                        graph.current_state = GraphState {
                            is_running: true,
                            completed: false,
                        };

                        let empty_ctx = telos_context::ScopedContext {
                            budget_tokens: 1000,
                            summary_tree: vec![],
                            precise_facts: vec![],
                        };

                        execution_engine
                            .run_graph(
                                &mut graph,
                                &empty_ctx,
                                registry_clone.as_ref(),
                                broker_bg.as_ref(),
                            )
                            .await;

                        // Fetch the results from the terminal nodes
                        let mut final_results = Vec::new();
                        for node_id in terminal_nodes {
                            if let Some(Ok(res)) = graph.node_results.get(&node_id) {
                                final_results.push(format!(
                                    "[{}] {}",
                                    node_id,
                                    String::from_utf8_lossy(&res.output_data)
                                ));
                            } else if let Some(Err(e)) = graph.node_results.get(&node_id) {
                                final_results.push(format!("[{}] Failed: {:?}", node_id, e));
                            }
                        }

                        let combined_result = if final_results.is_empty() {
                            "No result generated by graph".to_string()
                        } else {
                            final_results.join("\n")
                        };
                        broker_bg.publish_feedback(AgentFeedback::Output {
                            task_id: trace_id.to_string(),
                            session_id,
                            content: format!("Execution Complete. Responses:\n{}", combined_result),
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

                    tokio::spawn(async move {
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
                            graph.add_node(
                                "llm_node".to_string(),
                                Box::new(LlmPromptNode {
                                    prompt: format!(
                                        "Execute the following elevated user command: {}",
                                        payload
                                    ),
                                    gateway: gateway_clone.clone(),
                                }),
                            );
                            graph.current_state = GraphState {
                                is_running: true,
                                completed: false,
                            };

                            let empty_ctx = telos_context::ScopedContext {
                                budget_tokens: 1000,
                                summary_tree: vec![],
                                precise_facts: vec![],
                            };

                            execution_engine
                                .run_graph(
                                    &mut graph,
                                    &empty_ctx,
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
