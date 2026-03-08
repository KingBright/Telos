use crate::{ToolExecutor, ToolRegistry, ToolSchema};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::collections::HashMap;
use std::sync::RwLock;

pub struct VectorToolRegistry {
    tools: HashMap<String, ToolSchema>,
    embeddings_cache: HashMap<String, Vec<f32>>,
    executors: HashMap<String, std::sync::Arc<dyn ToolExecutor>>,
    model: RwLock<TextEmbedding>,
}

impl VectorToolRegistry {
    pub fn new() -> Self {
        let model = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2))
            .expect("Failed to initialize fastembed model");

        Self {
            tools: HashMap::new(),
            embeddings_cache: HashMap::new(),
            executors: HashMap::new(),
            model: RwLock::new(model),
        }
    }

    pub fn register_tool(&mut self, schema: ToolSchema, executor: Option<std::sync::Arc<dyn ToolExecutor>>) {
        let text_to_embed = format!("{} {}", schema.name, schema.description);

        // Generate embedding for the tool description
        let mut model = self.model.write().unwrap();
        let embeddings = model.embed(vec![text_to_embed], None)
            .expect("Failed to generate embedding");

        if let Some(embedding) = embeddings.into_iter().next() {
            self.embeddings_cache.insert(schema.name.clone(), embedding);
        }

        if let Some(exec) = executor {
            self.executors.insert(schema.name.clone(), exec);
        }

        self.tools.insert(schema.name.clone(), schema);
    }

    // Simple cosine similarity helper
    fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        let mut dot_product = 0.0;
        let mut norm_a = 0.0;
        let mut norm_b = 0.0;

        for (x, y) in a.iter().zip(b.iter()) {
            dot_product += x * y;
            norm_a += x * x;
            norm_b += y * y;
        }

        if norm_a == 0.0 || norm_b == 0.0 {
            0.0
        } else {
            dot_product / (norm_a.sqrt() * norm_b.sqrt())
        }
    }
}

impl ToolRegistry for VectorToolRegistry {
    fn discover_tools(&self, intent: &str, limit: usize) -> Vec<ToolSchema> {
        let mut model = self.model.write().unwrap();
        let query_embedding: Vec<f32> = match model.embed(vec![intent.to_string()], None) {
            Ok(embeddings) => {
                if let Some(emb) = embeddings.into_iter().next() {
                    emb
                } else {
                    return vec![];
                }
            },
            Err(_) => return vec![],
        };

        let mut scored_tools: Vec<(&String, f32)> = self.embeddings_cache
            .iter()
            .map(|(name, emb)| {
                let score = Self::cosine_similarity(&query_embedding, emb);
                (name, score)
            })
            .collect();

        // Sort descending by score
        scored_tools.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        scored_tools
            .into_iter()
            .take(limit)
            .filter_map(|(name, _)| self.tools.get(name).cloned())
            .collect()
    }

    fn get_executor(&self, tool_name: &str) -> Option<std::sync::Arc<dyn ToolExecutor>> {
        self.executors.get(tool_name).cloned()
    }
}
impl Default for VectorToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
