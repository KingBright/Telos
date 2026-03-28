use async_trait::async_trait;
use telos_core::{AgentInput, AgentOutput, SystemRegistry, HelpRequest};
use telos_dag::ExecutableNode;

/// SmartApprovalNode — a conditional human-in-the-loop gate.
///
/// Decision logic:
/// 1. If `schema_payload` contains `"skip_approval":"true"` → auto-approve immediately.
/// 2. Otherwise, emit `needs_help` to pause the DAG and wait for user feedback.
///    The DAG engine's timeout mechanism (default 120s) will auto-approve if user
///    does not respond. Quality standards remain unchanged regardless of path taken.
pub struct SmartApprovalNode;

impl SmartApprovalNode {
    /// Check whether the approval step should be skipped based on schema_payload flags.
    fn should_skip(input: &AgentInput) -> bool {
        if let Some(ref sp) = input.schema_payload {
            if sp.contains("skip_approval") && sp.contains("true") {
                return true;
            }
        }
        false
    }

    /// Extract the upstream output from dependencies (first available).
    fn extract_upstream(input: &AgentInput) -> serde_json::Value {
        input.dependencies.values().next()
            .and_then(|out| out.output.clone())
            .unwrap_or_else(|| serde_json::json!({}))
    }

    /// Format a concise summary of the upstream plan for user review.
    fn format_plan_summary(upstream: &serde_json::Value) -> String {
        if let Some(features) = upstream.get("features").and_then(|f| f.as_array()) {
            let feature_list: Vec<String> = features.iter().filter_map(|f| {
                let id = f.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let title = f.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
                Some(format!("  • {} — {}", id, title))
            }).collect();
            format!("Product Features (L1):\n{}", feature_list.join("\n"))
        } else if let Some(modules) = upstream.get("modules").and_then(|m| m.as_array()) {
            let mod_list: Vec<String> = modules.iter().filter_map(|m| {
                let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("Unnamed");
                Some(format!("  • {} — {}", id, name))
            }).collect();
            format!("Architecture Modules (L2):\n{}", mod_list.join("\n"))
        } else {
            serde_json::to_string_pretty(upstream).unwrap_or_else(|_| "No upstream data".to_string())
        }
    }
}

