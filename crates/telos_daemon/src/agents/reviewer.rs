use crate::agents::{
    async_trait, AgentInput, AgentOutput, Arc, ExecutableNode, GatewayManager, SystemRegistry,
};

pub struct ReviewAgent {
    pub gateway: Arc<GatewayManager>,
}

impl ReviewAgent {
    pub fn new(gateway: Arc<GatewayManager>) -> Self {
        Self { gateway }
    }
}

use telos_core::agent_traits::WorkerAgent;

#[async_trait]
impl WorkerAgent for ReviewAgent {
    fn worker_type(&self) -> &'static str {
        "reviewer"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content_to_review": { "type": "string" },
                "review_criteria": { "type": "string" }
            },
            "required": ["content_to_review", "review_criteria"]
        })
    }

    async fn execute_worker(&self, input: AgentInput, _registry: &dyn SystemRegistry) -> AgentOutput {
        // TODO: Implement Critique & Reflection Planning
        // Evaluates output/code against strict constraints and provides feedback
        AgentOutput::success(serde_json::json!({
            "text": format!("ReviewAgent execution stub for task [{}]", input.node_id)
        }))
    }
}

#[async_trait]
impl ExecutableNode for ReviewAgent {
    async fn execute(&self, input: AgentInput, registry: &dyn SystemRegistry) -> AgentOutput {
        self.execute_worker(input, registry).await
    }
}
