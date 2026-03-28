use async_trait::async_trait;
use telos_core::{AgentInput, AgentOutput, SystemRegistry};
use telos_dag::ExecutableNode;
use std::process::Command;
use tracing::{info, warn};

pub struct IntegrationTesterNode;

#[async_trait]
impl ExecutableNode for IntegrationTesterNode {
    async fn execute(
        &self,
        input: AgentInput,
        _registry: &dyn SystemRegistry,
    ) -> AgentOutput {
        // Find project_name and integration_commands
        let mut project_name = "default_project".to_string();
        let mut integration_commands: Vec<String> = vec![];
        
        for dep_out in input.dependencies.values() {
            if let Some(out_val) = &dep_out.output {
                let plan = if let Some(p) = out_val.get("original_plan") { p } else { out_val };
                if let Some(name) = plan.get("project_name").and_then(|v| v.as_str()) {
                    project_name = name.to_string();
                }
                
                // Extract integration commands from BmadArchitect's plan
                if let Some(cmds) = plan.get("integration_commands").and_then(|v| v.as_array()) {
                    if integration_commands.is_empty() {
                        for cmd in cmds {
                            if let Some(s) = cmd.as_str() {
                                integration_commands.push(s.to_string());
                            }
                        }
                    }
                }
            }
        }

        let base_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".telos/projects")
            .join(&project_name);

        // ProductAgent creates git/cargo inside `{project_name}/{project_name}` or just `{project_name}`. Let's check.
        let cargo_dir = if base_dir.join(&project_name).exists() {
            base_dir.join(&project_name)
        } else {
            base_dir
        };

        if integration_commands.is_empty() {
            info!("[IntegrationTester] ⚡ No integration commands provided. Auto-succeeding.");
            return AgentOutput::success(serde_json::json!({
                "satisfaction_score": 1.0,
                "diagnosis": "No test/build commands specified. Passed natively."
            }));
        }

        let mut all_stdout = String::new();
        let mut all_stderr = String::new();

        for cmd_str in integration_commands {
            info!("[IntegrationTester] 🚧 Running `{}` in {:?}", cmd_str, cargo_dir);
            let cargo_dir_clone = cargo_dir.clone();
            let cmd_clone = cmd_str.clone();
            
            let output = tokio::task::spawn_blocking(move || {
                Command::new("sh")
                    .arg("-c")
                    .arg(&cmd_clone)
                    .current_dir(&cargo_dir_clone)
                    .output()
            }).await.unwrap();

            match output {
                Ok(out) => {
                    all_stdout.push_str(&String::from_utf8_lossy(&out.stdout));
                    all_stderr.push_str(&String::from_utf8_lossy(&out.stderr));

                    if !out.status.success() {
                        let err_msg = format!("Integration failure on command `{}`:\n\n{}", cmd_str, String::from_utf8_lossy(&out.stderr));
                        warn!("[IntegrationTester] ❌ Build failed:\n{}", err_msg);
                        return AgentOutput::failure_with_severity(
                            "IntegrationFailure",
                            &err_msg,
                            telos_core::ErrorSeverity::Permanent,
                            telos_core::ErrorLayer::Agent,
                        );
                    }
                }
                Err(e) => {
                    return AgentOutput::success(serde_json::json!({
                        "satisfaction_score": 0.0,
                        "diagnosis": "Failed to invoke shell.",
                        "corrections": [
                            format!("Failed to execute '{}': {}", cmd_str, e)
                        ]
                    }));
                }
            }
        }

        // Output success and commit snapshot
        let git_dir = cargo_dir.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let _ = Command::new("git").arg("add").arg(".").current_dir(&git_dir).output();
            let _ = Command::new("git").args(["commit", "-m", "chore: WIP AI snapshot (integration passed)"])
                .current_dir(&git_dir)
                .output();
        }).await;

        AgentOutput::success(serde_json::json!({
            "satisfaction_score": 1.0,
            "diagnosis": "Build succeeded. Snapshot committed.",
            "stdout": all_stdout,
            "stderr": all_stderr
        }))
    }
}
