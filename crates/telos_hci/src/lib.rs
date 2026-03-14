use async_trait::async_trait;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use telos_core::{NodeStatus, RiskLevel};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

// Re-export types from telos_core that are part of the public API
pub use telos_core::{AgentOutput, HelpRequest};

// ============================================================================
// Log Level System
// ============================================================================

/// Log level for controlling feedback verbosity
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// Only final results and human intervention prompts
    Quiet = 0,
    /// Plan summary, node status, errors, progress (default)
    #[default]
    Normal = 1,
    /// Full Plan details, node execution content, intermediate results
    Verbose = 2,
    /// All internal state, error stacks, timing info
    Debug = 3,
}

impl LogLevel {
    /// Convert from u8 value
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => LogLevel::Quiet,
            1 => LogLevel::Normal,
            2 => LogLevel::Verbose,
            3 => LogLevel::Debug,
            _ => LogLevel::Normal,
        }
    }

    /// Convert to u8 value
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// Parse from string (case-insensitive)
    pub fn from_string(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "quiet" => LogLevel::Quiet,
            "normal" => LogLevel::Normal,
            "verbose" => LogLevel::Verbose,
            "debug" => LogLevel::Debug,
            _ => LogLevel::Normal,
        }
    }

    /// Check if feedback at the given minimum level should be shown
    pub fn should_show(&self, min_level: LogLevel) -> bool {
        self.to_u8() >= min_level.to_u8()
    }
}

/// Global log level manager (thread-safe)
pub struct LogLevelManager {
    level: AtomicU8,
}

impl LogLevelManager {
    /// Create a new LogLevelManager with the given initial level
    pub fn new(level: LogLevel) -> Self {
        Self {
            level: AtomicU8::new(level.to_u8()),
        }
    }

    /// Get the current log level
    pub fn get(&self) -> LogLevel {
        LogLevel::from_u8(self.level.load(Ordering::SeqCst))
    }

    /// Set the log level
    pub fn set(&self, level: LogLevel) {
        self.level.store(level.to_u8(), Ordering::SeqCst);
    }
}

impl Default for LogLevelManager {
    fn default() -> Self {
        Self::new(LogLevel::Normal)
    }
}

// Global static log level manager
static GLOBAL_LOG_LEVEL: std::sync::OnceLock<LogLevelManager> = std::sync::OnceLock::new();

/// Get the global log level manager
pub fn global_log_level() -> &'static LogLevelManager {
    GLOBAL_LOG_LEVEL.get_or_init(LogLevelManager::default)
}

// ============================================================================
// Plan and Node Info Structures
// ============================================================================

/// Information about a single node in the execution plan
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlanNodeInfo {
    pub id: String,
    pub task_type: String,         // "LLM" or "TOOL"
    pub prompt_preview: String,    // Truncated prompt for display
    pub dependencies: Vec<String>, // IDs of nodes this depends on
}

/// Information about the complete execution plan
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlanInfo {
    pub reply: Option<String>, // LLM's conversational response
    pub nodes: Vec<PlanNodeInfo>,
    pub total_steps: usize,
    pub estimated_complexity: Option<String>, // "low", "medium", "high"
}

/// Detailed information about node execution
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NodeExecutionDetail {
    pub node_id: String,
    pub task_type: String,
    pub input_preview: String,   // Truncated input for display
    pub started_at: Option<u64>, // Unix timestamp in ms
}

/// Progress information for the running task
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProgressInfo {
    pub completed: usize,
    pub total: usize,
    pub running: usize,
    pub failed: usize,
    pub pending: usize,
    pub percentage: u8,
    pub current_node_desc: Option<String>,
}

impl ProgressInfo {
    pub fn new(
        completed: usize,
        total: usize,
        running: usize,
        failed: usize,
        pending: usize,
        current_node_desc: Option<String>,
    ) -> Self {
        let percentage = if total > 0 {
            ((completed as f64 / total as f64) * 100.0) as u8
        } else {
            0
        };
        Self {
            completed,
            total,
            running,
            failed,
            pending,
            percentage,
            current_node_desc,
        }
    }
}

/// Detailed error information
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ErrorDetail {
    pub error_type: String, // e.g., "ExecutionFailed", "Timeout", "ToolNotFound"
    pub message: String,
    pub stack_trace: Option<String>,
    pub retry_suggested: bool,
}

