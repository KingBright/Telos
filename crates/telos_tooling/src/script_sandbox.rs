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

                    let client = builder.build().map_err(|e| e.to_string())?;
                    let resp = client.get(&url_str).send().await.map_err(|e| e.to_string())?;
                    resp.text().await.map_err(|e| e.to_string())
                })
            });
            result.map_err(|e| e.into())
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
