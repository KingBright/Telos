use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use async_trait::async_trait;

use crate::{
    LlmRequest, LlmResponse, GatewayError, QuotaExceededError, ModelGateway,
    rate_limiter::LeakyBucket, backoff::ExponentialBackoff,
};

#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn generate(&self, req: &LlmRequest) -> Result<LlmResponse, GatewayError>;
}

pub struct GatewayManager {
    provider: Arc<dyn ModelProvider>,
    backoff: ExponentialBackoff,
    rate_limiters: Mutex<HashMap<String, LeakyBucket>>,
    leak_rate: f64,
    capacity: usize,
}

impl GatewayManager {
    pub fn new(provider: Arc<dyn ModelProvider>, capacity: usize, leak_rate: f64) -> Self {
        Self {
            provider,
            backoff: ExponentialBackoff::default(),
            rate_limiters: Mutex::new(HashMap::new()),
            leak_rate,
            capacity,
        }
    }
}

#[async_trait]
impl ModelGateway for GatewayManager {
    async fn generate(&self, req: LlmRequest) -> Result<LlmResponse, GatewayError> {
        // Enforce rate limiting
        {
            let mut limiters = self.rate_limiters.lock().await;
            let limiter = limiters.entry(req.session_id.clone()).or_insert_with(|| {
                LeakyBucket::new(self.capacity, self.leak_rate)
            });

            // For now, we estimate the cost before the request.
            // A more complex system might block based on estimated cost and then adjust.
            // We'll use the budget_limit as a proxy for the token request size.
            if !limiter.try_consume(req.budget_limit) {
                return Err(GatewayError::TooManyRequests);
            }
        }

        let mut retries = 0;
        loop {
            match self.provider.generate(&req).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    match e {
                        GatewayError::TooManyRequests | GatewayError::ServiceUnavailable => {
                            if retries >= self.backoff.get_max_retries() {
                                return Err(e);
                            }
                            retries += 1;
                            self.backoff.wait(retries).await;
                        }
                        _ => return Err(e),
                    }
                }
            }
        }
    }

    fn check_budget(&self, _session_id: &str) -> Result<(), QuotaExceededError> {
        // This could query a persistent store for total session budget.
        // For the current implementation, we assume budget is checked/maintained elsewhere
        // or just returns Ok() as a placeholder.
        Ok(())
    }
}
