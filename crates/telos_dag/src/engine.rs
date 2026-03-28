use crate::{ExecutableNode, ExecutionEngine, StorageError, TaskGraph};
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use petgraph::Direction;
use std::collections::HashMap;
use std::time::Instant;
use telos_context::ScopedContext;
use telos_core::{AgentInput, AgentOutput, CorrectionDirective, DependencyType, ErrorSeverity, ExitCondition, LoopConfig, NodeStatus, SystemRegistry};
use telos_hci::{AgentFeedback, ErrorDetail, EventBroker, ProgressInfo, TaskSummary};
use tracing::{info_span, Instrument, info, warn, error};

// ============================================================================
// Corrective Loop Runtime State (Actor-Critic pattern)
// ============================================================================

/// Runtime state for a corrective loop pair (Actor + Critic)
struct LoopState {
    config: LoopConfig,
    iteration: usize,
    actor_node_id: String,
    critic_node_id: String,
    /// Historical scores for stagnation detection
    score_history: Vec<f32>,
    /// Best output seen so far (score, output)
    best_output: Option<(f32, AgentOutput)>,
    /// Pending correction to inject into next Actor iteration
    pending_correction: Option<CorrectionDirective>,
}

impl LoopState {
    fn new(config: LoopConfig, actor_node_id: String, critic_node_id: String) -> Self {
        Self {
            config,
            iteration: 0,
            actor_node_id,
            critic_node_id,
            score_history: Vec::new(),
            best_output: None,
            pending_correction: None,
        }
    }

    /// Record a score and update best_output if this is the best so far
    fn record_score(&mut self, score: f32, output: &AgentOutput) {
        self.score_history.push(score);
        if self.best_output.as_ref().map_or(true, |(best_score, _)| score > *best_score) {
            self.best_output = Some((score, output.clone()));
        }
    }

    /// Stagnation detection: last 2 scores vary by less than 0.05 from current
    fn is_stagnated(&self, current_score: f32) -> bool {
        if self.score_history.len() < 2 {
            return false;
        }
        // Never stagnate if the score is exactly 0.0 (indicates consistent compiler/validator failure).
        // Let the loop exhaust its max_iterations so escalation mechanisms like TechLead can intercept.
        if current_score <= 0.01 {
            return false;
        }
        let recent: Vec<f32> = self.score_history.iter().rev().take(2).copied().collect();
        let max_diff = recent.iter()
            .map(|s| (s - current_score).abs())
            .fold(0.0f32, f32::max);
        max_diff < 0.05
    }

    /// Take the best output, or fall back to a default failure
    fn take_best_output(&mut self) -> AgentOutput {
        self.best_output.take()
            .map(|(_, output)| output)
            .unwrap_or_else(|| AgentOutput::failure("LoopExhausted", "Loop exited without producing usable output"))
    }
}

pub trait NodeFactory: Send + Sync {
    fn create_node(&self, agent_type: &str, task: &str) -> Option<Box<dyn ExecutableNode>>;
}

/// 任务最终状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskFinalState {
    /// 成功完成
    Success,
    /// 部分完成（有些节点失败但非关键）
    PartialSuccess,
    /// 熔断停止（失败率过高）
    CircuitBroken,
    /// 致命错误
    FatalError,
    /// 用户取消
    Cancelled,
    /// 未知状态
    Unknown,
}

impl TaskFinalState {
    pub fn to_user_message(&self) -> &'static str {
        match self {
            Self::Success => "任务完成",
            Self::PartialSuccess => "任务部分完成",
            Self::CircuitBroken => "任务因故障熔断停止",
            Self::FatalError => "任务遇到致命错误",
            Self::Cancelled => "任务已取消",
            Self::Unknown => "任务状态未知",
        }
    }

    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success | Self::PartialSuccess)
    }
}

/// 熔断器配置
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// 触发熔断的失败率阈值（0.0 - 1.0）
    pub failure_rate_threshold: f32,
    /// 最小执行节点数才开始计算失败率
    pub min_nodes_for_circuit_break: usize,
    /// SubGraph maximum allowed nesting depth
    pub max_graph_depth: usize,
    /// Absolute maximum total nodes across all subgraphs to prevent exhaustion
    pub max_total_nodes: usize,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_rate_threshold: 0.5, // 50% 失败率触发熔断
            min_nodes_for_circuit_break: 3, // 至少执行 3 个节点
            max_graph_depth: 5,
            max_total_nodes: 50,
        }
    }
}

use std::sync::Arc;
use tokio::sync::RwLock;

/// Active task registry type
pub type ActiveTaskRegistry = Arc<RwLock<HashMap<String, telos_hci::ActiveTaskInfo>>>;

pub struct TokioExecutionEngine {
    node_factory: Option<std::sync::Arc<dyn NodeFactory>>,
    wakeup_tx: tokio::sync::mpsc::UnboundedSender<(String, String, String)>, // (task_id, node_id, instruction)
    wakeup_rx: tokio::sync::mpsc::UnboundedReceiver<(String, String, String)>,
    circuit_breaker_config: CircuitBreakerConfig,
    active_tasks: ActiveTaskRegistry,
    checkpoint_manager: Option<crate::checkpoint::CheckpointManager>,
    /// Optional Evolution evaluator for semantic drift detection in corrective loops
    evaluator: Option<std::sync::Arc<dyn telos_evolution::Evaluator>>,
}

