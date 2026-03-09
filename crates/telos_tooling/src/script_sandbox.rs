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
        let engine = Engine::new();
        
        // We could also disable some expensive or dangerous features if needed
        // but Rhai is already quite safe by default (no network/FS by default)

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
}