impl ErrorDetail {
    pub fn from_node_error(error: &telos_core::NodeError) -> Self {
        let (error_type, message) = match error {
            telos_core::NodeError::ExecutionFailed(msg) => {
                ("ExecutionFailed".to_string(), msg.clone())
            }
            telos_core::NodeError::Timeout => {
                ("Timeout".to_string(), "Operation timed out".to_string())
            }
            telos_core::NodeError::DependencyConflict => (
                "DependencyConflict".to_string(),
                "Dependency conflict occurred".to_string(),
            ),
        };
        Self {
            error_type,
            message,
            stack_trace: None,
            retry_suggested: matches!(error, telos_core::NodeError::Timeout),
        }
    }
}

/// Task completion summary
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaskSummary {
    pub success: bool,
    pub total_nodes: usize,
    pub completed_nodes: usize,
    pub failed_nodes: usize,
    pub total_time_ms: u64,
    pub summary: String,              // Human-readable summary
    pub failed_node_ids: Vec<String>, // IDs of failed nodes
}

/// Active task information for TUI monitoring
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ActiveTaskInfo {
    pub task_id: String,
    pub task_name: String,
    pub progress: ProgressInfo,
    pub running_nodes: Vec<String>,
    pub started_at_ms: u64,
}

// ============================================================================
// Event System (Input)
// ============================================================================

/// System global unified event bus data structure
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    UserInput {
        session_id: String,
        payload: String,
        trace_id: Uuid,
        project_id: Option<String>,
    },
    AutoTrigger {
        source: String,
        payload: Vec<u8>,
        trace_id: Uuid,
    },
    UserApproval {
        task_id: String,
        node_id: Option<String>,
        approved: bool,
        supplement_info: Option<String>,
        trace_id: Uuid,
    },
    ReplanRequested {
        node_id: String,
        reason: String,
        partial_result: telos_core::NodeResult,
        trace_id: Uuid,
    },
    /// Direct intervention logically aimed at an active Task DAG (Architect Agent)
    UserIntervention {
        task_id: String,
        node_id: Option<String>,
        instruction: String,
        trace_id: Uuid,
    },
    /// Change the log level at runtime
    SetLogLevel { level: LogLevel },
}

impl AgentEvent {
    pub fn trace_id(&self) -> Uuid {
        match self {
            AgentEvent::UserInput { trace_id, .. } => *trace_id,
            AgentEvent::AutoTrigger { trace_id, .. } => *trace_id,
            AgentEvent::UserApproval { trace_id, .. } => *trace_id,
            AgentEvent::ReplanRequested { trace_id, .. } => *trace_id,
            AgentEvent::UserIntervention { trace_id, .. } => *trace_id,
            AgentEvent::SetLogLevel { .. } => Uuid::nil(),
        }
    }

    // Checks if the event is considered non-critical.
    // In our case, ReplanRequested and UserApproval are critical. UserInput and AutoTrigger can be dropped on heavy backpressure.
    pub fn is_critical(&self) -> bool {
        matches!(
            self,
            AgentEvent::ReplanRequested { .. }
                | AgentEvent::UserApproval { .. }
                | AgentEvent::UserIntervention { .. }
                | AgentEvent::SetLogLevel { .. }
        )
    }
}

// ============================================================================
// Feedback System (Output)
// ============================================================================

