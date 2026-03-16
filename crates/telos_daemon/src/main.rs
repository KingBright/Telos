use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query,
        State,
    },
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
    Json, Router,
};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use telos_context::ContextManager;
use tokio::sync::Mutex as TokioMutex;
use tracing::{debug, info, warn, error};
use uuid::Uuid;
mod agents;
pub use agents::*;

// Core Traits and Primitives
use async_trait::async_trait;
use telos_context::providers::OpenAiProvider;
use telos_context::RaptorContextManager;
use telos_core::config::TelosConfig;
use telos_core::{AgentInput, AgentOutput, SystemRegistry};
use telos_dag::{ExecutableNode, ExecutionEngine, GraphState, NodeMetadata, TaskGraph};
use telos_hci::{
    global_log_level, AgentEvent, AgentFeedback, EventBroker, LogLevel,
    TaskSummary, TokioEventBroker,
};
use telos_memory::engine::{RedbGraphStore, MemoryOS};
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
        let messages: Vec<serde_json::Value> = req.messages.iter().map(|m| {
            serde_json::json!({
                "role": m.role,
                "content": m.content
            })
        }).collect();

        if let Some(last) = req.messages.last() {
             info!(
                "[GatewayAdapter] Calling LLM with {} messages, last message length: {} bytes",
                messages.len(),
                last.content.len()
            );
        }

        // Convert tools from LlmRequest to OpenAI format
        let tools = req.tools.as_ref().map(|tool_defs| {
            tool_defs.iter().map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters
                    }
                })
            }).collect::<Vec<_>>()
        });

        match self.inner.generate_chat_with_tools(messages, tools).await {
            Ok(resp) => {
                // Approximate tokens: ~4 chars per token
                let mut total_len = 0;
                for m in &req.messages {
                    total_len += m.content.len();
                }
                let estimated_tokens = (total_len + resp.content.len()) / 4;
                Ok(LlmResponse {
                    content: resp.content,
                    tokens_used: std::cmp::min(estimated_tokens, req.budget_limit),
                    tool_calls: resp.tool_calls.into_iter().map(|tc| {
                        telos_model_gateway::ToolCallRequest {
                            id: tc.id,
                            name: tc.name,
                            arguments: tc.arguments,
                        }
                    }).collect(),
                    finish_reason: resp.finish_reason,
                })
            }
            Err(e) => {
                let error_msg = e.message.to_lowercase();

                // Detect Zhipu API rate limit error (code 1302)
                if error_msg.contains("\"code\":1302")
                    || error_msg.contains("rate limit")
                    || error_msg.contains("频率限制")
                    || error_msg.contains("429")
                {
                    warn!("[GatewayAdapter] API Rate Limit detected: {}", e.message);
                    Err(GatewayError::TooManyRequests { retry_after_ms: None })
                }
                // Detect network-related errors for retry
                else if error_msg.contains("error sending request")
                    || error_msg.contains("connection")
                    || error_msg.contains("timeout")
                    || error_msg.contains("timedout")  // Rust's TimedOut variant
                    || error_msg.contains("dns")
                    || error_msg.contains("network")
                    || error_msg.contains("socket")
                    || error_msg.contains("http error")
                {
                    warn!("[GatewayAdapter] Network error, retrying: {}", e.message);
                    Err(GatewayError::from_network_error(&e.message))
                } else if error_msg.contains("503") || error_msg.contains("service unavailable") {
                    warn!("[GatewayAdapter] Service unavailable: {}", e.message);
                    Err(GatewayError::ServiceUnavailable { estimated_recovery_ms: None })
                } else {
                    error!("[GatewayAdapter] Other error: {}", e.message);
                    let is_retryable = e.is_retryable();
                    Err(GatewayError::Other {
                        message: e.message,
                        is_retryable,
                    })
                }
            }
        }
    }
}

// 2. System Registry
struct DaemonRegistry {
    gateway: Arc<GatewayManager>,
    memory_os: Arc<RedbGraphStore>,
    system_context: Arc<tokio::sync::RwLock<telos_core::SystemContext>>,
}

impl SystemRegistry for DaemonRegistry {
    fn get_memory_os(&self) -> Option<std::sync::Arc<dyn std::any::Any + Send + Sync>> {
        Some(self.memory_os.clone() as Arc<dyn std::any::Any + Send + Sync>)
    }

    fn get_model_gateway(&self) -> Option<std::sync::Arc<dyn std::any::Any + Send + Sync>> {
        Some(self.gateway.clone() as Arc<dyn std::any::Any + Send + Sync>)
    }

    fn get_system_context(&self) -> Option<telos_core::SystemContext> {
        if let Ok(ctx) = self.system_context.try_read() {
            Some(telos_core::SystemContext {
                current_time: format!("Today is {}", chrono::Local::now().format("%Y-%m-%d, %H:%M:%S %Z")),
                location: ctx.location.clone(),
            })
        } else {
            None
        }
    }
}

// 2.5 Node Factory
struct DaemonNodeFactory {
    gateway: Arc<GatewayManager>,
    tool_registry:
        std::sync::Arc<tokio::sync::RwLock<telos_tooling::retrieval::VectorToolRegistry>>,
    tools_dir: String,
}

impl telos_dag::engine::NodeFactory for DaemonNodeFactory {
    fn create_node(&self, agent_type: &str, _task: &str) -> Option<Box<dyn ExecutableNode>> {
        match agent_type {
            "architect" => Some(Box::new(ArchitectAgent::new(self.gateway.clone())) as Box<dyn ExecutableNode>),
            "coder" => Some(Box::new(CoderAgent::new(
                self.gateway.clone(),
                self.tool_registry.clone(),
                self.tools_dir.clone(),
            )) as Box<dyn ExecutableNode>),
            "reviewer" => Some(Box::new(ReviewAgent::new(self.gateway.clone())) as Box<dyn ExecutableNode>),
            "tester" => Some(Box::new(TestingAgent::new(self.gateway.clone())) as Box<dyn ExecutableNode>),
            "researcher" => Some(Box::new(DeepResearchAgent::new(
                self.gateway.clone(),
                self.tool_registry.clone(),
            )) as Box<dyn ExecutableNode>),
            "general" => Some(Box::new(GeneralAgent::new(
                self.gateway.clone(),
                self.tool_registry.clone(),
                self.tools_dir.clone(),
            )) as Box<dyn ExecutableNode>),
            "tool" => Some(Box::new(ToolNode {
                tool_name: _task.to_string(),
                tool_registry: self.tool_registry.clone(),
            }) as Box<dyn ExecutableNode>),
            "search_worker" => Some(Box::new(SearchWorkerAgent::new(
                self.gateway.clone(),
                self.tool_registry.clone(),
            )) as Box<dyn ExecutableNode>),
            _ => None,
        }
    }
}

// 3. Real Executable Node that calls the LLM dynamically

// --- Dynamic DAG Deserialization structs ---


/// Helper to truncate strings for preview
fn truncate_for_preview(s: &str, max_len: usize) -> String {
    if s.chars().count() > max_len {
        let truncated: String = s.chars().take(max_len).collect();
        format!("{}...", truncated)
    } else {
        s.to_string()
    }
}

/// Parse structured clarification options from agent response text.
/// Looks for emoji-prefixed lines like "🔍 搜索信息" or "- 搜索信息".
/// Falls back to default options if none detected.
fn parse_clarification_options(response: &str) -> Vec<telos_hci::ClarificationOption> {
    let mut options = Vec::new();
    let mut idx = 0;
    
    for line in response.lines() {
        let trimmed = line.trim();
        // Match lines starting with emoji, bullet, or number
        let is_option = trimmed.starts_with('•') 
            || trimmed.starts_with('-') 
            || trimmed.starts_with('*')
            || trimmed.chars().next().map(|c| !c.is_ascii() && !c.is_ascii_punctuation()).unwrap_or(false);
        
        if is_option && trimmed.len() > 2 {
            idx += 1;
            let label = trimmed.trim_start_matches(|c: char| c == '•' || c == '-' || c == '*' || c == ' ').trim().to_string();
            if !label.is_empty() {
                options.push(telos_hci::ClarificationOption {
                    id: format!("opt_{}", idx),
                    label: label.clone(),
                    description: String::new(),
                });
            }
        }
    }
    
    // If we didn't find structured options, provide defaults
    if options.is_empty() {
        options = vec![
            telos_hci::ClarificationOption { id: "opt_1".into(), label: "🔍 搜索信息".into(), description: "新闻、天气、知识问答".into() },
            telos_hci::ClarificationOption { id: "opt_2".into(), label: "💻 编程开发".into(), description: "编写、调试、审查代码".into() },
            telos_hci::ClarificationOption { id: "opt_3".into(), label: "📝 文档处理".into(), description: "写作、翻译、摘要".into() },
            telos_hci::ClarificationOption { id: "opt_4".into(), label: "📅 任务规划".into(), description: "计划、日程、方案设计".into() },
            telos_hci::ClarificationOption { id: "opt_5".into(), label: "🧮 计算分析".into(), description: "数学、数据、逻辑推理".into() },
        ];
    }
    
    options
}

