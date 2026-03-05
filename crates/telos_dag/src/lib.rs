use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::HashMap;
use telos_context::ScopedContext;
use telos_core::{NodeError, NodeResult, NodeStatus, SystemRegistry};
use telos_hci::EventBroker;

pub mod checkpoint;
pub mod engine;
#[cfg(test)]
mod tests;

#[async_trait::async_trait]
pub trait ExecutableNode: Send + Sync {
    async fn execute(
        &self,
        ctx: &ScopedContext,
        registry: &dyn SystemRegistry,
    ) -> Result<NodeResult, NodeError>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphState {
    pub is_running: bool,
    pub completed: bool,
}

pub struct TaskGraph {
    pub graph_id: String,
    // Original mapping of node ID to ExecutableNode logic
    pub nodes: HashMap<String, Box<dyn ExecutableNode>>,
    // Directed graph where weights are the string node IDs
    pub edges: DiGraph<String, ()>,
    // Mapping of node string ID to graph NodeIndex
    pub node_indices: HashMap<String, NodeIndex>,
    // Status tracking for each node
    pub node_statuses: HashMap<String, NodeStatus>,
    // Results from execution
    pub node_results: HashMap<String, Result<NodeResult, NodeError>>,
    pub current_state: GraphState,
}

impl TaskGraph {
    pub fn new(graph_id: String) -> Self {
        Self {
            graph_id,
            nodes: HashMap::new(),
            edges: DiGraph::new(),
            node_indices: HashMap::new(),
            node_statuses: HashMap::new(),
            node_results: HashMap::new(),
            current_state: GraphState {
                is_running: false,
                completed: false,
            },
        }
    }

    pub fn add_node(&mut self, id: String, node: Box<dyn ExecutableNode>) {
        let index = self.edges.add_node(id.clone());
        self.node_indices.insert(id.clone(), index);
        self.nodes.insert(id.clone(), node);
        self.node_statuses.insert(id, NodeStatus::Pending);
    }

    pub fn add_edge(&mut self, from: &str, to: &str) -> Result<(), String> {
        let from_idx = self.node_indices.get(from).copied().ok_or("From node not found")?;
        let to_idx = self.node_indices.get(to).copied().ok_or("To node not found")?;
        self.edges.add_edge(from_idx, to_idx, ());
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
pub enum StorageError {
    DiskFull,
    IoError,
    SerializationError,
    DeserializationError,
}

#[async_trait::async_trait]
pub trait ExecutionEngine {
    async fn run_graph(
        &mut self,
        graph: &mut TaskGraph,
        ctx: &ScopedContext,
        registry: &dyn SystemRegistry,
        broker: &dyn EventBroker,
    );
    fn checkpoint(&self, graph_id: &str) -> Result<(), StorageError>;
}
