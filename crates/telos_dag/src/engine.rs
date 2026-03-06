use crate::{ExecutionEngine, StorageError, TaskGraph};
use petgraph::Direction;
use std::collections::HashMap;
use telos_context::ScopedContext;
use telos_core::{NodeStatus, SystemRegistry};
use telos_hci::{AgentFeedback, EventBroker};
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use tracing::{info_span, Instrument};

pub struct TokioExecutionEngine {
    // Standard settings
}

impl TokioExecutionEngine {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait::async_trait]
impl ExecutionEngine for TokioExecutionEngine {
    async fn run_graph(
        &mut self,
        graph: &mut TaskGraph,
        ctx: &ScopedContext,
        registry: &dyn SystemRegistry,
        broker: &dyn EventBroker,
    ) {
        graph.current_state.is_running = true;

        let mut in_degrees: HashMap<String, usize> = HashMap::new();
        for (id, idx) in &graph.node_indices {
            let count = graph
                .edges
                .neighbors_directed(*idx, Direction::Incoming)
                .count();
            in_degrees.insert(id.clone(), count);
        }

        let mut ready_queue = Vec::new();
        for (id, count) in &in_degrees {
            if *count == 0 && graph.node_statuses.get(id) == Some(&NodeStatus::Pending) {
                ready_queue.push(id.clone());
            }
        }

        let mut active_tasks = 0;
        let mut completed_nodes = 0;
        let total_nodes = graph.nodes.len();

        let mut futures = FuturesUnordered::new();

        let graph_span = info_span!("run_graph", trace_id = %graph.graph_id);

        // We cannot hold a span enter guard across await points in async Rust.
        // Instead, we instrument the inner async block that we await.
        async {
            while completed_nodes < total_nodes {
                while let Some(node_id) = ready_queue.pop() {
                graph.node_statuses.insert(node_id.clone(), NodeStatus::Running);

                broker.publish_feedback(AgentFeedback::StateChanged {
                    task_id: graph.graph_id.clone(),
                    current_node: node_id.clone(),
                    status: NodeStatus::Running,
                });

                if let Some(node) = graph.nodes.remove(&node_id) {
                    let id_clone = node_id.clone();

                    let node_span = info_span!("execute_node", node_id = %id_clone);

                    futures.push(async move {
                        let res = node.execute(ctx, registry).instrument(node_span).await;
                        (id_clone, res, node)
                    });
                    active_tasks += 1;
                }
            }

            if active_tasks == 0 {
                // Deadlock or disconnected graph components
                break;
            }

            if let Some((node_id, result, node_box)) = futures.next().await {
                active_tasks -= 1;
                completed_nodes += 1;

                graph.nodes.insert(node_id.clone(), node_box);

                match &result {
                    Ok(res) => {
                        graph.node_statuses.insert(node_id.clone(), NodeStatus::Completed);
                        graph.node_results.insert(node_id.clone(), Ok(res.clone()));

                        broker.publish_feedback(AgentFeedback::StateChanged {
                            task_id: graph.graph_id.clone(),
                            current_node: node_id.clone(),
                            status: NodeStatus::Completed,
                        });

                        let node_idx = graph.node_indices.get(&node_id).unwrap();
                        let mut outgoing = graph.edges.neighbors_directed(*node_idx, Direction::Outgoing).detach();

                        while let Some(neighbor_idx) = outgoing.next_node(&graph.edges) {
                            if let Some(neighbor_id) = graph.edges.node_weight(neighbor_idx) {
                                if let Some(deg) = in_degrees.get_mut(neighbor_id) {
                                    *deg -= 1;
                                    if *deg == 0 && graph.node_statuses.get(neighbor_id) == Some(&NodeStatus::Pending) {
                                        ready_queue.push(neighbor_id.clone());
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        graph.node_statuses.insert(node_id.clone(), NodeStatus::Failed);
                        graph.node_results.insert(node_id.clone(), Err(e.clone()));

                        broker.publish_feedback(AgentFeedback::StateChanged {
                            task_id: graph.graph_id.clone(),
                            current_node: node_id.clone(),
                            status: NodeStatus::Failed,
                        });
                    }
                }
            }
        }

            graph.current_state.is_running = false;
            graph.current_state.completed = completed_nodes == total_nodes;
        }.instrument(graph_span).await;
    }

    fn checkpoint(&self, _graph_id: &str) -> Result<(), StorageError> {
        Ok(())
    }
}

impl Default for TokioExecutionEngine {
    fn default() -> Self {
        Self::new()
    }
}
