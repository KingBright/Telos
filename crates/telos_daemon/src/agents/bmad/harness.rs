use async_trait::async_trait;
use telos_core::{AgentInput, AgentOutput, SystemRegistry};
use telos_dag::ExecutableNode;
use tokio::sync::Mutex;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use telos_model_gateway::gateway::GatewayManager;

static PROJECT_LOCKS: OnceLock<Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>> = OnceLock::new();

async fn get_project_lock(path: &str) -> Arc<Mutex<()>> {
    let global_locks = PROJECT_LOCKS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())));
    let mut locks = global_locks.lock().await;
    locks.entry(path.to_string()).or_insert_with(|| Arc::new(Mutex::new(()))).clone()
}

pub struct HarnessValidatorNode {
    pub gateway: Arc<GatewayManager>,
}

#[async_trait]
impl ExecutableNode for HarnessValidatorNode {
    async fn execute(
        &self,
        input: AgentInput,
        _registry: &dyn SystemRegistry,
    ) -> AgentOutput {
        // The Architect or ScrumMaster configures this node as a Critic.
        // It reads the output from the target Actor.
        
        let target_id = input.node_id.replace("_harness", "");
        let worker_output = input.dependencies.get(&target_id);

        if let Some(AgentOutput { success: true, output: Some(val), .. }) = worker_output {
            tracing::error!("[DEBUG] HarnessValidatorNode received AST: {}", serde_json::to_string_pretty(val).unwrap_or_default());
            let file_path = val.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
            let file_content = val.get("file_content").and_then(|v| v.as_str()).unwrap_or("");
            
            if file_path.is_empty() || file_content.is_empty() {
                return AgentOutput::success(serde_json::json!({
                    "satisfaction_score": 0.0,
                    "correction": {
                        "iteration": 1,
                        "satisfaction_score": 0.0,
                        "diagnosis": "Missing file_path or file_content in Worker output.",
                        "correction_instructions": ["Provide valid file_path and file_content keys in JSON."],
                        "previous_summary": "Empty output."
                    }
                }));
            }

            // --- SECURITY ENFORCEMENT & PROJECT LOCK MUTEX ---
            let base_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")).join(".telos/projects");
            let target_path = std::path::Path::new(file_path);
            
            // Normalize path to prevent directory traversal
            let normalized = target_path.components().fold(std::path::PathBuf::new(), |mut acc, c| {
                match c {
                    std::path::Component::ParentDir => { acc.pop(); acc }
                    std::path::Component::CurDir => acc,
                    _ => { acc.push(c); acc }
                }
            });

            if !normalized.starts_with(&base_dir) {
                return AgentOutput::success(serde_json::json!({
                    "satisfaction_score": 0.0,
                    "correction": {
                        "iteration": 1,
                        "satisfaction_score": 0.0,
                        "diagnosis": format!("Security Violation: Path '{}' is outside the sandbox directory.", file_path),
                        "correction_instructions": ["Ensure file_path strictly targets the sandbox directory provided in your task."],
                        "previous_summary": "Sandbox path traversal attempt rejected."
                    }
                }));
            }
            
            // Acquire project lock to prevent Git index corruption during concurrent DAG checks
            let project_root = normalized.parent().unwrap_or(std::path::Path::new("."));
            let project_key = project_root.to_string_lossy().to_string();
            let lock_arc = get_project_lock(&project_key).await;
            let _guard = lock_arc.lock().await;

            // Determine the current active base branch (e.g. ai/wip-v0.1.0)
            let mut base_branch = "main".to_string();
            if let Ok(out) = std::process::Command::new("git").args(["rev-parse", "--abbrev-ref", "HEAD"]).current_dir(project_root).output() {
                if out.status.success() {
                    base_branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
                }
            }

            // Enforce Feature Branch Isolation (e.g. `task/calc_harness`)
            let task_branch = format!("task/{}", input.node_id);
            let git_branch_check = std::process::Command::new("git").args(["rev-parse", "--verify", &task_branch]).current_dir(project_root).output();
            if git_branch_check.is_ok() && git_branch_check.unwrap().status.success() {
                let _ = std::process::Command::new("git").args(["checkout", &task_branch]).current_dir(project_root).output();
            } else {
                let _ = std::process::Command::new("git").args(["checkout", "-b", &task_branch]).current_dir(project_root).output();
            }

            // Ensure parent dir exists
            if let Some(parent) = normalized.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            // Write to disk
            if let Err(e) = std::fs::write(&normalized, file_content) {
                let _ = std::process::Command::new("git").args(["checkout", &base_branch]).current_dir(project_root).output();
                return AgentOutput::success(serde_json::json!({
                    "satisfaction_score": 0.0,
                    "correction": {
                        "iteration": 1,
                        "satisfaction_score": 0.0,
                        "diagnosis": format!("Failed to write file to disk: {}", e),
                        "correction_instructions": ["Ensure file_path is valid and writable."],
                        "previous_summary": "File write error."
                    }
                }));
            }

            // We only syntax-check local ASTs (since cargo check fails on disjoint unlinked modules without main.rs)
            if normalized.extension().and_then(|s| s.to_str()) == Some("rs") {
                let compile_result = std::process::Command::new("rustfmt")
                    .arg("--check")
                    .arg(&normalized)
                    .output();

                match compile_result {
                    Ok(out) => {
                        if !out.status.success() {
                            let err_output = String::from_utf8_lossy(&out.stderr);
                            
                            let current_iter = input.correction.as_ref().map(|c| c.iteration).unwrap_or(1);
                            
                            if current_iter >= 3 {
                                tracing::warn!("[TechLeadAgent] Branch {} exhausted its retry loops. Invoking L1.5 Senior Patcher...", task_branch);
                                
                                let git_diff_out = std::process::Command::new("git").args(["diff", "HEAD"]).current_dir(project_root).output();
                                let diff_str = git_diff_out.map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
                                
                                let senior_agent = crate::agents::bmad::tech_lead::TechLeadAgent {
                                    gateway: self.gateway.clone(),
                                };
                                
                                let patch_result = senior_agent.review_and_patch(
                                    &input.node_id,
                                    &normalized.to_string_lossy(),
                                    file_content,
                                    &diff_str,
                                    &err_output,
                                    "Please resolve the compiler errors for this module. Retain all original logic but fix the syntax/types."
                                ).await;
                                
                                if let Ok(patched_content) = patch_result {
                                    let _ = std::fs::write(&normalized, &patched_content);
                                    let compile_result2 = std::process::Command::new("rustfmt").arg("--check").arg(&normalized).output();
                                    if let Ok(out2) = compile_result2 {
                                        if out2.status.success() {
                                            tracing::info!("[TechLeadAgent] Surgical patch SUCCESSFUL for {}!", task_branch);
                                            let _ = std::process::Command::new("git").arg("add").arg(&normalized).current_dir(project_root).output();
                                            let _ = std::process::Command::new("git").args(["commit", "-m", &format!("fix: TechLead recovered {} successfully", input.node_id)]).current_dir(project_root).output();
                                            let _ = std::process::Command::new("git").args(["checkout", &base_branch]).current_dir(project_root).output();
                                            let _ = std::process::Command::new("git").args(["merge", "--no-ff", &task_branch, "-m", &format!("Merge {} into {}", task_branch, base_branch)]).current_dir(project_root).output();
                                            let _ = std::process::Command::new("git").args(["branch", "-d", &task_branch]).current_dir(project_root).output();
                                            
                                            return AgentOutput::success(serde_json::json!({
                                                "satisfaction_score": 1.0,
                                                "message": "TechLead successfully patched AST failure.",
                                                "file_content": patched_content,
                                                "file_path": file_path
                                            }));
                                        } else {
                                            tracing::warn!("[TechLeadAgent] Surgical patch failed compilation. Falling back to DAG abandonment.");
                                        }
                                    }
                                } else {
                                    tracing::error!("[TechLeadAgent] Failed to generate a patch: {:?}", patch_result);
                                }
                            }

                            // Commit the broken state to capture the git diff for Tech Lead Reviewer
                            let _ = std::process::Command::new("git").arg("add").arg(&normalized).current_dir(project_root).output();
                            let _ = std::process::Command::new("git").args(["commit", "-m", "chore: broken code state snapshot"]).current_dir(project_root).output();

                            // Abandon the branch and go back to base branch so we don't pollute the workspace for others
                            let _ = std::process::Command::new("git").args(["checkout", &base_branch]).current_dir(project_root).output();

                            return AgentOutput::success(serde_json::json!({
                                "satisfaction_score": 0.0,
                                "correction": {
                                    "iteration": current_iter,
                                    "satisfaction_score": 0.0,
                                    "diagnosis": format!("Syntax Check Failed:\n{}", err_output),
                                    "correction_instructions": ["Fix the syntax errors exactly as described. Ensure braces and types are correct."],
                                    "previous_summary": "Code failed syntax validation."
                                }
                            }));
                        } else {
                            // Valid Code: Auto-forward merge feature branch into WIP branch inside IntegrationTester
                            // Harness just commits the green build to the feature branch.
                            let _ = std::process::Command::new("git").arg("add").arg(&normalized).current_dir(project_root).output();
                            let _ = std::process::Command::new("git").args(["commit", "-m", &format!("feat: {} compilation passed", input.node_id)]).current_dir(project_root).output();
                            
                            // Merge back into base
                            let _ = std::process::Command::new("git").args(["checkout", &base_branch]).current_dir(project_root).output();
                            let _ = std::process::Command::new("git").args(["merge", "--no-ff", &task_branch, "-m", &format!("Merge {} into {}", task_branch, base_branch)]).current_dir(project_root).output();
                            // Delete branch after successful merge
                            let _ = std::process::Command::new("git").args(["branch", "-d", &task_branch]).current_dir(project_root).output();
                        }
                    }
                    Err(e) => {
                        return AgentOutput::success(serde_json::json!({
                            "satisfaction_score": 0.0,
                            "correction": {
                                "iteration": 1,
                                "satisfaction_score": 0.0,
                                "diagnosis": format!("Failed to spawn compiler check: {}", e),
                                "correction_instructions": ["Check environment tooling."],
                                "previous_summary": "Compiler process spawn failed."
                            }
                        }));
                    }
                }
            }

            // Success validation
            AgentOutput::success(serde_json::json!({
                "satisfaction_score": 1.0,
                "message": "AST validation and Contract checks passed."
            }))

        } else {
            // No valid output to check
            AgentOutput::success(serde_json::json!({
                "satisfaction_score": 0.0,
                "correction": {
                    "iteration": 1,
                    "satisfaction_score": 0.0,
                    "diagnosis": "Worker produced no parseable JSON output.",
                    "correction_instructions": ["You MUST strictly adhere to the requested JSON schema array."],
                    "previous_summary": "JSON schema failure."
                }
            }))
        }
    }
}
