use rhai::{Engine, Scope, Dynamic, serde::to_dynamic};
use serde_json::Value;
use crate::ToolError;
use std::sync::Arc;

/// A lightweight, sandboxed script execution engine for dynamic tools.
pub struct ScriptSandbox {
    engine: Engine,
}

impl ScriptSandbox {
    /// Create a new ScriptSandbox with strict limits.
    pub fn new() -> Self {
        let mut engine = Engine::new();
        
        // Strict Security Limits (Note: some limits require specific cargo features in Rhai)
        // engine.set_max_operations(5000);   
        // engine.set_max_call_levels(10);    
        
        // Register HTTP GET Host Function
        engine.register_fn("http_get", |url: &str| -> Result<String, Box<rhai::EvalAltResult>> {
            let url_str = url.to_string();
            let result = tokio::task::block_in_place(move || {
                tokio::runtime::Handle::current().block_on(async move {
                    let mut builder = reqwest::Client::builder()
                        .timeout(std::time::Duration::from_secs(10));

                    if let Some(proxy_url) = std::env::var("TELOS_PROXY")
                        .or_else(|_| std::env::var("HTTPS_PROXY"))
                        .or_else(|_| std::env::var("HTTP_PROXY")).ok() {
                        if let Ok(proxy) = reqwest::Proxy::all(&proxy_url) {
                            builder = builder.proxy(proxy);
                        }
                    }

                    let client = builder.build().map_err(|e| format!("Client build error: {}", e))?;
                    let resp = client.get(&url_str).send().await.map_err(|e| {
                        if e.is_timeout() {
                            format!("HTTP timeout after 10s for URL: {}", url_str)
                        } else if e.is_connect() {
                            format!("Connection refused for URL: {}", url_str)
                        } else {
                            format!("HTTP request failed for {}: {}", url_str, e)
                        }
                    })?;
                    let status = resp.status();
                    if !status.is_success() {
                        return Err(format!("HTTP {} error from {}", status.as_u16(), url_str));
                    }
                    resp.text().await.map_err(|e| format!("Failed to read response body: {}", e))
                })
            });
            result.map_err(|e| e.into())
        });

        // Register HTTP GET with Fallback: tries multiple URLs sequentially, returns first success
        // Usage in Rhai: http_get_with_fallback("[\"url1\", \"url2\", \"url3\"]")
        engine.register_fn("http_get_with_fallback", |urls_json: &str| -> Result<String, Box<rhai::EvalAltResult>> {
            let urls_str = urls_json.to_string();
            let result = tokio::task::block_in_place(move || {
                tokio::runtime::Handle::current().block_on(async move {
                    let urls: Vec<String> = serde_json::from_str(&urls_str)
                        .map_err(|e| format!("Invalid URL array JSON: {}. Expected format: [\"url1\", \"url2\"]", e))?;
                    
                    if urls.is_empty() {
                        return Err("Empty URL list provided to http_get_with_fallback".to_string());
                    }

                    let mut builder = reqwest::Client::builder()
                        .timeout(std::time::Duration::from_secs(10));

                    if let Some(proxy_url) = std::env::var("TELOS_PROXY")
                        .or_else(|_| std::env::var("HTTPS_PROXY"))
                        .or_else(|_| std::env::var("HTTP_PROXY")).ok() {
                        if let Ok(proxy) = reqwest::Proxy::all(&proxy_url) {
                            builder = builder.proxy(proxy);
                        }
                    }

                    let client = builder.build().map_err(|e| format!("Client build error: {}", e))?;
                    let mut last_err = String::new();

                    for (i, url) in urls.iter().enumerate() {
                        match client.get(url).send().await {
                            Ok(resp) => {
                                if resp.status().is_success() {
                                    match resp.text().await {
                                        Ok(body) => return Ok(body),
                                        Err(e) => {
                                            last_err = format!("URL[{}] {}: body read failed: {}", i, url, e);
                                        }
                                    }
                                } else {
                                    last_err = format!("URL[{}] {}: HTTP {}", i, url, resp.status().as_u16());
                                }
                            }
                            Err(e) => {
                                last_err = format!("URL[{}] {}: {}", i, url, e);
                            }
                        }
                    }

                    Err(format!("All {} URLs failed. Last error: {}", urls.len(), last_err))
                })
            });
            result.map_err(|e| e.into())
        });

        // Register parse_json: converts a JSON string to a Rhai object map
        // Usage in Rhai: let obj = parse_json(json_string); obj["key"]
        engine.register_fn("parse_json", |json_str: &str| -> Result<Dynamic, Box<rhai::EvalAltResult>> {
            let value: Value = serde_json::from_str(json_str)
                .map_err(|e| format!("parse_json failed: {}", e))?;
            to_dynamic(value)
                .map_err(|e| -> Box<rhai::EvalAltResult> { format!("parse_json conversion failed: {}", e).into() })
        });

        // Register to_json: converts a Rhai value back to a JSON string
        engine.register_fn("to_json", |val: Dynamic| -> Result<String, Box<rhai::EvalAltResult>> {
            let json_val: Value = rhai::serde::from_dynamic(&val)
                .map_err(|e| -> Box<rhai::EvalAltResult> { format!("to_json failed: {}", e).into() })?;
            serde_json::to_string(&json_val)
                .map_err(|e| -> Box<rhai::EvalAltResult> { format!("to_json serialization failed: {}", e).into() })
        });

        // Register try_parse_json: safe JSON parsing that returns the raw string on failure
        // Usage in Rhai: let data = try_parse_json(body);
        // If body is valid JSON → returns parsed object map
        // If body is not JSON (e.g. ASCII art, HTML) → returns the original string as-is
        engine.register_fn("try_parse_json", |json_str: &str| -> Dynamic {
            match serde_json::from_str::<Value>(json_str) {
                Ok(value) => to_dynamic(value).unwrap_or_else(|_| Dynamic::from(json_str.to_string())),
                Err(_) => Dynamic::from(json_str.to_string()),
            }
        });

        Self { engine }
    }

