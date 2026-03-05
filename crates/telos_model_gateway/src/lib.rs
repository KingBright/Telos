use async_trait::async_trait;

pub struct Message {
    pub role: String,
    pub content: String,
}

pub struct Capability {
    pub requires_vision: bool,
    pub strong_reasoning: bool,
}

pub struct LlmRequest {
    pub messages: Vec<Message>,
    pub required_capabilities: Capability,
    pub budget_limit: usize,
}

pub struct LlmResponse {
    pub content: String,
    pub tokens_used: usize,
}

pub enum GatewayError {
    RateLimitExceeded,
    ServiceUnavailable,
}

pub enum QuotaExceededError {
    SessionBudgetExceeded,
}

#[async_trait]
pub trait ModelGateway: Send + Sync {
    async fn generate(&self, req: LlmRequest) -> Result<LlmResponse, GatewayError>;
    fn check_budget(&self, session_id: &str) -> Result<(), QuotaExceededError>;
}