/// Extracts user preferences, personal facts, and habits from a conversation
/// using LLM, then stores new facts as UserProfile memories (with deduplication).
async fn extract_and_store_user_profile(
    conversation: &str,
    gateway: Arc<GatewayManager>,
    memory_os: std::sync::Arc<telos_memory::RedbGraphStore>,
) {
    use telos_model_gateway::{Capability, LlmRequest, Message, ModelGateway};

    let extraction_prompt = format!(
        r#"Analyze the following conversation between a user and an AI assistant.
Extract ANY new personal information, preferences, habits, or facts about the user.

RULES:
- Extract ONLY facts about the USER (not the assistant)
- Each fact should be a single, concise statement
- Include: name, location, language preferences, work/profession, interests, habits, communication style, technical skill level, project details they mentioned
- Do NOT extract transient requests (e.g., "user asked about weather" is NOT a preference)
- DO extract persistent traits (e.g., "User prefers Chinese language responses", "User is a software developer", "User's project is called Telos")
- If NO new user information is found, return an empty array

Output ONLY a valid JSON object:
{{"facts": ["fact1", "fact2", ...]}}

Conversation:
{}
"#,
        conversation
    );

    let request = LlmRequest {
        session_id: "profile_extraction".to_string(),
        messages: vec![
            Message { role: "system".into(), content: "You are a precise information extraction system. Output only valid JSON.".into() },
            Message { role: "user".into(), content: extraction_prompt },
        ],
        required_capabilities: Capability { requires_vision: false, strong_reasoning: false },
        budget_limit: 1000,
        tools: None,
    };

    let llm_result = gateway.generate(request).await;
    let response_text = match llm_result {
        Ok(r) => r.content,
        Err(e) => {
            debug!("[UserProfile] LLM extraction failed: {:?}", e);
            return;
        }
    };

    // Parse JSON response
    let cleaned = response_text.trim().trim_start_matches("```json").trim_end_matches("```").trim();
    let parsed: serde_json::Value = match serde_json::from_str(cleaned) {
        Ok(v) => v,
        Err(_) => {
            debug!("[UserProfile] Failed to parse LLM extraction output: {}", &response_text[..response_text.len().min(200)]);
            return;
        }
    };

    let facts = match parsed.get("facts").and_then(|f| f.as_array()) {
        Some(arr) => arr,
        None => {
            debug!("[UserProfile] No 'facts' array in extraction output");
            return;
        }
    };

    if facts.is_empty() {
        debug!("[UserProfile] No new user facts extracted from conversation");
        return;
    }

    // Load existing UserProfile entries for deduplication
    let existing_profiles: Vec<String> = if let Ok(results) = memory_os.retrieve(
        telos_memory::MemoryQuery::TimeRange { start: 0, end: u64::MAX }
    ).await {
        results.iter()
            .filter(|e| e.memory_type == telos_memory::MemoryType::UserProfile)
            .map(|e| e.content.to_lowercase())
            .collect()
    } else {
        Vec::new()
    };

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    let mut stored_count = 0;
    for (i, fact_val) in facts.iter().enumerate() {
        if let Some(fact) = fact_val.as_str() {
            let fact_trimmed = fact.trim();
            if fact_trimmed.is_empty() {
                continue;
            }
            // Deduplication: skip if a similar fact already exists
            let fact_lower = fact_trimmed.to_lowercase();
            if existing_profiles.iter().any(|existing| {
                existing.contains(&fact_lower) || fact_lower.contains(existing.as_str())
            }) {
                debug!("[UserProfile] Skipping duplicate fact: {}", fact_trimmed);
                continue;
            }

            let entry = telos_memory::MemoryEntry::new(
                format!("profile_{}_{}", timestamp, i),
                telos_memory::MemoryType::UserProfile,
                fact_trimmed.to_string(),
                timestamp,
                None, // Embedding will be auto-generated by engine.rs store()
            );

            if let Err(e) = memory_os.store(entry).await {
                debug!("[UserProfile] Failed to store fact: {:?}", e);
            } else {
                stored_count += 1;
            }
        }
    }

    if stored_count > 0 {
        info!("[UserProfile] ✅ Extracted and stored {} new user profile facts", stored_count);
    }
}

struct LlmPromptNode {
    prompt: String,
    gateway: Arc<GatewayManager>,
}

#[async_trait]
impl ExecutableNode for LlmPromptNode {
    async fn execute(&self, input: AgentInput, _registry: &dyn SystemRegistry) -> AgentOutput {
        // Build context with dependencies if any
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

        let full_prompt = format!("{}{}", self.prompt, deps_context);

        let request = LlmRequest {
            session_id: format!("node_{}", input.node_id),
            messages: vec![telos_model_gateway::Message {
                role: "user".to_string(),
                content: full_prompt,
            }],
            required_capabilities: Capability {
                requires_vision: false,
                strong_reasoning: false,
            },
            budget_limit: 1000,
            tools: None,
        };

        match self.gateway.generate(request.clone()).await {
            Ok(res) => {
                let trace = telos_core::TraceLog::LlmCall {
                    request: serde_json::to_value(&request).unwrap_or_else(|_| serde_json::json!({ "error": "failed to serialize request" })),
                    response: serde_json::to_value(&res).unwrap_or_else(|_| serde_json::json!({ "error": "failed to serialize response" })),
                };
                AgentOutput::success(serde_json::json!({
                    "text": res.content
                })).with_trace(trace)
            },
            Err(e) => {
                // 使用用户友好的错误消息
                AgentOutput::from_gateway_error(
                    "LLMError",
                    &e.to_user_message(),
                    &format!("{:?}", e),
                    e.is_retryable(),
                )
            }
        }
    }
}



struct ToolNode {
    tool_name: String,
    tool_registry: Arc<tokio::sync::RwLock<telos_tooling::retrieval::VectorToolRegistry>>,
}

#[async_trait]
impl ExecutableNode for ToolNode {
    async fn execute(&self, input: AgentInput, _registry: &dyn SystemRegistry) -> AgentOutput {
        let registry_guard = self.tool_registry.read().await;
        let executor = match registry_guard.get_executor(&self.tool_name) {
            Some(e) => e,
            None => {
                return AgentOutput::failure(
                    "ToolNotFoundError",
                    &format!("Tool '{}' not found in registry", self.tool_name),
                );
            }
        };
        drop(registry_guard);

        // Tool input is expected to be a JSON object in schema_payload
        let params: serde_json::Value = if let Some(ref payload) = input.schema_payload {
            match serde_json::from_str(payload) {
                Ok(p) => p,
                Err(e) => {
                    return AgentOutput::failure(
                        "PayloadParseError",
                        &format!("Failed to parse tool payload: {}", e),
                    );
                }
            }
        } else {
            serde_json::json!({})
        };

        info!("[ToolNode] 🛠️  Executing tool: {} with params: {}", self.tool_name, params);

        match executor.call(params.clone()).await {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes).to_string();
                let (json_result, is_json) = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                    (json, true)
                } else {
                    (serde_json::json!({ "text": text.clone() }), false)
                };

                let trace = telos_core::TraceLog::ToolCall {
                    name: self.tool_name.clone(),
                    params: params.clone(),
                    result: json_result.clone(),
                };

                if is_json {
                    AgentOutput::success(json_result).with_trace(trace)
                } else {
                    AgentOutput::success(serde_json::json!({ "text": text })).with_trace(trace)
                }
            }
            Err(e) => AgentOutput::failure("ToolExecutionError", &format!("{:?}", e)),
        }
    }
}



// 4. Server App State
#[derive(Clone)]
struct AppState {
    broker: Arc<TokioEventBroker>,
    recent_traces: Arc<tokio::sync::RwLock<std::collections::VecDeque<telos_hci::AgentFeedback>>>,
    active_tasks: telos_dag::engine::ActiveTaskRegistry,
}

#[derive(Deserialize)]
struct RunRequest {
    payload: String,
    project_id: Option<String>,
    trace_id: Option<String>,
}

#[derive(Serialize)]
struct RunResponse {
    status: String,
    trace_id: String,
}

#[derive(Serialize)]
struct RunSyncResponse {
    status: String,
    trace_id: String,
    task_summary: Option<telos_hci::TaskSummary>,
    final_output: Option<String>,
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
    // 1. Initialize logging with timestamps, rotation, and size limits
    let log_dir = TelosConfig::logs_dir();
    let log_dir_str = log_dir.to_string_lossy();
    let _guard = telos_telemetry::init_standard_logging(
        "debug", // Default level, will be filtered by EnvFilter if TELOS_LOG_LEVEL is set
        Some(&log_dir_str),
        Some("daemon.log")
    );

    debug!("Initializing Telos Daemon...");

    let config = TelosConfig::load().expect(
        "Failed to load configuration. Please run `telos cli` first to complete initialization.",
    );

    // cleanup orphaned memory files from previous PID-suffix fallbacks
    let _ = TelosConfig::cleanup_orphaned_memory_files();

    // Set proxy environment variable from config if configured
    if let Some(ref proxy) = config.proxy {
        std::env::set_var("TELOS_PROXY", proxy);
        info!("[Daemon] Proxy configured: {}", proxy);
    }

    // Initialize SOUL (personality/identity) from SOUL.md
    agents::prompt_builder::init_soul(".");

