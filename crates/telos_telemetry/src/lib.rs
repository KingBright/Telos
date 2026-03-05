use std::collections::HashMap;
use telos_evolution::ExecutionTrace;

pub enum ExportError {
    IoError,
    SerializationFailed,
}

pub trait TelemetryProvider: Send + Sync {
    fn record_metric(&self, metric_name: &str, value: f64, tags: HashMap<String, String>);
    fn export_trace_log(&self, trace_id: &str) -> Result<ExecutionTrace, ExportError>;
}
