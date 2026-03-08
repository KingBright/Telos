pub mod native;
pub mod sandbox;
pub mod retrieval;

use telos_core::RiskLevel;
use async_trait::async_trait;
use serde_json::Value;

use serde::{Serialize, Deserialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonSchema {
    pub raw_schema: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters_schema: JsonSchema,
    pub risk_level: RiskLevel,
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
}

pub trait ToolRegistry: Send + Sync {
    fn discover_tools(&self, intent: &str, limit: usize) -> Vec<ToolSchema>;
    fn get_executor(&self, tool_name: &str) -> Option<std::sync::Arc<dyn ToolExecutor>>;
}

#[cfg(test)]
mod tests;
