use async_trait::async_trait;


pub mod backoff;
pub mod gateway;

// Re-export circuit breaker types from gateway
pub use gateway::{CircuitBreaker, CircuitBreakerConfig, CircuitState};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Capability {
    pub requires_vision: bool,
    pub strong_reasoning: bool,
}

/// Definition of a tool that can be passed to the LLM for function calling.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// A tool call request returned by the LLM when it decides to invoke a tool.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ToolCallRequest {
    /// Unique ID for this tool call (used to match results back)
    pub id: String,
    /// Name of the tool to invoke
    pub name: String,
    /// JSON-encoded arguments string
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LlmRequest {
    pub session_id: String,
    pub messages: Vec<Message>,
    pub required_capabilities: Capability,
    pub budget_limit: usize,
    /// Optional tool definitions for LLM function calling.
    /// When provided, the LLM may return tool_calls instead of content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LlmResponse {
    pub content: String,
    pub tokens_used: usize,
    /// Tool calls requested by the LLM (empty if the LLM returned text content)
    #[serde(default)]
    pub tool_calls: Vec<ToolCallRequest>,
    /// Finish reason from the API: "stop", "tool_calls", "length", etc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

/// 网络错误类型细分
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkErrorKind {
    /// 连接超时
    ConnectionTimeout,
    /// DNS 解析失败
    DnsError,
    /// 连接被拒绝
    ConnectionRefused,
    /// SSL/TLS 错误
    SslError,
    /// 读取超时
    ReadTimeout,
    /// 写入超时
    WriteTimeout,
    /// 连接中断
    ConnectionReset,
    /// 未知网络错误
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GatewayError {
    /// 429 - 请求过多
    TooManyRequests {
        retry_after_ms: Option<u64>,
    },
    /// 503 - 服务暂时不可用
    ServiceUnavailable {
        estimated_recovery_ms: Option<u64>,
    },
    /// 网络错误（可重试）
    NetworkError {
        kind: NetworkErrorKind,
        message: String,
        retry_suggested: bool,
    },
    /// 401/403 - 认证/授权错误（永久性）
    AuthenticationError {
        message: String,
    },
    /// 配额超限
    QuotaExceeded {
        quota_type: String,
    },
    /// 内容过滤
    ContentFiltered {
        reason: String,
    },
    /// 其他错误
    Other {
        message: String,
        is_retryable: bool,
    },
}

impl GatewayError {
    /// 转换为用户友好的消息
    pub fn to_user_message(&self) -> String {
        match self {
            Self::TooManyRequests { retry_after_ms } => {
                if let Some(ms) = retry_after_ms {
                    format!("服务繁忙，请稍后重试（建议等待 {} 秒）", ms / 1000)
                } else {
                    "服务繁忙，请稍后重试".to_string()
                }
            }
            Self::ServiceUnavailable { estimated_recovery_ms } => {
                if let Some(ms) = estimated_recovery_ms {
                    format!("服务暂时不可用，预计 {} 秒后恢复", ms / 1000)
                } else {
                    "服务暂时不可用，正在维护中".to_string()
                }
            }
            Self::NetworkError { kind, message, .. } => {
                match kind {
                    NetworkErrorKind::ConnectionTimeout => "网络连接超时，请检查网络".to_string(),
                    NetworkErrorKind::DnsError => "域名解析失败，请检查网络设置".to_string(),
                    NetworkErrorKind::ConnectionRefused => "服务器拒绝连接".to_string(),
                    NetworkErrorKind::SslError => "安全连接失败".to_string(),
                    NetworkErrorKind::ReadTimeout | NetworkErrorKind::WriteTimeout => {
                        "网络响应超时，请稍后重试".to_string()
                    }
                    NetworkErrorKind::ConnectionReset => "网络连接中断".to_string(),
                    NetworkErrorKind::Unknown => format!("网络错误: {}", message),
                }
            }
            Self::AuthenticationError { message } => {
                format!("认证失败: {}", message)
            }
            Self::QuotaExceeded { quota_type } => {
                format!("{} 配额已用尽，请升级套餐或等待重置", quota_type)
            }
            Self::ContentFiltered { reason } => {
                format!("内容被过滤: {}", reason)
            }
            Self::Other { message, .. } => message.clone(),
        }
    }

    /// 是否建议重试
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::TooManyRequests { .. } => true,
            Self::ServiceUnavailable { .. } => true,
            Self::NetworkError { retry_suggested, .. } => *retry_suggested,
            Self::AuthenticationError { .. } => false,
            Self::QuotaExceeded { .. } => false,
            Self::ContentFiltered { .. } => false,
            Self::Other { is_retryable, .. } => *is_retryable,
        }
    }

    /// 是否为永久性错误（需要人工干预）
    pub fn is_permanent(&self) -> bool {
        matches!(
            self,
            Self::AuthenticationError { .. }
                | Self::QuotaExceeded { .. }
                | Self::ContentFiltered { .. }
        )
    }

    /// 从 HTTP 状态码和响应体创建错误
    pub fn from_http_status(status: u16, body: &str) -> Self {
        match status {
            401 | 403 => Self::AuthenticationError {
                message: body.to_string(),
            },
            429 => {
                // 尝试从响应中解析 retry-after
                let retry_after_ms = body.parse::<u64>().ok().map(|s| s * 1000);
                Self::TooManyRequests { retry_after_ms }
            }
            503 => Self::ServiceUnavailable {
                estimated_recovery_ms: None,
            },
            _ => Self::Other {
                message: format!("HTTP {}: {}", status, body),
                is_retryable: status >= 500,
            },
        }
    }

    /// 从网络错误创建
    pub fn from_network_error(error: &str) -> Self {
        let kind = if error.contains("timeout") || error.contains("Timeout") {
            NetworkErrorKind::ConnectionTimeout
        } else if error.contains("dns") || error.contains("DNS") {
            NetworkErrorKind::DnsError
        } else if error.contains("refused") || error.contains("Refused") {
            NetworkErrorKind::ConnectionRefused
        } else if error.contains("ssl") || error.contains("SSL") || error.contains("TLS") {
            NetworkErrorKind::SslError
        } else if error.contains("reset") || error.contains("Reset") {
            NetworkErrorKind::ConnectionReset
        } else {
            NetworkErrorKind::Unknown
        };

        Self::NetworkError {
            kind,
            message: error.to_string(),
            retry_suggested: !matches!(kind, NetworkErrorKind::SslError),
        }
    }
}

impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_user_message())
    }
}

impl std::error::Error for GatewayError {}

#[derive(Debug, Clone, PartialEq)]
pub enum QuotaExceededError {
    SessionBudgetExceeded,
}

#[async_trait]
pub trait ModelGateway: Send + Sync {
    async fn generate(&self, req: LlmRequest) -> Result<LlmResponse, GatewayError>;
    fn check_budget(&self, session_id: &str) -> Result<(), QuotaExceededError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use std::time::Instant;
    use crate::gateway::{GatewayManager, ModelProvider};

    // --- Mock Provider ---
    struct MockProvider {
        // Option to fail n times before succeeding
        fail_count: Mutex<u32>,
        error_to_return: GatewayError,
    }

    #[async_trait]
    impl ModelProvider for MockProvider {
        async fn generate(&self, req: &LlmRequest) -> Result<LlmResponse, GatewayError> {
            let mut fails = self.fail_count.lock().await;
            if *fails > 0 {
                *fails -= 1;
                return Err(self.error_to_return.clone());
            }
            Ok(LlmResponse {
                content: "Success".to_string(),
                tokens_used: req.budget_limit,
                tool_calls: vec![],
                finish_reason: Some("stop".to_string()),
            })
        }
    }

    fn dummy_req(budget: usize) -> LlmRequest {
        LlmRequest {
            session_id: "test_session".to_string(),
            messages: vec![],
            required_capabilities: Capability { requires_vision: false, strong_reasoning: false },
            budget_limit: budget,
            tools: None,
        }
    }





    #[tokio::test]
    async fn test_gateway_exponential_backoff_success() {
        let provider = Arc::new(MockProvider {
            fail_count: Mutex::new(2), // Fails twice, succeeds third time
            error_to_return: GatewayError::ServiceUnavailable { estimated_recovery_ms: None },
        });

        let gateway = GatewayManager::new(provider, 0, 3);

        let start = Instant::now();
        let res = gateway.generate(dummy_req(10)).await;
        let elapsed = start.elapsed();

        assert!(res.is_ok());
        assert_eq!(res.unwrap().content, "Success");

        // Backoff default is 100ms base.
        // Attempt 1 fails: waits ~200ms
        // Attempt 2 fails: waits ~400ms
        // Total wait ~600ms, definitely > 100ms
        assert!(elapsed.as_millis() >= 100);
    }

    #[tokio::test]
    async fn test_gateway_exponential_backoff_max_retries() {
        let provider = Arc::new(MockProvider {
            fail_count: Mutex::new(10), // Fails more than rate_limit_max_retries (3)
            error_to_return: GatewayError::TooManyRequests { retry_after_ms: None },
        });

        // Use with_concurrency to set a low rate_limit_max_retries for fast test execution
        let gateway = GatewayManager::with_concurrency(provider, 3, 3, 0);

        let res = gateway.generate(dummy_req(10)).await;

        // Should return the error after exhausting rate-limit retries (3)
        assert!(matches!(res.unwrap_err(), GatewayError::TooManyRequests { .. }));
    }

    #[tokio::test]
    async fn test_gateway_overhead() {
        let provider = Arc::new(MockProvider {
            fail_count: Mutex::new(0),
            error_to_return: GatewayError::Other { message: "".to_string(), is_retryable: false },
        });

        let gateway = GatewayManager::new(provider, 0, 3);

        let start = Instant::now();
        let res = gateway.generate(dummy_req(10)).await;
        let elapsed = start.elapsed();

        assert!(res.is_ok());
        // Middleware overhead should be under 2ms
        assert!(elapsed.as_millis() <= 2);
    }

    #[test]
    fn test_gateway_error_user_message() {
        let error = GatewayError::TooManyRequests { retry_after_ms: Some(5000) };
        assert!(error.to_user_message().contains("5 秒"));

        let error = GatewayError::AuthenticationError { message: "Invalid API key".to_string() };
        assert!(error.to_user_message().contains("认证失败"));

        let error = GatewayError::NetworkError {
            kind: NetworkErrorKind::ConnectionTimeout,
            message: "timeout".to_string(),
            retry_suggested: true,
        };
        assert!(error.to_user_message().contains("超时"));
    }

    #[test]
    fn test_gateway_error_retryable() {
        assert!(GatewayError::TooManyRequests { retry_after_ms: None }.is_retryable());
        assert!(GatewayError::ServiceUnavailable { estimated_recovery_ms: None }.is_retryable());
        assert!(!GatewayError::AuthenticationError { message: String::new() }.is_retryable());
        assert!(!GatewayError::QuotaExceeded { quota_type: "tokens".to_string() }.is_retryable());
    }
}
