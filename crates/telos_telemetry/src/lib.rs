use std::collections::HashMap;
use telos_evolution::ExecutionTrace;
use tracing::{info, Span};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq)]
pub enum ExportError {
    IoError,
    SerializationFailed,
    NotFound,
}

pub trait TelemetryProvider: Send + Sync {
    fn record_metric(&self, metric_name: &str, value: f64, tags: HashMap<String, String>);
    fn export_trace_log(&self, trace_id: &str) -> Result<ExecutionTrace, ExportError>;

    /// Wraps a message payload and a tracing Span to pass context across asynchronous thread boundaries
    fn wrap_with_span<T>(&self, payload: T, parent_span: Span) -> (T, Span) {
        (payload, parent_span)
    }
}

pub struct OtlpTelemetryProvider {
    in_memory_traces: Arc<Mutex<HashMap<String, ExecutionTrace>>>,
}

impl OtlpTelemetryProvider {
    pub fn new() -> Self {
        Self {
            in_memory_traces: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Stores a trace manually mimicking an OTLP endpoint gathering completed traces
    pub fn store_trace(&self, trace: ExecutionTrace) {
        let mut traces = self.in_memory_traces.lock().unwrap();
        traces.insert(trace.task_id.clone(), trace);
    }
}

impl Default for OtlpTelemetryProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl TelemetryProvider for OtlpTelemetryProvider {
    fn record_metric(&self, metric_name: &str, value: f64, tags: HashMap<String, String>) {
        let tag_json = serde_json::to_string(&tags).unwrap_or_else(|_| "{}".to_string());
        info!(
            metric_name = metric_name,
            value = value,
            tags = %tag_json,
            "recorded metric"
        );
    }

    fn export_trace_log(&self, trace_id: &str) -> Result<ExecutionTrace, ExportError> {
        info!(trace_id = trace_id, "exporting trace log");
        let traces = self.in_memory_traces.lock().unwrap();

        traces.get(trace_id)
            .cloned()
            .ok_or(ExportError::NotFound)
    }
}

pub fn init_telemetry(log_dir: &str, file_prefix: &str) -> tracing_appender::non_blocking::WorkerGuard {
    let file_appender = tracing_appender::rolling::hourly(log_dir, file_prefix);
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::fmt()
        .json()
        .with_writer(non_blocking)
        .with_current_span(true)
        .with_span_list(true)
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .init();

    guard
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;
    use tracing::{span, Level, Instrument};
    use tokio::sync::mpsc;

    #[test]
    fn test_record_metric() {
        let provider = OtlpTelemetryProvider::new();
        let mut tags = HashMap::new();
        tags.insert("service".to_string(), "DAG".to_string());

        provider.record_metric("execution_time", 15.5, tags);
    }

    #[test]
    fn test_export_trace_log_not_found() {
        let provider = OtlpTelemetryProvider::new();
        let result = provider.export_trace_log("missing-trace-id");
        assert!(matches!(result, Err(ExportError::NotFound)));
    }

    #[test]
    fn test_store_and_export_trace() {
        let provider = OtlpTelemetryProvider::new();
        let trace = ExecutionTrace {
            task_id: "test-task".to_string(),
            steps: vec![],
            errors_encountered: vec![],
            success: true,
        };

        provider.store_trace(trace.clone());
        let result = provider.export_trace_log("test-task");
        assert!(matches!(result, Ok(t) if t == trace));
    }

    #[tokio::test]
    async fn test_distributed_span_propagation() {
        let (tx, mut rx) = mpsc::channel::<(String, Span)>(1);
        let provider = OtlpTelemetryProvider::new();

        // Spawn publisher task
        let parent_span = span!(Level::INFO, "parent_task");
        let _enter = parent_span.enter();

        let payload = "cross_thread_message".to_string();
        let (msg, span) = provider.wrap_with_span(payload, Span::current());

        tx.send((msg, span)).await.unwrap();

        // Spawn receiver task checking context
        let handle = tokio::spawn(async move {
            if let Some((msg, span)) = rx.recv().await {
                // Execute receiver work inside the propagated span context
                async {
                    assert_eq!(msg, "cross_thread_message");
                    info!("Received message across thread boundary");
                }.instrument(span).await;
            }
        });

        handle.await.unwrap();
    }

    #[test]
    fn test_init_telemetry() {
        let dir = tempdir().unwrap();
        let log_dir = dir.path().to_str().unwrap();
        let prefix = "test-log";

        // Call init, forcing flushing on drop
        {
            let _guard = init_telemetry(log_dir, prefix);
            let provider = OtlpTelemetryProvider::new();
            provider.record_metric("init_test", 1.0, HashMap::new());
        } // guard goes out of scope, flushing non-blocking logs to disk

        let mut files_found = 0;
        for entry in fs::read_dir(log_dir).unwrap() {
            let entry = entry.unwrap();
            let file_name = entry.file_name();
            let file_name_str = file_name.to_str().unwrap();
            if file_name_str.starts_with(prefix) {
                files_found += 1;
                let contents = fs::read_to_string(entry.path()).unwrap();
                assert!(contents.contains("init_test"));
            }
        }

        assert!(files_found > 0, "Log file was not created");
    }
}
