use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use async_trait::async_trait;
use tracing::warn;

use crate::{
    LlmRequest, LlmResponse, GatewayError, QuotaExceededError, ModelGateway,
    rate_limiter::SimpleRateLimiter,
    backoff::ExponentialBackoff,
};

#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn generate(&self, req: &LlmRequest) -> Result<LlmResponse, GatewayError>;
}

/// 熔断器状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// 正常状态，允许请求
    Closed,
    /// 熔断状态，拒绝请求
    Open,
    /// 半开状态，允许探测请求
    HalfOpen,
}

/// 熔断器配置
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// 触发熔断的失败次数阈值
    pub failure_threshold: u32,
    /// 熔断后等待恢复的时间（毫秒）
    pub recovery_timeout_ms: u64,
    /// 半开状态下允许的探测请求数
    pub half_open_max_requests: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            recovery_timeout_ms: 30000, // 30 秒
            half_open_max_requests: 1,
        }
    }
}

/// 熔断器
#[derive(Debug)]
pub struct CircuitBreaker {
    state: CircuitState,
    failure_count: u32,
    last_failure_time: Option<std::time::Instant>,
    half_open_requests: u32,
    config: CircuitBreakerConfig,
}

impl CircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            state: CircuitState::Closed,
            failure_count: 0,
            last_failure_time: None,
            half_open_requests: 0,
            config,
        }
    }

    /// 检查是否允许请求
    pub fn allow_request(&mut self) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // 检查是否超过恢复时间
                if let Some(last_failure) = self.last_failure_time {
                    let elapsed = last_failure.elapsed().as_millis() as u64;
                    if elapsed >= self.config.recovery_timeout_ms {
                        self.state = CircuitState::HalfOpen;
                        self.half_open_requests = 0;
                        return true;
                    }
                }
                false
            }
            CircuitState::HalfOpen => {
                if self.half_open_requests < self.config.half_open_max_requests {
                    self.half_open_requests += 1;
                    true
                } else {
                    false
                }
            }
        }
    }

    /// 记录成功
    pub fn record_success(&mut self) {
        match self.state {
            CircuitState::HalfOpen => {
                // 半开状态成功，恢复正常
                self.state = CircuitState::Closed;
                self.failure_count = 0;
                self.half_open_requests = 0;
            }
            CircuitState::Closed => {
                // 正常状态成功，重置失败计数
                self.failure_count = 0;
            }
            _ => {}
        }
    }

    /// 记录失败
    pub fn record_failure(&mut self) {
        self.last_failure_time = Some(std::time::Instant::now());

        match self.state {
            CircuitState::Closed => {
                self.failure_count += 1;
                if self.failure_count >= self.config.failure_threshold {
                    self.state = CircuitState::Open;
                }
            }
            CircuitState::HalfOpen => {
                // 半开状态失败，立即熔断
                self.state = CircuitState::Open;
                self.half_open_requests = 0;
            }
            _ => {}
        }
    }

    /// 获取当前状态
    pub fn state(&self) -> CircuitState {
        self.state
    }

    /// 获取恢复超时时间（毫秒）
    pub fn recovery_timeout_ms(&self) -> u64 {
        self.config.recovery_timeout_ms
    }

    /// 重置熔断器
    pub fn reset(&mut self) {
        self.state = CircuitState::Closed;
        self.failure_count = 0;
        self.last_failure_time = None;
        self.half_open_requests = 0;
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new(CircuitBreakerConfig::default())
    }
}

pub struct GatewayManager {
    provider: Arc<dyn ModelProvider>,
    backoff: ExponentialBackoff,
    rate_limiters: Mutex<HashMap<String, SimpleRateLimiter>>,
    max_concurrent_requests: usize, // 0 = unlimited
    circuit_breaker: Mutex<CircuitBreaker>,
}

impl GatewayManager {
    pub fn new(provider: Arc<dyn ModelProvider>, max_concurrent_requests: usize) -> Self {
        Self {
            provider,
            backoff: ExponentialBackoff::default(),
            rate_limiters: Mutex::new(HashMap::new()),
            max_concurrent_requests,
            circuit_breaker: Mutex::new(CircuitBreaker::default()),
        }
    }

    /// 创建带自定义熔断器配置的 GatewayManager
    pub fn with_circuit_breaker(
        provider: Arc<dyn ModelProvider>,
        max_concurrent_requests: usize,
        config: CircuitBreakerConfig,
    ) -> Self {
        Self {
            provider,
            backoff: ExponentialBackoff::default(),
            rate_limiters: Mutex::new(HashMap::new()),
            max_concurrent_requests,
            circuit_breaker: Mutex::new(CircuitBreaker::new(config)),
        }
    }

    /// 获取熔断器状态
    pub async fn circuit_state(&self) -> CircuitState {
        self.circuit_breaker.lock().await.state()
    }

    /// 重置熔断器
    pub async fn reset_circuit(&self) {
        self.circuit_breaker.lock().await.reset();
    }
}

#[async_trait]
impl ModelGateway for GatewayManager {
    async fn generate(&self, req: LlmRequest) -> Result<LlmResponse, GatewayError> {
        // 1. 检查熔断器
        {
            let mut cb = self.circuit_breaker.lock().await;
            if !cb.allow_request() {
                return Err(GatewayError::ServiceUnavailable {
                    estimated_recovery_ms: Some(cb.recovery_timeout_ms()),
                });
            }
        }

        // 2. 强制速率限制 (0 = 无限制)
        if self.max_concurrent_requests > 0 {
            let mut limiters = self.rate_limiters.lock().await;
            let limiter = limiters.entry(req.session_id.clone()).or_insert_with(|| {
                SimpleRateLimiter::new(self.max_concurrent_requests)
            });

            if !limiter.try_consume() {
                return Err(GatewayError::TooManyRequests { retry_after_ms: None });
            }
        }

        let mut retries = 0;
        loop {
            match self.provider.generate(&req).await {
                Ok(response) => {
                    // 成功时重置熔断器
                    self.circuit_breaker.lock().await.record_success();
                    return Ok(response);
                }
                Err(e) => {
                    // 永久性错误直接返回，不重试
                    if e.is_permanent() {
                        return Err(e);
                    }

                    // 检查是否可重试
                    if !e.is_retryable() {
                        return Err(e);
                    }

                    if retries >= self.backoff.get_max_retries() {
                        // 达到最大重试次数，记录失败到熔断器
                        self.circuit_breaker.lock().await.record_failure();
                        return Err(e);
                    }

                    retries += 1;
                    warn!(
                        "[Gateway] Request failed (attempt {}/{}), retrying: {}",
                        retries,
                        self.backoff.get_max_retries(),
                        e.to_user_message()
                    );
                    self.backoff.wait(retries).await;
                }
            }
        }
    }

    fn check_budget(&self, _session_id: &str) -> Result<(), QuotaExceededError> {
        Ok(())
    }
}