impl TokioExecutionEngine {
    pub fn new() -> Self {
        let (wakeup_tx, wakeup_rx) = tokio::sync::mpsc::unbounded_channel();
        let cp_mgr = std::env::var("HOME").ok()
            .map(|h| std::path::PathBuf::from(h).join(".telos").join("checkpoints.redb"))
            .and_then(|p| {
                let _ = std::fs::create_dir_all(p.parent().unwrap_or(std::path::Path::new(".")));
                crate::checkpoint::CheckpointManager::new(&p).ok()
            });
        if cp_mgr.is_some() {
            info!("[DAG Engine] ✅ Checkpoint manager initialized (redb)");
        }
        Self {
            node_factory: None,
            wakeup_tx,
            wakeup_rx,
            circuit_breaker_config: CircuitBreakerConfig::default(),
            active_tasks: Arc::new(RwLock::new(HashMap::new())),
            checkpoint_manager: cp_mgr,
            evaluator: None,
        }
    }

    pub fn active_tasks(&self) -> ActiveTaskRegistry {
        self.active_tasks.clone()
    }

    /// Serialize and persist real graph state to redb
    fn checkpoint_graph(&self, graph: &TaskGraph) {
        if let Some(ref mgr) = self.checkpoint_manager {
            if graph.current_state.completed {
                // Task is completed, we don't need to resume it anymore. Delete from DB to prevent bloat.
                if let Err(e) = mgr.delete_checkpoint(&graph.graph_id) {
                    warn!("[DAG Engine] Failed to delete completed checkpoint: {:?}", e);
                }
            } else {
                if let Ok(json) = serde_json::to_string(graph) {
                    if let Err(e) = mgr.save_checkpoint(&graph.graph_id, &json) {
                        warn!("[DAG Engine] Checkpoint save failed: {:?}", e);
                    }
                } else {
                    warn!("[DAG Engine] Failed to serialize TaskGraph for checkpointing");
                }
            }
        }
    }

    pub fn with_active_tasks(mut self, active_tasks: ActiveTaskRegistry) -> Self {
        self.active_tasks = active_tasks;
        self
    }

    pub fn with_node_factory(mut self, factory: std::sync::Arc<dyn NodeFactory>) -> Self {
        self.node_factory = Some(factory);
        self
    }

    /// 配置熔断器
    pub fn with_circuit_breaker_config(mut self, config: CircuitBreakerConfig) -> Self {
        self.circuit_breaker_config = config;
        self
    }

    /// Set the Evolution evaluator for semantic drift detection in corrective loops
    pub fn with_evaluator(mut self, evaluator: std::sync::Arc<dyn telos_evolution::Evaluator>) -> Self {
        self.evaluator = Some(evaluator);
        self
    }

    /// Get a sender to wake up nodes waiting for input
    pub fn get_wakeup_tx(&self) -> tokio::sync::mpsc::UnboundedSender<(String, String, String)> {
        self.wakeup_tx.clone()
    }

    /// Graph pruning: BFS from failed node to mark all downstream dependents as Skipped.
    /// This prevents the DAG from hanging when a node fails and downstream nodes
    /// can never have their in_degree decremented.
    fn prune_downstream(
        graph: &mut TaskGraph,
        failed_node_id: &str,
        broker: &dyn EventBroker,
        graph_id: &str,
    ) -> usize {
        use petgraph::Direction;
        let mut pruned_count = 0;
        let Some(&start_idx) = graph.node_indices.get(failed_node_id) else {
            return 0;
        };

        // BFS through all outgoing edges
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(start_idx);
        let mut visited = std::collections::HashSet::new();
        visited.insert(start_idx);

        while let Some(current) = queue.pop_front() {
            for neighbor in graph.edges.neighbors_directed(current, Direction::Outgoing) {
                if visited.contains(&neighbor) {
                    continue;
                }
                visited.insert(neighbor);

                // Find the node ID for this index
                if let Some(nid) = graph.node_indices.iter()
                    .find(|(_, &idx)| idx == neighbor)
                    .map(|(id, _)| id.clone())
                {
                    let status = graph.node_statuses.get(&nid).cloned();
                    if status == Some(NodeStatus::Pending) || status == Some(NodeStatus::WaitingForInput) {
                        graph.node_statuses.insert(nid.clone(), NodeStatus::Skipped);
                        pruned_count += 1;

                        broker.publish_feedback(AgentFeedback::StateChanged {
                            task_id: graph_id.to_string(),
                            current_node: nid.clone(),
                            status: NodeStatus::Skipped,
                        });

                        // Continue BFS — this node's children should also be skipped
                        queue.push_back(neighbor);
                    }
                }
            }
        }

        if pruned_count > 0 {
            info!(
                "[DAG Engine] ✂️ Pruned {} downstream nodes from failed '{}'",
                pruned_count, failed_node_id
            );
        }
        pruned_count
    }

