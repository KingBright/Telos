use crate::{JsonSchema, ToolError, ToolExecutor, ToolSchema};
use tracing::{info, debug, warn, error};
use async_trait::async_trait;
use serde_json::Value;
use std::fs;
use telos_core::RiskLevel;
use tokio::process::Command;

// --- Built-in Native Tools ---

// 1. File Reader Tool
#[derive(Clone)]
pub struct FsReadTool;

#[async_trait]
impl ToolExecutor for FsReadTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'path' parameter".into()))?;

        match fs::read_to_string(path) {
            Ok(content) => Ok(content.into_bytes()),
            Err(e) => Err(ToolError::ExecutionFailed(format!(
                "Failed to read file: {}",
                e
            ))),
        }
    }
}

impl FsReadTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "fs_read".into(),
            description: "Reads the content of a file from the disk. Requires a 'path' parameter."
                .into(),
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
            ..Default::default()
        }
    }
}

// 2. File Writer Tool
#[derive(Clone)]
pub struct FsWriteTool;

#[async_trait]
impl ToolExecutor for FsWriteTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'path' parameter".into()))?;

        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'content' parameter".into()))?;

        match fs::write(path, content) {
            Ok(_) => Ok(b"{\"status\":\"success\"}".to_vec()),
            Err(e) => Err(ToolError::ExecutionFailed(format!(
                "Failed to write file: {}",
                e
            ))),
        }
    }
}

impl FsWriteTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "fs_write".into(),
            description:
                "Writes content to a file on the disk. Requires 'path' and 'content' parameters."
                    .into(),
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
            ..Default::default()
        }
    }
}

// 3. Shell Execution Tool
#[derive(Clone)]
pub struct ShellExecTool;

#[async_trait]
impl ToolExecutor for ShellExecTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'command' parameter".into()))?;

        let output = Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to execute shell command: {}", e))
            })?;

        let result = if output.status.success() {
            String::from_utf8_lossy(&output.stdout).to_string()
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ToolError::ExecutionFailed(format!(
                "Command failed with error: {}",
                stderr
            )));
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
            ..Default::default()
        }
    }
}

// 4. Calculator Tool
#[derive(Clone)]
pub struct CalculatorTool;

#[async_trait]
impl ToolExecutor for CalculatorTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let expression = params
            .get("expression")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'expression' parameter".into()))?;

        // Use evalexpr for safe mathematical expression evaluation
        // For now, use a simple approach with basic operations
        let result = Self::evaluate_expression(expression)?;

        let output = serde_json::json!({
            "expression": expression,
            "result": result
        });

        Ok(serde_json::to_vec(&output)
            .unwrap_or_else(|_| format!("{{\"result\": {}}}", result).into_bytes()))
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
            ..Default::default()
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
            return Err(ToolError::ExecutionFailed(
                "Unexpected end of expression".into(),
            ));
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
                return Err(ToolError::ExecutionFailed(
                    "Missing closing parenthesis".into(),
                ));
            }
            *pos += 1;
            return Ok(result);
        }

        // Handle functions
        if c.is_alphabetic() {
            let start = *pos;
            while *pos < expr.len()
                && (expr.chars().nth(*pos).unwrap().is_alphanumeric()
                    || expr.chars().nth(*pos).unwrap() == '_')
            {
                *pos += 1;
            }
            let func_name = &expr[start..*pos];

            if *pos < expr.len() && expr.chars().nth(*pos).unwrap() == '(' {
                *pos += 1;
                let arg = Self::parse_expression(expr, pos)?;
                if *pos >= expr.len() || expr.chars().nth(*pos).unwrap() != ')' {
                    return Err(ToolError::ExecutionFailed(
                        "Missing closing parenthesis for function".into(),
                    ));
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
                    _ => {
                        return Err(ToolError::ExecutionFailed(format!(
                            "Unknown function: {}",
                            func_name
                        )))
                    }
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
            return Err(ToolError::ExecutionFailed(format!(
                "Expected number at position {}",
                *pos
            )));
        }

        let num_str = &expr[start..*pos];
        num_str
            .parse::<f64>()
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
        let wasm_path = params
            .get("wasm_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'wasm_path' parameter".into()))?;

        let schema_json = params
            .get("schema")
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'schema' parameter".into()))?;

        let _name = schema_json
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("unknown");

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
            ..Default::default()
        }
    }
}

