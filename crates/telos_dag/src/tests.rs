use crate::checkpoint::CheckpointManager;
use crate::engine::TokioExecutionEngine;
use crate::{ExecutableNode, ExecutionEngine, TaskGraph};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use telos_core::{AgentInput, AgentOutput, SystemRegistry, DependencyType};
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
        let arc_gateway: std::sync::Arc<dyn telos_model_gateway::ModelGateway> = self.gateway.clone();
        Some(std::sync::Arc::new(arc_gateway))
    }
}

// Mock ModelProvider and Gateway for the LlmTestNode
struct DummyModelProvider;
#[async_trait]
impl telos_model_gateway::gateway::ModelProvider for DummyModelProvider {
    async fn generate(&self, _req: &telos_model_gateway::LlmRequest) -> Result<telos_model_gateway::LlmResponse, telos_model_gateway::GatewayError> {
        Ok(telos_model_gateway::LlmResponse {
            content: "Mock LLM Response".to_string(),
            tokens_used: 10,
            tool_calls: vec![],
            finish_reason: Some("stop".to_string()),
        })
    }
}

// A node that tests LLM invocation
#[derive(Clone)]
struct LlmTestNode {
    _id: String,
}

#[async_trait]
impl ExecutableNode for LlmTestNode {
    async fn execute(
        &self,
        input: AgentInput,
        registry: &dyn SystemRegistry,
    ) -> AgentOutput {
        let gateway_any = match registry.get_model_gateway() {
            Some(g) => g,
            None => return AgentOutput::failure("GatewayNotFound", "Gateway not found"),
        };

        let gateway = match gateway_any.downcast_ref::<std::sync::Arc<dyn telos_model_gateway::ModelGateway>>() {
            Some(g) => g,
            None => return AgentOutput::failure("DowncastFailed", "Downcast failed"),
        };

        let req = telos_model_gateway::LlmRequest {
            session_id: "test".to_string(),
            messages: vec![],
            required_capabilities: telos_model_gateway::Capability { requires_vision: false, strong_reasoning: false },
            budget_limit: 100,
            tools: None,
        };

        match gateway.generate(req).await {
            Ok(response) => AgentOutput::success(serde_json::json!({
                "text": response.content,
                "node_id": input.node_id
            })),
            Err(_) => AgentOutput::failure("LLMGenerationFailed", "LLM generation failed"),
        }
    }
}

// A simple test node that just stores a completion count.
#[derive(Clone)]
struct TestNode {
    _id: String,
    counter: Arc<AtomicUsize>,
    output: Vec<u8>,
}

#[async_trait]
impl ExecutableNode for TestNode {
    async fn execute(
        &self,
        input: AgentInput,
        _registry: &dyn SystemRegistry,
    ) -> AgentOutput {
        self.counter.fetch_add(1, Ordering::SeqCst);
        AgentOutput::success(serde_json::json!({
            "text": String::from_utf8_lossy(&self.output).to_string(),
            "node_id": input.node_id,
            "dependencies": input.dependencies.keys().collect::<Vec<_>>()
        }))
    }
}

