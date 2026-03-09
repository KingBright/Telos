use crate::agents::{
    async_trait, AgentInput, AgentOutput, Arc, ExecutableNode, GatewayManager, SystemRegistry,
};

pub struct TestingAgent {
    pub gateway: Arc<GatewayManager>,
}

impl TestingAgent {
    pub fn new(gateway: Arc<GatewayManager>) -> Self {
        Self { gateway }
    }
}

use telos_core::agent_traits::ExpertAgent;

#[async_trait]
impl ExpertAgent for TestingAgent {
    async fn plan(&self, input: &AgentInput, _registry: &dyn SystemRegistry) -> AgentOutput {
        // TODO: Implement Adversarial/Edge-Case Planning
        // Generate test scripts, run them, analyze failures to break the code.
        AgentOutput::success(serde_json::json!({
            "text": format!("TestingAgent planning stub for task [{}]", input.node_id)
        }))
    }

    async fn summarize(&self, _input: &AgentInput, _registry: &dyn SystemRegistry) -> AgentOutput {
        AgentOutput::success(serde_json::json!({
            "text": "TestingAgent completed the task successfully."
        }))
    }
}

#[async_trait]
impl ExecutableNode for TestingAgent {
    async fn execute(&self, input: AgentInput, registry: &dyn SystemRegistry) -> AgentOutput {
        self.plan(&input, registry).await
    }
}