/// System feedback to UI/external interfaces
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum AgentFeedback {
    // === Original Feedback Types ===
    StateChanged {
        task_id: String,
        current_node: String,
        status: NodeStatus,
    },
    RequireHumanIntervention {
        task_id: String,
        prompt: String,
        risk_level: RiskLevel,
    },
    Output {
        task_id: String,
        session_id: String,
        content: String,
        is_final: bool,
    },

    // === New Enhanced Feedback Types ===
    /// Plan generation completed (Normal+)
    PlanCreated { task_id: String, plan: PlanInfo },

    /// Node started execution (Verbose+)
    NodeStarted {
        task_id: String,
        node_id: String,
        detail: NodeExecutionDetail,
    },

    /// Node completed successfully (Normal+)
    NodeCompleted {
        task_id: String,
        node_id: String,
        result_preview: String, // Truncated result for display
        execution_time_ms: u64,
    },

    /// Node failed (Normal+)
    NodeFailed {
        task_id: String,
        node_id: String,
        error: ErrorDetail,
    },

    /// Node needs help - cannot proceed without external assistance (Normal+)
    NodeNeedsHelp {
        task_id: String,
        node_id: String,
        help: HelpRequest,
    },

    /// Progress update (Normal+)
    ProgressUpdate {
        task_id: String,
        progress: ProgressInfo,
    },

    /// Task completed with summary (Always shown)
    TaskCompleted {
        task_id: String,
        summary: TaskSummary,
    },

    /// Log level changed notification (Always shown)
    LogLevelChanged {
        old_level: LogLevel,
        new_level: LogLevel,
    },

    /// Execution trace logs (LLM calls, Tool calls) (Verbose+)
    Trace {
        task_id: String,
        node_id: String,
        trace: telos_core::TraceLog,
    },
}

// ============================================================================
// Feedback Filtering
// ============================================================================

impl AgentFeedback {
    /// Get the minimum log level required to show this feedback
    pub fn min_level(&self) -> LogLevel {
        match self {
            // Always shown
            AgentFeedback::RequireHumanIntervention { .. }
            | AgentFeedback::TaskCompleted { .. }
            | AgentFeedback::LogLevelChanged { .. } => LogLevel::Quiet,

            // Normal and above
            AgentFeedback::Output { .. }
            | AgentFeedback::StateChanged { .. }
            | AgentFeedback::NodeNeedsHelp { .. }
            | AgentFeedback::ProgressUpdate { .. } => LogLevel::Normal,

            // Verbose and above
            AgentFeedback::NodeStarted { .. }
            | AgentFeedback::PlanCreated { .. }
            | AgentFeedback::NodeCompleted { .. }
            | AgentFeedback::NodeFailed { .. }
            | AgentFeedback::Trace { .. } => LogLevel::Verbose,
        }
    }

    /// Check if this feedback should be shown at the given log level
    pub fn should_show(&self, level: LogLevel) -> bool {
        level.should_show(self.min_level())
    }

    /// Get the task_id for this feedback if it has one
    pub fn task_id(&self) -> Option<&str> {
        match self {
            AgentFeedback::StateChanged { task_id, .. }
            | AgentFeedback::RequireHumanIntervention { task_id, .. }
            | AgentFeedback::Output { task_id, .. }
            | AgentFeedback::PlanCreated { task_id, .. }
            | AgentFeedback::NodeStarted { task_id, .. }
            | AgentFeedback::NodeCompleted { task_id, .. }
            | AgentFeedback::NodeFailed { task_id, .. }
            | AgentFeedback::NodeNeedsHelp { task_id, .. }
            | AgentFeedback::Trace { task_id, .. }
            | AgentFeedback::ProgressUpdate { task_id, .. }
            | AgentFeedback::TaskCompleted { task_id, .. } => Some(task_id),
            AgentFeedback::LogLevelChanged { .. } => None,
        }
    }

    /// Check if this is a final output (signals task completion)
    pub fn is_final(&self) -> bool {
        matches!(
            self,
            AgentFeedback::TaskCompleted { .. }
        )
    }
}

// ============================================================================
// Event Broker Trait and Implementation
// ============================================================================

#[async_trait]
pub trait EventBroker: Send + Sync {
    /// Publish an event. Non-critical events may be dropped under backpressure.
    async fn publish_event(&self, event: AgentEvent) -> Result<(), EventBrokerError>;
    /// Publish internal system feedback
    fn publish_feedback(&self, feedback: AgentFeedback);
    /// Subscribe to the feedback event bus.
    fn subscribe_feedback(&self) -> broadcast::Receiver<AgentFeedback>;
}

#[derive(Debug, PartialEq)]
pub enum EventBrokerError {
    ChannelFull,
    DuplicateEvent,
}

/// Concrete implementation based on Tokio mpsc and broadcast
pub struct TokioEventBroker {
    event_tx: mpsc::Sender<AgentEvent>,
    feedback_tx: broadcast::Sender<AgentFeedback>,
    seen_events: Mutex<LruCache<Uuid, ()>>,
}