// 6. Memory Recall Tool
#[derive(Clone)]
pub struct MemoryRecallTool;

#[async_trait]
impl ToolExecutor for MemoryRecallTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let query = params
            .get("query")
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
            ..Default::default()
        }
    }
}

// 7. Memory Store Tool
#[derive(Clone)]
pub struct MemoryStoreTool;

#[async_trait]
impl ToolExecutor for MemoryStoreTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let content = params
            .get("content")
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
            ..Default::default()
        }
    }
}

// 8. File Edit Tool
#[derive(Clone)]
pub struct FileEditTool;

#[async_trait]
impl ToolExecutor for FileEditTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'path'".into()))?;
        let search = params
            .get("search")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'search'".into()))?;
        let replace = params
            .get("replace")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'replace'".into()))?;

        // If file doesn't exist, treat it as empty.
        let mut content = std::fs::read_to_string(path).unwrap_or_else(|_| String::new());
        
        let modified_content = if search.is_empty() {
            // Overwrite if search string is empty
            replace.to_string()
        } else if content.contains(search) {
            content.replace(search, replace)
        } else {
            return Err(ToolError::ExecutionFailed(
                "Search string not found in file".into(),
            ));
        };

        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }

        std::fs::write(path, modified_content).map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to write {}: {}", path, e))
        })?;

        Ok(b"{\"status\": \"success\", \"message\": \"File updated successfully\"}".to_vec())
    }
}

impl FileEditTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "file_edit".into(),
            description: "Edits a file by replacing a search string with a replacement string."
                .into(),
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
            ..Default::default()
        }
    }
}

// 9. Glob Tool
#[derive(Clone)]
pub struct GlobTool;

#[async_trait]
impl ToolExecutor for GlobTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let pattern = params
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'pattern'".into()))?;

        let output = Command::new("find")
            .arg(".")
            .arg("-name")
            .arg(pattern)
            .output()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Command failed: {}", e)))?;

        Ok(output.stdout)
    }
}

impl GlobTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "glob".into(),
            description:
                "Finds files matching a pattern using 'find . -name <pattern>'. Example: '*.rs'"
                    .into(),
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
            ..Default::default()
        }
    }
}

// 10. Grep Tool
#[derive(Clone)]
pub struct GrepTool;

#[async_trait]
impl ToolExecutor for GrepTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let pattern = params
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'pattern'".into()))?;
        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let output = Command::new("grep")
            .arg("-rn")
            .arg(pattern)
            .arg(path)
            .output()
            .await
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
            ..Default::default()
        }
    }
}

// 11. Http Tool
#[derive(Clone)]
pub struct HttpTool;

#[async_trait]
impl ToolExecutor for HttpTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'url'".into()))?;

        let output = reqwest::get(url)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("HTTP request failed: {}", e)))?
            .bytes()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read response body: {}", e)))?;

        Ok(output.to_vec())
    }
}

impl HttpTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "http_get".into(),
            description: "Fetches the content of a URL.".into(),
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
            ..Default::default()
        }
    }
}

// 12. WebSearch Tool - 多搜索引擎支持
#[derive(Clone)]
pub struct WebSearchTool;

