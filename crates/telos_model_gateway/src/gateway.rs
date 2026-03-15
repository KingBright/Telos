use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore, OwnedSemaphorePermit};
use async_trait::async_trait;
use tracing::{warn, info};

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
                self.state = CircuitState::Closed;
                self.failure_count = 0;
                self.half_open_requests = 0;
            }
            CircuitState::Closed => {
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
    /// Global concurrency semaphore: serializes all LLM requests to prevent
    /// concurrent 429 storms from overwhelming the API provider.
    global_concurrency: Arc<Semaphore>,
    /// Maximum number of retries specifically for 429 rate-limit errors.
    /// After this many retries, the error is returned instead of retrying forever.
    rate_limit_max_retries: u32,
}

impl GatewayManager {
    pub fn new(provider: Arc<dyn ModelProvider>, max_concurrent_requests: usize) -> Self {
        Self {
            provider,
            backoff: ExponentialBackoff::default(),
            rate_limiters: Mutex::new(HashMap::new()),
            max_concurrent_requests,
            circuit_breaker: Mutex::new(CircuitBreaker::default()),
            global_concurrency: Arc::new(Semaphore::new(3)),
            rate_limit_max_retries: 10,
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
            global_concurrency: Arc::new(Semaphore::new(3)),
            rate_limit_max_retries: 10,
        }
    }

    /// Create with custom concurrency limits
    pub fn with_concurrency(
        provider: Arc<dyn ModelProvider>,
        max_concurrent_requests: usize,
        global_permits: usize,
        rate_limit_max_retries: u32,
    ) -> Self {
        Self {
            provider,
            backoff: ExponentialBackoff::default(),
            rate_limiters: Mutex::new(HashMap::new()),
            max_concurrent_requests,
            circuit_breaker: Mutex::new(CircuitBreaker::default()),
            global_concurrency: Arc::new(Semaphore::new(global_permits)),
            rate_limit_max_retries,
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

    /// Internal: execute an LLM request while holding a semaphore permit.
    /// `initial_retries` allows continuing the retry count across permit re-acquisitions.
    async fn generate_with_permit(
        &self,
        req: &LlmRequest,
        permit: OwnedSemaphorePermit,
        initial_retries: u32,
    ) -> Result<LlmResponse, GatewayError> {
        let mut retries = initial_retries;
        let mut _permit = permit;

        loop {
            match self.provider.generate(req).await {
                Ok(response) => {
                    self.circuit_breaker.lock().await.record_success();
                    return Ok(response);
                }
                Err(e) => {
                    if e.is_permanent() {
                        return Err(e);
                    }
                    if !e.is_retryable() {
                        return Err(e);
                    }

                    let is_rate_limit = matches!(e, GatewayError::TooManyRequests { .. });

                    if is_rate_limit {
                        if retries >= self.rate_limit_max_retries {
                            warn!(
                                "[Gateway] Rate limit retries exhausted ({}/{}). Returning error to allow system recovery.",
                                retries, self.rate_limit_max_retries
                            );
                            self.circuit_breaker.lock().await.record_failure();
                            return Err(e);
                        }

                        retries += 1;
                        info!(
                            "[Gateway] Request Rate Limited (attempt {}/{}), releasing permit and backing off...",
                            retries, self.rate_limit_max_retries
                        );

                        // CRITICAL: Release semaphore permit BEFORE sleeping.
                        // This lets other queued tasks proceed while we wait,
                        // avoiding a deadlock where all permits are held by sleeping tasks.
                        drop(_permit);
                        self.backoff.wait(retries).await;

                        // Re-acquire permit after backoff
                        _permit = self.global_concurrency.clone().acquire_owned().await
                            .map_err(|_| GatewayError::Other {
                                message: "Global concurrency semaphore closed during retry".to_string(),
                                is_retryable: false,
                            })?;
                        // Continue the loop with the new permit
                    } else {
                        // Non-rate-limit retryable error: standard max_retries
                        if retries >= self.backoff.get_max_retries() {
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
    }
}

#[async_trait]
impl ModelGateway for GatewayManager {
    async fn generate(&self, req: LlmRequest) -> Result<LlmResponse, GatewayError> {
        // 1. 检查熔断器，阻塞等待而不是直接拒绝
        loop {
            let allow = {
                let mut cb = self.circuit_breaker.lock().await;
                cb.allow_request()
            };

            if allow {
                break;
            }

            warn!("[Gateway] Circuit Breaker is active. Waiting 5s before re-evaluating...");
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
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

        // 3. Acquire global concurrency semaphore permit.
        //    This serializes all LLM requests system-wide, preventing concurrent
        //    429 storms from overwhelming the API provider.
        let permit = self.global_concurrency.clone().acquire_owned().await
            .map_err(|_| GatewayError::Other {
                message: "Global concurrency semaphore closed".to_string(),
                is_retryable: false,
            })?;

        // 4. Execute with the held permit
        self.generate_with_permit(&req, permit, 0).await
    }

    fn check_budget(&self, _session_id: &str) -> Result<(), QuotaExceededError> {
        Ok(())
    }
}
