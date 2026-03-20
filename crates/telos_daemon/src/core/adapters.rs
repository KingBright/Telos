use std::sync::Arc;
use tracing::{info, warn, error};

// Telemetry Metrics
use std::sync::atomic::Ordering;
use crate::core::metrics::METRICS;
use crate::core::metrics_store::{self, MetricEvent, now_ms};

// Core Traits and Primitives
use async_trait::async_trait;
use telos_context::providers::OpenAiProvider;
use telos_core::SystemRegistry;
use telos_memory::engine::RedbGraphStore;
use telos_model_gateway::gateway::{GatewayManager, ModelProvider};
use telos_model_gateway::{GatewayError, LlmRequest, LlmResponse};

// 1. Adapter to convert Context OpenAiProvider to Gateway ModelProvider for the Gateway Manager

pub struct GatewayAdapter {
    pub inner: OpenAiProvider,
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

        METRICS.llm_total_requests.fetch_add(1, Ordering::Relaxed);

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

        let model_name = self.inner.model_name().to_string();
        match self.inner.generate_chat_with_tools(messages, tools).await {
            Ok(resp) => {
                // Approximate tokens: ~4 chars per token
                let mut total_len = 0;
                for m in &req.messages {
                    total_len += m.content.len();
                }
                let estimated_tokens = (total_len + resp.content.len()) / 4;
                
                // Persist token usage
                METRICS.llm_cumulative_tokens.fetch_add(estimated_tokens, Ordering::Relaxed);
                // Rough cost: $0.002 per 1K tokens (configurable later)
                let cost = estimated_tokens as f64 * 0.000002;
                METRICS.llm_estimated_cost_x10000.fetch_add((cost * 10_000.0) as usize, Ordering::Relaxed);
                
                metrics_store::record(MetricEvent::LlmCall {
                    timestamp_ms: now_ms(),
                    agent_name: "system".to_string(), // Set by caller via thread-local in future
                    task_id: req.session_id.clone(),
                    model: model_name.clone(),
                    tokens: estimated_tokens,
                    estimated_cost: cost,
                });
                
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
                    METRICS.llm_http_429_errors.fetch_add(1, Ordering::Relaxed);
                    metrics_store::record(MetricEvent::LlmError {
                        timestamp_ms: now_ms(),
                        error_type: "429".to_string(),
                        model: model_name.clone(),
                    });
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
                    METRICS.llm_other_api_errors.fetch_add(1, Ordering::Relaxed);
                    metrics_store::record(MetricEvent::LlmError {
                        timestamp_ms: now_ms(),
                        error_type: "network".to_string(),
                        model: model_name.clone(),
                    });
                    Err(GatewayError::from_network_error(&e.message))
                } else if error_msg.contains("503") || error_msg.contains("service unavailable") {
                    warn!("[GatewayAdapter] Service unavailable: {}", e.message);
                    METRICS.llm_other_api_errors.fetch_add(1, Ordering::Relaxed);
                    metrics_store::record(MetricEvent::LlmError {
                        timestamp_ms: now_ms(),
                        error_type: "other".to_string(),
                        model: model_name.clone(),
                    });
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
pub struct DaemonRegistry {
    pub gateway: Arc<GatewayManager>,
    pub memory_os: Arc<RedbGraphStore>,
    pub system_context: Arc<tokio::sync::RwLock<telos_core::SystemContext>>,
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
