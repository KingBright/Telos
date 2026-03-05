use std::collections::HashMap;
use telos_core::{NodeResult, NodeError, SystemRegistry};
use telos_hci::EventBroker;
use telos_context::ScopedContext;
use petgraph::graph::DiGraph;

#[async_trait::async_trait]
pub trait ExecutableNode: Send + Sync {
    async fn execute(
        &self,
        ctx: &ScopedContext,
        registry: &dyn SystemRegistry,
    ) -> Result<NodeResult, NodeError>;
}

pub struct GraphState {
    pub is_running: bool,
}

pub struct TaskGraph {
    pub graph_id: String,
    pub nodes: HashMap<String, Box<dyn ExecutableNode>>,
    pub edges: DiGraph<String, ()>,
    pub current_state: GraphState,
}

pub enum StorageError {
    DiskFull,
    IoError,
}

#[async_trait::async_trait]
pub trait ExecutionEngine {
    async fn run_graph(&mut self, graph: TaskGraph, broker: &dyn EventBroker);
    fn checkpoint(&self, graph_id: &str) -> Result<(), StorageError>;
}
