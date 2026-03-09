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

// 4. Calculator Tool
#[derive(Clone)]
pub struct CalculatorTool;

#[async_trait]
impl ToolExecutor for CalculatorTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let expression = params.get("expression")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'expression' parameter".into()))?;

        // Use evalexpr for safe mathematical expression evaluation
        // For now, use a simple approach with basic operations
        let result = Self::evaluate_expression(expression)?;

        let output = serde_json::json!({
            "expression": expression,
            "result": result
        });

        Ok(serde_json::to_vec(&output).unwrap_or_else(|_| format!("{{\"result\": {}}}", result).into_bytes()))
    }
}

impl CalculatorTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "calculator".into(),
            description: "Evaluates mathematical expressions. Supports basic operations (+, -, *, /, ^), functions (sqrt, sin, cos, tan, log, exp, abs), and constants (pi, e). Requires an 'expression' parameter.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "expression": {
                            "type": "string",
                            "description": "Mathematical expression to evaluate, e.g., '2+2', 'sqrt(16)', 'sin(pi/2)'"
                        }
                    },
                    "required": ["expression"]
                }),
            },
            risk_level: RiskLevel::Normal,
        }
    }

    /// Safely evaluate a mathematical expression
    fn evaluate_expression(expr: &str) -> Result<f64, ToolError> {
        let expr = expr.trim().replace(" ", "");

        // Handle common constants
        let expr = expr.replace("pi", &std::f64::consts::PI.to_string());
        let expr = expr.replace("e", &std::f64::consts::E.to_string());

        // Simple recursive descent parser for basic expressions
        Self::parse_expression(&expr, &mut 0)
    }

    fn parse_expression(expr: &str, pos: &mut usize) -> Result<f64, ToolError> {
        let mut result = Self::parse_term(expr, pos)?;

        while *pos < expr.len() {
            let c = expr.chars().nth(*pos).unwrap();
            if c == '+' {
                *pos += 1;
                let term = Self::parse_term(expr, pos)?;
                result += term;
            } else if c == '-' {
                *pos += 1;
                let term = Self::parse_term(expr, pos)?;
                result -= term;
            } else {
                break;
            }
        }

        Ok(result)
    }

    fn parse_term(expr: &str, pos: &mut usize) -> Result<f64, ToolError> {
        let mut result = Self::parse_factor(expr, pos)?;

        while *pos < expr.len() {
            let c = expr.chars().nth(*pos).unwrap();
            if c == '*' {
                *pos += 1;
                let factor = Self::parse_factor(expr, pos)?;
                result *= factor;
            } else if c == '/' {
                *pos += 1;
                let factor = Self::parse_factor(expr, pos)?;
                if factor == 0.0 {
                    return Err(ToolError::ExecutionFailed("Division by zero".into()));
                }
                result /= factor;
            } else if c == '^' {
                *pos += 1;
                let factor = Self::parse_factor(expr, pos)?;
                result = result.powf(factor);
            } else {
                break;
            }
        }

        Ok(result)
    }

    fn parse_factor(expr: &str, pos: &mut usize) -> Result<f64, ToolError> {
        // Skip whitespace (already removed, but just in case)
        while *pos < expr.len() && expr.chars().nth(*pos).unwrap().is_whitespace() {
            *pos += 1;
        }

        if *pos >= expr.len() {
            return Err(ToolError::ExecutionFailed("Unexpected end of expression".into()));
        }

        let c = expr.chars().nth(*pos).unwrap();

        // Handle negative numbers
        if c == '-' {
            *pos += 1;
            return Ok(-Self::parse_factor(expr, pos)?);
        }

        // Handle parentheses
        if c == '(' {
            *pos += 1;
            let result = Self::parse_expression(expr, pos)?;
            if *pos >= expr.len() || expr.chars().nth(*pos).unwrap() != ')' {
                return Err(ToolError::ExecutionFailed("Missing closing parenthesis".into()));
            }
            *pos += 1;
            return Ok(result);
        }

        // Handle functions
        if c.is_alphabetic() {
            let start = *pos;
            while *pos < expr.len() && (expr.chars().nth(*pos).unwrap().is_alphanumeric() || expr.chars().nth(*pos).unwrap() == '_') {
                *pos += 1;
            }
            let func_name = &expr[start..*pos];

            if *pos < expr.len() && expr.chars().nth(*pos).unwrap() == '(' {
                *pos += 1;
                let arg = Self::parse_expression(expr, pos)?;
                if *pos >= expr.len() || expr.chars().nth(*pos).unwrap() != ')' {
                    return Err(ToolError::ExecutionFailed("Missing closing parenthesis for function".into()));
                }
                *pos += 1;

                let result = match func_name {
                    "sqrt" => arg.sqrt(),
                    "sin" => arg.sin(),
                    "cos" => arg.cos(),
                    "tan" => arg.tan(),
                    "log" => arg.ln(),
                    "log10" => arg.log10(),
                    "exp" => arg.exp(),
                    "abs" => arg.abs(),
                    "floor" => arg.floor(),
                    "ceil" => arg.ceil(),
                    "round" => arg.round(),
                    _ => return Err(ToolError::ExecutionFailed(format!("Unknown function: {}", func_name))),
                };
                return Ok(result);
            }
        }

        // Handle numbers
        let start = *pos;
        while *pos < expr.len() {
            let c = expr.chars().nth(*pos).unwrap();
            if c.is_ascii_digit() || c == '.' {
                *pos += 1;
            } else {
                break;
            }
        }

        if start == *pos {
            return Err(ToolError::ExecutionFailed(format!("Expected number at position {}", *pos)));
        }

        let num_str = &expr[start..*pos];
        num_str.parse::<f64>()
            .map_err(|e| ToolError::ExecutionFailed(format!("Invalid number '{}': {}", num_str, e)))
    }
}

