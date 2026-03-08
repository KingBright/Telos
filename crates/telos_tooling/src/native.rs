use crate::{JsonSchema, ToolError, ToolExecutor, ToolSchema};
use async_trait::async_trait;
use serde_json::Value;
use std::fs;
use std::process::Command;
use telos_core::RiskLevel;

// --- Built-in Native Tools ---

// 1. File Reader Tool
#[derive(Clone)]
pub struct FsReadTool;

#[async_trait]
impl ToolExecutor for FsReadTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let path = params.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'path' parameter".into()))?;

        match fs::read_to_string(path) {
            Ok(content) => Ok(content.into_bytes()),
            Err(e) => Err(ToolError::ExecutionFailed(format!("Failed to read file: {}", e))),
        }
    }
}

impl FsReadTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "fs_read".into(),
            description: "Reads the content of a file from the disk. Requires a 'path' parameter.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    },
                    "required": ["path"]
                }),
            },
            risk_level: RiskLevel::Normal,
        }
    }
}

// 2. File Writer Tool
#[derive(Clone)]
pub struct FsWriteTool;

#[async_trait]
impl ToolExecutor for FsWriteTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let path = params.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'path' parameter".into()))?;

        let content = params.get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'content' parameter".into()))?;

        match fs::write(path, content) {
            Ok(_) => Ok(b"{\"status\":\"success\"}".to_vec()),
            Err(e) => Err(ToolError::ExecutionFailed(format!("Failed to write file: {}", e))),
        }
    }
}

impl FsWriteTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "fs_write".into(),
            description: "Writes content to a file on the disk. Requires 'path' and 'content' parameters.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"]
                }),
            },
            risk_level: RiskLevel::HighRisk,
        }
    }
}

// 3. Shell Execution Tool
#[derive(Clone)]
pub struct ShellExecTool;

#[async_trait]
impl ToolExecutor for ShellExecTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let command = params.get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'command' parameter".into()))?;

        let output = Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to execute shell command: {}", e)))?;

        let result = if output.status.success() {
            String::from_utf8_lossy(&output.stdout).to_string()
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ToolError::ExecutionFailed(format!("Command failed with error: {}", stderr)));
        };

        Ok(result.into_bytes())
    }
}

impl ShellExecTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "shell_exec".into(),
            description: "Executes a shell command on the host OS. Useful for compiling code. Requires a 'command' parameter.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "command": { "type": "string" }
                    },
                    "required": ["command"]
                }),
            },
            risk_level: RiskLevel::HighRisk,
        }
    }
}

// 4. Tool Register Tool
// Allows dynamic registration of newly compiled Wasm modules.
// Because it needs to mutate the registry, we'll implement this with a reference or a specific API design later,
// as the registry is behind RwLock and managed globally.
// For now, returning a schema indicator so the DAEMON knows to handle it specifically as a macro-tool.
#[derive(Clone)]
pub struct ToolRegisterTool;

#[async_trait]
impl ToolExecutor for ToolRegisterTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        // This tool is currently a "Macro" intercepted by the daemon or handled via a dedicated channel.
        // It outputs the request for the host to process the actual registry mutation.
        let wasm_path = params.get("wasm_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'wasm_path' parameter".into()))?;

        let schema_json = params.get("schema")
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'schema' parameter".into()))?;

        let _name = schema_json.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");

        // We serialize this special JSON so the host (Daemon) can intercept it in the ExecutionEngine
        let out = serde_json::json!({
            "__macro__": "register_tool",
            "wasm_path": wasm_path,
            "schema": schema_json
        });

        Ok(serde_json::to_vec(&out).unwrap())
    }
}

impl ToolRegisterTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "register_tool".into(),
            description: "Registers a newly compiled WebAssembly tool into the system registry. Requires 'wasm_path' and 'schema' parameters.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "wasm_path": { "type": "string", "description": "Absolute or relative path to the .wasm file" },
                        "schema": { "type": "object", "description": "The ToolSchema JSON describing the new tool" }
                    },
                    "required": ["wasm_path", "schema"]
                }),
            },
            risk_level: RiskLevel::HighRisk,
        }
    }
}

// 5. File System List Directory Tool
#[derive(Clone)]
pub struct FsListDirTool;

#[async_trait]
impl ToolExecutor for FsListDirTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let path = params.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'path' parameter".into()))?;

        let dir = fs::read_dir(path)
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read directory: {}", e)))?;

        let mut entries = Vec::new();
        for entry in dir.flatten() {
                let metadata = entry.metadata().ok();
                let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);

                entries.push(serde_json::json!({
                    "name": entry.file_name().to_string_lossy().to_string(),
                    "is_dir": is_dir,
                    "size": size
                }));
        }

        let result = serde_json::to_vec(&entries)
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to serialize result: {}", e)))?;

        Ok(result)
    }
}

impl FsListDirTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "fs_list_dir".into(),
            description: "Lists the contents of a directory. Returns a JSON array of files and folders. Requires a 'path' parameter.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    },
                    "required": ["path"]
                }),
            },
            risk_level: RiskLevel::Normal,
        }
    }
}

// 6. Code Search Tool (Recursive Grep)
#[derive(Clone)]
pub struct CodeSearchTool;

#[async_trait]
impl ToolExecutor for CodeSearchTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let path = params.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'path' parameter".into()))?;

        let pattern = params.get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'pattern' parameter".into()))?;

        let root_path = std::path::Path::new(path);
        if !root_path.exists() {
             return Err(ToolError::ExecutionFailed(format!("Path does not exist: {}", path)));
        }

        let mut results = Vec::new();
        let mut to_visit = vec![root_path.to_path_buf()];

        while let Some(current_dir) = to_visit.pop() {
            if let Ok(entries) = fs::read_dir(&current_dir) {
                for entry in entries.flatten() {
                    let entry_path = entry.path();

                    if entry_path.is_dir() {
                        to_visit.push(entry_path);
                    } else if entry_path.is_file() {
                         if let Ok(file) = std::fs::File::open(&entry_path) {
                            use std::io::{BufRead, BufReader};
                            let reader = BufReader::new(file);

                            for (index, line) in reader.lines().enumerate() {
                                if let Ok(line_text) = line {
                                    if line_text.contains(pattern) {
                                        results.push(serde_json::json!({
                                            "file": entry_path.to_string_lossy().to_string(),
                                            "line_number": index + 1,
                                            "text": line_text.trim().to_string()
                                        }));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let result_bytes = serde_json::to_vec(&results)
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to serialize result: {}", e)))?;

        Ok(result_bytes)
    }
}

impl CodeSearchTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "code_search".into(),
            description: "Recursively searches for a text pattern in all files under a given path. Returns a JSON array of matches. Requires 'path' and 'pattern' parameters.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "pattern": { "type": "string" }
                    },
                    "required": ["path", "pattern"]
                }),
            },
            risk_level: RiskLevel::Normal,
        }
    }
}
