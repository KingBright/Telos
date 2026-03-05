use crate::checkpoint::CheckpointManager;
use crate::engine::TokioExecutionEngine;
use crate::{ExecutableNode, ExecutionEngine, TaskGraph};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use telos_context::ScopedContext;
use telos_core::{NodeError, NodeResult, SystemRegistry};
use telos_hci::{AgentEvent, AgentFeedback, EventBroker, EventBrokerError};
use tokio::sync::broadcast;
use uuid::Uuid;

// A dummy event broker for tests.
struct DummyBroker {
    tx: broadcast::Sender<AgentFeedback>,
}

impl DummyBroker {
    fn new() -> Self {
        let (tx, _) = broadcast::channel(10);
        Self { tx }
    }
}

#[async_trait]
impl EventBroker for DummyBroker {
    async fn publish_event(&self, _event: AgentEvent) -> Result<(), EventBrokerError> {
        Ok(())
    }
    fn publish_feedback(&self, feedback: AgentFeedback) {
        let _ = self.tx.send(feedback);
    }
    fn subscribe_feedback(&self) -> broadcast::Receiver<AgentFeedback> {
        self.tx.subscribe()
    }
}

struct DummyRegistry;
impl SystemRegistry for DummyRegistry {}

// A simple test node that just stores a completion count.
#[derive(Clone)]
struct TestNode {
    id: String,
    counter: Arc<AtomicUsize>,
    output: Vec<u8>,
}

#[async_trait]
impl ExecutableNode for TestNode {
    async fn execute(
        &self,
        _ctx: &ScopedContext,
        _registry: &dyn SystemRegistry,
    ) -> Result<NodeResult, NodeError> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        Ok(NodeResult {
            output_data: self.output.clone(),
            extracted_knowledge: None,
            next_routing_hint: None,
        })
    }
}

#[tokio::test]
async fn test_dag_execution_order() {
    let mut graph = TaskGraph::new("test_graph_1".into());
    let counter = Arc::new(AtomicUsize::new(0));

    let node_a = Box::new(TestNode {
        id: "A".into(),
        counter: counter.clone(),
        output: vec![1],
    });
    let node_b = Box::new(TestNode {
        id: "B".into(),
        counter: counter.clone(),
        output: vec![2],
    });
    let node_c = Box::new(TestNode {
        id: "C".into(),
        counter: counter.clone(),
        output: vec![3],
    });

    graph.add_node("A".into(), node_a);
    graph.add_node("B".into(), node_b);
    graph.add_node("C".into(), node_c);

    // A -> B -> C
    graph.add_edge("A", "B").unwrap();
    graph.add_edge("B", "C").unwrap();

    let mut engine = TokioExecutionEngine::new();
    let broker = DummyBroker::new();
    let registry = DummyRegistry;
    let ctx = ScopedContext {
        budget_tokens: 1000,
        summary_tree: vec![],
        precise_facts: vec![],
    };
    engine.run_graph(&mut graph, &ctx, &registry, &broker).await;

    assert!(graph.current_state.completed);
    assert_eq!(counter.load(Ordering::SeqCst), 3);

    // Check outputs
    assert_eq!(graph.node_results.get("A").unwrap().as_ref().unwrap().output_data, vec![1]);
    assert_eq!(graph.node_results.get("B").unwrap().as_ref().unwrap().output_data, vec![2]);
    assert_eq!(graph.node_results.get("C").unwrap().as_ref().unwrap().output_data, vec![3]);
}

#[tokio::test]
async fn test_parallel_execution() {
    let mut graph = TaskGraph::new("test_graph_2".into());
    let counter = Arc::new(AtomicUsize::new(0));

    // A and B have no dependencies, should run in parallel.
    // C depends on both A and B.
    graph.add_node("A".into(), Box::new(TestNode { id: "A".into(), counter: counter.clone(), output: vec![1] }));
    graph.add_node("B".into(), Box::new(TestNode { id: "B".into(), counter: counter.clone(), output: vec![2] }));
    graph.add_node("C".into(), Box::new(TestNode { id: "C".into(), counter: counter.clone(), output: vec![3] }));

    graph.add_edge("A", "C").unwrap();
    graph.add_edge("B", "C").unwrap();

    let mut engine = TokioExecutionEngine::new();
    let broker = DummyBroker::new();
    let registry = DummyRegistry;
    let ctx = ScopedContext {
        budget_tokens: 1000,
        summary_tree: vec![],
        precise_facts: vec![],
    };
    engine.run_graph(&mut graph, &ctx, &registry, &broker).await;

    assert!(graph.current_state.completed);
    assert_eq!(counter.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn test_checkpoint_recovery() {
    let temp_dir = std::env::temp_dir().join(format!("redb_{}", Uuid::new_v4()));
    let manager = CheckpointManager::new(&temp_dir).unwrap();

    // Save
    let sample_state = r#"{"running": true}"#;
    manager.save_checkpoint("graph_xyz", sample_state).unwrap();

    // Restore
    let restored = manager.restore_checkpoint("graph_xyz").unwrap();
    assert_eq!(restored.unwrap(), sample_state);

    // Cleanup
    let _ = std::fs::remove_file(&temp_dir);
}
