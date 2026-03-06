pub mod sandbox;
pub mod retrieval;

use telos_core::RiskLevel;
use async_trait::async_trait;
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct JsonSchema {
    pub raw_schema: Value,
}

#[derive(Clone, Debug)]
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
    fn get_executor(&self, tool_name: &str) -> Option<Box<dyn ToolExecutor>>;
}

#[cfg(test)]
mod tests;