impl TokioEventBroker {
    pub fn new(
        event_capacity: usize,
        feedback_capacity: usize,
        lru_cache_size: usize,
    ) -> (Self, mpsc::Receiver<AgentEvent>) {
        let (event_tx, event_rx) = mpsc::channel(event_capacity);
        let (feedback_tx, _) = broadcast::channel(feedback_capacity);

        let lru_cap = NonZeroUsize::new(lru_cache_size).unwrap_or(NonZeroUsize::new(1024).unwrap());

        let broker = TokioEventBroker {
            event_tx,
            feedback_tx,
            seen_events: Mutex::new(LruCache::new(lru_cap)),
        };

        (broker, event_rx)
    }
}

#[async_trait]
impl EventBroker for TokioEventBroker {
    fn publish_feedback(&self, feedback: AgentFeedback) {
        let _ = self.feedback_tx.send(feedback);
    }

    async fn publish_event(&self, event: AgentEvent) -> Result<(), EventBrokerError> {
        let trace_id = event.trace_id();

        {
            let mut seen = self.seen_events.lock().unwrap();
            if seen.contains(&trace_id) {
                return Err(EventBrokerError::DuplicateEvent);
            }
            seen.put(trace_id, ());
        }

        // Apply backpressure logic via try_send
        match self.event_tx.try_send(event.clone()) {
            Ok(_) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                // If channel is full, drop non-critical events
                if !event.is_critical() {
                    Err(EventBrokerError::ChannelFull)
                } else {
                    // For critical events, block and wait to ensure delivery
                    self.event_tx
                        .send(event)
                        .await
                        .map_err(|_| EventBrokerError::ChannelFull)
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(EventBrokerError::ChannelFull),
        }
    }

    fn subscribe_feedback(&self) -> broadcast::Receiver<AgentFeedback> {
        self.feedback_tx.subscribe()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_ordering() {
        assert!(LogLevel::Debug.should_show(LogLevel::Quiet));
        assert!(LogLevel::Debug.should_show(LogLevel::Normal));
        assert!(LogLevel::Debug.should_show(LogLevel::Verbose));
        assert!(LogLevel::Debug.should_show(LogLevel::Debug));

        assert!(LogLevel::Normal.should_show(LogLevel::Quiet));
        assert!(LogLevel::Normal.should_show(LogLevel::Normal));
        assert!(!LogLevel::Normal.should_show(LogLevel::Verbose));
        assert!(!LogLevel::Normal.should_show(LogLevel::Debug));

        assert!(LogLevel::Quiet.should_show(LogLevel::Quiet));
        assert!(!LogLevel::Quiet.should_show(LogLevel::Normal));
    }

    #[test]
    fn test_log_level_manager() {
        let manager = LogLevelManager::new(LogLevel::Normal);
        assert_eq!(manager.get(), LogLevel::Normal);

        manager.set(LogLevel::Verbose);
        assert_eq!(manager.get(), LogLevel::Verbose);
    }

    #[test]
    fn test_global_log_level() {
        let manager = global_log_level();
        manager.set(LogLevel::Normal);
        assert_eq!(manager.get(), LogLevel::Normal);
    }

    #[test]
    fn test_feedback_min_level() {
        let feedback = AgentFeedback::NodeStarted {
            task_id: "test".into(),
            node_id: "node1".into(),
            detail: NodeExecutionDetail {
                node_id: "node1".into(),
                task_type: "LLM".into(),
                input_preview: "test".into(),
                started_at: None,
            },
        };
        assert_eq!(feedback.min_level(), LogLevel::Verbose);

        let feedback = AgentFeedback::TaskCompleted {
            task_id: "test".into(),
            summary: TaskSummary {
                success: true,
                total_nodes: 1,
                completed_nodes: 1,
                failed_nodes: 0,
                total_time_ms: 100,
                summary: "Done".into(),
                failed_node_ids: vec![],
            },
        };
        assert_eq!(feedback.min_level(), LogLevel::Quiet);
    }

    #[test]
    fn test_progress_info() {
        let progress = ProgressInfo::new(3, 5, 1, 0, 1, None);
        assert_eq!(progress.completed, 3);
        assert_eq!(progress.total, 5);
        assert_eq!(progress.percentage, 60);
    }
}