#[tokio::test]
async fn test_dag_execution_order() {
    let mut graph = TaskGraph::new("test_graph_1".into());
    let counter = Arc::new(AtomicUsize::new(0));

    let node_a = Box::new(TestNode {
        _id: "A".into(),
        counter: counter.clone(),
        output: vec![1],
    });
    let node_b = Box::new(TestNode {
        _id: "B".into(),
        counter: counter.clone(),
        output: vec![2],
    });
    let node_c = Box::new(TestNode {
        _id: "C".into(),
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
    let ctx = telos_context::ScopedContext {
        budget_tokens: 1000,
        summary_tree: vec![],
        precise_facts: vec![],
    };
    engine.run_graph(&mut graph, &ctx, &registry, &broker).await;

    assert!(graph.current_state.completed);
    assert_eq!(counter.load(Ordering::SeqCst), 3);

    // Check outputs - now AgentOutput
    let result_a = graph.node_results.get("A").unwrap();
    assert!(result_a.success);
    let result_b = graph.node_results.get("B").unwrap();
    assert!(result_b.success);
    let result_c = graph.node_results.get("C").unwrap();
    assert!(result_c.success);
}

#[tokio::test]
async fn test_parallel_execution() {
    let mut graph = TaskGraph::new("test_graph_2".into());
    let counter = Arc::new(AtomicUsize::new(0));

    // A and B have no dependencies, should run in parallel.
    // C depends on both A and B.
    graph.add_node("A".into(), Box::new(TestNode { _id: "A".into(), counter: counter.clone(), output: vec![1] }));
    graph.add_node("B".into(), Box::new(TestNode { _id: "B".into(), counter: counter.clone(), output: vec![2] }));
    graph.add_node("C".into(), Box::new(TestNode { _id: "C".into(), counter: counter.clone(), output: vec![3] }));

    graph.add_edge("A", "C").unwrap();
    graph.add_edge("B", "C").unwrap();

    let mut engine = TokioExecutionEngine::new();
    let broker = DummyBroker::new();
    let registry = DummyRegistry;
    let ctx = telos_context::ScopedContext {
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
    let gateway = Arc::new(telos_model_gateway::gateway::GatewayManager::new(provider, 0, 3));
    let registry = GatewayRegistry { gateway: gateway.clone() };

    let mut graph = TaskGraph::new("llm_graph".into());
    graph.add_node("LlmNode".into(), Box::new(LlmTestNode { _id: "LlmNode".into() }));

    let mut engine = TokioExecutionEngine::new();
    let broker = DummyBroker::new();
    let ctx = telos_context::ScopedContext { budget_tokens: 1000, summary_tree: vec![], precise_facts: vec![] };

    engine.run_graph(&mut graph, &ctx, &registry, &broker).await;

    assert!(graph.current_state.completed);
    let result = graph.node_results.get("LlmNode").unwrap();
    assert!(result.success);
    assert_eq!(result.output.as_ref().unwrap()["text"], "Mock LLM Response");
}

#[tokio::test]
async fn test_dependency_types() {
    let mut graph = TaskGraph::new("test_dep_types".into());

    graph.add_node("A".into(), Box::new(TestNode { _id: "A".into(), counter: Arc::new(AtomicUsize::new(0)), output: vec![1] }));
    graph.add_node("B".into(), Box::new(TestNode { _id: "B".into(), counter: Arc::new(AtomicUsize::new(0)), output: vec![2] }));

    // Add edge with Data dependency type
    graph.add_edge_with_type("A", "B", DependencyType::Data).unwrap();

    // Verify edge type was stored
    assert_eq!(graph.edge_types.get(&"A|B".to_string()), Some(&DependencyType::Data));

    // Verify get_dependencies works
    let deps = graph.get_dependencies("B");
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0], ("A".to_string(), DependencyType::Data));
}

#[tokio::test]
async fn test_control_vs_data_dependency() {
    let mut graph = TaskGraph::new("test_control_vs_data".into());
    let counter_a = Arc::new(AtomicUsize::new(0));
    let counter_b = Arc::new(AtomicUsize::new(0));
    let counter_c = Arc::new(AtomicUsize::new(0));

    // A --data--> B
    // A --control--> C
    graph.add_node("A".into(), Box::new(TestNode { _id: "A".into(), counter: counter_a.clone(), output: b"data_from_a".to_vec() }));
    graph.add_node("B".into(), Box::new(TestNode { _id: "B".into(), counter: counter_b.clone(), output: b"data_from_b".to_vec() }));
    graph.add_node("C".into(), Box::new(TestNode { _id: "C".into(), counter: counter_c.clone(), output: b"data_from_c".to_vec() }));

    graph.add_edge_with_type("A", "B", DependencyType::Data).unwrap();
    graph.add_edge_with_type("A", "C", DependencyType::Control).unwrap();

    let mut engine = TokioExecutionEngine::new();
    let broker = DummyBroker::new();
    let registry = DummyRegistry;
    let ctx = telos_context::ScopedContext { budget_tokens: 1000, summary_tree: vec![], precise_facts: vec![] };

    engine.run_graph(&mut graph, &ctx, &registry, &broker).await;

    // All nodes should complete
    assert!(graph.current_state.completed);

    // Check that B received A's output (data dependency)
    let result_b = graph.node_results.get("B").unwrap();
    assert!(result_b.success);

    // Check that C completed but did NOT receive A's output (control dependency)
    let result_c = graph.node_results.get("C").unwrap();
    assert!(result_c.success);
}