impl WebSearchTool {
    fn create_client() -> Result<reqwest::Client, ToolError> {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|e| ToolError::ExecutionFailed(format!("Client build failed: {:?}", e)))
    }

    /// Create client with proxy support
    fn create_client_with_proxy(proxy_url: &str) -> Result<reqwest::Client, ToolError> {
        let proxy = reqwest::Proxy::all(proxy_url)
            .map_err(|e| ToolError::ExecutionFailed(format!("Invalid proxy URL: {:?}", e)))?;
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::limited(5))
            .proxy(proxy)
            .build()
            .map_err(|e| ToolError::ExecutionFailed(format!("Client build failed: {:?}", e)))
    }

    /// Get proxy URL from environment (TELOS_PROXY, HTTP_PROXY, or HTTPS_PROXY)
    fn get_proxy_url() -> Option<String> {
        std::env::var("TELOS_PROXY")
            .or_else(|_| std::env::var("HTTPS_PROXY"))
            .or_else(|_| std::env::var("HTTP_PROXY"))
            .ok()
    }

    /// 使用 DuckDuckGo 搜索
    async fn search_duckduckgo(query: &str, client: &reqwest::Client) -> Result<Vec<String>, ToolError> {
        let encoded_query = urlencoding::encode(query);
        let search_url = format!("https://html.duckduckgo.com/html/?q={}", encoded_query);

        let html = client.get(&search_url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("DuckDuckGo request failed: {:?}", e)))?
            .text()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read response: {:?}", e)))?;

        let mut results = Vec::new();
        for line in html.split('\n') {
            if line.contains("class=\"result__snippet\"") {
                let text = line
                    .replace("<a", "")
                    .replace("</a>", "")
                    .replace("<b>", "")
                    .replace("</b>", "")
                    .replace("class=\"result__snippet\"", "");
                let clean = text
                    .split('>')
                    .map(|s| s.split('<').next().unwrap_or(""))
                    .collect::<Vec<&str>>()
                    .join("");
                let trimmed = clean.trim().to_string();
                if !trimmed.is_empty() {
                    results.push(trimmed);
                }
            }
        }
        Ok(results)
    }

    /// 使用 Google 搜索（需要代理，是最好的搜索引擎）
    async fn search_google(query: &str, client: &reqwest::Client) -> Result<Vec<String>, ToolError> {
        let encoded_query = urlencoding::encode(query);
        let search_url = format!("https://www.google.com/search?q={}", encoded_query);

        let html = client.get(&search_url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .header("Accept-Language", "en-US,en;q=0.9,zh-CN;q=0.8,zh;q=0.7")
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Google request failed: {:?}", e)))?
            .text()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read response: {:?}", e)))?;

        let mut results = Vec::new();
        // Parse Google search results - look for snippet containers
        for line in html.split('\n') {
            // Google uses various classes for snippets
            if line.contains("data-sncf") || line.contains("class=\"VwiC3b") || line.contains("class=\"st") {
                let text = line
                    .replace("<em>", "")
                    .replace("</em>", "")
                    .replace("<b>", "")
                    .replace("</b>", "")
                    .replace("&amp;", "&")
                    .replace("&nbsp;", " ");
                let clean: String = text
                    .chars()
                    .skip_while(|c| *c != '>')
                    .skip(1)
                    .take_while(|c| *c != '<')
                    .collect();
                let trimmed = clean.trim().to_string();
                if !trimmed.is_empty() && trimmed.len() > 20 && !trimmed.starts_with("http") {
                    results.push(trimmed);
                }
            }
        }
        Ok(results)
    }

    /// 使用百度搜索（国内优先）
    async fn search_baidu(query: &str, client: &reqwest::Client) -> Result<Vec<String>, ToolError> {
        let encoded_query = urlencoding::encode(query);
        let search_url = format!("https://www.baidu.com/s?wd={}", encoded_query);

        let html = client.get(&search_url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .header("Accept-Language", "zh-CN,zh;q=0.9")
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Baidu request failed: {:?}", e)))?
            .text()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read response: {:?}", e)))?;

        let mut results = Vec::new();
        for line in html.split('\n') {
            if line.contains("class=\"c-abstract\"") || line.contains("class=\"content-right") {
                let clean = line
                    .replace("<em>", "")
                    .replace("</em>", "")
                    .replace("&nbsp;", " ");
                let text: String = clean
                    .chars()
                    .skip_while(|c| *c != '>')
                    .skip(1)
                    .take_while(|c| *c != '<')
                    .collect();
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() && trimmed.len() > 10 {
                    results.push(trimmed);
                }
            }
        }
        Ok(results)
    }

    /// 使用必应搜索（备选）
    async fn search_bing(query: &str, client: &reqwest::Client) -> Result<Vec<String>, ToolError> {
        let encoded_query = urlencoding::encode(query);
        let search_url = format!("https://www.bing.com/search?q={}", encoded_query);

        let html = client.get(&search_url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Bing request failed: {:?}", e)))?
            .text()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read response: {:?}", e)))?;

        let mut results = Vec::new();
        for line in html.split('\n') {
            if line.contains("class=\"b_caption\"") || line.contains("class=\"b_algoSlug\"") {
                let clean = line.replace("<p>", "").replace("</p>", "").replace("<strong>", "").replace("</strong>", "");
                let text: String = clean
                    .split('>')
                    .skip(1)
                    .flat_map(|s| s.split('<').next().unwrap_or("").chars())
                    .collect();
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() && trimmed.len() > 10 {
                    results.push(trimmed);
                }
            }
        }
        Ok(results)
    }
}

#[async_trait]
impl ToolExecutor for WebSearchTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'query'".into()))?;

        // Check if proxy should be used
        // Default: use proxy when available, unless explicitly set to false
        let proxy_url = Self::get_proxy_url();
        let skip_proxy = params
            .get("use_proxy")
            .map(|v| !v.as_bool().unwrap_or(true))
            .unwrap_or(false);

        // Create appropriate client
        let (client, using_proxy) = if !skip_proxy {
            if let Some(ref proxy) = proxy_url {
                info!("[WebSearch] 🌐 Using proxy: {}", proxy);
                match Self::create_client_with_proxy(proxy) {
                    Ok(c) => (c, true),
                    Err(e) => {
                        warn!("[WebSearch] ⚠️ Proxy creation failed: {:?}, using direct connection", e);
                        (Self::create_client()?, false)
                    }
                }
            } else {
                debug!("[WebSearch] 📍 No proxy configured, using direct connection");
                (Self::create_client()?, false)
            }
        } else {
            debug!("[WebSearch] 📍 Proxy skipped, using direct connection");
            (Self::create_client()?, false)
        };

        // Determine search engine order based on proxy setting
        // When using proxy: Google -> DuckDuckGo -> Bing -> Baidu
        // Without proxy: Baidu -> Bing -> DuckDuckGo
        let mut last_error_msg = String::new();

        if using_proxy {
            // International order with proxy: Google -> DuckDuckGo -> Bing -> Baidu
            // 1. Try Google (best search engine)
            match Self::search_google(query, &client).await {
                Ok(results) if !results.is_empty() => {
                    info!("[WebSearch] ✅ Google 返回 {} 条结果", results.len());
                    return Ok(serde_json::to_vec(&results).unwrap_or_default());
                }
                Ok(_) => warn!("[WebSearch] ⚠️ Google 返回空结果，尝试下一个引擎"),
                Err(e) => {
                    last_error_msg = format!("Google: {:?}", e);
                    warn!("[WebSearch] ❌ Google 失败，尝试下一个引擎");
                }
            }

            // 2. Try DuckDuckGo
            match Self::search_duckduckgo(query, &client).await {
                Ok(results) if !results.is_empty() => {
                    info!("[WebSearch] ✅ DuckDuckGo 返回 {} 条结果", results.len());
                    return Ok(serde_json::to_vec(&results).unwrap_or_default());
                }
                Ok(_) => warn!("[WebSearch] ⚠️ DuckDuckGo 返回空结果，尝试下一个引擎"),
                Err(e) => {
                    last_error_msg = format!("DuckDuckGo: {:?}", e);
                    warn!("[WebSearch] ❌ DuckDuckGo 失败，尝试下一个引擎");
                }
            }

            // 3. Try Bing
            match Self::search_bing(query, &client).await {
                Ok(results) if !results.is_empty() => {
                    info!("[WebSearch] ✅ Bing 返回 {} 条结果", results.len());
                    return Ok(serde_json::to_vec(&results).unwrap_or_default());
                }
                Ok(_) => warn!("[WebSearch] ⚠️ Bing 返回空结果，尝试下一个引擎"),
                Err(e) => {
                    last_error_msg = format!("Bing: {:?}", e);
                    warn!("[WebSearch] ❌ Bing 失败，尝试下一个引擎");
                }
            }

            // 4. Try Baidu
            match Self::search_baidu(query, &client).await {
                Ok(results) if !results.is_empty() => {
                    info!("[WebSearch] ✅ Baidu 返回 {} 条结果", results.len());
                    return Ok(serde_json::to_vec(&results).unwrap_or_default());
                }
                Ok(_) => warn!("[WebSearch] ⚠️ Baidu 返回空结果"),
                Err(e) => {
                    last_error_msg = format!("Baidu: {:?}", e);
                    error!("[WebSearch] ❌ Baidu 失败");
                }
            }
        } else {
            // Domestic order without proxy: Baidu -> Bing -> DuckDuckGo
            // 1. Try Baidu
            match Self::search_baidu(query, &client).await {
                Ok(results) if !results.is_empty() => {
                    info!("[WebSearch] ✅ Baidu 返回 {} 条结果", results.len());
                    return Ok(serde_json::to_vec(&results).unwrap_or_default());
                }
                Ok(_) => warn!("[WebSearch] ⚠️ Baidu 返回空结果，尝试下一个引擎"),
                Err(e) => {
                    last_error_msg = format!("Baidu: {:?}", e);
                    warn!("[WebSearch] ❌ Baidu 失败，尝试下一个引擎");
                }
            }

            // 2. Try Bing
            match Self::search_bing(query, &client).await {
                Ok(results) if !results.is_empty() => {
                    info!("[WebSearch] ✅ Bing 返回 {} 条结果", results.len());
                    return Ok(serde_json::to_vec(&results).unwrap_or_default());
                }
                Ok(_) => warn!("[WebSearch] ⚠️ Bing 返回空结果，尝试下一个引擎"),
                Err(e) => {
                    last_error_msg = format!("Bing: {:?}", e);
                    warn!("[WebSearch] ❌ Bing 失败，尝试下一个引擎");
                }
            }

            // 3. Try DuckDuckGo
            match Self::search_duckduckgo(query, &client).await {
                Ok(results) if !results.is_empty() => {
                    info!("[WebSearch] ✅ DuckDuckGo 返回 {} 条结果", results.len());
                    return Ok(serde_json::to_vec(&results).unwrap_or_default());
                }
                Ok(_) => warn!("[WebSearch] ⚠️ DuckDuckGo 返回空结果"),
                Err(e) => {
                    last_error_msg = format!("DuckDuckGo: {:?}", e);
                    error!("[WebSearch] ❌ DuckDuckGo 失败");
                }
            }
        }

        if last_error_msg.is_empty() {
            Err(ToolError::ExecutionFailed("All search engines returned empty results. This may occur if the network blocks the crawler. Try simpler keywords.".into()))
        } else {
            Err(ToolError::ExecutionFailed(format!("All search engines failed. Last error: {}", last_error_msg)))
        }
    }
}