// 5. Tool Register Tool
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

// 6. Memory Recall Tool
#[derive(Clone)]
pub struct MemoryRecallTool;

#[async_trait]
impl ToolExecutor for MemoryRecallTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let query = params.get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'query' parameter".into()))?;

        let out = serde_json::json!({
            "__macro__": "memory_recall",
            "query": query
        });

        Ok(serde_json::to_vec(&out).unwrap())
    }
}

impl MemoryRecallTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "memory_recall".into(),
            description: "Retrieves important semantic facts and historical context from the agent's long-term memory. Requires a 'query' parameter.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "The concept or entity to search for in long-term memory" }
                    },
                    "required": ["query"]
                }),
            },
            risk_level: RiskLevel::Normal,
        }
    }
}

// 7. Memory Store Tool
#[derive(Clone)]
pub struct MemoryStoreTool;

#[async_trait]
impl ToolExecutor for MemoryStoreTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let content = params.get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'content' parameter".into()))?;

        let out = serde_json::json!({
            "__macro__": "memory_store",
            "content": content
        });

        Ok(serde_json::to_vec(&out).unwrap())
    }
}

impl MemoryStoreTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "memory_store".into(),
            description: "Stores an important fact or insight into the agent's long-term semantic memory. Requires a 'content' parameter.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "content": { "type": "string", "description": "The exact fact, insight, or information to remember permanently" }
                    },
                    "required": ["content"]
                }),
            },
            risk_level: RiskLevel::Normal,
        }
    }
}

// 8. File Edit Tool
#[derive(Clone)]
pub struct FileEditTool;

#[async_trait]
impl ToolExecutor for FileEditTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let path = params.get("path").and_then(|v| v.as_str()).ok_or_else(|| ToolError::ExecutionFailed("Missing 'path'".into()))?;
        let search = params.get("search").and_then(|v| v.as_str()).ok_or_else(|| ToolError::ExecutionFailed("Missing 'search'".into()))?;
        let replace = params.get("replace").and_then(|v| v.as_str()).ok_or_else(|| ToolError::ExecutionFailed("Missing 'replace'".into()))?;

        let mut content = std::fs::read_to_string(path).map_err(|e| ToolError::ExecutionFailed(format!("Failed to read {}: {}", path, e)))?;
        if content.contains(search) {
            content = content.replace(search, replace);
            std::fs::write(path, content).map_err(|e| ToolError::ExecutionFailed(format!("Failed to write {}: {}", path, e)))?;
            Ok(b"{\"status\": \"success\", \"message\": \"Replaced occurrences\"}".to_vec())
        } else {
            Err(ToolError::ExecutionFailed("Search string not found in file".into()))
        }
    }
}

impl FileEditTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "file_edit".into(),
            description: "Edits a file by replacing a search string with a replacement string.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "search": { "type": "string" },
                        "replace": { "type": "string" }
                    },
                    "required": ["path", "search", "replace"]
                }),
            },
            risk_level: RiskLevel::HighRisk,
        }
    }
}

// 9. Glob Tool
#[derive(Clone)]
pub struct GlobTool;

#[async_trait]
impl ToolExecutor for GlobTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let pattern = params.get("pattern").and_then(|v| v.as_str()).ok_or_else(|| ToolError::ExecutionFailed("Missing 'pattern'".into()))?;

        let output = std::process::Command::new("find")
            .arg(".")
            .arg("-name")
            .arg(pattern)
            .output()
            .map_err(|e| ToolError::ExecutionFailed(format!("Command failed: {}", e)))?;

        Ok(output.stdout)
    }
}

