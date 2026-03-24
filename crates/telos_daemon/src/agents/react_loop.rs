//! Generic ReAct (Reason + Act) loop for LLM-driven tool calling.
//!
//! This module provides a reusable loop that any agent can use to:
//! 1. Send messages + tool definitions to the LLM
//! 2. Execute any tool_calls the LLM requests
//! 3. Feed results back to the LLM
//! 4. Repeat until the LLM returns a final text response (or limits are hit)
//!
//! Safety mechanisms:
//! - Max iteration limit (default 15)
//! - Consecutive error circuit breaker (default 3)
//! - Duplicate operation detection (same tool+args 3x → inject warning)
//! - Token budget tracking

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn, error, debug};

use telos_model_gateway::{
    ModelGateway, LlmRequest, Message, Capability,
    ToolDefinition,
};
use telos_model_gateway::gateway::GatewayManager;
use telos_tooling::{ToolExecutor, ToolRegistry, ToolSchema};
use telos_tooling::retrieval::VectorToolRegistry;
use telos_core::{AgentOutput, TraceLog};

/// Configuration for the ReAct loop
#[derive(Debug, Clone)]
pub struct ReactConfig {
    /// Maximum number of LLM call iterations (default: 15)
    pub max_iterations: usize,
    /// Maximum consecutive tool errors before stopping (default: 3)
    pub max_consecutive_errors: usize,
    /// Maximum times the same tool+args combo can repeat (default: 3)
    pub max_duplicate_calls: usize,
    /// Session ID for the LLM request
    pub session_id: String,
    /// Token budget limit
    pub budget_limit: usize,
}

impl Default for ReactConfig {
    fn default() -> Self {
        Self {
            max_iterations: 15,
            max_consecutive_errors: 3,
            max_duplicate_calls: 3,
            session_id: "react_loop".to_string(),
            budget_limit: 128_000,
        }
    }
}

/// Result of running the ReAct loop
#[derive(Debug)]
pub struct ReactResult {
    /// Final text output from the LLM
    pub content: String,
    /// Total iterations used
    pub iterations: usize,
    /// Total tool calls made
    pub tool_calls_made: usize,
    /// Total tokens consumed (approximate)
    pub tokens_used: usize,
    /// Whether the loop completed normally (vs hitting a limit)
    pub completed_normally: bool,
    /// Trace logs for observability
    pub trace_logs: Vec<TraceLog>,
}

/// The generic ReAct loop executor.
pub struct ReactLoop {
    gateway: Arc<GatewayManager>,
    tool_registry: Arc<RwLock<VectorToolRegistry>>,
    config: ReactConfig,
}

impl ReactLoop {
    pub fn new(
        gateway: Arc<GatewayManager>,
        tool_registry: Arc<RwLock<VectorToolRegistry>>,
        config: ReactConfig,
    ) -> Self {
        Self {
            gateway,
            tool_registry,
            config,
        }
    }

