use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::HashMap;
use telos_context::ScopedContext;
use telos_core::{AgentInput, AgentOutput, DependencyType, NodeStatus, SystemRegistry};
use telos_hci::EventBroker;
use tracing::warn;

pub mod checkpoint;
pub mod engine;
#[cfg(test)]
mod tests;

/// Metadata for a node in the execution graph
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct NodeMetadata {
    pub task_type: String,         // "LLM" or "TOOL"
    pub prompt_preview: String,    // Truncated prompt for display
    pub full_task: String,         // Full un-truncated payload for execution
    pub tool_name: Option<String>, // For TOOL type nodes
    pub schema_payload: Option<String>,
}

#[async_trait::async_trait]
pub trait ExecutableNode: Send + Sync {
    /// Execute this node with structured input/output
    async fn execute(&self, input: AgentInput, registry: &dyn SystemRegistry) -> AgentOutput;
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GraphState {
    pub is_running: bool,
    pub completed: bool,
}

fn default_schema_version() -> u32 {
    1 // Current schema version
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct TaskGraph {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub graph_id: String,
    // Original mapping of node ID to ExecutableNode logic
    #[serde(skip)]
    pub nodes: HashMap<String, Box<dyn ExecutableNode>>,
    // Directed graph where weights are the string node IDs
    pub edges: DiGraph<String, ()>,
    // Mapping of node string ID to graph NodeIndex
    pub node_indices: HashMap<String, NodeIndex>,
    // Status tracking for each node
    pub node_statuses: HashMap<String, NodeStatus>,
    // Results from execution (new: structured output)
    pub node_results: HashMap<String, AgentOutput>,
    // Edge types: "from|to" -> dependency type
    pub edge_types: HashMap<String, DependencyType>,
    // Metadata for each node (task_type, prompt_preview, etc.)
    pub node_metadata: HashMap<String, NodeMetadata>,
    pub current_state: GraphState,
    pub conversation_history: Vec<telos_core::ConversationMessage>,
}

impl TaskGraph {
    pub fn new(graph_id: String) -> Self {
        Self {
            schema_version: default_schema_version(),
            graph_id,
            nodes: HashMap::new(),
            edges: DiGraph::new(),
            node_indices: HashMap::new(),
            node_statuses: HashMap::new(),
            node_results: HashMap::new(),
            edge_types: HashMap::new(),
            node_metadata: HashMap::new(),
            current_state: GraphState {
                is_running: false,
                completed: false,
            },
            conversation_history: Vec::new(),
        }
    }

    pub fn add_node(&mut self, id: String, node: Box<dyn ExecutableNode>) {
        let index = self.edges.add_node(id.clone());
        self.node_indices.insert(id.clone(), index);
        self.nodes.insert(id.clone(), node);
        self.node_statuses.insert(id.clone(), NodeStatus::Pending);
        self.node_metadata.insert(id, NodeMetadata::default());
    }

    /// Add a node with explicit metadata
    pub fn add_node_with_metadata(
        &mut self,
        id: String,
        node: Box<dyn ExecutableNode>,
        metadata: NodeMetadata,
    ) {
        let index = self.edges.add_node(id.clone());
        self.node_indices.insert(id.clone(), index);
        self.nodes.insert(id.clone(), node);
        self.node_statuses.insert(id.clone(), NodeStatus::Pending);
        self.node_metadata.insert(id, metadata);
    }

    /// Set metadata for an existing node
    pub fn set_node_metadata(&mut self, id: &str, metadata: NodeMetadata) {
        if self.node_indices.contains_key(id) {
            self.node_metadata.insert(id.to_string(), metadata);
        }
    }

    /// Add an edge with dependency type
    pub fn add_edge(&mut self, from: &str, to: &str) -> Result<(), String> {
        self.add_edge_with_type(from, to, DependencyType::Data)
    }

    /// Add an edge with explicit dependency type
    pub fn add_edge_with_type(
        &mut self,
        from: &str,
        to: &str,
        dep_type: DependencyType,
    ) -> Result<(), String> {
        if from == to {
            // Self-edges are invalid in a Directed Acyclic Graph and cause deadlocks. Ignore them.
            warn!(
                "[TaskGraph] Attempted to add self-edge from {} to {}. Ignoring.",
                from, to
            );
            return Ok(());
        }

        let from_idx = self
            .node_indices
            .get(from)
            .copied()
            .ok_or("From node not found")?;
        let to_idx = self
            .node_indices
            .get(to)
            .copied()
            .ok_or("To node not found")?;
        self.edges.add_edge(from_idx, to_idx, ());
        self.edge_types
            .insert(format!("{}|{}", from, to), dep_type);
        Ok(())
    }

    /// Get dependencies for a node (incoming edges)
    pub fn get_dependencies(&self, node_id: &str) -> Vec<(String, DependencyType)> {
        let mut deps = Vec::new();
        if let Some(&node_idx) = self.node_indices.get(node_id) {
            use petgraph::Direction;
            let mut incoming = self
                .edges
                .neighbors_directed(node_idx, Direction::Incoming)
                .detach();
            while let Some(neighbor_idx) = incoming.next_node(&self.edges) {
                if let Some(from_id) = self.edges.node_weight(neighbor_idx) {
                    let dep_type = self
                        .edge_types
                        .get(&format!("{}|{}", from_id, node_id))
                        .copied()
                        .unwrap_or(DependencyType::Data);
                    deps.push((from_id.clone(), dep_type));
                }
            }
        }
        deps
    }

    /// Converts the current TaskGraph and its metadata into a serializable AgentSubGraph
    /// which can be stored as a Procedural Workflow Template.
    pub fn to_subgraph(&self) -> telos_core::AgentSubGraph {
        let mut nodes = Vec::new();
        for (id, meta) in &self.node_metadata {
            nodes.push(telos_core::SubGraphNode {
                id: id.clone(),
                agent_type: meta.task_type.clone(),
                task: meta.full_task.clone(),
                schema_payload: meta.schema_payload.clone().unwrap_or_default(),
                loop_config: None,
                is_critic: false,
            });
        }
        let mut edges = Vec::new();
        for (key_str, dep_type) in &self.edge_types {
            if let Some((from, to)) = key_str.split_once('|') {
                edges.push(telos_core::SubGraphEdge {
                    from: from.to_string(),
                    to: to.to_string(),
                    dep_type: *dep_type,
                });
            }
        }
        telos_core::AgentSubGraph { nodes, edges }
    }

    /// Rebuilds the runtime node logic (`Box<dyn ExecutableNode>`) from metadata.
    /// Used when restoring a checkpoint from disk.
    pub fn rebuild_nodes(&mut self, factory: &dyn engine::NodeFactory) -> Result<(), String> {
        for (id, meta) in &self.node_metadata {
            if let Some(executable) = factory.create_node(&meta.task_type, &meta.full_task) {
                self.nodes.insert(id.clone(), executable);
            } else {
                return Err(format!("Failed to rebuild node logic for node: {} (type: {})", id, meta.task_type));
            }
        }
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
