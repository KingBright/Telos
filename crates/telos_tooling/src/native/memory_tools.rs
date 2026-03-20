use crate::{JsonSchema, ToolError, ToolExecutor, ToolSchema};
use tracing::{info, debug, warn, error};
use async_trait::async_trait;
use serde_json::Value;
use telos_core::RiskLevel;
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

