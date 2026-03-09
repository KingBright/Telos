use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{AgentInput, AgentOutput, SystemRegistry};
// NOTE: telos_dag::ExecutableNode is outside this crate. 
// If ExpertAgent needs to implement ExecutableNode, we might need a generic or move this trait to telos_daemon. 
// For now, let's remove the `ExecutableNode` supertrait here to break the circular dependency. 
// We can implement `ExecutableNode` for the concrete structs in `telos_daemon` that implements `ExpertAgent`.

/// Base trait for Tier 2: ExpertAgents.
/// They receive a high level task from the Router, plan a DAG of Worker tasks,
/// and synthesize the results at the end.
#[async_trait]
pub trait ExpertAgent: Send + Sync {
    /// Plans the task by generating a DAG of `WorkerAgent` tasks.
    /// The generated Node prompts MUST match the target Worker's expected `WorkerInputSchema`.
    async fn plan(&self, input: &AgentInput, registry: &dyn SystemRegistry) -> AgentOutput;

    /// Summarizes the execution of the full DAG once it finishes.
    async fn summarize(&self, input: &AgentInput, registry: &dyn SystemRegistry) -> AgentOutput;
}

/// A standardized payload format that all Expert Agents must use when
/// scheduling a Worker Agent in the DAG plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerInputPayload {
    /// Specific instructions for the worker
    pub instructions: String,
    /// JSON string of the exact payload the worker expects.
    /// Example: For CoderAgent, this would be a JSON with `file_path` and `code`.
    pub schema_payload: String,
}

/// Base trait for Tier 3: WorkerAgents.
/// They execute narrow leaf nodes in the DAG.
#[async_trait]
pub trait WorkerAgent {
    /// The unique name of the worker (e.g. "coder", "tester", "researcher")
    fn worker_type(&self) -> &'static str;

    /// Returns the JSON Schema defining what this worker expects in `WorkerInputPayload::schema_payload`.
    /// The ExpertAgent will use this schema to construct the prompt for the worker.
    fn input_schema(&self) -> serde_json::Value;

    /// Executes the worker's specific domain logic based on the input.
    async fn execute_worker(
        &self,
        input: AgentInput,
        registry: &dyn SystemRegistry,
    ) -> AgentOutput;
}
