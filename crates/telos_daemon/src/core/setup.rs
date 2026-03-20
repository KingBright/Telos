use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;
use telos_core::config::TelosConfig;
use telos_tooling::retrieval::VectorToolRegistry;
use telos_model_gateway::gateway::GatewayManager;
use crate::core::mutate_tool::MutateTool;

pub async fn build_tool_registry(
    config: &TelosConfig,
    gateway: Arc<GatewayManager>,
) -> Arc<RwLock<VectorToolRegistry>> {
    let tool_registry = VectorToolRegistry::new_keyword_only();

    // Register centralized tool metrics hook — fires on EVERY tool call
    telos_tooling::set_tool_metrics_hook(|tool_name, success, _duration| {
        if success {
            crate::METRICS.tool_execution_success.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        } else {
            crate::METRICS.tool_execution_failure.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        crate::core::metrics_store::record(crate::core::metrics_store::MetricEvent::ToolExec {
            timestamp_ms: crate::core::metrics_store::now_ms(),
            tool_name: tool_name.to_string(),
            success,
            task_id: String::new(),
            agent_name: String::new(),
        });
    });

    // Register tool creation metrics hook — fires when create_rhai_tool succeeds/fails
    telos_tooling::set_tool_creation_hook(|tool_name, success, is_iteration| {
        if is_iteration {
            if success {
                crate::METRICS.tool_iteration_success.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            } else {
                crate::METRICS.tool_iteration_failure.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        } else {
            if success {
                crate::METRICS.tool_creation_success.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            } else {
                crate::METRICS.tool_creation_failure.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        crate::core::metrics_store::record(crate::core::metrics_store::MetricEvent::ToolCreation {
            timestamp_ms: crate::core::metrics_store::now_ms(),
            tool_name: tool_name.to_string(),
            success,
            is_iteration,
        });
    });
    
    // Register Native Tools
    tool_registry.register_tool(telos_tooling::native::FsReadTool::schema(), Some(std::sync::Arc::new(telos_tooling::native::FsReadTool)));
    tool_registry.register_tool(telos_tooling::native::FsWriteTool::schema(), Some(std::sync::Arc::new(telos_tooling::native::FsWriteTool)));
    tool_registry.register_tool(telos_tooling::native::ShellExecTool::schema(), Some(std::sync::Arc::new(telos_tooling::native::ShellExecTool)));
    tool_registry.register_tool(telos_tooling::native::CalculatorTool::schema(), Some(std::sync::Arc::new(telos_tooling::native::CalculatorTool)));
    tool_registry.register_tool(telos_tooling::native::ToolRegisterTool::schema(), Some(std::sync::Arc::new(telos_tooling::native::ToolRegisterTool)));
    tool_registry.register_tool(telos_tooling::native::MemoryRecallTool::schema(), Some(std::sync::Arc::new(telos_tooling::native::MemoryRecallTool)));
    tool_registry.register_tool(telos_tooling::native::MemoryStoreTool::schema(), Some(std::sync::Arc::new(telos_tooling::native::MemoryStoreTool)));
    tool_registry.register_tool(telos_tooling::native::FileEditTool::schema(), Some(std::sync::Arc::new(telos_tooling::native::FileEditTool)));
    tool_registry.register_tool(telos_tooling::native::GlobTool::schema(), Some(std::sync::Arc::new(telos_tooling::native::GlobTool)));
    tool_registry.register_tool(telos_tooling::native::GrepTool::schema(), Some(std::sync::Arc::new(telos_tooling::native::GrepTool)));
    tool_registry.register_tool(telos_tooling::native::HttpTool::schema(), Some(std::sync::Arc::new(telos_tooling::native::HttpTool)));
    tool_registry.register_tool(telos_tooling::native::WebSearchTool::schema(), Some(std::sync::Arc::new(telos_tooling::native::WebSearchTool)));
    tool_registry.register_tool(telos_tooling::native::WebScrapeTool::schema(), Some(std::sync::Arc::new(telos_tooling::native::WebScrapeTool)));
    tool_registry.register_tool(telos_tooling::native::GetTimeTool::schema(), Some(std::sync::Arc::new(telos_tooling::native::GetTimeTool)));
    tool_registry.register_tool(telos_tooling::native::LspTool::schema(), Some(std::sync::Arc::new(telos_tooling::native::LspTool)));

    // Wrap registry
    let tool_registry = std::sync::Arc::new(tokio::sync::RwLock::new(tool_registry));

    // Register CreateRhaiTool (needs reference to registry)
    {
        let wrapped_registry = telos_tooling::wrap_tool_registry(tool_registry.clone());
        let create_rhai = telos_tooling::native::CreateRhaiTool::new(wrapped_registry.clone());
        let list_rhai = telos_tooling::native::ListRhaiTools::new(config.tools_dir.clone());
        let discover_tools = telos_tooling::native::DiscoverTools::new(wrapped_registry.clone());
        let attach_note = telos_tooling::native::AttachToolNote::new(wrapped_registry.clone());
        let mutate_tool = MutateTool::new(wrapped_registry.clone(), gateway.clone(), config.tools_dir.clone());
        if let Ok(guard) = tool_registry.try_read() {
            guard.register_tool(telos_tooling::native::CreateRhaiTool::schema(), Some(std::sync::Arc::new(create_rhai)));
            guard.register_tool(telos_tooling::native::ListRhaiTools::schema(), Some(std::sync::Arc::new(list_rhai)));
            guard.register_tool(telos_tooling::native::DiscoverTools::schema(), Some(std::sync::Arc::new(discover_tools)));
            guard.register_tool(telos_tooling::native::AttachToolNote::schema(), Some(std::sync::Arc::new(attach_note)));
            guard.register_tool(MutateTool::schema(), Some(std::sync::Arc::new(mutate_tool)));
        }
    }

    // Auto-Load Persisted Rhai Tools
    let target_dir = std::path::Path::new(&config.tools_dir);
    if target_dir.exists() && target_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(target_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Ok(json_content) = std::fs::read_to_string(&path) {
                        if let Ok(schema) = serde_json::from_str::<telos_tooling::ToolSchema>(&json_content) {
                            let script_path = path.with_extension("rhai");
                            if script_path.exists() {
                                if let Ok(script_code) = std::fs::read_to_string(&script_path) {
                                    let sandbox = std::sync::Arc::new(telos_tooling::script_sandbox::ScriptSandbox::new());
                                    let native_registry = telos_tooling::wrap_tool_registry(tool_registry.clone());
                                    let script_executor: std::sync::Arc<dyn telos_tooling::ToolExecutor> = std::sync::Arc::new(
                                        telos_tooling::script_sandbox::ScriptExecutor::new(script_code, sandbox)
                                            .with_native_tools(native_registry)
                                    );
                                    let guard = tool_registry.write().await;
                                    guard.register_tool(schema, Some(script_executor));
                                    drop(guard);
                                    debug!("[Daemon] Auto-loaded persisted Rhai tool from {:?}", script_path.file_name().unwrap());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    tool_registry
}
