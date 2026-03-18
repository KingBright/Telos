#[cfg(feature = "full")]
pub mod native;
#[cfg(feature = "full")]
pub mod retrieval;
#[cfg(feature = "full")]
pub mod script_sandbox;

use async_trait::async_trait;
use serde_json::Value;
use telos_core::RiskLevel;

use serde::{Deserialize, Serialize};

#[cfg(feature = "full")]
pub use script_sandbox::{ScriptExecutor, ScriptSandbox};
#[cfg(feature = "full")]
pub use retrieval::VectorToolRegistry;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonSchema {
    pub raw_schema: Value,
}

impl Default for JsonSchema {
    fn default() -> Self {
        Self {
            raw_schema: serde_json::json!({}),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub parameters_schema: JsonSchema,
    #[serde(default)]
    pub risk_level: RiskLevel,
    /// Tool version for iteration tracking (defaults to "0.1.0")
    #[serde(default = "default_version")]
    pub version: String,
    /// Iteration count - how many times this tool has been updated
    #[serde(default)]
    pub iteration: u32,
    /// Parent tool name if this is an iteration of another tool
    #[serde(default)]
    pub parent_tool: Option<String>,
    /// Creation/update reason
    #[serde(default)]
    pub change_reason: Option<String>,
}

fn default_version() -> String {
    "0.1.0".to_string()
}

impl ToolSchema {
    /// Create a new tool schema with default version
    pub fn new(name: impl Into<String>, description: impl Into<String>, risk_level: RiskLevel) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters_schema: JsonSchema { raw_schema: serde_json::json!({}) },
            risk_level,
            version: default_version(),
            iteration: 0,
            parent_tool: None,
            change_reason: None,
        }
    }

    /// Create an iteration of this tool with incremented iteration count
    pub fn create_iteration(&self, reason: impl Into<String>) -> Self {
        Self {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters_schema: self.parameters_schema.clone(),
            risk_level: self.risk_level.clone(),
            version: self.version.clone(),
            iteration: self.iteration + 1,
            parent_tool: Some(self.name.clone()),
            change_reason: Some(reason.into()),
        }
    }

    /// Check if this tool is an iteration of another tool
    pub fn is_iteration(&self) -> bool {
        self.iteration > 0 || self.parent_tool.is_some()
    }
}

#[derive(Debug)]
pub enum ToolError {
    ExecutionFailed(String),
    SandboxViolation(String),
    Timeout,
}

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError>;
    
    /// Return the native source code of this tool if it is dynamically generated
    fn source_code(&self) -> Option<String> {
        None
    }
}

#[cfg(feature = "full")]
pub trait ToolRegistry: Send + Sync {
    fn discover_tools(&self, intent: &str, limit: usize) -> Vec<ToolSchema>;
    fn get_executor(&self, tool_name: &str) -> Option<std::sync::Arc<dyn ToolExecutor>>;
    fn list_all_tools(&self) -> Vec<ToolSchema>;
    fn register_dynamic_tool(&self, schema: ToolSchema, executor: std::sync::Arc<dyn ToolExecutor>) -> Result<(), String>;
}

/// A wrapper that allows an Arc<tokio::sync::RwLock<dyn ToolRegistry>> to be used where Arc<dyn ToolRegistry> is expected.
#[cfg(feature = "full")]
pub struct SharedToolRegistry<T: ToolRegistry + ?Sized> {
    inner: std::sync::Arc<tokio::sync::RwLock<T>>,
}

#[cfg(feature = "full")]
impl<T: ToolRegistry + ?Sized> SharedToolRegistry<T> {
    pub fn new(inner: std::sync::Arc<tokio::sync::RwLock<T>>) -> Self {
        Self { inner }
    }
}

#[cfg(feature = "full")]
impl<T: ToolRegistry + ?Sized> ToolRegistry for SharedToolRegistry<T> {
    fn discover_tools(&self, intent: &str, limit: usize) -> Vec<ToolSchema> {
        // We can't do this synchronously if it's a tokio RwLock, but discover_tools is sync.
        // This is a design conflict. However, VectorToolRegistry uses a sync lock internally if needed.
        // In this case, we might need to use a sync RwLock in main.rs instead of tokio's if we want sync discovery.
        // For now, let's assume we can try_read or that we'll change main.rs to use a sync lock for the registry.
        if let Ok(guard) = self.inner.try_read() {
            guard.discover_tools(intent, limit)
        } else {
            vec![]
        }
    }

    fn get_executor(&self, tool_name: &str) -> Option<std::sync::Arc<dyn ToolExecutor>> {
        if let Ok(guard) = self.inner.try_read() {
            guard.get_executor(tool_name)
        } else {
            None
        }
    }

    fn list_all_tools(&self) -> Vec<ToolSchema> {
        if let Ok(guard) = self.inner.try_read() {
            guard.list_all_tools()
        } else {
            vec![]
        }
    }

    fn register_dynamic_tool(&self, schema: ToolSchema, executor: std::sync::Arc<dyn ToolExecutor>) -> Result<(), String> {
        if let Ok(guard) = self.inner.try_read() {
            guard.register_dynamic_tool(schema, executor)
        } else {
            Err("Registry is locked".into())
        }
    }
}

pub fn wrap_tool_registry<T: ToolRegistry + 'static>(inner: std::sync::Arc<tokio::sync::RwLock<T>>) -> std::sync::Arc<dyn ToolRegistry> {
    std::sync::Arc::new(SharedToolRegistry::new(inner))
}

#[cfg(test)]
mod tests;
