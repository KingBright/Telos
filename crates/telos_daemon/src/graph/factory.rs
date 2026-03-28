use std::sync::Arc;

// Telemetry Metrics

// Core Traits and Primitives
use telos_dag::ExecutableNode;
use telos_model_gateway::gateway::GatewayManager;

// 1. Adapter to convert Context OpenAiProvider to Gateway ModelProvider for the Gateway Manager

use crate::agents::*;
use crate::graph::nodes::*;
pub struct DaemonNodeFactory {
    pub gateway: Arc<GatewayManager>,
    pub tool_registry:
        std::sync::Arc<tokio::sync::RwLock<telos_tooling::retrieval::VectorToolRegistry>>,
    pub tools_dir: String,
}

impl telos_dag::engine::NodeFactory for DaemonNodeFactory {
    fn create_node(&self, agent_type: &str, _task: &str) -> Option<Box<dyn ExecutableNode>> {
        match agent_type {
            "architect" => Some(Box::new(ArchitectAgent::new(self.gateway.clone())) as Box<dyn ExecutableNode>),
            "coder" => Some(Box::new(CoderAgent::new(
                self.gateway.clone(),
                self.tool_registry.clone(),
                self.tools_dir.clone(),
            )) as Box<dyn ExecutableNode>),
            "reviewer" => Some(Box::new(ReviewAgent::new(self.gateway.clone())) as Box<dyn ExecutableNode>),
            "tester" => Some(Box::new(TestingAgent::new(self.gateway.clone())) as Box<dyn ExecutableNode>),
            "researcher" => Some(Box::new(DeepResearchAgent::new(
                self.gateway.clone(),
                self.tool_registry.clone(),
            )) as Box<dyn ExecutableNode>),
            "general" => Some(Box::new(GeneralAgent::new(
                self.gateway.clone(),
                self.tool_registry.clone(),
                self.tools_dir.clone(),
            )) as Box<dyn ExecutableNode>),
            "tool" => Some(Box::new(ToolNode {
                tool_name: _task.to_string(),
                tool_registry: self.tool_registry.clone(),
            }) as Box<dyn ExecutableNode>),
            "search_worker" => Some(Box::new(SearchWorkerAgent::new(
                self.gateway.clone(),
                self.tool_registry.clone(),
            )) as Box<dyn ExecutableNode>),
            "product" => Some(Box::new(crate::agents::bmad::product::ProductAgent {
                gateway: self.gateway.clone(),
            }) as Box<dyn ExecutableNode>),
            "bmad_architect" => Some(Box::new(crate::agents::bmad::architect::BmadArchitectAgent {
                gateway: self.gateway.clone(),
            }) as Box<dyn ExecutableNode>),
            "scrum_master" => Some(Box::new(crate::agents::bmad::scrum_master::ScrumMasterAgent {
                gateway: self.gateway.clone(),
            }) as Box<dyn ExecutableNode>),
            "worker" => Some(Box::new(crate::agents::bmad::worker::WorkerAgent {
                gateway: self.gateway.clone(),
            }) as Box<dyn ExecutableNode>),
            "human_approval" | "smart_approval" => Some(Box::new(crate::agents::bmad::approval::SmartApprovalNode) as Box<dyn ExecutableNode>),
            "harness_validator" => Some(Box::new(crate::agents::bmad::harness::HarnessValidatorNode {
                gateway: self.gateway.clone(),
            }) as Box<dyn ExecutableNode>),
            "integration_tester" => Some(Box::new(crate::agents::bmad::integration::IntegrationTesterNode) as Box<dyn ExecutableNode>),
            _ => None,
        }
    }
}

// 3. Real Executable Node that calls the LLM dynamically

// --- Dynamic DAG Deserialization structs ---