    /// Run the ReAct loop with the given system prompt, user message, and available tools.
    ///
    /// The loop will:
    /// 1. Call the LLM with messages + tool definitions
    /// 2. If the LLM returns tool_calls → execute them, feed results back
    /// 3. If the LLM returns text only → return that as the final result
    /// 4. Repeat until max_iterations or other limits are hit
    pub async fn run(
        &self,
        system_prompt: String,
        user_message: String,
        available_tools: Vec<ToolSchema>,
        conversation_history: &[telos_core::ConversationMessage],
    ) -> ReactResult {
        let mut messages = vec![
            Message {
                role: "system".to_string(),
                content: system_prompt,
            },
        ];
        
        for msg in conversation_history {
            messages.push(Message {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
        }
        
        messages.push(Message {
            role: "user".to_string(),
            content: user_message,
        });

        // Convert ToolSchema → ToolDefinition for the LLM
        let tool_defs: Vec<ToolDefinition> = available_tools.iter().map(|t| {
            ToolDefinition {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters_schema.raw_schema.clone(),
            }
        }).collect();

        let mut iteration = 0;
        let mut total_tool_calls = 0;
        let mut total_tokens = 0;
        let mut consecutive_errors = 0;
        let mut trace_logs = Vec::new();
        // Track duplicate calls: (tool_name, args_hash) → count
        let mut call_history: Vec<(String, String)> = Vec::new();

        info!(
            "[ReactLoop] Starting with {} tools, max {} iterations",
            tool_defs.len(),
            self.config.max_iterations
        );

        loop {
            iteration += 1;
            if iteration > self.config.max_iterations {
                warn!(
                    "[ReactLoop] Hit max iterations ({}). Returning best available result.",
                    self.config.max_iterations
                );
                // Ask LLM for a final summary without tools
                let summary = self.force_final_answer(&messages).await;
                return ReactResult {
                    content: summary,
                    iterations: iteration - 1,
                    tool_calls_made: total_tool_calls,
                    tokens_used: total_tokens,
                    completed_normally: false,
                    trace_logs,
                };
            }

            // 1. Call LLM with tools
            let request = LlmRequest {
                session_id: self.config.session_id.clone(),
                messages: messages.clone(),
                required_capabilities: Capability {
                    requires_vision: false,
                    strong_reasoning: true,
                },
                budget_limit: self.config.budget_limit,
                tools: if tool_defs.is_empty() { None } else { Some(tool_defs.clone()) },
            };

            let response = match self.gateway.generate(request.clone()).await {
                Ok(r) => r,
                Err(e) => {
                    error!("[ReactLoop] LLM call failed at iteration {}: {}", iteration, e.to_user_message());
                    trace_logs.push(TraceLog::LlmCall {
                        request: serde_json::to_value(&request).unwrap_or_default(),
                        response: serde_json::json!({ "error": e.to_user_message() }),
                    });
                    return ReactResult {
                        content: format!("LLM call failed: {}", e.to_user_message()),
                        iterations: iteration,
                        tool_calls_made: total_tool_calls,
                        tokens_used: total_tokens,
                        completed_normally: false,
                        trace_logs,
                    };
                }
            };

            total_tokens += response.tokens_used;

            // Record trace
            trace_logs.push(TraceLog::LlmCall {
                request: serde_json::json!({
                    "iteration": iteration,
                    "message_count": messages.len(),
                    "tools_count": tool_defs.len(),
                }),
                response: serde_json::json!({
                    "content_len": response.content.len(),
                    "tool_calls": response.tool_calls.len(),
                    "finish_reason": &response.finish_reason,
                    "tokens_used": response.tokens_used,
                }),
            });

            info!(
                "[ReactLoop] Iteration {}: {} tool_calls, {} bytes content, finish_reason={:?}",
                iteration,
                response.tool_calls.len(),
                response.content.len(),
                response.finish_reason
            );

            // 2. If no tool_calls → LLM is done, return content
            if response.tool_calls.is_empty() {
                info!("[ReactLoop] ✅ Completed normally after {} iterations, {} tool calls", iteration, total_tool_calls);
                return ReactResult {
                    content: response.content,
                    iterations: iteration,
                    tool_calls_made: total_tool_calls,
                    tokens_used: total_tokens,
                    completed_normally: true,
                    trace_logs,
                };
            }

            // 3. Add assistant message with tool_calls to conversation
            // For OpenAI-compatible APIs, the assistant message that contains tool_calls
            // needs to be represented properly
            messages.push(Message {
                role: "assistant".to_string(),
                content: if response.content.is_empty() {
                    // When there are tool calls but no content, we still need a placeholder
                    format!("[Tool calls: {}]", response.tool_calls.iter()
                        .map(|tc| format!("{}({})", tc.name, &tc.arguments.chars().take(50).collect::<String>()))
                        .collect::<Vec<_>>()
                        .join(", "))
                } else {
                    response.content.clone()
                },
            });

            // 4. Execute each tool call
            for tool_call in &response.tool_calls {
                total_tool_calls += 1;

                // Duplicate detection
                let _call_sig = format!("{}:{}", tool_call.name, tool_call.arguments);
                let dup_count = call_history.iter().filter(|(n, a)| *n == tool_call.name && *a == tool_call.arguments).count();
                call_history.push((tool_call.name.clone(), tool_call.arguments.clone()));

                if dup_count >= self.config.max_duplicate_calls {
                    warn!(
                        "[ReactLoop] ⚠️ Duplicate call detected: {} called {} times with same args. Injecting warning.",
                        tool_call.name, dup_count + 1
                    );
                    messages.push(Message {
                        role: "tool".to_string(),
                        content: format!(
                            "{{\"error\": \"DUPLICATE_CALL_WARNING: You have called '{}' {} times with the same arguments. The result has not changed. Please try a DIFFERENT approach or tool, or provide your final answer.\", \"tool_call_id\": \"{}\"}}",
                            tool_call.name, dup_count + 1, tool_call.id
                        ),
                    });
                    continue;
                }

                // Execute the tool
                let tool_result = self.execute_tool(&tool_call.name, &tool_call.arguments).await;

                match &tool_result {
                    Ok(result_str) => {
                        consecutive_errors = 0;
                        debug!("[ReactLoop] Tool {} succeeded: {} bytes", tool_call.name, result_str.len());
                        messages.push(Message {
                            role: "tool".to_string(),
                            content: format!(
                                "{{\"tool_call_id\": \"{}\", \"result\": {}}}",
                                tool_call.id,
                                serde_json::to_string(result_str).unwrap_or_else(|_| format!("\"{}\"", result_str))
                            ),
                        });

                        trace_logs.push(TraceLog::ToolCall {
                            name: tool_call.name.clone(),
                            params: serde_json::from_str(&tool_call.arguments)
                                .unwrap_or_else(|_| serde_json::json!({"raw": &tool_call.arguments})),
                            result: serde_json::json!({ "success": true, "len": result_str.len() }),
                        });
                    }
                    Err(err) => {
                        consecutive_errors += 1;
                        warn!(
                            "[ReactLoop] Tool {} failed (consecutive errors: {}): {}",
                            tool_call.name, consecutive_errors, err
                        );
                        messages.push(Message {
                            role: "tool".to_string(),
                            content: format!(
                                "{{\"tool_call_id\": \"{}\", \"error\": \"{}\"}}",
                                tool_call.id,
                                err.replace('\"', "\\\"")
                            ),
                        });

                        trace_logs.push(TraceLog::ToolCall {
                            name: tool_call.name.clone(),
                            params: serde_json::from_str(&tool_call.arguments)
                                .unwrap_or_else(|_| serde_json::json!({"raw": &tool_call.arguments})),
                            result: serde_json::json!({ "error": err }),
                        });

                        // Circuit breaker: too many consecutive errors
                        if consecutive_errors >= self.config.max_consecutive_errors {
                            warn!(
                                "[ReactLoop] ⛔ {} consecutive errors. Stopping loop and requesting summary.",
                                consecutive_errors
                            );
                            messages.push(Message {
                                role: "system".to_string(),
                                content: format!(
                                    "IMPORTANT: {} consecutive tool errors have occurred. Please stop calling tools and provide your best answer based on the information gathered so far.",
                                    consecutive_errors
                                ),
                            });
                            // Do one more LLM call without tools to get a summary
                            let summary = self.force_final_answer(&messages).await;
                            return ReactResult {
                                content: summary,
                                iterations: iteration,
                                tool_calls_made: total_tool_calls,
                                tokens_used: total_tokens,
                                completed_normally: false,
                                trace_logs,
                            };
                        }
                    }
                }
            }
        }
    }

    /// Execute a single tool by name with JSON arguments.
    async fn execute_tool(&self, tool_name: &str, arguments: &str) -> Result<String, String> {
        // Parse arguments
        let params: serde_json::Value = serde_json::from_str(arguments)
            .map_err(|e| format!("Invalid JSON arguments: {}", e))?;

        // Get executor from registry
        let executor = {
            let guard = self.tool_registry.read().await;
            guard.get_executor(tool_name)
        };

        let executor = executor
            .ok_or_else(|| format!("Tool '{}' not found in registry", tool_name))?;

        // Execute with timeout (30 seconds for most tools, 120 for shell)
        let timeout_secs = if tool_name == "shell_exec" { 120 } else { 30 };
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            executor.call(params),
        )
        .await;

        match result {
            Ok(Ok(bytes)) => {
                String::from_utf8(bytes)
                    .map_err(|_| "Tool returned non-UTF8 output".to_string())
            }
            Ok(Err(tool_err)) => {
                Err(format!("Tool error: {}", tool_err))
            }
            Err(_) => {
                Err(format!("Tool '{}' timed out after {}s", tool_name, timeout_secs))
            }
        }
    }

    /// Force the LLM to give a final answer without any tools available.
    async fn force_final_answer(&self, messages: &[Message]) -> String {
        let mut final_messages = messages.to_vec();
        final_messages.push(Message {
            role: "system".to_string(),
            content: "Please provide your final, comprehensive answer based on all the information gathered so far. Do not request any more tool calls.".to_string(),
        });

        let request = LlmRequest {
            session_id: self.config.session_id.clone(),
            messages: final_messages,
            required_capabilities: Capability {
                requires_vision: false,
                strong_reasoning: true,
            },
            budget_limit: self.config.budget_limit,
            tools: None, // No tools → forces text response
        };

        match self.gateway.generate(request).await {
            Ok(r) => r.content,
            Err(e) => format!("Failed to generate final answer: {}", e.to_user_message()),
        }
    }

    /// Convert a ReactResult into an AgentOutput
    pub fn to_agent_output(result: ReactResult) -> AgentOutput {
        if result.completed_normally {
            let mut output = AgentOutput::success(serde_json::json!({
                "text": result.content,
                "react_meta": {
                    "iterations": result.iterations,
                    "tool_calls": result.tool_calls_made,
                    "tokens_used": result.tokens_used,
                    "completed_normally": true,
                }
            }));
            output.trace_logs = result.trace_logs;
            output
        } else if !result.content.is_empty() {
            // Partial result: has content but hit limits
            let mut output = AgentOutput::success(serde_json::json!({
                "text": result.content,
                "react_meta": {
                    "iterations": result.iterations,
                    "tool_calls": result.tool_calls_made,
                    "tokens_used": result.tokens_used,
                    "completed_normally": false,
                }
            }));
            output.trace_logs = result.trace_logs;
            output
        } else {
            AgentOutput::failure(
                "ReactLoopFailed",
                &format!("ReAct loop failed after {} iterations", result.iterations),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_react_config_defaults() {
        let config = ReactConfig::default();
        assert_eq!(config.max_iterations, 15);
        assert_eq!(config.max_consecutive_errors, 3);
        assert_eq!(config.max_duplicate_calls, 3);
    }

    #[test]
    fn test_react_result_to_agent_output() {
        let result = ReactResult {
            content: "Hello world".to_string(),
            iterations: 3,
            tool_calls_made: 5,
            tokens_used: 1000,
            completed_normally: true,
            trace_logs: vec![],
        };
        let output = ReactLoop::to_agent_output(result);
        assert!(output.success);
        let text = output.output.unwrap()["text"].as_str().unwrap().to_string();
        assert_eq!(text, "Hello world");
    }

    #[test]
    fn test_react_result_failure() {
        let result = ReactResult {
            content: "".to_string(),
            iterations: 15,
            tool_calls_made: 0,
            tokens_used: 0,
            completed_normally: false,
            trace_logs: vec![],
        };
        let output = ReactLoop::to_agent_output(result);
        assert!(!output.success);
    }
}