impl WebSearchTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "web_search".into(),
            description: "Searches the web using multiple search engines with automatic fallback. When proxy is configured: uses Google -> DuckDuckGo -> Bing -> Baidu. Without proxy: uses Baidu -> Bing -> DuckDuckGo (better for Chinese users). Set use_proxy=false to skip proxy when needed.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "The search query" },
                        "use_proxy": { "type": "boolean", "description": "Whether to use proxy (default: true when proxy is configured). Set to false to force direct connection for domestic content." }
                    },
                    "required": ["query"]
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}

// 13. Lsp Tool
#[derive(Clone)]
pub struct LspTool;

#[async_trait]
impl ToolExecutor for LspTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let symbol = params
            .get("symbol")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'symbol'".into()))?;

        let pattern = format!("(fn|struct|enum|trait) {}", symbol);
        let output = tokio::process::Command::new("grep")
            .arg("-rnE")
            .arg(&pattern)
            .arg(".")
            .output()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Command failed: {}", e)))?;

        Ok(output.stdout)
    }
}

impl LspTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "lsp_symbol_search".into(),
            description:
                "Finds definitions of symbols (structs, fns, enums) across the repository.".into(),
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
            ..Default::default()
        }
    }
}

// 14. Web Scrape Tool
#[derive(Clone)]
pub struct WebScrapeTool;