    /// Execute a script with the given JSON parameters.
    pub fn execute(&self, script: &str, params: Value) -> Result<Value, ToolError> {
        let mut scope = Scope::new();
        
        // Convert JSON params to Rhai Dynamic
        let dynamic_params = to_dynamic(params)
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to convert params to Rhai: {}", e)))?;
        
        scope.push("params", dynamic_params);

        // Execute the script
        let result: Dynamic = self.engine.eval_with_scope(&mut scope, script)
            .map_err(|e| ToolError::ExecutionFailed(format!("Script execution failed: {}", e)))?;

        // Convert Rhai result back to JSON
        let json_result: Value = rhai::serde::from_dynamic(&result)
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to convert script result to JSON: {}", e)))?;

        Ok(json_result)
    }
}

impl Default for ScriptSandbox {
    fn default() -> Self {
        Self::new()
    }
}

/// Executor for Rhai scripts.
pub struct ScriptExecutor {
    sandbox: Arc<ScriptSandbox>,
    script: String,
    native_tools: Option<Arc<dyn crate::ToolRegistry>>,
}

impl ScriptExecutor {
    pub fn new(script: String, sandbox: Arc<ScriptSandbox>) -> Self {
        Self {
            script,
            sandbox,
            native_tools: None,
        }
    }

    pub fn with_native_tools(mut self, registry: Arc<dyn crate::ToolRegistry>) -> Self {
        self.native_tools = Some(registry);
        self
    }
}

#[async_trait::async_trait]
impl crate::ToolExecutor for ScriptExecutor {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        // Simple execution for now. 
        // In the future, we can handle the __protocol__ loop here if then_process is true.
        let result = self.sandbox.execute(&self.script, params)?;
        
        // Convert JSON result to bytes for the Trait interface
        let bytes = serde_json::to_vec(&result)
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to serialize result: {}", e)))?;
            
        Ok(bytes)
    }

    fn source_code(&self) -> Option<String> {
        Some(self.script.clone())
    }
}
