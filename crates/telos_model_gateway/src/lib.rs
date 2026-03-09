use async_trait::async_trait;

pub mod rate_limiter;
pub mod backoff;
pub mod gateway;

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Capability {
    pub requires_vision: bool,
    pub strong_reasoning: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlmRequest {
    pub session_id: String,
    pub messages: Vec<Message>,
    pub required_capabilities: Capability,
    pub budget_limit: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlmResponse {
    pub content: String,
    pub tokens_used: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GatewayError {
    TooManyRequests, // 429
    ServiceUnavailable, // 503
    NetworkError(String), // Connection failures, timeouts, DNS errors
    Other(String),
}

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
    use crate::rate_limiter::LeakyBucket;

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
            })
        }
    }

    fn dummy_req(budget: usize) -> LlmRequest {
        LlmRequest {
            session_id: "test_session".to_string(),
            messages: vec![],
            required_capabilities: Capability { requires_vision: false, strong_reasoning: false },
            budget_limit: budget,
        }
    }

    #[test]
    fn test_leaky_bucket_rate_limiting() {
        // capacity of 100, leaks 10 tokens per second
        let mut bucket = LeakyBucket::new(100, 10.0);

        // consume all
        assert!(bucket.try_consume(100));

        // try consume 1 more, should fail immediately since leak is very slow (10/s)
        assert!(!bucket.try_consume(1));
    }

    #[tokio::test]
    async fn test_gateway_rate_limit_rejection() {
        let provider = Arc::new(MockProvider {
            fail_count: Mutex::new(0),
            error_to_return: GatewayError::TooManyRequests,
        });

        // 100 capacity, very slow leak
        let gateway = GatewayManager::new(provider, 100, 1.0);

        // First request works
        let res1 = gateway.generate(dummy_req(100)).await;
        assert!(res1.is_ok());

        // Second request gets rate limited immediately without hitting provider
        let res2 = gateway.generate(dummy_req(1)).await;
        assert_eq!(res2.unwrap_err(), GatewayError::TooManyRequests);
    }

    #[tokio::test]
    async fn test_gateway_exponential_backoff_success() {
        let provider = Arc::new(MockProvider {
            fail_count: Mutex::new(2), // Fails twice, succeeds third time
            error_to_return: GatewayError::ServiceUnavailable,
        });

        let gateway = GatewayManager::new(provider, 1000, 1000.0);

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
            fail_count: Mutex::new(10), // Fails more than max retries (5)
            error_to_return: GatewayError::TooManyRequests,
        });

        let gateway = GatewayManager::new(provider, 1000, 1000.0);

        let res = gateway.generate(dummy_req(10)).await;

        // Should return the error after exhausting retries
        assert_eq!(res.unwrap_err(), GatewayError::TooManyRequests);
    }

    #[tokio::test]
    async fn test_gateway_overhead() {
        let provider = Arc::new(MockProvider {
            fail_count: Mutex::new(0),
            error_to_return: GatewayError::Other("".to_string()),
        });

        let gateway = GatewayManager::new(provider, 1000, 1000.0);

        let start = Instant::now();
        let res = gateway.generate(dummy_req(10)).await;
        let elapsed = start.elapsed();

        assert!(res.is_ok());
        // Middleware overhead should be under 2ms
        assert!(elapsed.as_millis() <= 2);
    }
}
