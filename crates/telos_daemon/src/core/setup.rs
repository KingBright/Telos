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
    memory_os: Arc<telos_memory::engine::RedbGraphStore>,
) -> Arc<RwLock<VectorToolRegistry>> {
    let tools_dir = std::path::PathBuf::from(&config.tools_dir);
    let tool_registry = VectorToolRegistry::new_keyword_only(tools_dir);

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
    tool_registry.register_tool(telos_tooling::native::ProjectCreateTool::schema(), Some(std::sync::Arc::new(telos_tooling::native::ProjectCreateTool)));
    tool_registry.register_tool(telos_tooling::native::ProjectMetaReadTool::schema(), Some(std::sync::Arc::new(telos_tooling::native::ProjectMetaReadTool)));
    tool_registry.register_tool(telos_tooling::native::ProjectMetaWriteTool::schema(), Some(std::sync::Arc::new(telos_tooling::native::ProjectMetaWriteTool)));
    tool_registry.register_tool(telos_tooling::native::ProjectIterateTool::schema(), Some(std::sync::Arc::new(telos_tooling::native::ProjectIterateTool)));
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
        
        // Register global health tracking — fires in InstrumentedToolExecutor::call()
        telos_tooling::set_tool_health_registry(wrapped_registry.clone());
        
        let rhai_studio = telos_tooling::native::RhaiToolStudio::new(wrapped_registry.clone());
        let discover_tools = telos_tooling::native::DiscoverTools::new(wrapped_registry.clone());
        let attach_note = telos_tooling::native::AttachToolNote::new(wrapped_registry.clone());
        let mutate_tool = MutateTool::new(wrapped_registry.clone(), gateway.clone(), config.tools_dir.clone());
        
        if let Ok(guard) = tool_registry.try_read() {
            guard.register_tool(telos_tooling::native::RhaiToolStudio::schema(), Some(std::sync::Arc::new(rhai_studio)));
            guard.register_tool(telos_tooling::native::DiscoverTools::schema(), Some(std::sync::Arc::new(discover_tools)));
            guard.register_tool(telos_tooling::native::AttachToolNote::schema(), Some(std::sync::Arc::new(attach_note)));
            guard.register_tool(MutateTool::schema(), Some(std::sync::Arc::new(mutate_tool)));

            // Mission Scheduling Tools
            guard.register_tool(
                crate::core::schedule_tools::ScheduleMissionTool::schema(), 
                Some(std::sync::Arc::new(crate::core::schedule_tools::ScheduleMissionTool::new(memory_os.clone())))
            );
            guard.register_tool(
                crate::core::schedule_tools::ListScheduledMissionsTool::schema(), 
                Some(std::sync::Arc::new(crate::core::schedule_tools::ListScheduledMissionsTool::new(memory_os.clone())))
            );
            guard.register_tool(
                crate::core::schedule_tools::CancelMissionTool::schema(), 
                Some(std::sync::Arc::new(crate::core::schedule_tools::CancelMissionTool::new(memory_os.clone())))
            );
        }
    }

    // Note: Custom Rhai tools are auto-loaded by VectorToolRegistry::load_saved_tools() during construction.
    // No need for manual auto-load here.

    tool_registry
}