    // Initialize global log level from config
    let initial_log_level = LogLevel::from_string(&config.log_level);
    global_log_level().set(initial_log_level);
    debug!("Log level set to: {:?}", initial_log_level);

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
    // Use max_concurrent_requests from config (0 = unlimited, default 20)
    let gateway = Arc::new(GatewayManager::new(
        gateway_adapter,
        config.max_concurrent_requests,
    ));

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
    tool_registry.register_tool(
        telos_tooling::native::WebScrapeTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::WebScrapeTool)),
    );
    // Register utility tools
    tool_registry.register_tool(
        telos_tooling::native::GetTimeTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::GetTimeTool)),
    );
    // Register LSP tool for code navigation
    tool_registry.register_tool(
        telos_tooling::native::LspTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::LspTool)),
    );
    // Register Web Search tool
    tool_registry.register_tool(
        telos_tooling::native::WebSearchTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::WebSearchTool)),
    );

    // Wrap registry in Arc<RwLock<...>> early so we can pass it to WASM executors
    let tool_registry = std::sync::Arc::new(tokio::sync::RwLock::new(tool_registry));

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
                            let script_path = path.with_extension("rhai");
                            if script_path.exists() {
                                if let Ok(script_code) = std::fs::read_to_string(&script_path) {
                                    let sandbox = std::sync::Arc::new(telos_tooling::ScriptSandbox::new());
                                    let native_registry = telos_tooling::wrap_tool_registry(tool_registry.clone());
                                    let script_executor: std::sync::Arc<dyn telos_tooling::ToolExecutor> = std::sync::Arc::new(
                                        telos_tooling::ScriptExecutor::new(script_code, sandbox)
                                            .with_native_tools(native_registry)
                                    );

                                    let mut guard = tool_registry.write().await;
                                    guard.register_tool(
                                        schema,
                                        Some(script_executor),
                                    );
                                    drop(guard);
                                    debug!(
                                        "[Daemon] Auto-loaded persisted Rhai tool from {:?}",
                                        script_path.file_name().unwrap()
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Initialize MemoryOS - we no longer use PID-suffix fallbacks to avoid file sprawl.
    let memory_os_instance = match RedbGraphStore::new(&config.db_path) {
        Ok(store) => Arc::new(store),
        Err(e) => {
            error!("[Daemon] Failed to initialize MemoryOS database at {}: {}. If the database is locked by another instance, please close it first.", config.db_path, e);
            panic!("MemoryOS initialization failed");
        }
    };
    let system_context = Arc::new(tokio::sync::RwLock::new(telos_core::SystemContext {
        current_time: String::new(),
        location: config.default_location.clone().unwrap_or_else(|| "Unknown Location".to_string()),
    }));

    if config.default_location.is_none() {
        let sys_ctx_clone = system_context.clone();
        tokio::spawn(async move {
            loop {
                if let Ok(resp) = reqwest::get("http://ip-api.com/json/").await {
                    if let Ok(json) = resp.json::<serde_json::Value>().await {
                        let mut city = json.get("city").and_then(|v| v.as_str()).unwrap_or("Unknown City").to_string();
                        let as_str = json.get("as").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
                        let isp_str = json.get("isp").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
                        
                        // Extract finer city details from ASN/ISP if "city" defaults poorly
                        if as_str.contains("suzhou") || isp_str.contains("suzhou") {
                            city = "Suzhou".to_string();
                        } else if as_str.contains("shanghai") || isp_str.contains("shanghai") {
                            city = "Shanghai".to_string();
                        }

                        let loc = format!("{}, {}, {}", 
                            city,
                            json.get("regionName").and_then(|v| v.as_str()).unwrap_or("Unknown Region"),
                            json.get("country").and_then(|v| v.as_str()).unwrap_or("Unknown Country")
                        );
                        let mut w = sys_ctx_clone.write().await;
                        w.location = loc;
                    }
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            }
        });
    }

    let registry = Arc::new(DaemonRegistry {
        gateway: gateway.clone(),
        memory_os: memory_os_instance.clone(),
        system_context,
    });
    // Reuse the same memory instance instead of creating a duplicate
    let _memory = memory_os_instance.clone();
    // Using cloud embeddings as configured
    let context_manager = Arc::new(RaptorContextManager::new(
        Arc::new(openai_provider.clone()),
        Arc::new(openai_provider.clone()),
        Some(memory_os_instance.clone() as Arc<dyn telos_memory::integration::MemoryIntegration>),
    ));

    // Initialize Evolution Evaluator
    let evaluator = Arc::new(
        telos_evolution::evaluator::ActorCriticEvaluator::new()
            .expect("Failed to initialize ActorCriticEvaluator"),
    );

    // --- BACKGROUND EVENT LOOP ---
    let broker_bg = Arc::clone(&broker);
    let gateway_clone = gateway.clone();
    let registry_clone = registry.clone();
    let tool_registry = tool_registry.clone();
    let loop_config = config.clone();
    let paused_tasks: Arc<TokioMutex<HashMap<String, String>>> =
        Arc::new(TokioMutex::new(HashMap::new()));
    let paused_tasks_bg = paused_tasks.clone();
    let wakeup_map: Arc<
        TokioMutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<(String, String, String)>>>,
    > = Arc::new(TokioMutex::new(HashMap::new()));
    let wakeup_map_bg = wakeup_map.clone();

    // --- EVOLUTION EVALUATION WORKER ---
    let (distillation_tx, mut distillation_rx) = tokio::sync::mpsc::unbounded_channel::<telos_evolution::ExecutionTrace>();
    let evaluator_worker = evaluator.clone();
    let registry_worker = registry.clone();
    
    tokio::spawn(async move {
        debug!("[Daemon] 🧵 Evolution worker thread started, listening for traces...");
        use telos_evolution::Evaluator;
        use telos_memory::integration::MemoryIntegration;

        while let Some(trace) = distillation_rx.recv().await {
            let trace_id = trace.task_id.clone();
            
            // Log extraction attempt
            debug!("[Daemon] 🧠 Evolution worker processing trace {}...", trace_id);
            
            // Distill Experience asynchronously (CPU/Network bound)
            if let Some(skill) = evaluator_worker.distill_experience(&trace).await {
                info!("[Daemon] 🧠 Telos distilled a new SynthesizedSkill from task {}!", trace_id);
                
                let skill_string = format!(
                    "Distilled Skill for task '{}':\nTrigger: {}\nCode:\n{}",
                    trace.steps.first().map(|s| s.input_data.as_str()).unwrap_or("Unknown"),
                    skill.trigger_condition,
                    skill.executable_code
                );
                
                // Save to Long Term Memory
                let _ = registry_worker.memory_os.store_semantic_fact(skill_string).await;
                debug!("[Daemon] 📥 Distilled skill securely archived in Long-Term Memory.");
            }
        }
    });

    let distillation_tx_bg = distillation_tx.clone();
    
    // Global active tasks registry
    let active_tasks: telos_dag::engine::ActiveTaskRegistry = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
    let active_tasks_loop = active_tasks.clone();

    // Global short-term session memory for Context/History Injection
    let global_session_logs: Arc<tokio::sync::RwLock<std::collections::VecDeque<telos_context::LogEntry>>> = Arc::new(tokio::sync::RwLock::new(std::collections::VecDeque::with_capacity(20)));
    let session_logs_loop = global_session_logs.clone();

    tokio::spawn(async move {
        debug!("[Daemon] Event loop started.");
        while let Some(event) = event_rx.recv().await {
            match event {
                AgentEvent::SetLogLevel { level } => {
                    let old_level = global_log_level().get();
                    global_log_level().set(level);
                    broker_bg.publish_feedback(AgentFeedback::LogLevelChanged {
                        old_level,
                        new_level: level,
                    });
                    debug!("[Daemon] Log level changed: {:?} -> {:?}", old_level, level);
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
                    let distillation_tx_spawn = distillation_tx_bg.clone();
                    let session_logs_loop = session_logs_loop.clone();

                    let node_factory = std::sync::Arc::new(DaemonNodeFactory {
                        gateway: gateway_clone.clone(),
                        tool_registry: tool_registry.clone(),
                        tools_dir: config.tools_dir.clone(),
                    });

                    let mut execution_engine = telos_dag::engine::TokioExecutionEngine::new()
                        .with_node_factory(node_factory)
                        .with_active_tasks(active_tasks_loop.clone());

                    let context_manager_spawn = context_manager.clone();
                    let active_tasks_spawn = active_tasks_loop.clone();
                    tokio::spawn(async move {
                        let task_start_time = Instant::now();
                        debug!(
                            "[Daemon] Received UserInput: {} (trace: {})",
                            payload, trace_id
                        );
                        let current_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;

                        // -- GLOBAL SESSION HISTORY INJECTION --
                        let mut recent_history_text = String::new();
                        {
                            let mut logs_w = session_logs_loop.write().await;
                            
                            // Maintain max 20 turns
                            while logs_w.len() > 20 {
                                logs_w.pop_front();
                            }

                            if !logs_w.is_empty() {
                                recent_history_text.push_str("[GLOBAL CONVERSATION HISTORY]\nThe user has interacted with you previously in this session. Interactions are numbered chronologically (#1 = first/earliest). Use this to resolve pronouns, references like \"第一个问题\", and maintain context:\n");
                                for (i, log) in logs_w.iter().enumerate() {
                                    recent_history_text.push_str(&format!("#{}: {}\n", i + 1, log.message));
                                }
                                recent_history_text.push('\n');
                            } else {
                                // Session memory is empty — preload from persistent memory (telos_memory)
                                // This bridges headless CLI calls and daemon restarts
                                if let Some(mem_any) = registry_clone.get_memory_os() {
                                    if let Ok(mem_os) = mem_any.clone().downcast::<std::sync::Arc<dyn telos_memory::engine::MemoryOS>>() {
                                        let twenty_four_hours_ago = current_ms.saturating_sub(86_400_000);
                                        if let Ok(results) = mem_os.retrieve(telos_memory::MemoryQuery::TimeRange {
                                            start: twenty_four_hours_ago,
                                            end: current_ms,
                                        }).await {
                                            let mut interaction_entries: Vec<&telos_memory::MemoryEntry> = results.iter()
                                                .filter(|e| e.memory_type == telos_memory::MemoryType::InteractionEvent)
                                                .collect();
                                            // Sort chronologically: oldest first
                                            interaction_entries.sort_by_key(|e| e.created_at);
                                            if !interaction_entries.is_empty() {
                                                recent_history_text.push_str("[GLOBAL CONVERSATION HISTORY (from persistent memory)]\nThe user has interacted with you recently. Interactions are numbered chronologically (#1 = first/earliest). Use this to resolve references like \"第一个问题\" or \"之前\":\n");
                                                for (i, entry) in interaction_entries.iter().take(30).enumerate() {
                                                    recent_history_text.push_str(&format!("#{}: {}\n", i + 1, entry.content));
                                                }
                                                recent_history_text.push('\n');
                                                debug!("[Daemon] Preloaded {} interaction events from persistent memory (24h window)", interaction_entries.len());
                                            }
                                        }
                                    }
                                }
                            }
                            
                            // Immediately append the user's new query so it's logged
                            logs_w.push_back(telos_context::LogEntry {
                                timestamp: current_ms,
                                message: format!("User: {}", payload),
                            });
                        }
                        // ----------------------------------------

                        // --- USER PROFILE INJECTION (Long-Term User Knowledge) ---
                        {
                            if let Some(mem_any) = registry_clone.get_memory_os() {
                                if let Ok(mem_os) = mem_any.clone().downcast::<std::sync::Arc<dyn telos_memory::engine::MemoryOS>>() {
                                    // Load ALL UserProfile memories (these are permanent user facts)
                                    if let Ok(results) = mem_os.retrieve(telos_memory::MemoryQuery::TimeRange {
                                        start: 0,
                                        end: u64::MAX,
                                    }).await {
                                        let profile_entries: Vec<&telos_memory::MemoryEntry> = results.iter()
                                            .filter(|e| e.memory_type == telos_memory::MemoryType::UserProfile)
                                            .collect();
                                        if !profile_entries.is_empty() {
                                            recent_history_text.push_str("[USER PROFILE — PERSISTENT KNOWLEDGE ABOUT YOUR OWNER]\nThe following are facts, preferences, and personal information you have learned about the user (your 主人) through past interactions. Use this to personalize your responses:\n");
                                            for entry in &profile_entries {
                                                recent_history_text.push_str(&format!("• {}\n", entry.content));
                                            }
                                            recent_history_text.push('\n');
                                            debug!("[Daemon] Injected {} UserProfile facts into router context", profile_entries.len());
                                        }
                                    }
                                }
                            }
                        }
                        // ----------------------------------------

                        {
                            let mut w = active_tasks_spawn.write().await;
                            w.insert(
                                trace_id.to_string(),
                                telos_hci::ActiveTaskInfo {
                                    task_id: trace_id.to_string(),
                                    task_name: trace_id.to_string(),
                                    progress: telos_hci::ProgressInfo::new(0, 1, 1, 0, 0, None),
                                    running_nodes: vec!["router".to_string()],
                                    started_at_ms: current_ms,
                                }
                            );
                        }

                        // --- CONTEXTUAL BYPASS ---
                        let is_resume = paused_tasks_bg
                            .lock()
                            .await
                            .contains_key(&trace_id.to_string());

                        if is_resume {
                            debug!("[Daemon] Contextual Bypass: Task {} is active, injecting Architect for replan.", trace_id);
                            let _original_payload =
                                paused_tasks_bg.lock().await.remove(&trace_id.to_string());

                            broker_bg.publish_feedback(AgentFeedback::StateChanged {
                                task_id: trace_id.to_string(),
                                current_node: "replan_architect".into(),
                                status: telos_core::NodeStatus::Running,
                            });

                            let mut graph = TaskGraph::new(trace_id.to_string());
                            graph.add_node_with_metadata(
                                "replan_architect".to_string(),
                                Box::new(agents::architect::ArchitectAgent::new(
                                    gateway_clone.clone(),
                                )),
                                NodeMetadata {
                                    task_type: "architect".to_string(),
                                    prompt_preview: truncate_for_preview(&payload, 100),
                                    tool_name: None,
                                    schema_payload: None,
                                },
                            );

                            graph.current_state = GraphState {
                                is_running: true,
                                completed: false,
                            };

                            let scoped_ctx = telos_context::ScopedContext {
                                budget_tokens: 128_000,
                                summary_tree: vec![],
                                precise_facts: vec![],
                            };

                            execution_engine
                                .run_graph(
                                    &mut graph,
                                    &scoped_ctx,
                                    registry_clone.as_ref(),
                                    broker_bg.as_ref(),
                                )
                                .await;

                            let mut completed_nodes = 0;
                            let mut failed_nodes = 0;
                            let mut failed_node_ids = Vec::new();
                            for (id, status) in &graph.node_statuses {
                                if *status == telos_core::NodeStatus::Completed {
                                    completed_nodes += 1;
                                } else if *status == telos_core::NodeStatus::Failed {
                                    failed_nodes += 1;
                                    failed_node_ids.push(id.clone());
                                }
                            }
                            let task_success = failed_nodes == 0;
                            let summary = if task_success {
                                "Task Replan Completed".to_string()
                            } else {
                                "Task Replan Failed".to_string()
                            };

                            let combined_result = graph
                                .node_results
                                .iter()
                                .filter_map(|(id, out)| {
                                    out.output.as_ref().map(|v| format!("{}:\n{}", id, v))
                                })
                                .collect::<Vec<_>>()
                                .join("\n\n");

                            broker_bg.publish_feedback(AgentFeedback::Output {
                                task_id: trace_id.to_string(),
                                session_id: session_id.clone(),
                                content: format!("{}\n\n{}", summary, combined_result),
                                is_final: true,
                                silent: false,
                            });

                            broker_bg.publish_feedback(AgentFeedback::TaskCompleted {
                                task_id: trace_id.to_string(),
                                summary: TaskSummary {
                                    fulfilled: task_success,
                                    completed: true,
                                    total_nodes: graph.node_statuses.len(),
                                    completed_nodes,
                                    failed_nodes,
                                    total_time_ms: task_start_time.elapsed().as_millis() as u64,
                                    summary: summary.clone(),
                                    failed_node_ids,
                                },
                            });
                        } else {
                            // --- NORMAL PLANNING & EXECUTION ---
                            let mut enriched_payload = payload.clone();

                            if let Some(pid) = &project_id {
                                debug!("[Daemon] Active Project ID: {}", pid);
                                if let Ok(Some(project)) =
                                    telos_project::manager::ProjectRegistry::new().get_project(pid)
                                {
                                    let working_dir = project.path.clone();
                                    debug!("[Daemon] Project working directory: {:?}", working_dir);

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
                                    
                                    debug!(
                                        "[Daemon] Dynamically injected project context into payload."
                                    );
                                }
                            }

                            // --- ACTIVE TASK INJECTION FOR ROUTER OMNISCIENCE ---
                            let active_tasks_snapshot = {
                                let w = active_tasks_spawn.read().await;
                                if w.is_empty() {
                                    String::new()
                                } else {
                                    let mut tasks_desc = Vec::new();
                                    for (id, state) in w.iter() {
                                        if id != &trace_id.to_string() {
                                            let nodes_str = if state.running_nodes.is_empty() { "Pending".to_string() } else { state.running_nodes.join(", ") };
                                            tasks_desc.push(format!("- Task ID [{}]: {}", id, nodes_str));
                                        }
                                    }
                                    if tasks_desc.is_empty() {
                                        String::new()
                                    } else {
                                        format!("[SYSTEM: Active Background Tasks]\nThe system is currently executing the following tasks in the background:\n{}\n\n", tasks_desc.join("\n"))
                                    }
                                }
                            };
                            
                            if !active_tasks_snapshot.is_empty() {
                                enriched_payload = format!("{}{}", active_tasks_snapshot, enriched_payload);
                            }

                            // --- TIER 1: ROUTER AGENT DISPATCH ---
                            broker_bg.publish_feedback(AgentFeedback::StateChanged {
                                task_id: trace_id.to_string(),
                                current_node: "routing".into(),
                                status: telos_core::NodeStatus::Running,
                            });

                            let router = agents::router::RouterAgent::new(gateway_clone.clone(), config.router_persona_name.clone(), config.router_persona_trait.clone());
                            let mut router_input = telos_core::AgentInput {
                                node_id: "router_main".to_string(),
                                task: enriched_payload.clone(),
                                dependencies: Default::default(),
                                schema_payload: None,
                                memory_context: Some(recent_history_text.clone()),
                                correction: None,
                            };

                            let mut route_result = telos_core::AgentOutput::failure("Init", "Not started");
                            let mut tool_used = false;

                           // --- ROUTER REACT LOOP for TOOL ACCESS ---
                            let mut tool_attempts_used = 0u32;
                            for attempt in 0..3 {
                                route_result = router.execute(router_input.clone(), registry_clone.as_ref()).await;
                                
                                if !route_result.success {
                                    break;
                                }

                                if let Some(route_data) = route_result.output.as_ref() {
                                    // Intercept "tool"
                                    if let Some(tool_name) = route_data.get("tool").and_then(|v| v.as_str()) {
                                        if tool_name == "memory_read" {
                                            tool_attempts_used = attempt + 1;
                                            let query_val = route_data.get("query").and_then(|v| v.as_str()).unwrap_or("");
                                            debug!("[Daemon] Router triggered memory_read tool for query: {}", query_val);
                                            
                                            // Provide feedback to UI that we are searching memory
                                            broker_bg.publish_feedback(AgentFeedback::Output {
                                                task_id: trace_id.to_string(),
                                                session_id: session_id.clone(),
                                                content: format!("*(Router is recalling memory for: {})*", query_val),
                                                is_final: false,
                                                silent: false,
                                            });

                                            let mut memory_findings = String::new();
                                            if let Some(mem_any) = registry_clone.get_memory_os() {
                                                if let Ok(mem_os) = mem_any.clone().downcast::<std::sync::Arc<dyn telos_memory::engine::MemoryOS>>() {
                                                    // Dual-strategy query: EntityLookup (keyword) + TimeRange (recency)
                                                    let mut all_entries: Vec<(u64, String)> = Vec::new(); // (created_at, content)
                                                    
                                                    // Strategy 1: Keyword-based EntityLookup
                                                    if let Ok(results) = mem_os.retrieve(telos_memory::MemoryQuery::EntityLookup { entity: query_val.to_string() }).await {
                                                        for e in results.iter().filter(|e| e.memory_type == telos_memory::MemoryType::Semantic || e.memory_type == telos_memory::MemoryType::InteractionEvent) {
                                                            all_entries.push((e.created_at, e.content.clone()));
                                                        }
                                                    }
                                                    
                                                    // Strategy 2: Recent interactions (last 1 hour)
                                                    let one_hour_ago = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                                                    let one_hour_ago = one_hour_ago.saturating_sub(3600_000);
                                                    if let Ok(results) = mem_os.retrieve(telos_memory::MemoryQuery::TimeRange { start: one_hour_ago, end: u64::MAX }).await {
                                                        for e in results.iter().filter(|e| e.memory_type == telos_memory::MemoryType::InteractionEvent) {
                                                            if !all_entries.iter().any(|(_, c)| c == &e.content) {
                                                                all_entries.push((e.created_at, e.content.clone()));
                                                            }
                                                        }
                                                    }
                                                    
                                                    // Sort chronologically (oldest first) and format with ordinals
                                                    all_entries.sort_by_key(|(ts, _)| *ts);
                                                    
                                                    if !all_entries.is_empty() {
                                                        let formatted: Vec<String> = all_entries.iter().enumerate()
                                                            .map(|(i, (_, content))| format!("#{}: {}", i + 1, content))
                                                            .collect();
                                                        let merged = formatted.join("\n");
                                                        let truncated: String = if merged.chars().count() > 2000 { merged.chars().take(2000).collect::<String>() + "..." } else { merged };
                                                        memory_findings = format!("[TOOL RESULT: memory_read]\nFound relevant memories (chronological order, #1 = earliest):\n{}\n\n", truncated);
                                                    } else {
                                                        memory_findings = format!("[TOOL RESULT: memory_read]\nNo relevant memories found for '{}'.\n\n", query_val);
                                                    }
                                                }
                                            }

                                            // Append findings back to Router memory context and loop
                                            let existing_mem = router_input.memory_context.unwrap_or_default();
                                            router_input.memory_context = Some(format!("{}{}", existing_mem, memory_findings));
                                            tool_used = true;
                                            continue; 
                                        }
                                    }
                                    
                                    // If we reach here, it's either "direct_reply" or "route", routing is finished
                                    break;
                                }
                            }
                            
                            // --- GRACEFUL DEGRADATION: Final synthesis pass ---
                            // If the loop exhausted all 3 tool attempts without converging on
                            // a direct_reply or route decision, give the Router ONE final chance
                            // to synthesize an answer from whatever it gathered, or escalate.
                            if tool_attempts_used >= 3 && route_result.success {
                                if let Some(route_data) = route_result.output.as_ref() {
                                    if route_data.get("tool").is_some() && route_data.get("direct_reply").is_none() && route_data.get("route").is_none() {
                                        debug!("[Daemon] Router exhausted all tool attempts without converging. Triggering final synthesis pass.");
                                        let existing_mem = router_input.memory_context.unwrap_or_default();
                                        router_input.memory_context = Some(format!(
                                            "{}[SYSTEM NOTE: You have used all your tool attempts. You MUST now make a final decision: either provide a direct_reply with your best answer based on whatever information you found (even if partial), or route to an appropriate expert if you believe only a deeper search pipeline can answer the question. Do NOT request any more tools.]\n\n",
                                            existing_mem
                                        ));
                                        route_result = router.execute(router_input.clone(), registry_clone.as_ref()).await;
                                    }
                                }
                            }
                            
                            if !route_result.success {
                                let error_msg = route_result.error.map(|e| e.message).unwrap_or_else(|| "Unknown routing error".to_string());
                                broker_bg.publish_feedback(AgentFeedback::Output {
                                    task_id: trace_id.to_string(),
                                    session_id: session_id.clone(),
                                    content: format!("Routing Failed: {}", error_msg),
                                    is_final: true,
                                    silent: false,
                                });
                                broker_bg.publish_feedback(AgentFeedback::TaskCompleted {
                                    task_id: trace_id.to_string(),
                                    summary: TaskSummary {
                                        fulfilled: false,
                                        completed: true,
                                        total_nodes: 1,
                                        completed_nodes: 0,
                                        failed_nodes: 1,
                                        total_time_ms: task_start_time.elapsed().as_millis() as u64,
                                        summary: "Router Task Failed".to_string(),
                                        failed_node_ids: vec!["router_main".to_string()],
                                    },
                                });
                                // Remove from active tasks before return
                                {
                                    let mut w = active_tasks_spawn.write().await;
                                    w.remove(&trace_id.clone().to_string());
                                }
                                return;
                            }

                            // Parse router output
                            let route_data = route_result.output.unwrap_or_default();
                            
                            // --- DIRECT REPLY SHORT-CIRCUIT (with QA Gate) ---
                            if let Some(direct_reply) = route_data.get("direct_reply").and_then(|v| v.as_str()) {
                                debug!("[Daemon] Router generated a direct reply. Running QA verification...");
                                
                                // QA Gate: evaluate direct_reply quality before accepting
                                let qa_result = router.evaluate(&payload, direct_reply, registry_clone.as_ref()).await;
                                let qa_accepted = if qa_result.success {
                                    qa_result.output.as_ref()
                                        .and_then(|json| json.get("is_acceptable").and_then(|v| v.as_bool()))
                                        .unwrap_or(true) // default accept if parsing fails
                                } else {
                                    true // default accept if QA call itself fails
                                };

                                if qa_accepted {
                                    debug!("[Daemon] QA Gate approved direct reply.");
                                    broker_bg.publish_feedback(AgentFeedback::Output {
                                        task_id: trace_id.to_string(),
                                        session_id: session_id.clone(),
                                        content: direct_reply.to_string(),
                                        is_final: true,
                                        silent: false,
                                    });
                                    
                                    broker_bg.publish_feedback(AgentFeedback::TaskCompleted {
                                        task_id: trace_id.to_string(),
                                        summary: TaskSummary {
                                            fulfilled: true,
                                            completed: true,
                                            total_nodes: 1,
                                            completed_nodes: 1,
                                            failed_nodes: 0,
                                            total_time_ms: task_start_time.elapsed().as_millis() as u64,
                                            summary: "Direct Router Reply".to_string(),
                                            failed_node_ids: vec![],
                                        },
                                    });

                                    // Persist to long-term memory
                                    if let Some(mem_any) = registry_clone.get_memory_os() {
                                        if let Ok(mem_os) = mem_any.downcast::<std::sync::Arc<dyn telos_memory::engine::MemoryOS>>() {
                                            let conversation = format!("[User]: {}\n[Assistant ({} Persona)]: {}", payload, config.router_persona_name, direct_reply);
                                            let conv_for_profile = conversation.clone();
                                            let current_time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
                                            let entry = telos_memory::MemoryEntry::new(
                                                uuid::Uuid::new_v4().to_string(),
                                                telos_memory::MemoryType::InteractionEvent,
                                                conversation,
                                                current_time,
                                                None,
                                            );
                                            if let Err(e) = mem_os.store(entry).await {
                                                error!("[Daemon] Failed to store direct reply interaction memory: {:?}", e);
                                            }
                                            // Background: Extract user preferences from this conversation
                                            let gw_for_profile = gateway_clone.clone();
                                            let mem_for_profile = registry_clone.memory_os.clone();
                                            tokio::spawn(async move {
                                                extract_and_store_user_profile(&conv_for_profile, gw_for_profile, mem_for_profile).await;
                                            });
                                        }
                                    }

                                    // Remove from active tasks
                                    {
                                        let mut w = active_tasks_spawn.write().await;
                                        w.remove(&trace_id.to_string());
                                    }
                                    return; // QA passed, skip DAG
                                } else {
                                    // QA rejected direct_reply — fallthrough to Expert routing
                                    let critique = qa_result.output.as_ref()
                                        .and_then(|json| json.get("critique").and_then(|v| v.as_str()))
                                        .unwrap_or("Direct reply did not adequately answer the user's question.");
                                    debug!("[Daemon] QA Gate REJECTED direct reply. Falling through to Expert routing. Critique: {}", critique);
                                    broker_bg.publish_feedback(AgentFeedback::Output {
                                        task_id: trace_id.to_string(),
                                        session_id: session_id.clone(),
                                        content: format!("🔄 Direct reply rejected by QA. Routing to expert. Critique: {}", critique),
                                        is_final: false,
                                        silent: false,
                                    });
                                    // Don't return — fall through to expert routing below
                                }
                            }

                            let expert_route = route_data.get("route").and_then(|v| v.as_str()).unwrap_or("general_expert");
                            let route_reason = route_data.get("reason").and_then(|v| v.as_str()).unwrap_or("Fallback to general expert.");

                            broker_bg.publish_feedback(AgentFeedback::Output {
                                task_id: trace_id.to_string(),
                                session_id: session_id.clone(),
                                content: format!("Router Decision: Dispatching to `{}`. Reason: {}", expert_route, route_reason),
                                is_final: false, // Not final, we are just starting
                                silent: false,
                            });

                            // Build context using the context manager with memory integration for the Expert
                            let raw_ctx = telos_context::RawContext {
                                history_logs: vec![], // Could include session history here
                                retrieved_docs: vec![],
                            };
                            let ctx_req = telos_context::NodeRequirement {
                                required_tokens: 2000,
                                query: enriched_payload.clone(),
                            };
                            let actual_ctx = context_manager_spawn.compress_for_node(&raw_ctx, &ctx_req).await
                            .unwrap_or_else(|e| {
                                debug!("[Daemon] Context compression failed: {:?}, using empty context", e);
                                telos_context::ScopedContext {
                                    budget_tokens: 2000,
                                    summary_tree: vec![],
                                    precise_facts: vec![],
                                }
                            });

                            debug!(
                                "[Daemon] Context prepared with {} summary nodes and {} precise facts",
                                actual_ctx.summary_tree.len(),
                                actual_ctx.precise_facts.len()
                            );

                            // --- TIER 2: EXPERT AGENT PLANNING & DAG EXECUTION ---
                            let mut attempt = 0;
                            const MAX_ATTEMPTS: usize = 3;
                            let mut loop_final_response = String::new();
                            let mut loop_qa_accepted = false;
                            let mut loop_completed_nodes = 0;
                            let mut loop_failed_nodes = 0;
                            let mut loop_failed_node_ids = vec![];
                            let mut loop_summary = String::new();
                            let mut total_time_ms = 0;
                            let mut loop_final_trace_steps = Vec::new();

                            while attempt < MAX_ATTEMPTS {
                                attempt += 1;
                                debug!("[Daemon] Starting execution attempt {}/{}", attempt, MAX_ATTEMPTS);

                                let mut graph = TaskGraph::new(trace_id.to_string());
                                let mut terminal_nodes = vec![];

                                // Instantiate the specific expert dynamically.
                                let expert_node: Box<dyn ExecutableNode> = match expert_route {
                                    "software_expert" => Box::new(agents::architect::ArchitectAgent::new(gateway_clone.clone())) as Box<dyn ExecutableNode>,
                                    "research_expert" => Box::new(agents::researcher::DeepResearchAgent::new(gateway_clone.clone(), tool_registry.clone())) as Box<dyn ExecutableNode>,
                                    "qa_expert" => Box::new(agents::tester::TestingAgent::new(gateway_clone.clone())) as Box<dyn ExecutableNode>,
                                    _ => Box::new(agents::general::GeneralAgent::new(
                                        gateway_clone.clone(),
                                        tool_registry.clone(),
                                        config.tools_dir.clone(),
                                    )) as Box<dyn ExecutableNode>,
                                };

                                graph.add_node_with_metadata(
                                    "expert_execution".to_string(),
                                    expert_node,
                                    NodeMetadata {
                                        task_type: expert_route.to_string(),
                                        prompt_preview: truncate_for_preview(&enriched_payload, 100),
                                        tool_name: None,
                                        schema_payload: None,
                                    },
                                );
                                graph.current_state = GraphState {
                                    is_running: true,
                                    completed: false,
                                };
                                terminal_nodes.push("expert_execution".to_string());


                                // Build context using the context manager with memory integration
                                let raw_ctx = telos_context::RawContext {
                                    history_logs: vec![], // Could include session history here
                                    retrieved_docs: vec![],
                                };
                                let ctx_req = telos_context::NodeRequirement {
                                    required_tokens: 2000,
                                    query: enriched_payload.clone(),
                                };
                                let actual_ctx = context_manager_spawn.compress_for_node(&raw_ctx, &ctx_req).await
                                .unwrap_or_else(|e| {
                                    debug!("[Daemon] Context compression failed: {:?}, using empty context", e);
                                    telos_context::ScopedContext {
                                        budget_tokens: 1000,
                                        summary_tree: vec![],
                                        precise_facts: vec![],
                                    }
                                });

                                debug!(
                                    "[Daemon] Context prepared with {} summary nodes and {} precise facts",
                                    actual_ctx.summary_tree.len(),
                                    actual_ctx.precise_facts.len()
                                );

                                execution_engine
                                    .run_graph(
                                        &mut graph,
                                        &actual_ctx,
                                        registry_clone.as_ref(),
                                        broker_bg.as_ref(),
                                    )
                                    .await;

                                // Calculate task summary
                                total_time_ms = task_start_time.elapsed().as_millis() as u64;
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

                                // Fetch the results from the terminal nodes dynamically
                                let mut final_results = Vec::new();
                                use petgraph::Direction;
                                
                                for (node_id, &node_idx) in &graph.node_indices {
                                    // A terminal node has no outgoing edges
                                    if graph.edges.neighbors_directed(node_idx, Direction::Outgoing).count() == 0 {
                                        // Ignore dummy nodes whose output is just "Research plan generated" if they spawned a subgraph
                                        if let Some(res) = graph.node_results.get(node_id) {
                                            let output_str = res
                                                .output
                                                .as_ref()
                                                .map(|v| v.to_string())
                                                .unwrap_or_else(|| "No output".to_string());
                                                
                                            if !output_str.contains("Research plan generated") {
                                                if res.success {
                                                    final_results.push(format!("[{}] {}", node_id, output_str));
                                                } else {
                                                    let error_str = res
                                                        .error
                                                        .as_ref()
                                                        .map(|e| format!("{}: {}", e.error_type, e.message))
                                                        .unwrap_or_else(|| "Unknown error".to_string());
                                                    final_results
                                                        .push(format!("[{}] Failed: {}", node_id, error_str));
                                                }
                                            }
                                        }
                                    }
                                }

                                let combined_result = if final_results.is_empty() {
                                    "No result generated by graph".to_string()
                                } else {
                                    final_results.join("\n")
                                };

                                // Detect if the result actually contains error messages
                                let result_has_error = combined_result.to_lowercase().contains("error")
                                    || combined_result.to_lowercase().contains("failed")
                                    || combined_result.to_lowercase().contains("not found")
                                    || combined_result.to_lowercase().contains("unavailable")
                                    || combined_result.contains("失败");

                                let task_success = failed_nodes == 0 && !result_has_error;

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

                                // --- ROUTE AGENT SYNTHESIS ---
                                // Delegate the final summary to the ExpertAgent that planned the execution
                                let expert_agent_for_summary: Box<dyn telos_core::agent_traits::ExpertAgent> = match expert_route {
                                    "software_expert" => Box::new(agents::architect::ArchitectAgent::new(gateway_clone.clone())) as Box<dyn telos_core::agent_traits::ExpertAgent>,
                                    "research_expert" => Box::new(agents::researcher::DeepResearchAgent::new(gateway_clone.clone(), tool_registry.clone())) as Box<dyn telos_core::agent_traits::ExpertAgent>,
                                    "qa_expert" => Box::new(agents::tester::TestingAgent::new(gateway_clone.clone())) as Box<dyn telos_core::agent_traits::ExpertAgent>,
                                    _ => Box::new(agents::general::GeneralAgent::new(
                                        gateway_clone.clone(),
                                        tool_registry.clone(),
                                        config.tools_dir.clone(),
                                    )) as Box<dyn telos_core::agent_traits::ExpertAgent>,
                                };

                                let summary_input = telos_core::AgentInput {
                                    node_id: "expert_summary".to_string(),
                                    task: payload.to_string(),
                                    dependencies: {
                                        let mut deps = std::collections::HashMap::new();
                                        deps.insert("dag_results".to_string(), AgentOutput::success(
                                            serde_json::json!({"text": combined_result})
                                        ));
                                        deps
                                    },
                                    schema_payload: None,
                                    memory_context: router_input.memory_context.clone(),
                                    correction: None,
                                };

                                let summary_output = expert_agent_for_summary
                                    .summarize(&summary_input, registry_clone.as_ref())
                                    .await;

                                let final_response = if summary_output.success {
                                    summary_output
                                        .output
                                        .as_ref()
                                        .and_then(|json| json.get("text").and_then(|t| t.as_str()))
                                        .unwrap_or("No summary provided by expert.")
                                        .to_string()
                                } else {
                                    format!(
                                        "{}\n\n(Note: Failed to generate final summary: {:?})",
                                        combined_result, summary_output.error
                                    )
                                };

                                loop_final_response = final_response.clone();

                                loop_completed_nodes = completed_nodes;
                                loop_failed_nodes = failed_nodes;
                                loop_failed_node_ids = failed_node_ids.clone();
                                loop_summary = summary.clone();

                                loop_final_trace_steps.clear();
                                for (node_id, output) in &graph.node_results {
                                    let input_data = graph.node_metadata.get(node_id).map(|m| m.prompt_preview.clone()).unwrap_or_default();
                                    let error_opt = output.error.as_ref().map(|e| telos_core::NodeError::ExecutionFailed(e.message.clone()));
                                    loop_final_trace_steps.push(telos_evolution::TraceStep {
                                        node_id: node_id.clone(),
                                        input_data,
                                        output_data: output.output.as_ref().map(|v| v.to_string()),
                                        error: error_opt,
                                    });
                                }

                                // --- ROUTER EVALUATION ---
                                let eval_output = router.evaluate(&payload, &final_response, registry_clone.as_ref()).await;
                                if eval_output.success {
                                    if let Some(json) = eval_output.output {
                                        let is_acceptable = json.get("is_acceptable").and_then(|v| v.as_bool()).unwrap_or(false);
                                        let is_clarification = json.get("is_clarification").and_then(|v| v.as_bool()).unwrap_or(false);
                                        let critique = json.get("critique").and_then(|v| v.as_str()).unwrap_or("");
                                        
                                        if is_clarification {
                                            // Clarification is a valid response — send ClarificationNeeded and complete
                                            debug!("[Daemon] QA identified clarification response — delivering to user.");
                                            loop_qa_accepted = true;
                                            
                                            // Parse options from the response text — build structured options
                                            let options = parse_clarification_options(&final_response);
                                            broker_bg.publish_feedback(AgentFeedback::ClarificationNeeded {
                                                task_id: trace_id.to_string(),
                                                session_id: session_id.clone(),
                                                prompt: final_response.clone(),
                                                options,
                                            });
                                            break;
                                        } else if is_acceptable || attempt == MAX_ATTEMPTS {
                                            loop_qa_accepted = is_acceptable;
                                            if !is_acceptable {
                                                debug!("[Daemon] Max attempts reached despite router rejection. Proceeding anyway.");
                                            } else {
                                                debug!("[Daemon] Router evaluated result as acceptable.");
                                            }
                                            break;
                                        } else {
                                            debug!("[Daemon] Router rejected the output. Critique: {}", critique);
                                            broker_bg.publish_feedback(AgentFeedback::Output {
                                                task_id: trace_id.to_string(),
                                                session_id: session_id.clone(),
                                                content: format!("🔄 Router QA rejected output.\nCritique: {}", critique),
                                                is_final: false,
                                                silent: false,
                                            });
                                            enriched_payload = format!(
                                                "Task:\n{}\n\n[PERSONA CONTEXT]\n{}\n\n[SYSTEM DIRECTIVE — MANDATORY]\n\
                                                 Your previous attempt was REJECTED by the QA evaluator.\n\
                                                 Critique: {}\n\n\
                                                 You MUST autonomously retry with an improved strategy.\n\
                                                 DO NOT ask the user for permission, clarification, or confirmation.\n\
                                                 DO NOT say 'if you want me to continue' or similar phrases.\n\
                                                 Execute the corrected approach IMMEDIATELY and deliver the result.",
                                                payload, agents::prompt_builder::get_soul(), critique
                                            );
                                        }
                                    } else {
                                        break;
                                    }
                                } else {
                                    debug!("[Daemon] Router evaluation failed, continuing with generated output.");
                                    break;
                                }
                            }

                            // Send Output FIRST (is_final: true) so UI displays the final result text block
                            broker_bg.publish_feedback(AgentFeedback::Output {
                                task_id: trace_id.to_string(),
                                session_id: session_id.clone(),
                                content: loop_final_response.clone(),
                                is_final: true,
                                silent: false,
                            });

                            // Publish TaskCompleted feedback LAST, which breaks the CLI stream
                            broker_bg.publish_feedback(AgentFeedback::TaskCompleted {
                                task_id: trace_id.to_string(),
                                summary: TaskSummary {
                                    fulfilled: loop_qa_accepted,
                                    completed: true,
                                    total_nodes: loop_completed_nodes + loop_failed_nodes,
                                    completed_nodes: loop_completed_nodes,
                                    failed_nodes: loop_failed_nodes,
                                    total_time_ms,
                                    summary: loop_summary,
                                    failed_node_ids: loop_failed_node_ids,
                                },
                            });
                            // --- INTERACTION EVENT PERSISTENCE (Global Long-Term Memory) ---
                            let interaction_content = format!("User: {}\nAssistant: {}", payload, loop_final_response);
                            let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
                            let interaction_entry = telos_memory::MemoryEntry::new(
                                format!("interaction_{}_{}", session_id, timestamp),
                                telos_memory::MemoryType::InteractionEvent,
                                interaction_content.clone(),
                                timestamp,
                                None,
                            );

                            let memory_clone = registry_clone.memory_os.clone();
                            let session_id_for_mem = session_id.clone();
                            tokio::spawn(async move {
                                if let Err(e) = memory_clone.store(interaction_entry).await {
                                    debug!("[Daemon] ⚠️ Failed to store InteractionEvent for session {}: {}", session_id_for_mem, e);
                                } else {
                                    debug!("[Daemon] 📥 Successfully archived global InteractionEvent.");
                                }
                            });

                            // --- USER PROFILE EXTRACTION (Background) ---
                            let gw_for_profile = gateway_clone.clone();
                            let mem_for_profile = registry_clone.memory_os.clone();
                            let conv_for_profile = interaction_content;
                            tokio::spawn(async move {
                                extract_and_store_user_profile(&conv_for_profile, gw_for_profile, mem_for_profile).await;
                            });

                            // --- EVOLUTION & MEMORY INTEGRATION LOOP ---
                            let trace = telos_evolution::ExecutionTrace {
                                task_id: trace_id.to_string(),
                                steps: loop_final_trace_steps,
                                errors_encountered: vec![],
                                success: loop_qa_accepted,
                            };
                            
                                // Send trace to asynchronous Evolution worker for Skill Distillation
                                if let Err(e) = distillation_tx_spawn.send(trace) {
                                    debug!("[Daemon] ⚠️ Failed to send trace {} to evolution queue: {}", trace_id, e);
                                }
                            
                            // Cleanup task from registry
                            {
                                let mut w = active_tasks_spawn.write().await;
                                w.remove(&trace_id.clone().to_string());
                            }
                            }
                        });
                    }
                AgentEvent::UserIntervention {
                    task_id,
                    node_id,
                    instruction,
                    trace_id: _,
                } => {
                    debug!(
                        "[Daemon] UserIntervention for task {}: {}",
                        task_id, instruction
                    );
                    if let Some(node) = node_id {
                        let lock = wakeup_map_bg.lock().await;
                        if let Some(tx) = lock.get(&task_id) {
                            let _ = tx.send((task_id.clone(), node, instruction));
                        }
                    } else {
                        // Default to first waiting node if we can't be sure, but usually targeted.
                        debug!(
                            "[Daemon] Warning: Targeted intervention missing node_id for task {}",
                            task_id
                        );
                    }
                }
                AgentEvent::UserApproval {
                    task_id,
                    node_id: _,
                    approved,
                    supplement_info: _,
                    trace_id: _,
                } => {
                    let broker_bg = broker_bg.clone();
                    let gateway_clone = gateway_clone.clone();
                    let registry_clone = registry_clone.clone();
                    let paused_tasks_bg = paused_tasks_bg.clone();
                    let tool_registry = tool_registry.clone();
                    let config = loop_config.clone();

                    let node_factory = std::sync::Arc::new(DaemonNodeFactory {
                        gateway: gateway_clone.clone(),
                        tool_registry: tool_registry.clone(),
                        tools_dir: config.tools_dir.clone(),
                    });

                    let mut execution_engine = telos_dag::engine::TokioExecutionEngine::new()
                        .with_node_factory(node_factory)
                        .with_active_tasks(active_tasks_loop.clone());

                    let context_manager_approval = context_manager.clone();
                    tokio::spawn(async move {
                        let task_start_time = Instant::now();
                        debug!(
                            "[Daemon] Received UserApproval for task {} (approved: {})",
                            task_id, approved
                        );

                        if !approved {
                            broker_bg.publish_feedback(AgentFeedback::Output {
                                task_id: task_id.clone(),
                                session_id: "default".into(),
                                content: "Task Rejected.".into(),
                                is_final: true,
                                silent: false,
                            });
                            paused_tasks_bg.lock().await.remove(&task_id);

                            // Publish TaskCompleted for rejected task
                            broker_bg.publish_feedback(AgentFeedback::TaskCompleted {
                                task_id: task_id.clone(),
                                summary: TaskSummary {
                                    fulfilled: false,
                                    completed: true,
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
                                silent: false,
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
                                    schema_payload: None,
                                },
                            );
                            graph.current_state = GraphState {
                                is_running: true,
                                completed: false,
                            };

                            let raw_ctx = telos_context::RawContext {
                                history_logs: vec![],
                                retrieved_docs: vec![telos_context::Document {
                                    doc_id: "user_input".to_string(),
                                    content: payload.clone(),
                                }],
                            };
                            let req = telos_context::NodeRequirement {
                                required_tokens: 1000,
                                query: payload.clone(),
                            };
                            let actual_ctx = context_manager_approval
                                .compress_for_node(&raw_ctx, &req)
                                .await
                                .unwrap_or_else(|_e| telos_context::ScopedContext {
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
                                Some(res) if res.success => res
                                    .output
                                    .as_ref()
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| "No output".to_string()),
                                Some(res) => res
                                    .error
                                    .as_ref()
                                    .map(|e| format!("Error: {}", e.message))
                                    .unwrap_or_else(|| "Unknown error".to_string()),
                                None => "No result generated by node".to_string(),
                            };

                            let total_time_ms = task_start_time.elapsed().as_millis() as u64;
                            let task_success = graph
                                .node_statuses
                                .get("llm_node")
                                .map(|s| *s == telos_core::NodeStatus::Completed)
                                .unwrap_or(false);
                        }
                    });
                }
                _ => {}
            }
        }
    });

    // Start Bot Provider in background if configured
    if let Some(bot_token) = config.telegram_bot_token.clone() {
        info!("Starting Telegram Bot Provider from Daemon...");
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
                error!("Failed to start bot provider: {}", e);
            }
        });
    }

    let recent_traces = Arc::new(tokio::sync::RwLock::new(std::collections::VecDeque::with_capacity(100)));
    let mut rx = broker.subscribe_feedback();
    let traces_bg = recent_traces.clone();
    tokio::spawn(async move {
        while let Ok(feedback) = rx.recv().await {
            if let telos_hci::AgentFeedback::Trace { .. } = &feedback {
                let mut q = traces_bg.write().await;
                if q.len() >= 100 {
                    q.pop_front();
                }
                q.push_back(feedback);
            }
        }
    });

    // --- API SERVER ---
    let state = AppState {
        broker: broker.clone(),
        recent_traces,
        active_tasks,
    };

    let app = Router::new()
        .route("/api/v1/run", post(handle_run))
        .route("/api/v1/run_sync", post(handle_run_sync))
        .route("/api/v1/approve", post(handle_approve))
        .route("/api/v1/intervention", post(handle_intervention))
        .route("/api/v1/clarify", post(handle_clarify))
        .route("/api/v1/stream", get(ws_handler))
        .route("/api/v1/log-level", get(get_log_level).post(set_log_level))
        .route("/api/v1/traces", get(get_traces))
        .route("/api/v1/tasks/active", get(get_active_tasks))
        .route("/ui", get(serve_ui))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    info!("Telos Daemon listening on ws://0.0.0.0:3000/api/v1/stream");

    axum::serve(listener, app).await?;

    Ok(())
}

async fn get_traces(State(state): State<AppState>) -> Json<serde_json::Value> {
    let q = state.recent_traces.read().await;
    let traces: Vec<_> = q.iter().cloned().collect();
    Json(serde_json::json!({
        "traces": traces
    }))
}

async fn get_active_tasks(State(state): State<AppState>) -> Json<serde_json::Value> {
    let w = state.active_tasks.read().await;
    let tasks: Vec<_> = w.values().cloned().collect();
    Json(serde_json::json!({
        "active_tasks": tasks
    }))
}

async fn serve_ui() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("../static/index.html"))
}