impl GlobTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "glob".into(),
            description: "Finds files matching a pattern using 'find . -name <pattern>'. Example: '*.rs'".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string" }
                    },
                    "required": ["pattern"]
                }),
            },
            risk_level: RiskLevel::Normal,
        }
    }
}

// 10. Grep Tool
#[derive(Clone)]
pub struct GrepTool;

#[async_trait]
impl ToolExecutor for GrepTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let pattern = params.get("pattern").and_then(|v| v.as_str()).ok_or_else(|| ToolError::ExecutionFailed("Missing 'pattern'".into()))?;
        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let output = std::process::Command::new("grep")
            .arg("-rn")
            .arg(pattern)
            .arg(path)
            .output()
            .map_err(|e| ToolError::ExecutionFailed(format!("Command failed: {}", e)))?;

        Ok(output.stdout)
    }
}

impl GrepTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "grep".into(),
            description: "Searches for a pattern in files using 'grep -rn <pattern> <path>'. Returns matches with line numbers.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string" },
                        "path": { "type": "string", "description": "Defaults to '.'" }
                    },
                    "required": ["pattern"]
                }),
            },
            risk_level: RiskLevel::Normal,
        }
    }
}

// 11. Http Tool
#[derive(Clone)]
pub struct HttpTool;

#[async_trait]
impl ToolExecutor for HttpTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let url = params.get("url").and_then(|v| v.as_str()).ok_or_else(|| ToolError::ExecutionFailed("Missing 'url'".into()))?;

        let output = std::process::Command::new("curl")
            .arg("-sL")
            .arg(url)
            .output()
            .map_err(|e| ToolError::ExecutionFailed(format!("Command failed: {}", e)))?;

        Ok(output.stdout)
    }
}

impl HttpTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "http_get".into(),
            description: "Fetches the content of a URL using curl.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string" }
                    },
                    "required": ["url"]
                }),
            },
            risk_level: RiskLevel::Normal,
        }
    }
}

// 12. WebSearch Tool
#[derive(Clone)]
pub struct WebSearchTool;

#[async_trait]
impl ToolExecutor for WebSearchTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let query = params.get("query").and_then(|v| v.as_str()).ok_or_else(|| ToolError::ExecutionFailed("Missing 'query'".into()))?;
        let encoded_query = urlencoding::encode(query);
        let search_url = format!("https://html.duckduckgo.com/html/?q={}", encoded_query);

        let output = std::process::Command::new("curl")
            .arg("-sL")
            .arg("-H")
            .arg("User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
            .arg(&search_url)
            .output()
            .map_err(|e| ToolError::ExecutionFailed(format!("Command failed: {}", e)))?;

        let html = String::from_utf8_lossy(&output.stdout);
        let mut results = Vec::new();
        for line in html.split('\n') {
            if line.contains("class=\"result__snippet\"") {
                let text = line.replace("<a", "").replace("</a>", "").replace("<b>", "").replace("</b>", "").replace("class=\"result__snippet\"", "");
                let clean = text.split('>').map(|s| s.split('<').next().unwrap_or("")).collect::<Vec<&str>>().join("");
                results.push(clean.trim().to_string());
            }
        }

        Ok(serde_json::to_vec(&results).unwrap_or_default())
    }
}

impl WebSearchTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "web_search".into(),
            description: "Searches the web for a query and returns snippets.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" }
                    },
                    "required": ["query"]
                }),
            },
            risk_level: RiskLevel::Normal,
        }
    }
}

// 13. Lsp Tool
#[derive(Clone)]
pub struct LspTool;

#[async_trait]
impl ToolExecutor for LspTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let symbol = params.get("symbol").and_then(|v| v.as_str()).ok_or_else(|| ToolError::ExecutionFailed("Missing 'symbol'".into()))?;

        let pattern = format!("(fn|struct|enum|trait) {}", symbol);
        let output = std::process::Command::new("grep")
            .arg("-rnE")
            .arg(&pattern)
            .arg(".")
            .output()
            .map_err(|e| ToolError::ExecutionFailed(format!("Command failed: {}", e)))?;

        Ok(output.stdout)
    }
}

impl LspTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "lsp_symbol_search".into(),
            description: "Finds definitions of symbols (structs, fns, enums) across the repository.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "symbol": { "type": "string" }
                    },
                    "required": ["symbol"]
                }),
            },
            risk_level: RiskLevel::Normal,
        }
    }
}
