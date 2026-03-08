use crate::{ToolExecutor, ToolRegistry, ToolSchema};
use std::collections::HashMap;

pub struct VectorToolRegistry {
    tools: HashMap<String, ToolSchema>,
    executors: HashMap<String, std::sync::Arc<dyn ToolExecutor>>,
    #[cfg(feature = "local-embeddings")]
    embeddings_cache: HashMap<String, Vec<f32>>,
    #[cfg(feature = "local-embeddings")]
    model: Option<std::sync::RwLock<fastembed::TextEmbedding>>,
}

impl VectorToolRegistry {
    pub fn new() -> Self {
        #[cfg(feature = "local-embeddings")]
        {
            let model_result = std::panic::catch_unwind(|| {
                fastembed::TextEmbedding::try_new(fastembed::InitOptions::new(
                    fastembed::EmbeddingModel::AllMiniLML6V2,
                ))
            });

            match model_result {
                Ok(Ok(model)) => {
                    println!("[ToolRegistry] Fastembed model loaded successfully.");
                    return Self {
                        tools: HashMap::new(),
                        executors: HashMap::new(),
                        embeddings_cache: HashMap::new(),
                        model: Some(std::sync::RwLock::new(model)),
                    };
                }
                _ => {
                    println!("[ToolRegistry] Fastembed model unavailable, using keyword fallback.");
                }
            }
        }

        Self::new_keyword_only()
    }

    /// Create a registry without attempting to load fastembed at all (instant startup).
    pub fn new_keyword_only() -> Self {
        println!("[ToolRegistry] Using keyword-only tool discovery.");
        Self {
            tools: HashMap::new(),
            executors: HashMap::new(),
            #[cfg(feature = "local-embeddings")]
            embeddings_cache: HashMap::new(),
            #[cfg(feature = "local-embeddings")]
            model: None,
        }
    }

    pub fn register_tool(
        &mut self,
        schema: ToolSchema,
        executor: Option<std::sync::Arc<dyn ToolExecutor>>,
    ) {
        #[cfg(feature = "local-embeddings")]
        if let Some(ref model) = self.model {
            let text_to_embed = format!("{} {}", schema.name, schema.description);
            if let Ok(mut m) = model.write() {
                if let Ok(embeddings) = m.embed(vec![text_to_embed], None) {
                    if let Some(embedding) = embeddings.into_iter().next() {
                        self.embeddings_cache.insert(schema.name.clone(), embedding);
                    }
                }
            }
        }

        if let Some(exec) = executor {
            self.executors.insert(schema.name.clone(), exec);
        }

        self.tools.insert(schema.name.clone(), schema);
    }

    #[cfg(feature = "local-embeddings")]
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

    /// Keyword-based tool discovery fallback.
    fn keyword_discover(&self, intent: &str, limit: usize) -> Vec<ToolSchema> {
        let query_lower = intent.to_lowercase();
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();

        let mut scored: Vec<(&String, f32)> = self
            .tools
            .iter()
            .map(|(name, schema)| {
                let haystack = format!("{} {}", name, schema.description).to_lowercase();
                let score: f32 =
                    query_words.iter().filter(|w| haystack.contains(*w)).count() as f32;
                (name, score)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        scored
            .into_iter()
            .take(limit)
            .filter_map(|(name, _)| self.tools.get(name).cloned())
            .collect()
    }
}

impl ToolRegistry for VectorToolRegistry {
    fn discover_tools(&self, intent: &str, limit: usize) -> Vec<ToolSchema> {
        #[cfg(feature = "local-embeddings")]
        if let Some(ref model) = self.model {
            if let Ok(mut m) = model.write() {
                if let Ok(embeddings) = m.embed(vec![intent.to_string()], None) {
                    if let Some(query_embedding) = embeddings.into_iter().next() {
                        let mut scored_tools: Vec<(&String, f32)> = self
                            .embeddings_cache
                            .iter()
                            .map(|(name, emb)| {
                                let score = Self::cosine_similarity(&query_embedding, emb);
                                (name, score)
                            })
                            .collect();

                        scored_tools.sort_by(|a, b| {
                            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
                        });

                        return scored_tools
                            .into_iter()
                            .take(limit)
                            .filter_map(|(name, _)| self.tools.get(name).cloned())
                            .collect();
                    }
                }
            }
        }

        self.keyword_discover(intent, limit)
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