async fn handle_run(
    State(state): State<AppState>,
    Json(req): Json<RunRequest>,
) -> Json<RunResponse> {
    let trace_id = req.trace_id.and_then(|id| Uuid::parse_str(&id).ok()).unwrap_or_else(Uuid::new_v4);
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

async fn handle_run_sync(
    State(state): State<AppState>,
    Json(req): Json<RunRequest>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let trace_id = req.trace_id.clone().and_then(|id| Uuid::parse_str(&id).ok()).unwrap_or_else(Uuid::new_v4);
    let trace_id_str = trace_id.to_string();
    
    // Subscribe *before* dispatching to avoid race conditions
    let mut rx = state.broker.subscribe_feedback();
    
    let _ = state
        .broker
        .publish_event(AgentEvent::UserInput {
            session_id: "default".into(),
            payload: req.payload,
            trace_id,
            project_id: req.project_id,
        })
        .await;

    let stream = async_stream::stream! {
        // Send initial acknowledgment
        yield Ok(Event::default().event("started").data(
            serde_json::json!({"trace_id": trace_id_str}).to_string()
        ));

        while let Ok(feedback) = rx.recv().await {
            match feedback {
                AgentFeedback::TaskCompleted { task_id, summary } if task_id == trace_id_str => {
                    let summary_json = serde_json::to_string(&summary).unwrap_or_default();
                    yield Ok(Event::default().event("completed").data(summary_json));
                    break;
                }
                AgentFeedback::Output { task_id, content, is_final, .. } if task_id == trace_id_str => {
                    if is_final {
                        yield Ok(Event::default().event("output").data(content));
                    } else {
                        yield Ok(Event::default().event("heartbeat").data(content));
                    }
                }
                AgentFeedback::ClarificationNeeded { task_id, prompt, options, .. } if task_id == trace_id_str => {
                    let data = serde_json::json!({
                        "prompt": prompt,
                        "options": options,
                    });
                    yield Ok(Event::default().event("clarification").data(data.to_string()));
                }
                _ => {}
            }
        }
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
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
            node_id: None,
            approved: req.approved,
            supplement_info: None,
            trace_id,
        })
        .await;

    Json(ApproveResponse {
        status: "approval received".into(),
    })
}