#[async_trait]
impl ExecutableNode for SmartApprovalNode {
    async fn execute(
        &self,
        input: AgentInput,
        _registry: &dyn SystemRegistry,
    ) -> AgentOutput {
        let upstream_data = Self::extract_upstream(&input);

        // --- Path 1: Woken up by human intervention (resume after pause) ---
        if input.task.contains("[Human Intervention / Expert Help]:") {
            let parts: Vec<&str> = input.task.split("[Human Intervention / Expert Help]:").collect();
            let feedback = parts.last().unwrap_or(&"").trim();

            return AgentOutput::success(serde_json::json!({
                "status": "approved",
                "approval_type": "human",
                "feedback": feedback,
                "original_plan": upstream_data,
            }));
        }

        // --- Path 2: Auto-approved by timeout (DAG engine sends this) ---
        if input.task.contains("[Auto-Approved: Timeout]") {
            tracing::info!("[SmartApproval] ⏱️ Auto-approved due to user timeout");
            return AgentOutput::success(serde_json::json!({
                "status": "approved",
                "approval_type": "timeout_auto",
                "feedback": "Auto-approved: user did not respond within the timeout window. Full quality standards maintained.",
                "original_plan": upstream_data,
            }));
        }

        // --- Path 3: Skip approval (user explicitly asked or simple task) ---
        if Self::should_skip(&input) {
            tracing::info!("[SmartApproval] ⚡ Skipping approval — user or system requested fast-track");
            return AgentOutput::success(serde_json::json!({
                "status": "approved",
                "approval_type": "auto_skip",
                "feedback": "Approval skipped per user/system directive. Full quality standards maintained.",
                "original_plan": upstream_data,
            }));
        }

        // --- Path 4: Interactive mode — pause and wait for user ---
        let plan_summary = Self::format_plan_summary(&upstream_data);

        AgentOutput {
            success: false,
            output: None,
            sub_graph: None,
            error: None,
            needs_help: Some(HelpRequest {
                help_type: "UserConfirmation".to_string(),
                detail: format!(
                    "📋 Please review the generated plan below. Reply 'approve' to proceed, or provide feedback for adjustments.\n\n{}\n\n⏱️ This will auto-proceed in 120 seconds if no response is received.",
                    plan_summary
                ),
                suggestions: vec!["approve".to_string(), "reject".to_string()],
            }),
            trace_logs: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use telos_core::{AgentInput, AgentOutput};
    use std::collections::HashMap;

    struct DummyRegistry;
    impl telos_core::SystemRegistry for DummyRegistry {}

    #[tokio::test]
    async fn test_smart_approval_skip() {
        let node = SmartApprovalNode;
        let registry = DummyRegistry;
        let mut deps = HashMap::new();
        deps.insert("product_agent".to_string(), AgentOutput::success(serde_json::json!({"features": []})));

        let input = AgentInput {
            node_id: "approval_l1".to_string(),
            task: "Review plan".to_string(),
            dependencies: deps,
            schema_payload: Some(r#"{"skip_approval":"true"}"#.to_string()),
            conversation_history: vec![],
            memory_context: None,
            correction: None,
        };

        let output = node.execute(input, &registry).await;
        assert!(output.success);
        let val = output.output.unwrap();
        assert_eq!(val.get("approval_type").unwrap().as_str().unwrap(), "auto_skip");
    }

    #[tokio::test]
    async fn test_smart_approval_interactive_pause() {
        let node = SmartApprovalNode;
        let registry = DummyRegistry;
        let mut deps = HashMap::new();
        deps.insert("product_agent".to_string(), AgentOutput::success(serde_json::json!({"features": [
            {"id": "FEAT-001", "title": "User Auth"}
        ]})));

        let input = AgentInput {
            node_id: "approval_l1".to_string(),
            task: "Review plan".to_string(),
            dependencies: deps,
            schema_payload: None,
            conversation_history: vec![],
            memory_context: None,
            correction: None,
        };

        let output = node.execute(input, &registry).await;
        assert!(!output.success);
        assert!(output.needs_help.is_some());
        let help = output.needs_help.unwrap();
        assert_eq!(help.help_type, "UserConfirmation");
        assert!(help.detail.contains("FEAT-001"));
        assert!(help.detail.contains("120 seconds"));
    }

    #[tokio::test]
    async fn test_smart_approval_resume_with_human_feedback() {
        let node = SmartApprovalNode;
        let registry = DummyRegistry;
        let mut deps = HashMap::new();
        deps.insert("product_agent".to_string(), AgentOutput::success(serde_json::json!({"features": []})));

        let input = AgentInput {
            node_id: "approval_l1".to_string(),
            task: "Review plan\n\n[Human Intervention / Expert Help]:\nlooks good, approve!".to_string(),
            dependencies: deps,
            schema_payload: None,
            conversation_history: vec![],
            memory_context: None,
            correction: None,
        };

        let output = node.execute(input, &registry).await;
        assert!(output.success);
        let val = output.output.unwrap();
        assert_eq!(val.get("status").unwrap().as_str().unwrap(), "approved");
        assert_eq!(val.get("approval_type").unwrap().as_str().unwrap(), "human");
        assert_eq!(val.get("feedback").unwrap().as_str().unwrap(), "looks good, approve!");
    }

    #[tokio::test]
    async fn test_smart_approval_timeout_auto() {
        let node = SmartApprovalNode;
        let registry = DummyRegistry;
        let mut deps = HashMap::new();
        deps.insert("product_agent".to_string(), AgentOutput::success(serde_json::json!({"features": []})));

        let input = AgentInput {
            node_id: "approval_l1".to_string(),
            task: "Review plan\n\n[Auto-Approved: Timeout]".to_string(),
            dependencies: deps,
            schema_payload: None,
            conversation_history: vec![],
            memory_context: None,
            correction: None,
        };

        let output = node.execute(input, &registry).await;
        assert!(output.success);
        let val = output.output.unwrap();
        assert_eq!(val.get("approval_type").unwrap().as_str().unwrap(), "timeout_auto");
    }
}
