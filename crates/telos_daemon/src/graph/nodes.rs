use std::sync::Arc;
use tracing::info;

// Telemetry Metrics

// Core Traits and Primitives
use async_trait::async_trait;
use telos_core::{AgentInput, AgentOutput, SystemRegistry};
use telos_dag::ExecutableNode;
use telos_model_gateway::gateway::{GatewayManager, ModelProvider};
use telos_model_gateway::{Capability, LlmRequest, ModelGateway};
use telos_tooling::ToolRegistry;

// 1. Adapter to convert Context OpenAiProvider to Gateway ModelProvider for the Gateway Manager

    pub fn truncate_for_preview(s: &str, max_len: usize) -> String {
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
    pub fn parse_clarification_options(response: &str) -> Vec<telos_hci::ClarificationOption> {
    let mut options = Vec::new();
    let mut idx = 0;
    
    for line in response.lines() {
        let trimmed = line.trim();
        // Match lines starting with explicit list markers only:
        // - Bullet: •, -, *
        // - Numbered: 1. 2. 3. or 1) 2) 3)
        let is_option = trimmed.starts_with('•') 
            || (trimmed.starts_with('-') && trimmed.len() > 2 && trimmed.chars().nth(1) == Some(' '))
            || (trimmed.starts_with('*') && trimmed.len() > 2 && trimmed.chars().nth(1) == Some(' '))
            || trimmed.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
               && (trimmed.contains(". ") || trimmed.contains(") "));
        
        if is_option && trimmed.len() > 2 {
            idx += 1;
            let label = trimmed.trim_start_matches(|c: char| c == '•' || c == '-' || c == '*' || c.is_ascii_digit() || c == '.' || c == ')' || c == ' ').trim().to_string();
            if !label.is_empty() && label.len() > 1 {
                options.push(telos_hci::ClarificationOption {
                    id: format!("opt_{}", idx),
                    label: label.clone(),
                    description: String::new(),
                });
            }
        }
    }
    
    // Removed the fallback to default options (搜索信息, etc.).
    // If the agent asks an open-ended question without explicit options,
    // the user should just type their reply without seeing unrelated buttons.
    
    options
}

/// Extracts user preferences, personal facts, and habits from a conversation
/// using LLM, then stores new facts as UserProfile memories (with deduplication).
/// Compress a long assistant response for session_logs storage.
/// Short responses (≤500 chars) are returned as-is.
/// Long responses are summarized via LLM to preserve key facts while reducing token usage.

pub struct LlmPromptNode {
    pub prompt: String,
    pub gateway: Arc<GatewayManager>,
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



pub struct ToolNode {
    pub tool_name: String,
    pub tool_registry: Arc<tokio::sync::RwLock<telos_tooling::retrieval::VectorToolRegistry>>,
}

#[async_trait]
impl ExecutableNode for ToolNode {
    async fn execute(&self, input: AgentInput, _registry: &dyn SystemRegistry) -> AgentOutput {
        // Tool input is expected to be a JSON object in schema_payload
        let mut actual_tool_name = self.tool_name.clone();
        let mut actual_params = serde_json::json!({});

        if let Some(ref payload) = input.schema_payload {
            match serde_json::from_str::<serde_json::Value>(payload) {
                Ok(mut p) => {
                    // If the LLM outputted {"tool_name": "...", "parameters": {...}}, extract them.
                    if let Some(t) = p.get("tool_name").and_then(|v| v.as_str()) {
                        actual_tool_name = t.to_string();
                    }
                    if let Some(params) = p.get_mut("parameters") {
                        actual_params = params.take();
                    } else {
                        actual_params = p; // fallback to the raw payload if 'parameters' wrapper not found
                    }
                }
                Err(e) => {
                    return AgentOutput::failure(
                        "PayloadParseError",
                        &format!("Failed to parse tool payload: {}", e),
                    );
                }
            }
        }

        let registry_guard = self.tool_registry.read().await;
        let executor = match registry_guard.get_executor(&actual_tool_name) {
            Some(e) => e,
            None => {
                return AgentOutput::failure(
                    "ToolNotFoundError",
                    &format!("Tool '{}' not found in registry (Task alias: '{}')", actual_tool_name, self.tool_name),
                );
            }
        };
        drop(registry_guard);

        info!("[ToolNode] 🛠️  Executing tool: {} with params: {}", actual_tool_name, actual_params);

        match executor.call(actual_params.clone()).await {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes).to_string();
                let (json_result, format_type) = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                    (json, "json")
                } else {
                    (serde_json::json!({ "raw_output": text.clone() }), "raw_text")
                };

                let trace = telos_core::TraceLog::ToolCall {
                    name: actual_tool_name.clone(),
                    params: actual_params.clone(),
                    result: json_result.clone(),
                };

                // Wrap output with metadata for the Summarizer to understand the source
                let output = serde_json::json!({
                    "text": if format_type == "json" {
                        serde_json::to_string_pretty(&json_result).unwrap_or_else(|_| text.clone())
                    } else {
                        text
                    },
                    "tool_name": actual_tool_name,
                    "format": format_type,
                });
                AgentOutput::success(output).with_trace(trace)
            }
            Err(e) => {
                AgentOutput::failure("ToolExecutionError", &format!("{:?}", e))
            }
        }
    }
}



// 4. Server App State