#[async_trait]
impl ToolExecutor for WebScrapeTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'url'".into()))?;

        let client = reqwest::Client::new();
        let html_content = client.get(url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to fetch URL: {}", e)))?
            .text()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read HTML: {}", e)))?;

        let document = scraper::Html::parse_document(&html_content);
        let mut clean_text = String::new();
        
        for node in document.tree.nodes() {
            if let Some(text_node) = node.value().as_text() {
                let mut should_ignore = false;
                for parent in node.ancestors() {
                    if let Some(elem) = scraper::ElementRef::wrap(parent) {
                        let tag = elem.value().name();
                        if matches!(tag, "script" | "style" | "noscript" | "head") {
                            should_ignore = true;
                            break;
                        }
                    }
                }
                
                if !should_ignore {
                    let ts = text_node.trim();
                    if !ts.is_empty() {
                        clean_text.push_str(ts);
                        clean_text.push(' ');
                    }
                }
            }
        }

        Ok(clean_text.into_bytes())
    }
}

impl WebScrapeTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "web_scrape".into(),
            description: "Fetches a webpage and extracts clean readability text. Keywords: scrape, web_scrape, fetch, html, text, extraction.".into(),
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
            ..Default::default()
        }
    }
}

// 15. Get Location Tool
#[derive(Clone)]
pub struct GetLocationTool;