    /// Calculate progress info from the graph state
    fn calculate_progress(graph: &TaskGraph) -> ProgressInfo {
        let mut completed = 0;
        let mut running = 0;
        let mut failed = 0;
        let mut pending = 0;
        let mut running_nodes_desc = Vec::new();

        for (node_id, status) in &graph.node_statuses {
            match status {
                NodeStatus::Completed => completed += 1,
                NodeStatus::Running => {
                    running += 1;
                    if let Some(meta) = graph.node_metadata.get(node_id) {
                        let desc = if meta.prompt_preview.is_empty() {
                            meta.task_type.clone()
                        } else {
                            format!("{} - {}", meta.task_type, Self::truncate(&meta.prompt_preview, 50))
                        };
                        running_nodes_desc.push(desc);
                    } else {
                        running_nodes_desc.push(node_id.clone());
                    }
                }
                NodeStatus::Failed => failed += 1,
                NodeStatus::Skipped => completed += 1, // Skipped nodes count as resolved
                NodeStatus::Pending => pending += 1,
                NodeStatus::WaitingForInput => pending += 1, // Treat as pending for progress
            }
        }

        let total = graph.nodes.len();
        let current_node_desc = if running_nodes_desc.is_empty() {
            None
        } else {
            Some(running_nodes_desc.join(", "))
        };
        ProgressInfo::new(completed, total, running, failed, pending, current_node_desc)
    }

    /// Truncate a string for display purposes (UTF-8 safe)
    fn truncate(s: &str, max_len: usize) -> String {
        if s.len() > max_len {
            // Use char_indices to find a safe boundary
            let mut result = String::new();
            let mut byte_count = 0;
            for ch in s.chars() {
                if byte_count + ch.len_utf8() > max_len {
                    break;
                }
                result.push(ch);
                byte_count += ch.len_utf8();
            }
            format!("{}...", result)
        } else {
            s.to_string()
        }
    }

    /// Collect dependency outputs for a node
    fn collect_dependency_outputs(
        graph: &TaskGraph,
        node_id: &str,
    ) -> HashMap<String, AgentOutput> {
        let mut deps = HashMap::new();
        for (from_id, dep_type) in graph.get_dependencies(node_id) {
            if dep_type == DependencyType::Data {
                if let Some(output) = graph.node_results.get(&from_id) {
                    deps.insert(from_id, output.clone());
                }
            }
        }
        deps
    }

