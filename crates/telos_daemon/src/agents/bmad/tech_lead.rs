use std::sync::Arc;
use telos_model_gateway::gateway::GatewayManager;
use telos_model_gateway::{LlmRequest, Message, Capability, ModelGateway};
use serde_json::Value;

pub struct TechLeadAgent {
    pub gateway: Arc<GatewayManager>,
}

impl TechLeadAgent {
    pub async fn review_and_patch(
        &self,
        node_id: &str,
        file_path: &str,
        file_content: &str,
        git_diff: &str,
        compiler_stderr: &str,
        original_dev_task: &str,
    ) -> Result<String, String> {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "patched_content": { "type": "string", "description": "The entirely rewritten file content fixing the syntax/logic errors." },
                "explanation": { "type": "string", "description": "Explanation of the patch applied." }
            },
            "required": ["patched_content", "explanation"]
        });

        let system_prompt = "You are the L1.5 SeniorCoderAgent (Tech Lead).\n\
            A junior WorkerAgent has failed to compile a file 3 times on the localized feature branch.\n\
            Your job is to read the Exact Broken Code, the Git Diff of what they tried, and the Compiler `stderr`.\n\
            You must apply surgical fixes to resolve the compiler errors (e.g. lifetime bounds, trait imports, missing fields).\n\
            Do NOT hallucinate a completely different architecture. Just fix the broken module so it compiles.\n\
            You MUST output the FULL entire replaced file content using `submit_patch` tool.".to_string();

        let context = format!(
            "=== ORIGINAL DEV TASK ===\n{}\n\n=== FILE PATH ===\n{}\n\n=== BROKEN FILE CONTENT ===\n{}\n\n=== COMPILER STDERR ===\n{}\n\n=== JUNIOR'S GIT DIFF ===\n{}\n",
            original_dev_task, file_path, file_content, compiler_stderr, git_diff
        );

        let messages = vec![
            Message { role: "system".to_string(), content: system_prompt },
            Message { role: "user".to_string(), content: context },
        ];

        let tool_def = telos_model_gateway::ToolDefinition {
            name: "submit_patch".to_string(),
            description: "Submit the patched file content".to_string(),
            parameters: schema,
        };

        let req = LlmRequest {
            session_id: format!("tech_lead_{}", node_id),
            messages,
            required_capabilities: Capability { requires_vision: false, strong_reasoning: true },
            budget_limit: 40_000,
            tools: Some(vec![tool_def]),
        };

        match self.gateway.generate(req).await {
            Ok(res) => {
                let raw_content = if let Some(tc) = res.tool_calls.first() {
                     tc.arguments.clone()
                } else {
                     res.content
                };

                let content = raw_content.trim()
                    .trim_start_matches("```json")
                    .trim_start_matches("```")
                    .trim_end_matches("```")
                    .trim();

                if let Ok(parsed) = serde_json::from_str::<Value>(content) {
                    if let Some(patched) = parsed.get("patched_content").and_then(|v| v.as_str()) {
                        Ok(patched.to_string())
                    } else {
                        Err("TechLead failed: Missing patched_content in JSON.".to_string())
                    }
                } else {
                    Err("TechLead failed to parse JSON.".to_string())
                }
            }
            Err(e) => Err(format!("Gateway error: {:?}", e)),
        }
    }

    /// Global Workspace Patcher for IntegrationTester Failures
    pub async fn review_and_patch_workspace(
        &self,
        project_dir: &std::path::Path,
        compiler_stderr: &str,
        git_diff: &str,
    ) -> Result<Vec<String>, String> {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string", "description": "The relative file path from the project root (e.g. src/main.rs)" },
                "patched_content": { "type": "string", "description": "The FULL entirely rewritten file content fixing the errors for this specific file." },
                "explanation": { "type": "string", "description": "Explanation of the patch applied." }
            },
            "required": ["file_path", "patched_content", "explanation"]
        });

        let system_prompt = "You are the L1.5 SeniorCoderAgent (Tech Lead).\n\
            The project has failed global integration compilation (`cargo check`).\n\
            Your job is to read the Compiler `stderr` and the `git diff` of what the workers just wrote.\n\
            You must apply surgical fixes to multiple files to resolve the compiler errors (e.g. trait bound mismatches, missing struct fields, incorrect visibility).\n\
            Do NOT hallucinate a completely different architecture. Just fix the broken modules so the project compiles.\n\
            CRITICAL INSTRUCTION: You MUST use the `submit_file_patch` tool to submit your fixes! Do NOT output markdown or conversational text. You can call the `submit_file_patch` tool MULTIPLE TIMES if multiple files need to be fixed.\n\
            For each file, you MUST output the FULL ENTIRE replaced file content via the tool arguments.".to_string();

        let context = format!(
            "=== GLOBAL COMPILER STDERR ===\n{}\n\n=== RECENT WORKER GIT DIFF ===\n{}\n\nIdentify the broken files and submit full file replacements.",
            compiler_stderr, git_diff
        );

        let messages = vec![
            Message { role: "system".to_string(), content: system_prompt },
            Message { role: "user".to_string(), content: context },
        ];

        let tool_def = telos_model_gateway::ToolDefinition {
            name: "submit_file_patch".to_string(),
            description: "Submit a fully patched file content for a specific file".to_string(),
            parameters: schema,
        };

        let req = LlmRequest {
            session_id: "tech_lead_global_workspace".to_string(),
            messages,
            required_capabilities: Capability { requires_vision: false, strong_reasoning: true },
            budget_limit: 80_000,
            tools: Some(vec![tool_def]),
        };

        match self.gateway.generate(req).await {
            Ok(res) => {
                let mut patched_files = Vec::new();
                for tc in res.tool_calls {
                    if tc.name == "submit_file_patch" {
                        if let Ok(parsed) = serde_json::from_str::<Value>(&tc.arguments) {
                            if let (Some(path), Some(content)) = (
                                parsed.get("file_path").and_then(|v| v.as_str()),
                                parsed.get("patched_content").and_then(|v| v.as_str())
                            ) {
                                let abs_path = project_dir.join(path);
                                if let Err(e) = std::fs::write(&abs_path, content) {
                                    tracing::error!("[TechLeadAgent] Failed to write patch to {:?}: {}", abs_path, e);
                                } else {
                                    patched_files.push(path.to_string());
                                }
                            }
                        }
                    }
                }
                
                if patched_files.is_empty() {
                    Err("TechLead didn't submit any valid file patches.".to_string())
                } else {
                    Ok(patched_files)
                }
            }
            Err(e) => Err(format!("Gateway error: {:?}", e)),
        }
    }
}
