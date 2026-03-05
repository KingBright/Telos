pub mod providers;
pub mod clustering;
pub mod raptor;

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
}

impl RaptorContextManager {
    pub fn new(
        embedding_provider: Arc<dyn EmbeddingProvider>,
        llm_provider: Arc<dyn LlmProvider>,
    ) -> Self {
        Self {
            tree: RwLock::new(RaptorTree::new()),
            embedding_provider,
            llm_provider,
        }
    }
}

#[async_trait]
impl ContextManager for RaptorContextManager {
    async fn compress_for_node(&self, raw: &RawContext, node_req: &NodeRequirement) -> Result<ScopedContext, String> {
        let tree_read = self.tree.read().await;

        // 1. If the tree is empty, we must build it from the RawContext
        // In a real system, the tree might be built asynchronously in the background.
        // For this V1, we build/update it if it's empty.
        // (A more robust version would check if the RawContext has new documents since the last build).
        let needs_build = tree_read.nodes.is_empty();
        drop(tree_read); // Release read lock early

        if needs_build {
            let mut tree_write = self.tree.write().await;
            // Double-check pattern
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

        // 2. Retrieve relevant nodes using the query
        let tree_read = self.tree.read().await;
        let retrieved_nodes = tree_read.retrieve(
            &node_req.query,
            node_req.required_tokens,
            self.embedding_provider.clone(),
        ).await?;

        // 3. Map retrieved RAPTOR nodes to the expected SummaryNode output format
        let summary_tree = retrieved_nodes.into_iter().map(|n| SummaryNode {
            summary_text: n.text,
            children_ids: n.children_ids,
        }).collect();

        Ok(ScopedContext {
            budget_tokens: node_req.required_tokens,
            summary_tree,
            precise_facts: vec![], // Facts extraction integration goes here
        })
    }

    async fn ingest_new_info(&mut self, info: NodeResult) -> Result<(), String> {
        // Here we handle integrating new information from a completed DAG node back into the context.

        let output_str = String::from_utf8(info.output_data).unwrap_or_default();

        if output_str.is_empty() {
            return Ok(());
        }

        // We append this new information into the tree.
        // For now, we rebuild the tree with the new text.
        // A full implementation would append the nodes and trigger an incremental re-clustering.

        let mut tree_write = self.tree.write().await;
        // In this MVP, we simply re-run the build on the new output data to add to the existing tree structure.
        // Because `build` appends new base nodes and re-clusters, we can just call it with the new text.
        tree_write.build(
            &output_str,
            self.embedding_provider.clone(),
            self.llm_provider.clone(),
        ).await?;

        // In the future (Module 4 Integration), we would also take `info.extracted_knowledge`
        // and send it to the Memory OS via `MemoryManager::store(Semantic(...))`.

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
        let mut manager = RaptorContextManager::new(mock_provider.clone(), mock_provider.clone());

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

        // Test compression (building tree and retrieving)
        let scoped_ctx = manager.compress_for_node(&raw_context, &req).await.unwrap();

        assert!(scoped_ctx.summary_tree.len() > 0);
        assert!(scoped_ctx.budget_tokens == 50);

        // Test ingestion of new information
        let new_info = NodeResult {
            output_data: "The weather in Tokyo is cloudy.".as_bytes().to_vec(),
            extracted_knowledge: None,
            next_routing_hint: None,
        };

        let ingest_result = manager.ingest_new_info(new_info).await;
        assert!(ingest_result.is_ok());

        // Verify that the tree size increased
        let tree_read = manager.tree.read().await;
        assert!(tree_read.nodes.len() > scoped_ctx.summary_tree.len());
    }
}
