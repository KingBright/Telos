use async_trait::async_trait;
use std::sync::Arc;
use telos_core::{AgentInput, AgentOutput, SystemRegistry};
use telos_dag::ExecutableNode;
use telos_model_gateway::gateway::GatewayManager;
use telos_model_gateway::{LlmRequest, Message, Capability, ModelGateway};
use telos_core::{SubGraphNode, AgentSubGraph};
use serde_json;

pub struct ScrumMasterAgent {
    pub gateway: Arc<GatewayManager>,
}

#[async_trait]
impl ExecutableNode for ScrumMasterAgent {
    async fn execute(
        &self,
        input: AgentInput,
        _registry: &dyn SystemRegistry,
    ) -> AgentOutput {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "title": { "type": "string" },
                            "belong_to_module": { "type": "string" },
                            "target_file": { "type": "string", "description": "Relative path from project root ONLY, e.g., src/utils.rs or src/types.rs. NEVER use absolute paths or dump everything in 'src/main.rs'." },
                            "instruction": { "type": "string" },
                            "enforced_contracts": { "type": "array", "items": { "type": "string" } },
                            "dependencies": { "type": "array", "items": { "type": "string" }, "description": "Array of task IDs this task depends on (must be built first)" }
                        },
                        "required": ["id", "title", "belong_to_module", "target_file", "instruction", "enforced_contracts"]
                    }
                }
            },
            "required": ["tasks"]
        });

        let system_prompt = "You are the ScrumMasterAgent (L3 Meta-Graph). \
            Your job is to read L2 TechModules and Contracts, and decompose them into actionable `DevTask` tickets. \
            Each DevTask MUST target a specific file, belong to a module, and explicitly list enforced Contracts. \
            CRITICAL: Do NOT put everything in `src/main.rs`! Distribute code logically into appropriately named module files (e.g. src/repl.rs, src/types.rs) and ensure the application entry module creates `src/main.rs` to import them.\n\
            YOU MUST assign an explicit array of task `dependencies` (task IDs) to enforce that foundational traits and modules are written BEFORE their dependents. \
            You MUST use the provided `decompose_to_dev_tasks` tool to submit the result. Do not output anything else.".to_string();

        let messages = vec![
            Message { role: "system".to_string(), content: system_prompt },
            Message { role: "user".to_string(), content: input.task.clone() },
        ];

        let tool_def = telos_model_gateway::ToolDefinition {
            name: "decompose_to_dev_tasks".to_string(),
            description: "Submit the actionable DevTask tickets".to_string(),
            parameters: schema,
        };

        let req = LlmRequest {
            session_id: format!("scrum_master_{}", input.node_id),
            messages,
            required_capabilities: Capability { requires_vision: false, strong_reasoning: true },
            budget_limit: 15_000,
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
                
                if let Ok(mut parsed) = serde_json::from_str::<serde_json::Value>(content) {
                    let mut nodes = Vec::new();
                    let mut edges = Vec::new();
                    
                    let mut modules = None;
                    let mut contracts = None;
                    let mut project_name = "default_project".to_string();
                    for dep_out in input.dependencies.values() {
                        if let Some(out_val) = &dep_out.output {
                            let plan = if let Some(p) = out_val.get("original_plan") { p } else { out_val };
                            if plan.get("modules").is_some() && plan.get("contracts").is_some() {
                                modules = plan.get("modules");
                                contracts = plan.get("contracts");
                            }
                            if let Some(name) = plan.get("project_name").and_then(|v| v.as_str()) {
                                project_name = name.to_string();
                            }
                        }
                    }
                    
                    let base_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."))
                        .join(".telos/projects")
                        .join(&project_name);

                    // To support topological edges, map task.id -> (worker_id, critic_id)
                    let mut task_id_map = std::collections::HashMap::new();

                    if let Some(tasks) = parsed.get_mut("tasks").and_then(|t| t.as_array_mut()) {
                        // Pass 1: generate unique worker/harness IDs
                        for (i, task_v) in tasks.iter_mut().enumerate() {
                            let task_id = task_v.get("id").and_then(|v| v.as_str()).unwrap_or("worker_task").to_string();
                            let worker_id = format!("{}_{}", task_id, i);
                            let critic_id = format!("{}_harness", worker_id);
                            task_id_map.insert(task_id.clone(), (worker_id, critic_id));
                        }

                        // Parse the local telos_project.toml to register tasks
                        let toml_path = base_dir.join(".telos_project.toml");
                        let mut toml_str = std::fs::read_to_string(&toml_path).unwrap_or_default();

                        // Pass 2: generate sub_graphs and edges
                        for (i, task_v) in tasks.iter_mut().enumerate() {
                            let task_id = task_v.get("id").and_then(|v| v.as_str()).unwrap_or("worker_task").to_string();
                            let (worker_id, critic_id) = task_id_map.get(&task_id).unwrap().clone();
                            
                            // Append to Kanban tracker
                            let title = task_v.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled Task");
                            toml_str.push_str(&format!("{} = \"Pending\" # {}\n", task_id, title));

                            if let Some(obj) = task_v.as_object_mut() {
                                if let Some(target_file) = obj.get("target_file").and_then(|v| v.as_str()) {
                                    let clean_target = target_file
                                        .trim_start_matches(&base_dir.to_string_lossy().to_string())
                                        .trim_start_matches("~/")
                                        .trim_start_matches(&format!(".telos/projects/{}/", project_name))
                                        .trim_start_matches(&format!("{}/", project_name))
                                        .trim_start_matches('/');
                                    let abs_path = base_dir.join(clean_target).to_string_lossy().to_string();
                                    obj.insert("target_file".to_string(), serde_json::json!(abs_path));
                                }
                            }
                            
                            let module_id = task_v.get("belong_to_module").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let enforced = task_v.get("enforced_contracts").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                            
                            let mut module_info = String::new();
                            if let Some(mod_array) = modules.and_then(|v| v.as_array()) {
                                if let Some(m) = mod_array.iter().find(|x| x.get("id").and_then(|v| v.as_str()) == Some(module_id.as_str())) {
                                    module_info = serde_json::to_string_pretty(m).unwrap_or_default();
                                }
                            }
                            
                            let mut contracts_info = Vec::new();
                            if let Some(con_array) = contracts.and_then(|v| v.as_array()) {
                                for c_ref in enforced {
                                    if let Some(c_id) = c_ref.as_str() {
                                        if let Some(c) = con_array.iter().find(|x| x.get("id").and_then(|v| v.as_str()) == Some(c_id)) {
                                            contracts_info.push(serde_json::to_string_pretty(c).unwrap_or_default());
                                        }
                                    }
                                }
                            }
                            
                            let mvc = format!(
                                "=== MINIMAL VIABLE CONTEXT (MVC) ===\nModule constraints:\n{}\n\nStrict Contracts to Enforce:\n{}\n",
                                module_info,
                                contracts_info.join("\n---\n")
                            );
                            
                            // Emit WorkerAgent
                            nodes.push(SubGraphNode {
                                id: worker_id.clone(),
                                agent_type: "worker".to_string(),
                                task: serde_json::to_string_pretty(task_v).unwrap_or_default(),
                                schema_payload: mvc,
                                loop_config: Some(telos_core::LoopConfig {
                                    max_iterations: 3,
                                    exit_condition: telos_core::ExitCondition::SatisfactionThreshold(1.0),
                                    critic_node_id: critic_id.clone(),
                                }),
                                is_critic: false,
                            });
                            
                            // Emit HarnessValidator
                            nodes.push(SubGraphNode {
                                id: critic_id.clone(),
                                agent_type: "harness_validator".to_string(),
                                task: "AST Compilation Check".to_string(),
                                schema_payload: String::new(),
                                loop_config: None,
                                is_critic: true,
                            });
                            
                            // Edge: Worker -> Harness
                            edges.push(telos_core::SubGraphEdge {
                                from: worker_id.clone(),
                                to: critic_id.clone(),
                                dep_type: telos_core::DependencyType::Data,
                            });
                            
                            // Add Topological Dependency Edges based on LLC dispatch array
                            if let Some(deps) = task_v.get("dependencies").and_then(|v| v.as_array()) {
                                for dep in deps {
                                    if let Some(dep_id_str) = dep.as_str() {
                                        if let Some((_, dep_harness_id)) = task_id_map.get(dep_id_str) {
                                            edges.push(telos_core::SubGraphEdge {
                                                from: dep_harness_id.clone(),
                                                to: worker_id.clone(),
                                                dep_type: telos_core::DependencyType::Data, // Worker blocked until Dep Harness verifies!
                                            });
                                        }
                                    }
                                }
                            }
                        }
                        
                        // Save the Kanban tracker back to disk
                        let _ = std::fs::write(&toml_path, toml_str);
                    } // End of task array loop
                    
                    if !nodes.is_empty() {
                        let integration_id = format!("{}_integration_tester", input.node_id);
                        
                        // Emit the IntegrationTester Node
                        nodes.push(SubGraphNode {
                            id: integration_id.clone(),
                            agent_type: "integration_tester".to_string(),
                            task: "Compile and test the aggregated codebase".to_string(),
                            schema_payload: String::new(),
                            loop_config: None,
                            is_critic: false, 
                        });
                        
                        // Link all generated worker Harnesses (Critics) to the Integration Tester
                        // Wait, we can find them by iterating over what was just pushed
                        let harness_ids: Vec<String> = nodes.iter()
                            .filter(|n| n.agent_type == "harness_validator")
                            .map(|n| n.id.clone())
                            .collect();
                            
                        for h_id in harness_ids {
                            edges.push(telos_core::SubGraphEdge {
                                from: h_id,
                                to: integration_id.clone(),
                                dep_type: telos_core::DependencyType::Data,
                            });
                        }
                    }

                    if nodes.is_empty() {
                        AgentOutput::success(parsed)
                    } else {
                        AgentOutput::with_subgraph(parsed, AgentSubGraph { nodes, edges })
                    }
                } else {
                    crate::agents::parse_failure("JsonParseError", "ScrumMasterAgent", content)
                }
            }
            Err(e) => crate::agents::from_gateway_error(e, "ScrumMasterAgent"),
        }
    }
}
