use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use crate::{JsonSchema, ToolError, ToolExecutor, ToolSchema};
use telos_core::RiskLevel;

fn get_project_dir(project_name: &str) -> PathBuf {
    let mut path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    path.push(".telos");
    path.push("projects");
    path.push(project_name);
    path
}

// 1. Create Project Tool
#[derive(Clone)]
pub struct ProjectCreateTool;

#[async_trait]
impl ToolExecutor for ProjectCreateTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'name'".into()))?;
        
        let path = get_project_dir(name);
        
        if path.exists() {
            return Ok(serde_json::to_vec(&serde_json::json!({
                "status": "error",
                "message": format!("Project '{}' already exists at {:?}", name, path)
            })).unwrap());
        }

        tokio::fs::create_dir_all(&path).await.map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        let meta_file = path.join("meta.json");
        tokio::fs::write(&meta_file, "{\n  \"features\": [],\n  \"modules\": [],\n  \"contracts\": [],\n  \"tasks\": []\n}").await.map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        Ok(serde_json::to_vec(&serde_json::json!({
            "status": "success",
            "path": path.to_string_lossy(),
            "message": format!("Project '{}' created successfully", name)
        })).unwrap())
    }
}

impl ProjectCreateTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "create_project".into(),
            description: "Creates a new managed project workspace directory in ~/.telos/projects/ and initializes its meta.json.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "The unique name of the project" },
                        "description": { "type": "string", "description": "Short description" }
                    },
                    "required": ["name"]
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}

// 2. Read Project Meta Tool
#[derive(Clone)]
pub struct ProjectMetaReadTool;

#[async_trait]
impl ToolExecutor for ProjectMetaReadTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'name'".into()))?;
        
        let meta_file = get_project_dir(name).join("meta.json");
        if !meta_file.exists() {
            return Err(ToolError::ExecutionFailed(format!("meta.json not found for {}", name)));
        }

        let content = tokio::fs::read_to_string(&meta_file).await.map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        Ok(content.into_bytes())
    }
}

impl ProjectMetaReadTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "read_project_meta".into(),
            description: "Reads the L1-L3 meta-graph data (meta.json) for a given project.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Project name" }
                    },
                    "required": ["name"]
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}

// 3. Write Project Meta Tool
#[derive(Clone)]
pub struct ProjectMetaWriteTool;

#[async_trait]
impl ToolExecutor for ProjectMetaWriteTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'name'".into()))?;
            
        let meta_json = params
            .get("meta_json")
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'meta_json'".into()))?;
        
        let meta_file = get_project_dir(name).join("meta.json");
        let content = serde_json::to_string_pretty(meta_json).map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        
        tokio::fs::write(&meta_file, content).await.map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        
        Ok(serde_json::to_vec(&serde_json::json!({
            "status": "success",
            "message": "Project meta updated successfully"
        })).unwrap())
    }
}

impl ProjectMetaWriteTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "update_project_meta".into(),
            description: "Updates the L1-L3 meta-graph data (meta.json) for a given project. You must provide the entire JSON graph object to overwrite it.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Project name" },
                        "meta_json": { "type": "object", "description": "The entire meta graph object containing features, modules, contracts, tasks arrays." }
                    },
                    "required": ["name", "meta_json"]
                }),
            },
            risk_level: RiskLevel::HighRisk,
            ..Default::default()
        }
    }
}

// 4. Project Iteration Tool
#[derive(Clone)]
pub struct ProjectIterateTool;

#[async_trait]
impl ToolExecutor for ProjectIterateTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let name = params.get("name").and_then(|v| v.as_str()).ok_or_else(|| ToolError::ExecutionFailed("Missing 'name'".into()))?;
        let updated_contract = params.get("updated_contract").ok_or_else(|| ToolError::ExecutionFailed("Missing 'updated_contract'".into()))?;
        
        // 1. Load meta.json
        let meta_file = get_project_dir(name).join("meta.json");
        if !meta_file.exists() {
            return Err(ToolError::ExecutionFailed(format!("meta.json not found for {}", name)));
        }
        let content = tokio::fs::read_to_string(&meta_file).await.map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        let mut meta: Value = serde_json::from_str(&content).map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        
        let contract_id = updated_contract.get("id").and_then(|v| v.as_str()).unwrap_or("");
        
        let mut cascading_tasks = vec![];
        let mut affected_modules = vec![];

        // 2. Diff and Update Contracts
        if let Some(contracts) = meta.get_mut("contracts").and_then(|v| v.as_array_mut()) {
            for c in contracts.iter_mut() {
                if c.get("id").and_then(|v| v.as_str()) == Some(contract_id) {
                    *c = updated_contract.clone();
                    // Identify dependent modules
                    if let Some(consumers) = c.get("consumer_module_ids").and_then(|v| v.as_array()) {
                        for consumer in consumers {
                            if let Some(c_id) = consumer.as_str() {
                                affected_modules.push(c_id.to_string());
                            }
                        }
                    }
                    if let Some(provider) = c.get("provider_module_id").and_then(|v| v.as_str()) {
                        affected_modules.push(provider.to_string());
                    }
                }
            }
        }
        
        // 3. Mark affected modules as Outdated
        if let Some(modules) = meta.get_mut("modules").and_then(|v| v.as_array_mut()) {
            for m in modules.iter_mut() {
                if let Some(m_id) = m.get("id").and_then(|v| v.as_str()) {
                    if affected_modules.contains(&m_id.to_string()) {
                        m["status"] = serde_json::json!("Proposed"); // Downgrading status
                    }
                }
            }
        }

        // 4. Generate new DevTasks for ScrumMasterAgent
        if let Some(tasks) = meta.get_mut("tasks").and_then(|v| v.as_array_mut()) {
            for mod_id in &affected_modules {
                let new_task = serde_json::json!({
                    "id": format!("task_iter_{}_{}", contract_id, uuid::Uuid::new_v4().to_string().chars().take(6).collect::<String>()),
                    "title": format!("Implement updated contract {}", contract_id),
                    "belong_to_module": mod_id,
                    "target_file": "TBD",
                    "instruction": format!("The contract {} has been iteratively updated. Ensure this module complies with the new strict schema bindings.", contract_id),
                    "enforced_contracts": [contract_id],
                    "status": "Todo",
                    "harness_feedback": []
                });
                tasks.push(new_task.clone());
                cascading_tasks.push(new_task);
            }
        }

        // 5. Save new snapshot
        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let backup_file = get_project_dir(name).join(format!("meta.v{}.json", timestamp));
        tokio::fs::write(&backup_file, content).await.ok(); // Write backup

        // Write new meta
        let new_content = serde_json::to_string_pretty(&meta).map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        tokio::fs::write(&meta_file, new_content).await.map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        Ok(serde_json::to_vec(&serde_json::json!({
            "status": "success",
            "message": format!("Contract iterated. Cascaded {} new DevTasks.", affected_modules.len()),
            "affected_modules": affected_modules,
            "new_tasks": cascading_tasks
        })).unwrap())
    }
}

impl ProjectIterateTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "iterate_project_contract".into(),
            description: "Updates a Contract in a project and auto-generates cascading DevTasks for all provider/consumer TechModules affected by the change.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Project name" },
                        "updated_contract": { "type": "object", "description": "The modified Contract object." }
                    },
                    "required": ["name", "updated_contract"]
                }),
            },
            risk_level: RiskLevel::HighRisk,
            ..Default::default()
        }
    }
}
