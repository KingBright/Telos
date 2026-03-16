use crate::clustering::{cosine_similarity, gmm_soft_cluster, parse_into_edus, Edu};
use crate::providers::{EmbeddingProvider, LlmProvider};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct RaptorNode {
    pub id: String,
    pub text: String,
    pub embedding: Vec<f32>,
    pub children_ids: Vec<String>,
    pub level: usize,
}

pub struct RaptorTree {
    pub nodes: Vec<RaptorNode>,
}

impl RaptorTree {
    pub fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    /// Recursively clusters and summarizes nodes to build the RAPTOR tree bottom-up.
    /// Uses GMM soft clustering so EDUs can appear in multiple cluster summaries,
    /// preserving cross-topic context that hard K-Means assignment would lose.
    pub async fn build(
        &mut self,
        text: &str,
        embedding_provider: Arc<dyn EmbeddingProvider>,
        llm_provider: Arc<dyn LlmProvider>,
    ) -> Result<(), String> {
        // 1. Parse text into EDUs (leaf nodes)
        let mut edus = parse_into_edus(text, "root");

        // 2. Embed the base EDUs
        for edu in &mut edus {
            let emb = embedding_provider
                .embed(&edu.text)
                .await
                .map_err(|e| format!("Embedding failed: {}", e.message))?;
            edu.embedding = Some(emb);

            self.nodes.push(RaptorNode {
                id: edu.id.clone(),
                text: edu.text.clone(),
                embedding: edu.embedding.clone().unwrap(),
                children_ids: vec![],
                level: 0,
            });
        }

        // 3. Iteratively cluster and summarize to build higher levels
        let mut current_level_nodes = edus.clone();
        let mut current_level = 0;
        let max_levels = 3; // Limit the tree height for simplicity

        while current_level_nodes.len() > 1 && current_level < max_levels {
            current_level += 1;

            // Heuristic for k: roughly 5 items per cluster, minimum 1
            let k = (current_level_nodes.len() / 5).max(1);
            // GMM soft clustering: EDUs with responsibility > 0.15 can appear in multiple clusters
            let clusters = gmm_soft_cluster(&current_level_nodes, k, 20, 0.15);

            let mut next_level_nodes = Vec::new();

            for (cluster_id, members) in clusters {
                // Gather text from all children in this cluster (weighted by responsibility)
                let mut cluster_text = String::new();
                let mut children_ids = Vec::new();
                for (id, _weight) in &members {
                    if let Some(node) = current_level_nodes.iter().find(|n| n.id == *id) {
                        cluster_text.push_str(&node.text);
                        cluster_text.push(' ');
                    }
                    children_ids.push(id.clone());
                }

                // Summarize the cluster
                let summary = llm_provider
                    .summarize(&cluster_text)
                    .await
                    .map_err(|e| format!("Summarization failed: {}", e.message))?;

                // Embed the summary
                let summary_embedding = embedding_provider
                    .embed(&summary)
                    .await
                    .map_err(|e| format!("Embedding failed: {}", e.message))?;

                let new_node_id = format!("level_{}_cluster_{}", current_level, cluster_id);

                let new_raptor_node = RaptorNode {
                    id: new_node_id.clone(),
                    text: summary.clone(),
                    embedding: summary_embedding.clone(),
                    children_ids: children_ids.clone(),
                    level: current_level,
                };

                self.nodes.push(new_raptor_node.clone());

                next_level_nodes.push(Edu {
                    id: new_node_id,
                    text: summary,
                    embedding: Some(summary_embedding),
                });
            }

            current_level_nodes = next_level_nodes;
        }

        Ok(())
    }

    /// Top-down retrieval of the most relevant contexts given a query
    pub async fn retrieve(
        &self,
        query: &str,
        budget_tokens: usize,
        embedding_provider: Arc<dyn EmbeddingProvider>,
    ) -> Result<Vec<RaptorNode>, String> {
        let query_embedding = embedding_provider
            .embed(query)
            .await
            .map_err(|e| format!("Query embedding failed: {}", e.message))?;

        let mut retrieved_nodes = Vec::new();
        let mut current_tokens = 0;

        // Approximate token count (words * 1.3)
        let estimate_tokens = |text: &str| -> usize {
            (text.split_whitespace().count() as f32 * 1.3) as usize
        };

        // Score all nodes
        let mut scored_nodes: Vec<(&RaptorNode, f32)> = self.nodes.iter().map(|n| {
            let sim = cosine_similarity(&n.embedding, &query_embedding);
            (n, sim)
        }).collect();

        // Sort by similarity descending
        scored_nodes.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Select top nodes until budget is hit
        for (node, _score) in scored_nodes {
            let tokens = estimate_tokens(&node.text);
            if current_tokens + tokens <= budget_tokens {
                retrieved_nodes.push(node.clone());
                current_tokens += tokens;
            } else {
                break; // Budget exhausted
            }
        }

        Ok(retrieved_nodes)
    }
}

impl Default for RaptorTree {
    fn default() -> Self {
        Self::new()
    }
}
