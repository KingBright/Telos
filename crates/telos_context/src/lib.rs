pub mod providers;
pub mod clustering;
pub mod raptor;
pub mod ast_parser;

use telos_core::NodeResult;
use crate::providers::{EmbeddingProvider, LlmProvider};
use crate::raptor::RaptorTree;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: u64,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct Document {
    pub doc_id: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct RawContext {
    pub history_logs: Vec<LogEntry>,
    pub retrieved_docs: Vec<Document>,
}

#[derive(Debug, Clone)]
pub struct SummaryNode {
    pub summary_text: String,
    pub children_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Fact {
    pub entity: String,
    pub relation: String,
    pub target: String,
}

#[derive(Debug, Clone)]
pub struct ScopedContext {
    pub budget_tokens: usize,
    pub summary_tree: Vec<SummaryNode>,
    pub precise_facts: Vec<Fact>,
}

#[derive(Debug, Clone)]
pub struct NodeRequirement {
    pub required_tokens: usize,
    // Typically, a node also provides a query/prompt to retrieve relevant context
    pub query: String,
}

#[async_trait]
pub trait ContextManager: Send + Sync {
    async fn compress_for_node(&self, raw: &RawContext, node_req: &NodeRequirement) -> Result<ScopedContext, String>;
    async fn ingest_new_info(&mut self, info: NodeResult) -> Result<(), String>;
}

/// The concrete implementation of ContextManager using RAPTOR
pub struct RaptorContextManager {
    tree: RwLock<RaptorTree>,
    embedding_provider: Arc<dyn EmbeddingProvider>,
    llm_provider: Arc<dyn LlmProvider>,
    pub memory_integration: Option<Arc<dyn telos_memory::integration::MemoryIntegration>>,
}

impl RaptorContextManager {
    pub fn new(
        embedding_provider: Arc<dyn EmbeddingProvider>,
        llm_provider: Arc<dyn LlmProvider>,
        memory_integration: Option<Arc<dyn telos_memory::integration::MemoryIntegration>>,
    ) -> Self {
        Self {
            tree: RwLock::new(RaptorTree::new()),
            embedding_provider,
            llm_provider,
            memory_integration,
        }
    }
}

#[async_trait]
impl ContextManager for RaptorContextManager {
    async fn compress_for_node(&self, raw: &RawContext, node_req: &NodeRequirement) -> Result<ScopedContext, String> {
        let tree_read = self.tree.read().await;

        let needs_build = tree_read.nodes.is_empty();
        drop(tree_read);

        if needs_build {
            let mut tree_write = self.tree.write().await;
            if tree_write.nodes.is_empty() {
                let mut combined_text = String::new();
                for log in &raw.history_logs {
                    combined_text.push_str(&log.message);
                    combined_text.push('\n');
                }
                for doc in &raw.retrieved_docs {
                    combined_text.push_str(&doc.content);
                    combined_text.push('\n');
                }

                if !combined_text.is_empty() {
                    tree_write.build(
                        &combined_text,
                        self.embedding_provider.clone(),
                        self.llm_provider.clone(),
                    ).await?;
                }
            }
        }

        let tree_read = self.tree.read().await;
        let retrieved_nodes = tree_read.retrieve(
            &node_req.query,
            node_req.required_tokens,
            self.embedding_provider.clone(),
        ).await?;

        let summary_tree = retrieved_nodes.into_iter().map(|n| SummaryNode {
            summary_text: n.text,
            children_ids: n.children_ids,
        }).collect();

        let mut precise_facts = vec![];
        if let Some(ref mem) = self.memory_integration {
            if let Ok(facts) = mem.retrieve_semantic_facts(node_req.query.clone()).await {
                for f in facts {
                    precise_facts.push(Fact {
                        entity: "Memory".into(),
                        relation: "recalls".into(),
                        target: f,
                    });
                }
            }
        }

        Ok(ScopedContext {
            budget_tokens: node_req.required_tokens,
            summary_tree,
            precise_facts,
        })
    }

    async fn ingest_new_info(&mut self, info: NodeResult) -> Result<(), String> {
        let output_str = String::from_utf8(info.output_data).unwrap_or_default();

        if output_str.is_empty() {
            return Ok(());
        }

        let mut tree_write = self.tree.write().await;
        tree_write.build(
            &output_str,
            self.embedding_provider.clone(),
            self.llm_provider.clone(),
        ).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::MockApiProvider;

    #[tokio::test]
    async fn test_raptor_context_manager() {
        let mock_provider = Arc::new(MockApiProvider::new());
        let mut manager = RaptorContextManager::new(mock_provider.clone(), mock_provider.clone(), None);

        let raw_context = RawContext {
            history_logs: vec![
                LogEntry { timestamp: 1, message: "User requested a weather update.".to_string() },
                LogEntry { timestamp: 2, message: "System initialized weather module.".to_string() },
            ],
            retrieved_docs: vec![
                Document { doc_id: "doc1".to_string(), content: "The weather in New York is sunny.".to_string() },
                Document { doc_id: "doc2".to_string(), content: "The weather in London is rainy.".to_string() },
            ],
        };

        let req = NodeRequirement {
            required_tokens: 50,
            query: "What is the weather like in New York?".to_string(),
        };

        let scoped_ctx = manager.compress_for_node(&raw_context, &req).await.unwrap();

        assert!(!scoped_ctx.summary_tree.is_empty());
        assert!(scoped_ctx.budget_tokens == 50);

        let new_info = NodeResult {
            output_data: "The weather in Tokyo is cloudy.".as_bytes().to_vec(),
            extracted_knowledge: None,
            next_routing_hint: None,
        };

        let ingest_result = manager.ingest_new_info(new_info).await;
        assert!(ingest_result.is_ok());

        let tree_read = manager.tree.read().await;
        assert!(tree_read.nodes.len() > scoped_ctx.summary_tree.len());
    }
}