    /// 检查是否应该熔断
    fn should_circuit_break(
        completed_nodes: usize,
        failed_nodes: usize,
        config: &CircuitBreakerConfig,
    ) -> bool {
        if completed_nodes < config.min_nodes_for_circuit_break {
            return false;
        }

        let failure_rate = failed_nodes as f32 / completed_nodes as f32;
        failure_rate >= config.failure_rate_threshold
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
        let _start_time = Instant::now();

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
        let mut waiting_for_input_count = 0;
        let mut completed_nodes = 0;
        let mut failed_nodes = 0;
        let mut total_nodes = graph.nodes.len();

        // 跟踪最终状态
        let mut final_state = TaskFinalState::Unknown;
        let mut fatal_error_encountered = false;

        let mut futures = FuturesUnordered::new();
        let graph_id = graph.graph_id.clone();
        let graph_span = info_span!("run_graph", trace_id = %graph_id);
        let circuit_config = self.circuit_breaker_config.clone();

        // Register active task
        let active_tasks_ref = self.active_tasks.clone();
        let current_time_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        let initial_progress = Self::calculate_progress(graph);
        {
            let mut w = active_tasks_ref.write().await;
            let existing_name = w.get(&graph_id).map(|info| info.task_name.clone()).unwrap_or_else(|| graph_id.clone());
            w.insert(
                graph_id.clone(),
                telos_hci::ActiveTaskInfo {
                    task_id: graph_id.clone(),
                    task_name: existing_name,
                    progress: initial_progress,
                    running_nodes: vec![],
                    started_at_ms: current_time_ms,
                },
            );
        }

        let active_tasks_ref = self.active_tasks.clone();
        async move {
            let mut last_progress_update = Instant::now();
            let progress_update_interval = std::time::Duration::from_millis(500);

            // Corrective loop states: keyed by Critic node ID
            let mut loop_states: HashMap<String, LoopState> = HashMap::new();
            
            // Timeout tasks for human approval nodes
            let mut timeout_handles: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();

            while completed_nodes < total_nodes {
                // 0. Cancellation check
                {
                    let still_active = {
                        let w = active_tasks_ref.read().await;
                        w.contains_key(&graph_id)
                    };
                    if !still_active {
                        warn!("[DAG Engine] 🛑 Task {} was removed from active tasks! Aborting.", graph_id);
                        final_state = TaskFinalState::Cancelled;
                        break;
                    }
                }

                // 1. 熔断检查：失败率 > 阈值时停止
                if Self::should_circuit_break(completed_nodes, failed_nodes, &circuit_config) {
                    error!(
                        "[DAG Engine] 🔴 Circuit breaker triggered! Failure rate: {:.1}% ({}/{}). Stopping execution.",
                        (failed_nodes as f32 / completed_nodes as f32) * 100.0,
                        failed_nodes,
                        completed_nodes
                    );
                    final_state = TaskFinalState::CircuitBroken;
                    break;
                }

                // 2. 检查致命错误
                if fatal_error_encountered {
                    final_state = TaskFinalState::FatalError;
                    break;
                }

                // Queue ready nodes
                while let Some(node_id) = ready_queue.pop() {
                    let metadata = graph.node_metadata.get(&node_id).cloned().unwrap_or_default();
                    let task_type = metadata.task_type.clone();
                    let prompt_preview = metadata.prompt_preview.clone();
                    let full_task = metadata.full_task.clone();

                    graph.node_statuses.insert(node_id.clone(), NodeStatus::Running);
                    let dependencies = Self::collect_dependency_outputs(graph, &node_id);

                    // Centralized AOP-style logging & feedback
                    match task_type.to_lowercase().as_str() {
                        "architect" => info!("[Architect] 🏗️  Planning decomposition for: \"{}\"", Self::truncate(&prompt_preview, 100)),
                        "coder" => info!("[Coder] 💻 Implementing: \"{}\"", node_id),
                        "reviewer" => info!("[Reviewer] 🔍 Critiquing: \"{}\"", node_id),
                        "researcher" => info!("[Researcher] 📚 Validating: \"{}\"", node_id),
                        "tool" => info!("[Tool] 🛠️  Executing: {} for \"{}\"", metadata.tool_name.as_deref().unwrap_or("Unknown"), node_id),
                        _ => info!("[Agent] 🤖 Executing: {} (\"{}\")", task_type, node_id),
                    }

                    broker.publish_feedback(AgentFeedback::StateChanged {
                        task_id: graph_id.clone(),
                        current_node: node_id.clone(),
                        status: NodeStatus::Running,
                    });

                    broker.publish_feedback(AgentFeedback::NodeStarted {
                        task_id: graph_id.clone(),
                        node_id: node_id.clone(),
                        detail: telos_hci::NodeExecutionDetail {
                            node_id: node_id.clone(),
                            task_type: task_type.clone(),
                            input_preview: Self::truncate(&prompt_preview, 100),
                            started_at: Some(std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64),
                        },
                    });

                    let progress = Self::calculate_progress(graph);
                    broker.publish_feedback(AgentFeedback::ProgressUpdate {
                        task_id: graph_id.clone(),
                        progress: progress.clone(),
                    });

                    // Update ActiveTaskInfo
                    {
                        let mut w = active_tasks_ref.write().await;
                        if let Some(mut info) = w.get_mut(&graph_id) {
                            info.progress = progress;
                            if !info.running_nodes.contains(&node_id) {
                                info.running_nodes.push(node_id.clone());
                            }
                        }
                    }

                    if let Some(node) = graph.nodes.remove(&node_id) {
                        let id_clone = node_id.clone();
                        let node_start_time = Instant::now();
                        let node_span = info_span!("execute_node", node_id = %id_clone);
                        let memory_context = if !ctx.precise_facts.is_empty() {
                            let mut facts_str = String::from("[MEMORY CONTEXT]\nThe following semantic memories might be relevant to your task:\n");
                            for fact in &ctx.precise_facts {
                                facts_str.push_str(&format!("- {}\n", fact.target));
                            }
                            facts_str.push_str("\n");
                            Some(facts_str)
                        } else {
                            None
                        };

                        // Check if this node has a pending correction from a loop
                        let correction = loop_states.values_mut()
                            .find(|ls| ls.actor_node_id == node_id)
                            .and_then(|ls| ls.pending_correction.take());

                        let agent_input = AgentInput {
                            node_id: id_clone.clone(),
                            task: full_task.clone(),
                            dependencies,
                            schema_payload: graph.node_metadata.get(&node_id).and_then(|m| m.schema_payload.clone()),
                            conversation_history: graph.conversation_history.clone(),
                            memory_context,
                            correction,
                        };

                        futures.push(async move {
                            let node_span_clone = node_span.clone();
                            
                            // Supervisor layer: catch panics from node.execute
                            use futures::FutureExt;
                            let exec_future = node.execute(agent_input, registry).instrument(node_span_clone);
                            
                            let output = match std::panic::AssertUnwindSafe(exec_future).catch_unwind().await {
                                Ok(out) => out,
                                Err(err) => {
                                    let msg = if let Some(s) = err.downcast_ref::<&'static str>() {
                                        s.to_string()
                                    } else if let Some(s) = err.downcast_ref::<&String>() {
                                        s.to_string()
                                    } else if let Some(s) = err.downcast_ref::<String>() {
                                        s.clone()
                                    } else {
                                        "Unknown panic during node execution".to_string()
                                    };
                                    
                                    tracing::error!("[DAG Engine] 🚨 FATAL: Node panicked! Captured by Supervisor. Error: {}", msg);
                                    
                                    telos_core::AgentOutput {
                                        success: false,
                                        output: None,
                                        needs_help: None,
                                        sub_graph: None,
                                        trace_logs: vec![],
                                        error: Some(telos_core::AgentErrorDetail {
                                            error_type: "WorkerPanic".to_string(),
                                            message: format!("Node abruptly panicked: {}", msg),
                                            technical_detail: None,
                                            severity: telos_core::ErrorSeverity::Fatal,
                                            layer: telos_core::ErrorLayer::Dag,
                                            retry_suggested: false,
                                        }),
                                    }
                                }
                            };
                            (id_clone, output, node, node_start_time)
                        });
                        active_tasks += 1;
                    }
                }

                if active_tasks == 0 && ready_queue.is_empty() && waiting_for_input_count == 0 {
                    error!("[DAG Engine] 💀 Deadlock detected! Active tasks: 0, Ready: 0, Waiting: 0. Remaining nodes: {}", total_nodes - completed_nodes);
                    fatal_error_encountered = true;
                    break;
                }

                tokio::select! {
                    result = futures.next(), if active_tasks > 0 => {
                        if let Some((node_id, mut output, node_box, node_start_time)) = result {
                            active_tasks -= 1;
                            let execution_time_ms = node_start_time.elapsed().as_millis() as u64;

                            graph.nodes.insert(node_id.clone(), node_box);
                            graph.node_results.insert(node_id.clone(), output.clone());

                            // Checkpoint after each node completion for crash recovery
                            self.checkpoint_graph(graph);

                            // Publish TraceLogs
                            for trace in &output.trace_logs {
                                broker.publish_feedback(AgentFeedback::Trace {
                                    task_id: graph_id.clone(),
                                    node_id: node_id.clone(),
                                    trace: trace.clone(),
                                });
                            }

                            // Handle Help Request (Interaction Pause)
                            if let Some(ref help) = &output.needs_help {
                                graph.node_statuses.insert(node_id.clone(), NodeStatus::WaitingForInput);
                                waiting_for_input_count += 1;
                                warn!("[DAG Engine] ⏳ Node \"{}\" waiting for input: {}", node_id, help.detail);

                                broker.publish_feedback(AgentFeedback::NodeNeedsHelp {
                                    task_id: graph_id.clone(),
                                    node_id: node_id.clone(),
                                    help: help.clone(),
                                });

                                broker.publish_feedback(AgentFeedback::StateChanged {
                                    task_id: graph_id.clone(),
                                    current_node: node_id.clone(),
                                    status: NodeStatus::WaitingForInput,
                                });

                                let progress = Self::calculate_progress(graph);
                                broker.publish_feedback(AgentFeedback::ProgressUpdate {
                                    task_id: graph_id.clone(),
                                    progress: progress.clone(),
                                });

                                // Update ActiveTaskInfo state to waiting/pending
                                {
                                    let mut w = active_tasks_ref.write().await;
                                    if let Some(mut info) = w.get_mut(&graph_id) {
                                        info.progress = progress;
                                        info.running_nodes.retain(|n| n != &node_id);
                                    }
                                }

                                // Spawn a 120s timeout auto-wakeup task.
                                // If the user responds before timeout, the wakeup is
                                // safely ignored (node will no longer be WaitingForInput).
                                {
                                    let timeout_wakeup_tx = self.wakeup_tx.clone();
                                    let timeout_graph_id = graph_id.clone();
                                    let timeout_node_id = node_id.clone();
                                    let handle = tokio::spawn(async move {
                                        tokio::time::sleep(tokio::time::Duration::from_secs(120)).await;
                                        let _ = timeout_wakeup_tx.send((
                                            timeout_graph_id.clone(),
                                            timeout_node_id.clone(),
                                            "[Auto-Approved: Timeout]".to_string(),
                                        ));
                                        tracing::info!(
                                            "[DAG Engine] ⏱️ Timeout auto-wakeup sent for node \"{}\" in graph \"{}\"",
                                            timeout_node_id, timeout_graph_id
                                        );
                                    });
                                    timeout_handles.insert(node_id.clone(), handle);
                                }
                                continue;
                            }

                            // Dynamic SubGraph Injection
                            if let Some(sub_graph) = output.sub_graph.take() {
                                let current_depth = node_id.matches("__").count() + 1;
                                
                                if current_depth >= circuit_config.max_graph_depth || total_nodes + sub_graph.nodes.len() > circuit_config.max_total_nodes {
                                    error!("[DAG Engine] 🔴 Circuit breaker: SubGraph rejected. Max depth/nodes exceeded (Depth: {}, Projected Total Nodes: {})", current_depth, total_nodes + sub_graph.nodes.len());
                                    // Force failure to prevent infinite loop
                                    output.success = false;
                                    output.error = Some(telos_core::AgentErrorDetail {
                                        error_type: "CircuitBreak_Exhaustion".to_string(),
                                        message: format!("SubGraph dynamically rejected to prevent stack exhaustion or infinite AI loops. Depth: {}", current_depth),
                                        technical_detail: None,
                                        severity: telos_core::ErrorSeverity::Fatal,
                                        layer: telos_core::ErrorLayer::Dag,
                                        retry_suggested: false,
                                    });
                                } else if let Some(factory) = &self.node_factory {
                                    let mut added_nodes = Vec::new();
                                    let mut loop_registrations: Vec<(String, LoopConfig, String)> = Vec::new();

                                    for sg_node in &sub_graph.nodes {
                                        let full_id = format!("{}__{}", node_id, sg_node.id);
                                        if let Some(executable) = factory.create_node(&sg_node.agent_type, &sg_node.task) {
                                            let combined_task = if sg_node.schema_payload.is_empty() {
                                                sg_node.task.clone()
                                            } else {
                                                format!("{}\n\nStrict Output Requirements:\n{}", sg_node.task, sg_node.schema_payload)
                                            };
                                            
                                            graph.add_node_with_metadata(
                                                full_id.clone(),
                                                executable,
                                                crate::NodeMetadata {
                                                    task_type: sg_node.agent_type.clone(),
                                                    prompt_preview: Self::truncate(&combined_task, 100),
                                                    full_task: combined_task.clone(),
                                                    tool_name: None,
                                                    schema_payload: if sg_node.schema_payload.is_empty() { None } else { Some(sg_node.schema_payload.clone()) },
                                                },
                                            );
                                            in_degrees.insert(full_id.clone(), 0);
                                            added_nodes.push(full_id.clone());
                                            total_nodes += 1;

                                            // Detect loop_config for LoopState registration
                                            if let Some(ref lc) = sg_node.loop_config {
                                                let critic_full_id = format!("{}__{}", node_id, lc.critic_node_id);
                                                loop_registrations.push((full_id, lc.clone(), critic_full_id));
                                            }
                                        }
                                    }
                                    for sg_edge in &sub_graph.edges {
                                        let from_id = format!("{}__{}", node_id, sg_edge.from);
                                        let to_id = format!("{}__{}", node_id, sg_edge.to);
                                        if graph.add_edge_with_type(&from_id, &to_id, sg_edge.dep_type).is_ok() {
                                            if let Some(deg) = in_degrees.get_mut(&to_id) {
                                                *deg += 1;
                                            }
                                        }
                                    }
                                    for full_id in added_nodes {
                                        if in_degrees.get(&full_id) == Some(&0) {
                                            ready_queue.push(full_id);
                                        }
                                    }

                                    // Register corrective loop states
                                    for (actor_id, config, critic_id) in loop_registrations {
                                        info!("[DAG Engine] 🔄 Registered corrective loop: Actor={} ↔ Critic={} (max_iter={})",
                                            actor_id, critic_id, config.max_iterations);
                                        loop_states.insert(critic_id.clone(), LoopState::new(config, actor_id, critic_id));
                                    }
                                }
                            }

                            if output.success {
                                // === Corrective Loop: Check if this Critic node triggers a re-loop ===
                                if let Some(loop_state) = loop_states.get_mut(&node_id) {
                                    // Extract satisfaction_score from Critic output
                                    let score = output.output.as_ref()
                                        .and_then(|v| v.get("satisfaction_score"))
                                        .and_then(|v| v.as_f64())
                                        .unwrap_or(0.0) as f32;

                                    // Record this score and Actor's output as candidate for best_output
                                    // (We use the Actor's output stored in node_results, not the Critic's)
                                    let actor_output = graph.node_results.get(&loop_state.actor_node_id)
                                        .cloned()
                                        .unwrap_or_else(|| output.clone());
                                    loop_state.record_score(score, &actor_output);

                                    // Check exit conditions
                                    let should_exit = match &loop_state.config.exit_condition {
                                        ExitCondition::SatisfactionThreshold(threshold) => score >= *threshold,
                                        ExitCondition::OutputContains(key) => output.output.as_ref()
                                            .and_then(|v| v.get(key.as_str()))
                                            .is_some(),
                                    };
                                    let stagnated = loop_state.is_stagnated(score);
                                    let max_reached = loop_state.iteration >= loop_state.config.max_iterations;

                                    // Semantic drift detection via Evolution evaluator (4th exit condition)
                                    let semantic_loop_detected = if !should_exit && !stagnated && !max_reached {
                                        if let Some(ref evaluator) = self.evaluator {
                                            // Build a mini-trace from the loop's score history
                                            let trace = telos_evolution::ExecutionTrace {
                                                task_id: graph_id.clone(),
                                                steps: loop_state.score_history.iter().enumerate().map(|(i, &s)| {
                                                    telos_evolution::TraceStep {
                                                        node_id: format!("{}__iter{}", loop_state.actor_node_id, i),
                                                        input_data: format!("iteration_{}", i),
                                                        output_data: Some(format!("score={:.2}", s)),
                                                        error: None,
                                                    }
                                                }).collect(),
                                                errors_encountered: vec![],
                                                success: true,
                                                sub_graph: None,
                                                reused_workflow_ids: vec![],
                                            };
                                            match evaluator.detect_drift(&trace).await {
                                                Err(telos_evolution::DriftWarning::SemanticLoop) => {
                                                    warn!("[DAG Engine] 🔄🛑 Semantic loop detected by Evolution evaluator: Actor={}, iter={}",
                                                        loop_state.actor_node_id, loop_state.iteration);
                                                    true
                                                }
                                                _ => false,
                                            }
                                        } else {
                                            false
                                        }
                                    } else {
                                        false
                                    };

                                    if should_exit {
                                        info!("[DAG Engine] 🔄✅ Loop exiting (satisfied): Actor={}, iter={}, score={:.2}",
                                            loop_state.actor_node_id, loop_state.iteration, score);
                                    } else if stagnated {
                                        warn!("[DAG Engine] 🔄⚠️ Loop exiting (stagnated): Actor={}, iter={}, score={:.2}, history={:?}",
                                            loop_state.actor_node_id, loop_state.iteration, score, loop_state.score_history);
                                    } else if max_reached {
                                        warn!("[DAG Engine] 🔄⚠️ Loop exiting (max iterations): Actor={}, iter={}/{}",
                                            loop_state.actor_node_id, loop_state.iteration, loop_state.config.max_iterations);
                                    } else if semantic_loop_detected {
                                        warn!("[DAG Engine] 🔄⚠️ Loop exiting (semantic loop): Actor={}, iter={}",
                                            loop_state.actor_node_id, loop_state.iteration);
                                    }

                                    if !(should_exit || stagnated || max_reached || semantic_loop_detected) {
                                        // === Re-loop: Generate CorrectionDirective and re-queue Actor ===
                                        let diagnosis = output.output.as_ref()
                                            .and_then(|v| v.get("diagnosis").and_then(|d| d.as_str()))
                                            .unwrap_or("No specific diagnosis")
                                            .to_string();

                                        let corrections: Vec<String> = output.output.as_ref()
                                            .and_then(|v| v.get("corrections"))
                                            .and_then(|v| v.as_array())
                                            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                                            .unwrap_or_default();

                                        let previous_summary: String = output.output.as_ref()
                                            .and_then(|v| v.get("previous_summary").and_then(|s| s.as_str()))
                                            .unwrap_or("")
                                            .to_string();

                                        let directive = CorrectionDirective {
                                            iteration: loop_state.iteration + 1,
                                            satisfaction_score: score,
                                            diagnosis: diagnosis.clone(),
                                            correction_instructions: corrections.clone(),
                                            previous_summary,
                                        };

                                        loop_state.iteration += 1;
                                        loop_state.pending_correction = Some(directive);

                                        info!("[DAG Engine] 🔄🔁 Loop continuing: Actor={}, iter={}, score={:.2}, diagnosis='{}', corrections={:?}",
                                            loop_state.actor_node_id, loop_state.iteration, score, diagnosis, corrections);

                                        // Re-set Actor node to Pending and re-queue
                                        let actor_id = loop_state.actor_node_id.clone();
                                        graph.node_statuses.insert(actor_id.clone(), NodeStatus::Pending);
                                        // Also reset Critic so it can run again after Actor
                                        graph.node_statuses.insert(node_id.clone(), NodeStatus::Pending);
                                        
                                        // CRITICAL: We must increment the Critic's in_degrees by 1 because the Actor is going to run again.
                                        // When the Actor completes, it will decrement this back to 0 and push the Critic to the ready queue.
                                        if let Some(deg) = in_degrees.get_mut(&node_id) {
                                            *deg += 1;
                                        }

                                        ready_queue.push(actor_id);

                                        // Don't count as completed — loop continues
                                        continue;
                                    } else {
                                        // Exit loop — replace output with best Actor output
                                        let mut best = loop_state.take_best_output();
                                        
                                        // If loop exited due to stagnation or max iterations (i.e. not satisfied), 
                                        // explicitly fail the output to trigger downstream pruning and Macro Reset.
                                        if !should_exit {
                                            best.success = false;
                                            best.error = Some(telos_core::AgentErrorDetail::permanent(
                                                "LoopExhausted",
                                                "Worker failed to satisfy Critic within the corrective loop limits.",
                                                telos_core::ErrorLayer::Dag,
                                            ));
                                        }

                                        // Store the best Actor output as the Critic node's result
                                        // so downstream nodes see the Actor's work, not the Critic's eval
                                        graph.node_results.insert(node_id.clone(), best.clone());
                                        output = best;
                                    }
                                }
                                // === End Corrective Loop ===
                            }

                            if output.success {
                                graph.node_statuses.insert(node_id.clone(), NodeStatus::Completed);
                                info!("[DAG Engine] ✓ Node \"{}\" completed in {}ms", node_id, execution_time_ms);

                                broker.publish_feedback(AgentFeedback::StateChanged {
                                    task_id: graph_id.clone(),
                                    current_node: node_id.clone(),
                                    status: NodeStatus::Completed,
                                });

                                broker.publish_feedback(AgentFeedback::NodeCompleted {
                                    task_id: graph_id.clone(),
                                    node_id: node_id.clone(),
                                    result_preview: output.output.as_ref().map(|o| Self::truncate(&o.to_string(), 100)).unwrap_or_else(|| "No output".to_string()),
                                    execution_time_ms,
                                });

                                // Record NodeExecution performance metric
                                let node_type_for_metric = graph.node_metadata.get(&node_id)
                                    .map(|m| m.task_type.clone())
                                    .unwrap_or_else(|| "unknown".to_string());
                                telos_core::metrics::record(telos_core::metrics::MetricEvent::NodeExecution {
                                    timestamp_ms: telos_core::metrics::now_ms(),
                                    task_id: graph_id.clone(),
                                    node_id: node_id.clone(),
                                    node_type: node_type_for_metric,
                                    elapsed_ms: execution_time_ms,
                                    success: true,
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
                                completed_nodes += 1;
                            } else {
                                graph.node_statuses.insert(node_id.clone(), NodeStatus::Failed);
                                completed_nodes += 1;
                                failed_nodes += 1;

                                // 检查是否为致命错误
                                if let Some(ref error) = output.error {
                                    if error.severity == ErrorSeverity::Fatal {
                                        fatal_error_encountered = true;
                                        error!(
                                            "[DAG Engine] 💀 Fatal error encountered in node \"{}\": {}",
                                            node_id, error.message
                                        );
                                    }
                                }

                                broker.publish_feedback(AgentFeedback::StateChanged {
                                    task_id: graph_id.clone(),
                                    current_node: node_id.clone(),
                                    status: NodeStatus::Failed,
                                });

                                broker.publish_feedback(AgentFeedback::NodeFailed {
                                    task_id: graph_id.clone(),
                                    node_id: node_id.clone(),
                                    error: output.error.as_ref().map(|e| ErrorDetail {
                                        error_type: e.error_type.clone(),
                                        message: e.message.clone(),
                                        stack_trace: e.technical_detail.clone(),
                                        retry_suggested: e.retry_suggested,
                                    }).unwrap_or(ErrorDetail {
                                        error_type: "Unknown".to_string(),
                                        message: "Node execution failed".to_string(),
                                        stack_trace: None,
                                        retry_suggested: false,
                                    }),
                                });

                                // Record NodeExecution metric for failed node
                                let node_type_for_metric = graph.node_metadata.get(&node_id)
                                    .map(|m| m.task_type.clone())
                                    .unwrap_or_else(|| "unknown".to_string());
                                telos_core::metrics::record(telos_core::metrics::MetricEvent::NodeExecution {
                                    timestamp_ms: telos_core::metrics::now_ms(),
                                    task_id: graph_id.clone(),
                                    node_id: node_id.clone(),
                                    node_type: node_type_for_metric,
                                    elapsed_ms: execution_time_ms,
                                    success: false,
                                });

                                // Graph pruning: mark all downstream dependents as Skipped
                                let pruned = Self::prune_downstream(graph, &node_id, broker, &graph_id);
                                completed_nodes += pruned; // Count pruned as resolved for loop exit
                            }

                            // Update active tasks map running nodes removal
                            {
                                let mut w = active_tasks_ref.write().await;
                                if let Some(mut info) = w.get_mut(&graph_id) {
                                    info.running_nodes.retain(|n| n != &node_id);
                                }
                            }

                            // Regular progress update (if didn't update already)
                            if last_progress_update.elapsed() >= progress_update_interval {
                                broker.publish_feedback(AgentFeedback::ProgressUpdate {
                                    task_id: graph_id.clone(),
                                    progress: Self::calculate_progress(graph),
                                });
                                last_progress_update = Instant::now();
                            }
                        }
                    }
                    Some((wake_task_id, wake_node_id, instruction)) = self.wakeup_rx.recv() => {
                        if wake_task_id == graph_id {
                            if let Some(status) = graph.node_statuses.get_mut(&wake_node_id) {
                                if *status == NodeStatus::WaitingForInput {
                                    *status = NodeStatus::Pending;
                                    
                                    // Cancel pending timeout if it exists
                                    if let Some(handle) = timeout_handles.remove(&wake_node_id) {
                                        handle.abort();
                                    }

                                    if waiting_for_input_count > 0 {
                                        waiting_for_input_count -= 1;
                                    }
                                    
                                    // Inject the user instruction into the node's task metadata
                                    if let Some(metadata) = graph.node_metadata.get_mut(&wake_node_id) {
                                        let intervention = format!("\n\n[Human Intervention / Expert Help]:\n{}", instruction);
                                        metadata.prompt_preview.push_str(&intervention);
                                        metadata.full_task.push_str(&intervention);
                                    }
                                    
                                    ready_queue.push(wake_node_id.clone());
                                    info!("[DAG Engine] ⚡ Node \"{}\" woken up by user input", wake_node_id);

                                    broker.publish_feedback(AgentFeedback::StateChanged {
                                        task_id: graph_id.clone(),
                                        current_node: wake_node_id.clone(),
                                        status: NodeStatus::Pending,
                                    });

                                    broker.publish_feedback(AgentFeedback::ProgressUpdate {
                                        task_id: graph_id.clone(),
                                        progress: Self::calculate_progress(graph),
                                    });
                                }
                            }
                        }
                    }
                }
            }

            // Deregister active task once completed
            {
                let mut w = active_tasks_ref.write().await;
                w.remove(&graph_id);
            }

            // 确定最终状态
            if fatal_error_encountered {
                final_state = TaskFinalState::FatalError;
            } else if final_state == TaskFinalState::Unknown {
                if completed_nodes == total_nodes && failed_nodes == 0 {
                    final_state = TaskFinalState::Success;
                } else if failed_nodes == 0 {
                    final_state = TaskFinalState::Success;
                } else if completed_nodes > failed_nodes {
                    final_state = TaskFinalState::PartialSuccess;
                } else {
                    final_state = TaskFinalState::Unknown;
                }
            }

            graph.current_state.is_running = false;
            graph.current_state.completed = final_state.is_success();

            // Clear checkpoint since execution reached a terminal state (success or abort)
            if let Some(ref mgr) = self.checkpoint_manager {
                let _ = mgr.delete_checkpoint(&graph_id);
            }

            // 4. Removed duplicate send_task_completed.
            // The caller (main.rs daemon loop) is responsible for emitting TaskCompleted 
            // after performing the Router evaluation and outputting final responses.
        }
        .instrument(graph_span)
        .await;
    }

    fn checkpoint(&self, graph_id: &str) -> Result<(), StorageError> {
        // No-op — use checkpoint_graph() for full serialization
        Ok(())
    }
}

impl Default for TokioExecutionEngine {
    fn default() -> Self {
        Self::new()
    }
}