#[async_trait]
impl ToolExecutor for GetLocationTool {
    async fn call(&self, _params: Value) -> Result<Vec<u8>, ToolError> {
        let response = reqwest::get("http://ip-api.com/json/")
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to fetch location: {}", e)))?
            .json::<serde_json::Value>()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to parse location JSON: {}", e)))?;

        let location = serde_json::json!({
            "lat": response.get("lat"),
            "lon": response.get("lon"),
            "country": response.get("country"),
            "province": response.get("regionName"),
            "city": response.get("city")
        });

        Ok(serde_json::to_vec(&location).unwrap_or_default())
    }
}

impl GetLocationTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "get_location".into(),
            description: "Gets the current geographical location based on IP. Keywords: location, geolocation, lat, lon, country, province, city, get_location.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}

// 16. Get Time Tool
#[derive(Clone)]
pub struct GetTimeTool;

#[async_trait]
impl ToolExecutor for GetTimeTool {
    async fn call(&self, _params: Value) -> Result<Vec<u8>, ToolError> {
        let now = std::time::SystemTime::now();
        let timestamp_ms = now.duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let formatted_time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S.%3f").to_string();

        let result = serde_json::json!({
            "formatted_time": formatted_time,
            "timestamp_ms": timestamp_ms
        });

        Ok(serde_json::to_vec(&result).unwrap_or_default())
    }
}

impl GetTimeTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "get_time".into(),
            description: "Gets the current local time. Keywords: time, get_time, get_current_time, clock, current, date, timestamp.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}
