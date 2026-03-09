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

impl telos_evolution::TraceExport for OtlpTelemetryProvider {
    fn export_trace(&self, trace_id: &str) -> Option<ExecutionTrace> {
        self.export_trace_log(trace_id).ok()
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

/// Initializes a standardized logging subscriber for the Telos system.
/// This includes:
/// - Human-readable console output with timestamps
/// - (Optional) Persistent file logging
/// - Automatic LogLevel filtering based on `TELOS_LOG_LEVEL` or provided level
pub fn init_logging(log_level: &str) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level));

    // Create a formatter with timestamps
    let fmt = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_file(true)
        .with_line_number(true);

    // Default to plain text with timestamps
    fmt.init();
}

/// A more advanced initialization that supports non-blocking file logging and custom formats.
pub fn init_standard_logging(
    log_level: &str,
    log_dir: Option<&str>,
    file_prefix: Option<&str>
) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{fmt, Registry};

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level));

    let mut guard = None;

    let registry = Registry::default().with(filter);

    // Console layer
    let console_layer = fmt::Layer::default()
        .with_writer(std::io::stderr) // Standard practice to log to stderr
        .with_ansi(true)
        .with_target(true)
        .with_file(true)
        .with_line_number(true)
        .with_timer(tracing_subscriber::fmt::time::ChronoLocal::rfc_3339()); // Standard readable timestamps

    let registry = registry.with(console_layer);

    // Optional File layer
    if let (Some(dir), Some(prefix)) = (log_dir, file_prefix) {
        // Ensure log directory exists
        if !std::path::Path::new(dir).exists() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Run cleanup before starting
        let _ = cleanup_old_logs(dir, 1024 * 1024 * 1024); // 1GB limit

        let file_appender = tracing_appender::rolling::hourly(dir, prefix);
        let (non_blocking, g) = tracing_appender::non_blocking(file_appender);
        guard = Some(g);

        let file_layer = fmt::Layer::default()
            .with_writer(non_blocking)
            .with_ansi(false) // No colors in files
            .with_target(true)
            .with_file(true)
            .with_line_number(true)
            .with_timer(tracing_subscriber::fmt::time::ChronoLocal::rfc_3339());

        registry.with(file_layer).init();
    } else {
        registry.init();
    }

    guard
}

/// Simple cleanup function to keep log directory under a certain size.
/// It sorts files by modification time and deletes the oldest ones until under the limit.
pub fn cleanup_old_logs(dir: &str, max_size_bytes: u64) -> std::io::Result<()> {
    let path = std::path::Path::new(dir);
    if !path.exists() {
        return Ok(());
    }

    let mut files = Vec::new();
    let mut current_size = 0;

    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_file() {
            let modified = metadata.modified()?;
            files.push((entry.path(), metadata.len(), modified));
            current_size += metadata.len();
        }
    }

    if current_size <= max_size_bytes {
        return Ok(());
    }

    // Sort by modified time: oldest first
    files.sort_by_key(|&(_, _, modified)| modified);

    for (path, size, _) in files {
        if current_size <= max_size_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            current_size -= size;
            info!(path = ?path, "Deleted old log file to free up space");
        }
    }

    Ok(())
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
    async fn test_evaluator_with_telemetry() {
        use telos_evolution::evaluator::ActorCriticEvaluator;
        use telos_evolution::TraceStep;

        let provider = OtlpTelemetryProvider::new();

        // Mock a successful trace
        let step1 = TraceStep {
            node_id: "search_db".to_string(),
            input_data: "find user".to_string(),
            output_data: Some("user found".to_string()),
            error: None,
        };

        let trace = ExecutionTrace {
            task_id: "task_eval".to_string(),
            steps: vec![step1],
            errors_encountered: vec![],
            success: true,
        };

        provider.store_trace(trace);

        let evaluator = ActorCriticEvaluator::new().expect("Failed to initialize embedder");

        let skill = evaluator.evaluate_from_source("task_eval", &provider).await.expect("Should not fail to read trace");
        assert!(skill.is_some());

        let synthesized = skill.unwrap();
        assert_eq!(synthesized.executable_code, "Execute sequence: [search_db]");
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
