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
impl SystemRegistry for DummyRegistry {
    fn get_model_gateway(&self) -> Option<std::sync::Arc<dyn std::any::Any + Send + Sync>> {
        None // For basic tests, we don't need the gateway
    }
}

// A more advanced test registry that provides a mock gateway
struct GatewayRegistry {
    gateway: std::sync::Arc<dyn telos_model_gateway::ModelGateway>,
}

impl SystemRegistry for GatewayRegistry {
    fn get_model_gateway(&self) -> Option<std::sync::Arc<dyn std::any::Any + Send + Sync>> {
        // Need to wrap the trait object Arc into another Arc<Any> layer to safely downcast
        // Or rather, box the Arc itself into Any. Since `Arc<T>: Any` if `T: 'static`.
        // Let's return Arc::new(self.gateway.clone())
        let arc_gateway: std::sync::Arc<dyn telos_model_gateway::ModelGateway> = self.gateway.clone();
        Some(std::sync::Arc::new(arc_gateway))
    }
}

// Mock ModelProvider and Gateway for the LlmTestNode
struct DummyModelProvider;
#[async_trait]
impl telos_model_gateway::gateway::ModelProvider for DummyModelProvider {
    async fn generate(&self, req: &telos_model_gateway::LlmRequest) -> Result<telos_model_gateway::LlmResponse, telos_model_gateway::GatewayError> {
        Ok(telos_model_gateway::LlmResponse {
            content: "Mock LLM Response".to_string(),
            tokens_used: 10,
        })
    }
}

// A node that tests LLM invocation
#[derive(Clone)]
struct LlmTestNode {
    id: String,
}

#[async_trait]
impl ExecutableNode for LlmTestNode {
    async fn execute(
        &self,
        _ctx: &ScopedContext,
        registry: &dyn SystemRegistry,
    ) -> Result<NodeResult, NodeError> {
        let gateway_any = registry.get_model_gateway()
            .ok_or_else(|| NodeError::ExecutionFailed("Gateway not found".to_string()))?;

        let gateway = gateway_any.downcast_ref::<std::sync::Arc<dyn telos_model_gateway::ModelGateway>>()
            .ok_or_else(|| NodeError::ExecutionFailed("Downcast failed".to_string()))?;

        let req = telos_model_gateway::LlmRequest {
            session_id: "test".to_string(),
            messages: vec![],
            required_capabilities: telos_model_gateway::Capability { requires_vision: false, strong_reasoning: false },
            budget_limit: 100,
        };

        let response = gateway.generate(req).await
            .map_err(|_| NodeError::ExecutionFailed("LLM generation failed".to_string()))?;

        Ok(NodeResult {
            output_data: response.content.into_bytes(),
            extracted_knowledge: None,
            next_routing_hint: None,
        })
    }
}

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

#[tokio::test]
async fn test_llm_node_integration() {
    let provider = Arc::new(DummyModelProvider);
    let gateway = Arc::new(telos_model_gateway::gateway::GatewayManager::new(provider, 100, 10.0));
    let registry = GatewayRegistry { gateway: gateway.clone() };

    let mut graph = TaskGraph::new("llm_graph".into());
    graph.add_node("LlmNode".into(), Box::new(LlmTestNode { id: "LlmNode".into() }));

    let mut engine = TokioExecutionEngine::new();
    let broker = DummyBroker::new();
    let ctx = ScopedContext { budget_tokens: 1000, summary_tree: vec![], precise_facts: vec![] };

    engine.run_graph(&mut graph, &ctx, &registry, &broker).await;

    assert!(graph.current_state.completed);
    let result = graph.node_results.get("LlmNode").unwrap().as_ref().unwrap();
    assert_eq!(result.output_data, b"Mock LLM Response");
}
