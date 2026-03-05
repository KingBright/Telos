use telos_core::NodeError;

pub struct ExecutionTrace {
    pub task_id: String,
    pub node_path: Vec<String>,
    pub errors_encountered: Vec<NodeError>,
}

pub struct SynthesizedSkill {
    pub trigger_condition: String,
    pub executable_code: String,
    pub success_rate: f32,
}

pub enum DriftWarning {
    SemanticLoop,
    TargetDrift,
}

pub trait Evaluator: Send + Sync {
    fn detect_drift(&self, trace: &ExecutionTrace) -> Result<(), DriftWarning>;
    fn distill_experience(&self, trace: &ExecutionTrace) -> Option<SynthesizedSkill>;
}
