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

// 12. WebSearch Tool - 多搜索引擎支持 (结构化输出)

/// Structured search result with title, URL, and snippet
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

impl SearchResult {
    fn clean_text(s: &str) -> String {
        s.replace("&amp;", "&")
            .replace("&nbsp;", " ")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .trim()
            .to_string()
    }
}

#[derive(Clone)]
pub struct WebSearchTool;

impl WebSearchTool {
    fn create_client() -> Result<reqwest::Client, ToolError> {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|e| ToolError::ExecutionFailed(format!("Client build failed: {:?}", e)))
    }

    /// Create client with proxy support
    fn create_client_with_proxy(proxy_url: &str) -> Result<reqwest::Client, ToolError> {
        let proxy = reqwest::Proxy::all(proxy_url)
            .map_err(|e| ToolError::ExecutionFailed(format!("Invalid proxy URL: {:?}", e)))?;
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
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

    /// Fetch HTML from a search engine URL
    async fn fetch_html(client: &reqwest::Client, url: &str, accept_lang: &str) -> Result<String, ToolError> {
        client.get(url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .header("Accept-Language", accept_lang)
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Request failed: {:?}", e)))?
            .text()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read response: {:?}", e)))
    }

    /// 使用 DuckDuckGo 搜索 — CSS selector + fallback
    async fn search_duckduckgo(query: &str, client: &reqwest::Client) -> Result<Vec<SearchResult>, ToolError> {
        let encoded_query = urlencoding::encode(query);

        // Try HTML endpoint first, then lite endpoint
        let urls = [
            format!("https://html.duckduckgo.com/html/?q={}", encoded_query),
            format!("https://lite.duckduckgo.com/lite/?q={}", encoded_query),
        ];

        for search_url in &urls {
            let html = match Self::fetch_html(client, search_url, "en-US,en;q=0.9,zh-CN;q=0.8").await {
                Ok(h) => h,
                Err(_) => continue,
            };

            // Check for bot detection
            if html.contains("bots use DuckDuckGo") || html.contains("blocked") {
                warn!("[WebSearch] DuckDuckGo bot detection triggered on {}", search_url);
                continue;
            }

            let document = scraper::Html::parse_document(&html);

            // Strategy 1: CSS selectors
            let result_sel = scraper::Selector::parse(".result").unwrap();
            let title_sel = scraper::Selector::parse(".result__a").unwrap();
            let snippet_sel = scraper::Selector::parse(".result__snippet").unwrap();
            let url_sel = scraper::Selector::parse(".result__url").unwrap();

            let mut results = Vec::new();
            for result in document.select(&result_sel).take(10) {
                let title = result.select(&title_sel).next()
                    .map(|e| SearchResult::clean_text(&e.text().collect::<String>()))
                    .unwrap_or_default();
                let url = result.select(&title_sel).next()
                    .and_then(|e| e.value().attr("href"))
                    .map(|href| {
                        if let Some(pos) = href.find("uddg=") {
                            urlencoding::decode(&href[pos + 5..]).unwrap_or_default().to_string()
                        } else if href.starts_with("http") {
                            href.to_string()
                        } else {
                            result.select(&url_sel).next()
                                .map(|e| {
                                    let u = e.text().collect::<String>().trim().to_string();
                                    if !u.starts_with("http") { format!("https://{}", u) } else { u }
                                })
                                .unwrap_or_default()
                        }
                    })
                    .unwrap_or_default();
                let snippet = result.select(&snippet_sel).next()
                    .map(|e| SearchResult::clean_text(&e.text().collect::<String>()))
                    .unwrap_or_default();

                if !title.is_empty() && !snippet.is_empty() {
                    results.push(SearchResult { title, url, snippet });
                }
            }

            if !results.is_empty() {
                return Ok(results);
            }

            // Strategy 2: Fallback — old line-based string matching (for HTML structure changes)
            debug!("[WebSearch] DDG CSS selectors found nothing, trying string fallback. HTML len={}", html.len());
            let mut fallback_results = Vec::new();
            for line in html.split('\n') {
                if line.contains("result__snippet") || line.contains("result-snippet") {
                    let text = line.replace("<b>", "").replace("</b>", "").replace("<a", "").replace("</a>", "");
                    let clean: String = text
                        .split('>')
                        .map(|s| s.split('<').next().unwrap_or(""))
                        .collect::<Vec<&str>>()
                        .join("")
                        .trim().to_string();
                    if !clean.is_empty() && clean.len() > 20 {
                        fallback_results.push(SearchResult { title: String::new(), url: String::new(), snippet: clean });
                    }
                }
            }
            if !fallback_results.is_empty() {
                info!("[WebSearch] DDG fallback parser found {} results", fallback_results.len());
                return Ok(fallback_results);
            }

            // If lite endpoint HTML has results in <td> tags (lite format)
            for line in html.split('\n') {
                if line.contains("class=\"result-link\"") || line.contains("class=\"link-text\"") {
                    let text: String = line.split('>').skip(1)
                        .flat_map(|s| s.split('<').next().unwrap_or("").chars())
                        .collect();
                    let trimmed = text.trim().to_string();
                    if !trimmed.is_empty() && trimmed.len() > 10 {
                        fallback_results.push(SearchResult { title: trimmed, url: String::new(), snippet: String::new() });
                    }
                }
            }
            if !fallback_results.is_empty() {
                return Ok(fallback_results);
            }

            debug!("[WebSearch] DDG no results from '{}'. HTML preview: {}", search_url, &html.chars().take(300).collect::<String>());
        }
        Ok(vec![])
    }

    /// 使用 Bing 搜索 — CSS selector + fallback
    async fn search_bing(query: &str, client: &reqwest::Client) -> Result<Vec<SearchResult>, ToolError> {
        let encoded_query = urlencoding::encode(query);
        let search_url = format!("https://cn.bing.com/search?q={}&ensearch=0", encoded_query);
        let html = Self::fetch_html(client, &search_url, "zh-CN,zh;q=0.9,en;q=0.8").await?;

        let document = scraper::Html::parse_document(&html);

        // Strategy 1: CSS selectors (try multiple known selectors)
        let result_selectors = [".b_algo", "li.b_algo", ".b_results .b_algo"];
        let mut results = Vec::new();

        for sel_str in &result_selectors {
            if let Ok(result_sel) = scraper::Selector::parse(sel_str) {
                let title_sel = scraper::Selector::parse("h2 a").unwrap();
                let snippet_sel = scraper::Selector::parse("p, .b_caption p, .b_lineclamp2, .b_algoSlug").unwrap();

                for result in document.select(&result_sel).take(10) {
                    let title_elem = result.select(&title_sel).next();
                    let title = title_elem
                        .map(|e| SearchResult::clean_text(&e.text().collect::<String>()))
                        .unwrap_or_default();
                    let url = title_elem
                        .and_then(|e| e.value().attr("href"))
                        .unwrap_or("")
                        .to_string();
                    let snippet = result.select(&snippet_sel).next()
                        .map(|e| SearchResult::clean_text(&e.text().collect::<String>()))
                        .unwrap_or_default();

                    if !title.is_empty() && !url.is_empty() {
                        results.push(SearchResult { title, url, snippet });
                    }
                }
                if !results.is_empty() {
                    return Ok(results);
                }
            }
        }

        // Strategy 2: Fallback string matching
        debug!("[WebSearch] Bing CSS selectors found nothing. HTML len={}", html.len());
        for line in html.split('\n') {
            if line.contains("class=\"b_caption\"") || line.contains("class=\"b_algoSlug\"") || line.contains("class=\"b_lineclamp") {
                let clean = line.replace("<p>", "").replace("</p>", "").replace("<strong>", "").replace("</strong>", "");
                let text: String = clean
                    .split('>')
                    .skip(1)
                    .flat_map(|s| s.split('<').next().unwrap_or("").chars())
                    .collect();
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() && trimmed.len() > 10 {
                    results.push(SearchResult { title: String::new(), url: String::new(), snippet: trimmed });
                }
            }
        }
        if !results.is_empty() {
            info!("[WebSearch] Bing fallback parser found {} results", results.len());
        } else {
            debug!("[WebSearch] Bing no results. HTML preview: {}", &html.chars().take(300).collect::<String>());
        }
        Ok(results)
    }

    /// 使用百度搜索 — CSS selector + fallback
    async fn search_baidu(query: &str, client: &reqwest::Client) -> Result<Vec<SearchResult>, ToolError> {
        let encoded_query = urlencoding::encode(query);
        let search_url = format!("https://www.baidu.com/s?wd={}", encoded_query);
        let html = Self::fetch_html(client, &search_url, "zh-CN,zh;q=0.9").await?;

        let document = scraper::Html::parse_document(&html);

        // Strategy 1: CSS selectors
        let result_selectors = [".result.c-container", ".c-container", "div.result"];
        let mut results = Vec::new();

        for sel_str in &result_selectors {
            if let Ok(result_sel) = scraper::Selector::parse(sel_str) {
                let title_sel = scraper::Selector::parse("h3 a").unwrap();
                let snippet_sel = scraper::Selector::parse(".c-abstract, .c-span-last, .content-right_8Sakl").unwrap();

                for result in document.select(&result_sel).take(10) {
                    let title_elem = result.select(&title_sel).next();
                    let title = title_elem
                        .map(|e| SearchResult::clean_text(&e.text().collect::<String>()))
                        .unwrap_or_default();
                    let url = title_elem
                        .and_then(|e| e.value().attr("href"))
                        .unwrap_or("")
                        .to_string();
                    let snippet = result.select(&snippet_sel).next()
                        .map(|e| SearchResult::clean_text(&e.text().collect::<String>()))
                        .unwrap_or_default();

                    if !title.is_empty() {
                        results.push(SearchResult { title, url, snippet });
                    }
                }
                if !results.is_empty() {
                    return Ok(results);
                }
            }
        }

        // Strategy 2: fallback string matching
        debug!("[WebSearch] Baidu CSS selectors found nothing. HTML len={}", html.len());
        for line in html.split('\n') {
            if line.contains("class=\"c-abstract\"") || line.contains("class=\"content-right") {
                let clean = line.replace("<em>", "").replace("</em>", "").replace("&nbsp;", " ");
                let text: String = clean.chars()
                    .skip_while(|c| *c != '>')
                    .skip(1)
                    .take_while(|c| *c != '<')
                    .collect();
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() && trimmed.len() > 10 {
                    results.push(SearchResult { title: String::new(), url: String::new(), snippet: trimmed });
                }
            }
        }
        if !results.is_empty() {
            info!("[WebSearch] Baidu fallback parser found {} results", results.len());
        } else {
            debug!("[WebSearch] Baidu no results. HTML preview: {}", &html.chars().take(300).collect::<String>());
        }
        Ok(results)
    }

    /// 使用 Google 搜索（需要代理，反爬严格，作为备选）
    async fn search_google(query: &str, client: &reqwest::Client) -> Result<Vec<SearchResult>, ToolError> {
        let encoded_query = urlencoding::encode(query);
        let search_url = format!("https://www.google.com/search?q={}&hl=zh-CN", encoded_query);
        let html = Self::fetch_html(client, &search_url, "zh-CN,zh;q=0.9,en;q=0.8").await?;

        let document = scraper::Html::parse_document(&html);
        // Google's structure: div.g contains h3 (title link) and div.VwiC3b (snippet)
        let result_sel = scraper::Selector::parse("div.g").unwrap();
        let title_sel = scraper::Selector::parse("h3").unwrap();
        let link_sel = scraper::Selector::parse("a[href]").unwrap();
        let snippet_sel = scraper::Selector::parse("div.VwiC3b, span.st").unwrap();

        let mut results = Vec::new();
        for result in document.select(&result_sel).take(10) {
            let title = result.select(&title_sel).next()
                .map(|e| SearchResult::clean_text(&e.text().collect::<String>()))
                .unwrap_or_default();
            let url = result.select(&link_sel).next()
                .and_then(|e| e.value().attr("href"))
                .map(|h| {
                    // Google sometimes wraps URLs like /url?q=<actual>&sa=...
                    if h.starts_with("/url?q=") {
                        h[7..].split('&').next().unwrap_or(h)
                            .to_string()
                    } else {
                        h.to_string()
                    }
                })
                .unwrap_or_default();
            let snippet = result.select(&snippet_sel).next()
                .map(|e| SearchResult::clean_text(&e.text().collect::<String>()))
                .unwrap_or_default();

            if !title.is_empty() && url.starts_with("http") {
                results.push(SearchResult { title, url, snippet });
            }
        }
        Ok(results)
    }

    /// 使用 Serper API 搜索 (serper.dev) — 直接返回结构化 JSON
    async fn search_serper(query: &str) -> Result<Vec<SearchResult>, ToolError> {
        let api_key = std::env::var("SERPER_API_KEY")
            .map_err(|_| ToolError::ExecutionFailed("SERPER_API_KEY not set".into()))?;

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| ToolError::ExecutionFailed(format!("Client build failed: {:?}", e)))?;

        let body = serde_json::json!({
            "q": query,
            "gl": "cn",
            "hl": "zh-cn",
            "num": 10
        });

        let resp = client.post("https://google.serper.dev/search")
            .header("X-API-KEY", &api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Serper request failed: {:?}", e)))?;

        if !resp.status().is_success() {
            return Err(ToolError::ExecutionFailed(format!("Serper returned HTTP {}", resp.status())));
        }

        let data: serde_json::Value = resp.json().await
            .map_err(|e| ToolError::ExecutionFailed(format!("Serper JSON decode failed: {:?}", e)))?;

        let mut results = Vec::new();
        if let Some(organic) = data.get("organic").and_then(|v| v.as_array()) {
            for item in organic.iter().take(10) {
                let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let url = item.get("link").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let snippet = item.get("snippet").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if !title.is_empty() {
                    results.push(SearchResult { title, url, snippet });
                }
            }
        }

        // Also check answerBox for quick answers (weather, etc.)
        if let Some(answer_box) = data.get("answerBox") {
            let title = answer_box.get("title").and_then(|v| v.as_str()).unwrap_or("Answer").to_string();
            let snippet = answer_box.get("answer")
                .or_else(|| answer_box.get("snippet"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !snippet.is_empty() {
                results.insert(0, SearchResult { title, url: String::new(), snippet });
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

        // Tier 1: Serper API (if API key configured) — fastest, most reliable
        let has_serper = std::env::var("SERPER_API_KEY").is_ok();
        if has_serper {
            match Self::search_serper(query).await {
                Ok(results) if !results.is_empty() => {
                    info!("[WebSearch] ✅ Serper API 返回 {} 条结构化结果", results.len());
                    return Ok(serde_json::to_vec(&results).unwrap_or_default());
                }
                Ok(_) => warn!("[WebSearch] ⚠️ Serper API 返回空结果，降级到网页搜索引擎"),
                Err(e) => warn!("[WebSearch] ⚠️ Serper API 失败: {:?}，降级到网页搜索引擎", e),
            }
        }

        // Tier 2: Web scraping fallback
        // KEY INSIGHT: Domestic engines (cn.bing.com, Baidu) MUST connect directly (no proxy) —
        // they are fast and reliable in China. Proxy only helps for international engines (DDG).
        // Google is removed: it returns JS-rendered HTML with no parseable results via HTTP.
        
        let direct_client = Self::create_client()?;
        
        let proxy_url = Self::get_proxy_url();
        let proxy_client = if let Some(ref proxy) = proxy_url {
            match Self::create_client_with_proxy(proxy) {
                Ok(c) => Some(c),
                Err(e) => {
                    warn!("[WebSearch] ⚠️ Proxy client creation failed: {:?}", e);
                    None
                }
            }
        } else {
            None
        };

        let max_retries = 2;
        let mut last_error_msg = String::new();

        // Helper macro for trying a search engine with a specific client
        macro_rules! try_engine {
            ($name:expr, $method:ident, $client:expr) => {
                match Self::$method(query, $client).await {
                    Ok(results) if !results.is_empty() => {
                        info!("[WebSearch] ✅ {} 返回 {} 条结构化结果", $name, results.len());
                        return Ok(serde_json::to_vec(&results).unwrap_or_default());
                    }
                    Ok(_) => warn!("[WebSearch] ⚠️ {} 返回空结果，尝试下一个引擎", $name),
                    Err(e) => {
                        last_error_msg = format!("{}: {:?}", $name, e);
                        warn!("[WebSearch] ❌ {} 失败: {}", $name, last_error_msg);
                    }
                }
            };
        }

        for attempt in 1..=max_retries {
            if attempt > 1 {
                let sleep_time = std::time::Duration::from_secs(2);
                warn!("[WebSearch] 🔄 Retry {}/{}...", attempt, max_retries);
                tokio::time::sleep(sleep_time).await;
            }

            // 1. Bing CN — ALWAYS direct (domestic, fast, 0.3s typical)
            try_engine!("Bing CN", search_bing, &direct_client);
            
            // 2. Baidu — ALWAYS direct (domestic, reliable)
            try_engine!("Baidu", search_baidu, &direct_client);
            
            // 3. DuckDuckGo — ONLY through proxy (blocked in China without proxy)
            if let Some(ref pc) = proxy_client {
                try_engine!("DuckDuckGo", search_duckduckgo, pc);
            }
        }

        if last_error_msg.is_empty() {
            Err(ToolError::ExecutionFailed("All search engines returned empty results. Try simpler keywords.".into()))
        } else {
            Err(ToolError::ExecutionFailed(format!("All search engines failed after {} retries. Last error: {}", max_retries, last_error_msg)))
        }
    }
}

impl WebSearchTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "web_search".into(),
            description: "Searches the web. Uses Serper API (Google-quality results) when SERPER_API_KEY is configured. Falls back to Bing/Baidu web scraping. Set use_proxy=false to force direct connection.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "The search query" },
                        "use_proxy": { "type": "boolean", "description": "Whether to use proxy for web scraping fallback (default: true when proxy is configured)." }
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

// 14. Web Scrape Tool — 增强版 (代理/超时/Readability/截断)
#[derive(Clone)]
pub struct WebScrapeTool;

impl WebScrapeTool {
    /// Get proxy URL from environment (reuses same env vars as WebSearchTool)
    fn get_proxy_url() -> Option<String> {
        std::env::var("TELOS_PROXY")
            .or_else(|_| std::env::var("HTTPS_PROXY"))
            .or_else(|_| std::env::var("HTTP_PROXY"))
            .ok()
    }

    fn create_client() -> Result<reqwest::Client, ToolError> {
        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::limited(5));

        if let Some(proxy_url) = Self::get_proxy_url() {
            if let Ok(proxy) = reqwest::Proxy::all(&proxy_url) {
                debug!("[WebScrape] Using proxy: {}", proxy_url);
                builder = builder.proxy(proxy);
            }
        }

        builder.build()
            .map_err(|e| ToolError::ExecutionFailed(format!("Client build failed: {:?}", e)))
    }

    /// Extract main content from HTML using readability heuristics
    fn extract_readable_content(document: &scraper::Html) -> String {
        // Priority order for content extraction:
        // 1. <article> element (most semantic)
        // 2. <main> element
        // 3. Elements with content-like class/id names
        // 4. Largest text block fallback

        let content_selectors = [
            "article",
            "main",
            "[role=\"main\"]",
            ".post-content",
            ".article-content",
            ".entry-content",
            ".content",
            "#content",
            ".post-body",
            ".article-body",
        ];

        for sel_str in &content_selectors {
            if let Ok(sel) = scraper::Selector::parse(sel_str) {
                if let Some(elem) = document.select(&sel).next() {
                    let text = Self::extract_clean_text_from_element(&elem);
                    if text.len() > 200 {
                        return text;
                    }
                }
            }
        }

        // Fallback: extract all body text, filtering out noise
        Self::extract_body_text(document)
    }

    /// Extract clean text from an element, skipping script/style/nav/footer
    fn extract_clean_text_from_element(elem: &scraper::ElementRef) -> String {
        let skip_tags = ["script", "style", "noscript", "nav", "footer", "header", "aside", "form", "iframe"];
        let mut text = String::new();

        for node in elem.descendants() {
            if let Some(text_node) = node.value().as_text() {
                // Check if any ancestor is a skip tag
                let mut should_skip = false;
                for parent in node.ancestors() {
                    if let Some(parent_elem) = scraper::ElementRef::wrap(parent) {
                        if skip_tags.contains(&parent_elem.value().name()) {
                            should_skip = true;
                            break;
                        }
                    }
                }
                if !should_skip {
                    let t = text_node.trim();
                    if !t.is_empty() {
                        text.push_str(t);
                        text.push(' ');
                    }
                }
            }
        }
        text
    }

    /// Extract text from body, filtering common noise elements
    fn extract_body_text(document: &scraper::Html) -> String {
        let mut clean_text = String::new();
        let skip_tags = ["script", "style", "noscript", "head", "nav", "footer", "header", "aside", "form", "iframe"];

        for node in document.tree.nodes() {
            if let Some(text_node) = node.value().as_text() {
                let mut should_ignore = false;
                for parent in node.ancestors() {
                    if let Some(elem) = scraper::ElementRef::wrap(parent) {
                        let tag = elem.value().name();
                        if skip_tags.contains(&tag) {
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
        clean_text
    }

    /// Extract page title
    fn extract_title(document: &scraper::Html) -> String {
        if let Ok(sel) = scraper::Selector::parse("title") {
            if let Some(elem) = document.select(&sel).next() {
                return elem.text().collect::<String>().trim().to_string();
            }
        }
        String::new()
    }
}

#[async_trait]
impl ToolExecutor for WebScrapeTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'url'".into()))?;

        let client = Self::create_client()?;
        let html_content = client.get(url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to fetch URL: {}", e)))?
            .text()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read HTML: {}", e)))?;

        let document = scraper::Html::parse_document(&html_content);
        let title = Self::extract_title(&document);
        let content = Self::extract_readable_content(&document);

        // Truncate to max 5000 chars to avoid overwhelming LLM
        let max_chars = 5000;
        let truncated: String = content.chars().take(max_chars).collect();
        let was_truncated = content.len() > max_chars;
        let word_count = truncated.split_whitespace().count();

        let result = serde_json::json!({
            "title": title,
            "url": url,
            "content": truncated,
            "word_count": word_count,
            "truncated": was_truncated,
        });

        info!("[WebScrape] Extracted {} chars from '{}' (title: '{}')", truncated.len(), url, title);
        Ok(serde_json::to_vec(&result).unwrap_or_else(|_| truncated.into_bytes()))
    }
}

impl WebScrapeTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "web_scrape".into(),
            description: "Fetches a webpage and extracts clean readable content using smart content extraction. Returns structured JSON with title, content, and word count. Supports proxy. Keywords: scrape, web_scrape, fetch, html, text, extraction.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "The URL to scrape" }
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
