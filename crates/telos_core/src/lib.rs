pub mod config;
// --- Core Primitives shared across Telos modules ---

#[derive(Debug, Clone, PartialEq)]
pub struct Knowledge {
    pub key_insights: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeResult {
    pub output_data: Vec<u8>,
    pub extracted_knowledge: Option<Knowledge>,
    pub next_routing_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NodeStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
#[derive(serde::Serialize, serde::Deserialize)]
pub enum RiskLevel {
    Normal,
    HighRisk,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NodeError {
    ExecutionFailed(String),
    Timeout,
    DependencyConflict,
}

pub trait SystemRegistry: Send + Sync {
    // Defines standard registry lookup mechanisms across the system

    /// Dynamically retrieves a ModelGateway instance without causing circular dependencies.
    fn get_model_gateway(&self) -> Option<std::sync::Arc<dyn std::any::Any + Send + Sync>> {
        None
    }
}
