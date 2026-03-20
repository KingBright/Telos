pub mod architect;
pub mod coder;
pub mod evolutor;
pub mod general;
pub mod prompt_builder;
pub mod react_loop;
pub mod researcher;
pub mod reviewer;
pub mod router;
pub mod search_worker;
pub mod tester;

// Re-export common dependencies that agents might need
pub use async_trait::async_trait;
pub use std::sync::Arc;
pub use telos_core::{AgentInput, AgentOutput, AgentErrorDetail, ErrorSeverity, ErrorLayer, SystemRegistry};
pub use telos_dag::ExecutableNode;
pub use telos_model_gateway::gateway::GatewayManager;
use telos_model_gateway::GatewayError;

pub use architect::ArchitectAgent;
pub use coder::CoderAgent;
pub use general::GeneralAgent;
pub use researcher::DeepResearchAgent;
pub use reviewer::ReviewAgent;
pub use search_worker::SearchWorkerAgent;
pub use tester::TestingAgent;

/// 从 GatewayError 创建 AgentOutput
/// 将技术性错误转换为用户友好的业务状态
pub fn from_gateway_error(error: GatewayError, context: &str) -> AgentOutput {
    let user_message = error.to_user_message();
    let technical_detail = format!("{}: {:?}", context, error);
    let retry_suggested = error.is_retryable();

    let severity = if error.is_permanent() {
        ErrorSeverity::Permanent
    } else if retry_suggested {
        ErrorSeverity::Transient
    } else {
        ErrorSeverity::Permanent
    };

    AgentOutput {
        success: false,
        output: None,
        trace_logs: vec![],
        sub_graph: None,
        error: Some(AgentErrorDetail {
            error_type: "GatewayError".to_string(),
            message: format!("{} ({})", user_message, context),
            technical_detail: Some(technical_detail),
            severity,
            layer: ErrorLayer::Gateway,
            retry_suggested,
        }),
        needs_help: None,
    }
}

/// 创建解析错误的 AgentOutput
pub fn parse_failure(error_type: &str, context: &str, raw_output: &str) -> AgentOutput {
    AgentOutput {
        success: false,
        output: None,
        trace_logs: vec![],
        sub_graph: None,
        error: Some(AgentErrorDetail::permanent(
            error_type,
            &format!("{}: 数据格式解析失败", context),
            ErrorLayer::Agent,
        ).with_technical_detail(format!("Raw output: {}", raw_output))),
        needs_help: None,
    }
}

/// 创建需要人工干预的 AgentOutput
pub fn needs_intervention(help_type: &str, detail: &str, suggestions: Vec<String>) -> AgentOutput {
    AgentOutput::help(help_type, detail, suggestions)
}

/// Helper method to build the combined Environment and Memory context block
pub fn build_system_prompt_context(registry: &dyn SystemRegistry, ctx: &telos_context::ScopedContext) -> String {
    let mut context_block = String::new();
    
    // Inject Environment Context
    if let Some(sys_ctx) = registry.get_system_context() {
        context_block.push_str(&format!("[ENVIRONMENT CONTEXT]\nLocal Time: {}\nPhysical Location: {}\n\n", sys_ctx.current_time, sys_ctx.location));
    }

    // Inject Semantic Memory Context (if any facts were retrieved)
    if !ctx.precise_facts.is_empty() {
        context_block.push_str("[MEMORY CONTEXT]\nThe following semantic memories might be relevant to the user's current request:\n");
        for fact in &ctx.precise_facts {
            context_block.push_str(&format!("- {}\n", fact.target));
        }
        context_block.push_str("\n");
    }

    context_block
}
