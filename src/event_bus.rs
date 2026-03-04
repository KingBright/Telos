use async_trait::async_trait;
use lru::LruCache;
use std::sync::Mutex;
use std::num::NonZeroUsize;
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

// Placeholder structs for DAG nodes
#[derive(Debug, Clone, PartialEq)]
pub struct NodeResult {
    pub output_data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NodeStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RiskLevel {
    Normal,
    HighRisk,
}

/// 系统全局统一事件总线数据结构
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    UserInput {
        session_id: String,
        payload: String,
        trace_id: Uuid,
    },
    AutoTrigger {
        source: String,
        payload: Vec<u8>,
        trace_id: Uuid,
    },
    UserApproval {
        task_id: String,
        approved: bool,
        supplement_info: Option<String>,
        trace_id: Uuid,
    },
    ReplanRequested {
        node_id: String,
        reason: String,
        partial_result: NodeResult,
        trace_id: Uuid,
    },
}

impl AgentEvent {
    pub fn trace_id(&self) -> Uuid {
        match self {
            AgentEvent::UserInput { trace_id, .. } => *trace_id,
            AgentEvent::AutoTrigger { trace_id, .. } => *trace_id,
            AgentEvent::UserApproval { trace_id, .. } => *trace_id,
            AgentEvent::ReplanRequested { trace_id, .. } => *trace_id,
        }
    }

    // Checks if the event is considered non-critical.
    // In our case, ReplanRequested and UserApproval are critical. UserInput and AutoTrigger can be dropped on heavy backpressure.
    pub fn is_critical(&self) -> bool {
        match self {
            AgentEvent::ReplanRequested { .. } | AgentEvent::UserApproval { .. } => true,
            _ => false,
        }
    }
}

/// 系统给UI/外部的反馈数据结构
#[derive(Debug, Clone, PartialEq)]
pub enum AgentFeedback {
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
        session_id: String,
        content: String,
        is_final: bool,
    },
}

#[async_trait]
pub trait EventBroker: Send + Sync {
    /// 发布一个事件。如果遇到背压，可能导致非核心事件被丢弃。
    async fn publish_event(&self, event: AgentEvent) -> Result<(), EventBrokerError>;
    /// 订阅反馈事件总线。
    fn subscribe_feedback(&self) -> broadcast::Receiver<AgentFeedback>;
}

#[derive(Debug, PartialEq)]
pub enum EventBrokerError {
    ChannelFull,
    DuplicateEvent,
}

/// 基于 Tokio mpsc 和 broadcast 的具体实现
pub struct TokioEventBroker {
    event_tx: mpsc::Sender<AgentEvent>,
    feedback_tx: broadcast::Sender<AgentFeedback>,
    seen_events: Mutex<LruCache<Uuid, ()>>,
}

impl TokioEventBroker {
    pub fn new(event_capacity: usize, feedback_capacity: usize, lru_cache_size: usize) -> (Self, mpsc::Receiver<AgentEvent>) {
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

    /// Provide internal system access to broadcast feedback out
    pub fn publish_feedback(&self, feedback: AgentFeedback) {
        let _ = self.feedback_tx.send(feedback);
    }
}

#[async_trait]
impl EventBroker for TokioEventBroker {
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
                    self.event_tx.send(event).await.map_err(|_| EventBrokerError::ChannelFull)
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err(EventBrokerError::ChannelFull)
            }
        }
    }

    fn subscribe_feedback(&self) -> broadcast::Receiver<AgentFeedback> {
        self.feedback_tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn test_event_idempotency() {
        let (broker, mut _rx) = TokioEventBroker::new(10, 10, 100);
        let uuid = Uuid::new_v4();
        let event = AgentEvent::UserInput {
            session_id: "s1".to_string(),
            payload: "hello".to_string(),
            trace_id: uuid,
        };

        // First time should succeed
        assert_eq!(broker.publish_event(event.clone()).await, Ok(()));

        // Second time with exact same uuid should fail
        assert_eq!(broker.publish_event(event).await, Err(EventBrokerError::DuplicateEvent));
    }

    #[tokio::test]
    async fn test_backpressure_drops_non_critical() {
        // Channel size 1
        let (broker, mut _rx) = TokioEventBroker::new(1, 10, 100);

        let event1 = AgentEvent::UserInput {
            session_id: "s1".to_string(),
            payload: "fill".to_string(),
            trace_id: Uuid::new_v4(),
        };
        assert_eq!(broker.publish_event(event1).await, Ok(())); // Fills channel

        let event2 = AgentEvent::UserInput {
            session_id: "s2".to_string(),
            payload: "drop".to_string(),
            trace_id: Uuid::new_v4(),
        };
        // Channel is full, non-critical event2 should be dropped
        assert_eq!(broker.publish_event(event2).await, Err(EventBrokerError::ChannelFull));
    }

    #[tokio::test]
    async fn test_backpressure_waits_for_critical() {
        // Channel size 1
        let (broker, mut rx) = TokioEventBroker::new(1, 10, 100);

        let event1 = AgentEvent::UserInput {
            session_id: "s1".to_string(),
            payload: "fill".to_string(),
            trace_id: Uuid::new_v4(),
        };
        assert_eq!(broker.publish_event(event1).await, Ok(())); // Fills channel

        let event2 = AgentEvent::UserApproval {
            task_id: "t1".to_string(),
            approved: true,
            supplement_info: None,
            trace_id: Uuid::new_v4(),
        };

        // event2 is critical. Publish should block until room is available.
        // We simulate a consumer freeing up the channel after a short delay.
        let consume_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            rx.recv().await; // consume event1
            rx.recv().await; // consume event2
        });

        // The publish should eventually succeed when the consumer runs
        let result = timeout(Duration::from_millis(200), broker.publish_event(event2)).await;
        assert_eq!(result.unwrap(), Ok(()));

        consume_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_feedback_broadcasting() {
        let (broker, _rx) = TokioEventBroker::new(10, 10, 100);
        let mut sub1 = broker.subscribe_feedback();
        let mut sub2 = broker.subscribe_feedback();

        let feedback = AgentFeedback::Output {
            session_id: "sess1".to_string(),
            content: "done".to_string(),
            is_final: true,
        };

        broker.publish_feedback(feedback.clone());

        assert_eq!(sub1.recv().await.unwrap(), feedback);
        assert_eq!(sub2.recv().await.unwrap(), feedback);
    }
}
