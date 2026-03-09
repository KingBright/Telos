use crate::{ExecutionEngine, StorageError, TaskGraph};
use petgraph::Direction;
use std::collections::HashMap;
use std::time::Instant;
use telos_context::ScopedContext;
use telos_core::{NodeStatus, SystemRegistry};
use telos_hci::{
    AgentFeedback, EventBroker, ErrorDetail, NodeExecutionDetail, ProgressInfo,
};
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

    /// Calculate progress info from the graph state
    fn calculate_progress(graph: &TaskGraph) -> ProgressInfo {
        let mut completed = 0;
        let mut running = 0;
        let mut failed = 0;
        let mut pending = 0;

        for status in graph.node_statuses.values() {
            match status {
                NodeStatus::Completed => completed += 1,
                NodeStatus::Running => running += 1,
                NodeStatus::Failed => failed += 1,
                NodeStatus::Pending => pending += 1,
            }
        }

        let total = graph.nodes.len();
        ProgressInfo::new(completed, total, running, failed, pending)
    }

    /// Truncate a string for display purposes
    fn truncate(s: &str, max_len: usize) -> String {
        if s.len() > max_len {
            format!("{}...", &s[..max_len])
        } else {
            s.to_string()
        }
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
        let mut failed_nodes = 0;
        let total_nodes = graph.nodes.len();

        let mut futures = FuturesUnordered::new();

        let graph_span = info_span!("run_graph", trace_id = %graph.graph_id);

        // We cannot hold a span enter guard across await points in async Rust.
        // Instead, we instrument the inner async block that we await.
        async {
            // Track progress update frequency
            let mut last_progress_update = Instant::now();
            let progress_update_interval = std::time::Duration::from_millis(500);

            while completed_nodes < total_nodes {
                while let Some(node_id) = ready_queue.pop() {
                    graph.node_statuses.insert(node_id.clone(), NodeStatus::Running);

                    // Get node metadata
                    let metadata = graph.node_metadata.get(&node_id).cloned().unwrap_or_default();
                    let task_type = metadata.task_type.clone();
                    let prompt_preview = metadata.prompt_preview.clone();

                    // Publish StateChanged feedback (for backward compatibility)
                    broker.publish_feedback(AgentFeedback::StateChanged {
                        task_id: graph.graph_id.clone(),
                        current_node: node_id.clone(),
                        status: NodeStatus::Running,
                    });

                    // Publish NodeStarted feedback (Verbose+)
                    let input_preview = Self::truncate(&prompt_preview, 200);
                    broker.publish_feedback(AgentFeedback::NodeStarted {
                        task_id: graph.graph_id.clone(),
                        node_id: node_id.clone(),
                        detail: NodeExecutionDetail {
                            node_id: node_id.clone(),
                            task_type: task_type.clone(),
                            input_preview: input_preview.clone(),
                            started_at: Some(
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis() as u64
                            ),
                        },
                    });

                    if let Some(node) = graph.nodes.remove(&node_id) {
                        let id_clone = node_id.clone();
                        let start_time = Instant::now();

                        let node_span = info_span!("execute_node", node_id = %id_clone);

                        futures.push(async move {
                            let res = node.execute(ctx, registry).instrument(node_span).await;
                            (id_clone, res, node, start_time)
                        });
                        active_tasks += 1;
                    }
                }

                if active_tasks == 0 {
                    // Deadlock or disconnected graph components
                    break;
                }

                if let Some((node_id, result, node_box, start_time)) = futures.next().await {
                    active_tasks -= 1;
                    completed_nodes += 1;

                    let execution_time_ms = start_time.elapsed().as_millis() as u64;

                    graph.nodes.insert(node_id.clone(), node_box);

                    match &result {
                        Ok(res) => {
                            graph.node_statuses.insert(node_id.clone(), NodeStatus::Completed);
                            graph.node_results.insert(node_id.clone(), Ok(res.clone()));

                            // Publish StateChanged feedback
                            broker.publish_feedback(AgentFeedback::StateChanged {
                                task_id: graph.graph_id.clone(),
                                current_node: node_id.clone(),
                                status: NodeStatus::Completed,
                            });

                            // Publish NodeCompleted feedback (Normal+)
                            let result_preview = Self::truncate(
                                &String::from_utf8_lossy(&res.output_data),
                                200,
                            );
                            broker.publish_feedback(AgentFeedback::NodeCompleted {
                                task_id: graph.graph_id.clone(),
                                node_id: node_id.clone(),
                                result_preview,
                                execution_time_ms,
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
                            failed_nodes += 1;

                            // Publish StateChanged feedback
                            broker.publish_feedback(AgentFeedback::StateChanged {
                                task_id: graph.graph_id.clone(),
                                current_node: node_id.clone(),
                                status: NodeStatus::Failed,
                            });

                            // Publish NodeFailed feedback (Normal+)
                            broker.publish_feedback(AgentFeedback::NodeFailed {
                                task_id: graph.graph_id.clone(),
                                node_id: node_id.clone(),
                                error: ErrorDetail::from_node_error(e),
                            });
                        }
                    }

                    // Publish ProgressUpdate periodically (Normal+)
                    if last_progress_update.elapsed() >= progress_update_interval {
                        let progress = Self::calculate_progress(graph);
                        broker.publish_feedback(AgentFeedback::ProgressUpdate {
                            task_id: graph.graph_id.clone(),
                            progress,
                        });
                        last_progress_update = Instant::now();
                    }
                }
            }

            // Final progress update
            let final_progress = Self::calculate_progress(graph);
            broker.publish_feedback(AgentFeedback::ProgressUpdate {
                task_id: graph.graph_id.clone(),
                progress: final_progress,
            });

            graph.current_state.is_running = false;
            graph.current_state.completed = completed_nodes == total_nodes;
        }
        .instrument(graph_span)
        .await;
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
