use telos_core::NodeError;
use async_trait::async_trait;

pub mod evaluator;

#[derive(Debug, Clone, PartialEq)]
pub struct TraceStep {
    pub node_id: String,
    pub input_data: String,
    pub output_data: Option<String>,
    pub error: Option<NodeError>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionTrace {
    pub task_id: String,
    pub steps: Vec<TraceStep>,
    pub errors_encountered: Vec<NodeError>,
    pub success: bool,
    pub sub_graph: Option<telos_core::AgentSubGraph>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SynthesizedSkill {
    pub trigger_condition: String,
    pub executable_code: String,
    pub success_rate: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DriftWarning {
    SemanticLoop,
    TargetDrift,
}

#[async_trait]
pub trait Evaluator: Send + Sync {
    async fn detect_drift(&self, trace: &ExecutionTrace) -> Result<(), DriftWarning>;
    async fn distill_experience(&self, trace: &ExecutionTrace) -> Option<SynthesizedSkill>;
}

pub trait TraceExport: Send + Sync {
    fn export_trace(&self, trace_id: &str) -> Option<ExecutionTrace>;
}
