
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore, OwnedSemaphorePermit};
use async_trait::async_trait;
use tracing::{warn, info, error};
use std::time::Duration;

use crate::{
    LlmRequest, LlmResponse, GatewayError, QuotaExceededError, ModelGateway,

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
    circuit_breaker: Mutex<CircuitBreaker>,
    /// Global concurrency semaphore: serializes all LLM requests to prevent
    /// concurrent 429 storms from overwhelming the API provider.
    global_concurrency: Arc<Semaphore>,
    /// Maximum number of retries specifically for 429 rate-limit errors.
    /// After this many retries, the error is returned instead of retrying forever.
    rate_limit_max_retries: u32,
    /// Proactive throttle in milliseconds to inject before every active request.
    throttle_ms: u64,
}

impl GatewayManager {
    pub fn new(
        provider: Arc<dyn ModelProvider>,
        throttle_ms: u64,
        global_permits: usize,
    ) -> Self {
        Self {
            provider,
            backoff: ExponentialBackoff::default(),
            circuit_breaker: Mutex::new(CircuitBreaker::default()),
            global_concurrency: Arc::new(Semaphore::new(global_permits)),
            rate_limit_max_retries: 10,
            throttle_ms,
        }
    }

    pub fn with_circuit_breaker(
        provider: Arc<dyn ModelProvider>,
        config: CircuitBreakerConfig,
        throttle_ms: u64,
        global_permits: usize,
    ) -> Self {
        Self {
            provider,
            backoff: ExponentialBackoff::default(),
            circuit_breaker: Mutex::new(CircuitBreaker::new(config)),
            global_concurrency: Arc::new(Semaphore::new(global_permits)),
            rate_limit_max_retries: 10,
            throttle_ms,
        }
    }

    /// Create with custom concurrency limits
    pub fn with_concurrency(
        provider: Arc<dyn ModelProvider>,
        global_permits: usize,
        rate_limit_max_retries: u32,
        throttle_ms: u64,
    ) -> Self {
        Self {
            provider,
            backoff: ExponentialBackoff::default(),
            circuit_breaker: Mutex::new(CircuitBreaker::default()),
            global_concurrency: Arc::new(Semaphore::new(global_permits)),
            rate_limit_max_retries,
            throttle_ms,
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

        // Per-call timeout: prevents indefinite hangs when the API accepts
        // the TCP connection but never sends a response.
        const PER_CALL_TIMEOUT: Duration = Duration::from_secs(120);

        loop {
            let call_result = match tokio::time::timeout(
                PER_CALL_TIMEOUT,
                self.provider.generate(req),
            ).await {
                Ok(result) => result,
                Err(_elapsed) => {
                    warn!("[Gateway] LLM API call timed out after {}s", PER_CALL_TIMEOUT.as_secs());
                    Err(GatewayError::NetworkError {
                        message: format!("LLM API call timed out after {}s", PER_CALL_TIMEOUT.as_secs()),
                        kind: crate::NetworkErrorKind::ConnectionTimeout,
                        retry_suggested: true,
                    })
                }
            };

            match call_result {
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
                            "[Gateway] Request Rate Limited (attempt {}/{}), holding permit and sleeping...",
                            retries, self.rate_limit_max_retries
                        );

                        // CRITICAL FIX: DO NOT release the permit here.
                        // Since 429 means the global API channel is saturated, holding this permit
                        // acts as a natural system-wide pause, preventing a "thundering herd" of
                        // queued agents from waking up just to hit immediate 429s and uselessly escalating
                        // their retry timers.
                        self.backoff.wait(retries).await;
                        
                        // Continue the loop while still holding the original permit
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
        } // end loop
    }
}

#[async_trait]
impl ModelGateway for GatewayManager {
    async fn generate(&self, req: LlmRequest) -> Result<LlmResponse, GatewayError> {
        let session_tag = &req.session_id;
        
        // 1. 检查熔断器，阻塞等待而不是直接拒绝
        let mut cb_wait_count = 0u32;
        loop {
            let allow = {
                let mut cb = self.circuit_breaker.lock().await;
                cb.allow_request()
            };

            if allow {
                break;
            }

            cb_wait_count += 1;
            if cb_wait_count > 12 { // Max 60s waiting for circuit breaker
                error!("[Gateway] Circuit Breaker stuck open for 60s, failing request for session={}", session_tag);
                return Err(GatewayError::Other {
                    message: "Circuit breaker stuck open for 60s".to_string(),
                    is_retryable: true,
                });
            }
            warn!("[Gateway] Circuit Breaker is active (attempt {}). Waiting 5s for session={}...", cb_wait_count, session_tag);
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }

        // 2. Acquire global concurrency semaphore permit with timeout.
        let available = self.global_concurrency.available_permits();
        info!("[Gateway] session={} acquiring semaphore permit (available: {}/{})", 
            session_tag, available, 7);
        
        let permit = match tokio::time::timeout(
            std::time::Duration::from_secs(60),
            self.global_concurrency.clone().acquire_owned()
        ).await {
            Ok(Ok(permit)) => {
                info!("[Gateway] session={} acquired permit", session_tag);
                permit
            }
            Ok(Err(_)) => {
                error!("[Gateway] session={} semaphore closed", session_tag);
                return Err(GatewayError::Other {
                    message: "Global concurrency semaphore closed".to_string(),
                    is_retryable: false,
                });
            }
            Err(_) => {
                error!("[Gateway] session={} TIMEOUT waiting 60s for semaphore permit (available was: {})", session_tag, available);
                return Err(GatewayError::NetworkError {
                    message: format!("Semaphore acquisition timed out after 60s (available permits was: {})", available),
                    kind: crate::NetworkErrorKind::ConnectionTimeout,
                    retry_suggested: true,
                });
            }
        };

        // 3. [THROTTLING] Proactively pace requests to avoid Zhipu API strict RPS limits.
        if self.throttle_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.throttle_ms)).await;
        }

        // 4. Execute with the held permit
        self.generate_with_permit(&req, permit, 0).await
    }

    fn check_budget(&self, _session_id: &str) -> Result<(), QuotaExceededError> {
        Ok(())
    }
}