#[derive(serde::Deserialize)]
pub struct InterventionRequest {
    pub task_id: String,
    pub node_id: Option<String>,
    pub instruction: String,
}

#[derive(serde::Serialize)]
pub struct InterventionResponse {
    pub status: String,
}

async fn handle_intervention(
    State(state): State<AppState>,
    Json(req): Json<InterventionRequest>,
) -> Json<InterventionResponse> {
    let trace_id = Uuid::new_v4();
    let _ = state
        .broker
        .publish_event(AgentEvent::UserIntervention {
            task_id: req.task_id,
            node_id: req.node_id,
            instruction: req.instruction,
            trace_id,
        })
        .await;

    Json(InterventionResponse {
        status: "intervention received".into(),
    })
}

#[derive(serde::Deserialize)]
pub struct ClarifyRequest {
    pub task_id: String,
    pub selected_option_id: Option<String>,
    pub free_text: Option<String>,
}

#[derive(serde::Serialize)]
pub struct ClarifyResponse {
    pub status: String,
}

async fn handle_clarify(
    State(state): State<AppState>,
    Json(req): Json<ClarifyRequest>,
) -> Json<ClarifyResponse> {
    let trace_id = Uuid::new_v4();
    let _ = state
        .broker
        .publish_event(AgentEvent::ClarificationResponse {
            task_id: req.task_id,
            selected_option_id: req.selected_option_id,
            free_text: req.free_text,
            trace_id,
        })
        .await;

    Json(ClarifyResponse {
        status: "clarification received".into(),
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

    // Persist to config file
    if let Ok(mut config) = TelosConfig::load() {
        config.log_level = format!("{:?}", new_level).to_lowercase();
        if let Err(e) = config.save() {
            error!("Failed to persist log level to config: {}", e);
        }
    }

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

#[derive(serde::Deserialize)]
pub struct WsQuery {
    pub trace_id: Option<String>,
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state, query.trace_id))
}

/// 创建系统取消通知（当通道关闭时发送）
fn create_cancellation_feedback(task_id: &str) -> AgentFeedback {
    AgentFeedback::TaskCompleted {
        task_id: task_id.to_string(),
        summary: telos_hci::TaskSummary {
            fulfilled: false,
            completed: true,
            total_nodes: 0,
            completed_nodes: 0,
            failed_nodes: 0,
            total_time_ms: 0,
            summary: "任务已取消：系统连接断开".to_string(),
            failed_node_ids: vec![],
        },
    }
}

async fn handle_socket(mut socket: WebSocket, state: AppState, filter_trace_id: Option<String>) {
    let mut rx = state.broker.subscribe_feedback();
    let mut current_task_id: Option<String> = None;

    loop {
        tokio::select! {
            // 处理来自 broker 的反馈
            result = rx.recv() => {
                match result {
                    Ok(feedback) => {
                        // 跟踪当前任务ID
                        if let Some(task_id) = feedback.task_id() {
                            current_task_id = Some(task_id.to_string());
                        }

                        // Apply trace_id filter if it was requested via query parameter
                        if let Some(expected_trace_id) = &filter_trace_id {
                            // Since task_id is identical to trace_id for CLI runs, we filter on task_id
                            if let Some(t_id) = feedback.task_id() {
                                if t_id != expected_trace_id {
                                    continue;
                                }
                            } else {
                                // If a message has no task_id, we probably don't want to blindly forward it 
                                // to a trace-specific socket, except maybe LogLevelChanged which is global.
                                if !matches!(feedback, telos_hci::AgentFeedback::LogLevelChanged { .. }) {
                                    continue;
                                }
                            }
                        }

                        let msg_str = serde_json::to_string(&feedback).unwrap_or_else(|_| "{}".to_string());

                        if socket.send(Message::Text(msg_str)).await.is_err() {
                            debug!("[WebSocket] Failed to send message, client disconnected");
                            break;
                        }

                        // 如果是最终消息，正常退出
                        if feedback.is_final() {
                            debug!("[WebSocket] Task completed, closing connection");
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        // 通道关闭，发送系统通知
                        debug!("[WebSocket] Broker channel closed, sending cancellation notice");
                        if let Some(task_id) = &current_task_id {
                            let cancellation = create_cancellation_feedback(task_id);
                            let msg_str = serde_json::to_string(&cancellation).unwrap_or_else(|_| "{}".to_string());
                            let _ = socket.send(Message::Text(msg_str)).await;
                        }
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        // 消息积压，继续运行但记录警告
                        warn!("[WebSocket] Warning: Lagged {} messages, continuing...", n);
                        continue;
                    }
                }
            }
            // 处理来自客户端的消息（心跳、关闭请求等）
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Ping(data))) => {
                        if socket.send(Message::Pong(data)).await.is_err() {
                            debug!("[WebSocket] Failed to send pong");
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {
                        // Pong received, continue
                    }
                    Some(Ok(Message::Close(_))) => {
                        debug!("[WebSocket] Client requested close");
                        break;
                    }
                    Some(Err(e)) => {
                        debug!("[WebSocket] WebSocket error: {:?}", e);
                        break;
                    }
                    None => {
                        debug!("[WebSocket] WebSocket stream ended");
                        break;
                    }
                    _ => {
                        // Ignore other message types
                    }
                }
            }
        }
    }

    // 清理：发送关闭帧
    let _ = socket.close().await;
}
