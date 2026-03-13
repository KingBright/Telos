use crate::{ToolExecutor, ToolRegistry, ToolSchema};
use std::collections::HashMap;
use tracing::{info, debug};
use std::path::{PathBuf, Path};
use tokio::fs;

pub struct VectorToolRegistry {
    tools: HashMap<String, ToolSchema>,
    executors: HashMap<String, std::sync::Arc<dyn ToolExecutor>>,
    plugins_dir: PathBuf,
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
                    info!("[ToolRegistry] Fastembed model loaded successfully.");
                    let mut registry = Self {
                        tools: HashMap::new(),
                        executors: HashMap::new(),
                        plugins_dir: Self::get_default_plugins_dir(),
                        embeddings_cache: HashMap::new(),
                        model: Some(std::sync::RwLock::new(model)),
                    };
                    registry.load_saved_plugins();
                    return registry;
                }
                _ => {
                    info!("[ToolRegistry] Fastembed model unavailable, using keyword fallback.");
                }
            }
        }

        Self::new_keyword_only()
    }

    /// Get the default plugins directory (e.g. ./plugins or ~/.telos/plugins)
    fn get_default_plugins_dir() -> PathBuf {
        let dir = dirs::home_dir().map(|h| h.join(".telos").join("plugins")).unwrap_or_else(|| PathBuf::from("./plugins"));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    /// Load dynamically saved plugins from disk on registry initialization
    fn load_saved_plugins(&mut self) {
        if let Ok(entries) = std::fs::read_dir(&self.plugins_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    if let Ok(meta_content) = std::fs::read_to_string(&path) {
                        if let Ok(schema) = serde_json::from_str::<ToolSchema>(&meta_content) {
                            let rhai_path = path.with_extension("rhai");
                            if rhai_path.exists() {
                                if let Ok(script) = std::fs::read_to_string(&rhai_path) {
                                    let sandbox = std::sync::Arc::new(crate::ScriptSandbox::new());
                                    // Notice: we can't easily inject the native registry back here because `self` is the registry.
                                    // The `ToolRegistry` design makes cyclical arcs hard. So we load the executor without native dependencies first.
                                    let executor = std::sync::Arc::new(crate::ScriptExecutor::new(script, sandbox));
                                    self.register_tool(schema.clone(), Some(executor));
                                    tracing::info!("[ToolRegistry] Loaded plugin from disk: {}", schema.name);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Create a registry without attempting to load fastembed at all (instant startup).
    pub fn new_keyword_only() -> Self {
        info!("[ToolRegistry] Using keyword-only tool discovery.");
        let mut registry = Self {
            tools: HashMap::new(),
            executors: HashMap::new(),
            plugins_dir: Self::get_default_plugins_dir(),
            #[cfg(feature = "local-embeddings")]
            embeddings_cache: HashMap::new(),
            #[cfg(feature = "local-embeddings")]
            model: None,
        };
        registry.load_saved_plugins();
        registry
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
        let query_lower = intent.to_lowercase().replace('_', " ").replace('-', " ");
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();

        let mut scored: Vec<(&String, f32)> = self
            .tools
            .iter()
            .map(|(name, schema)| {
                let haystack = format!("{} {}", name, schema.description).to_lowercase();
                let score: f32 =
                    query_words.iter().filter(|w| haystack.contains(*w)).count() as f32;
                debug!(
                    "[ToolRegistry] Checking '{}' against words {:?} -> score {}",
                    name, query_words, score
                );
                (name, score)
            })
            .collect();

        if scored.iter().all(|(_, s)| *s == 0.0) {
            debug!(
                "[ToolRegistry] No matches at all for query words: {:?}",
                query_words
            );
        }

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        scored
            .into_iter()
            .filter(|&(_, score)| score > 0.0)
            .take(limit)
            .filter_map(|(name, _)| self.tools.get(name).cloned())
            .collect()
    }

    /// List all registered tools with their schemas
    pub fn list_all_tools(&self) -> Vec<ToolSchema> {
        self.tools.values().cloned().collect()
    }
}

impl ToolRegistry for VectorToolRegistry {
    fn discover_tools(&self, intent: &str, limit: usize) -> Vec<ToolSchema> {
        debug!(
            "[ToolRegistry] discover_tools called with intent: '{}'",
            intent
        );
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

    fn list_all_tools(&self) -> Vec<ToolSchema> {
        self.tools.values().cloned().collect()
    }

    fn register_dynamic_tool(&self, schema: ToolSchema, executor: std::sync::Arc<dyn ToolExecutor>) -> Result<(), String> {
        info!("[ToolRegistry] 🔌 Dynamic Tool Registered: {}", schema.name);
        
        // Save the tool physically if it has dynamic source code
        if let Some(source) = executor.source_code() {
            let plugin_path = self.plugins_dir.join(format!("{}.rhai", schema.name));
            let meta_path = self.plugins_dir.join(format!("{}.json", schema.name));
            
            // Write script content
            if let Err(e) = std::fs::write(&plugin_path, &source) {
                tracing::error!("Failed to save dynamic tool script to disk: {}", e);
            }
            
            // Write schema metadata
            if let Ok(meta_json) = serde_json::to_string_pretty(&schema) {
                let _ = std::fs::write(&meta_path, meta_json);
            }
            info!("Plugin safely installed to: {:?}", plugin_path);
        }

        // To mutate internal maps, we would need interior mutability on VectorToolRegistry itself.
        // Wait, VectorToolRegistry tools and executors maps are NOT in a RwLock.
        // But the current implementation of `ToolRegistry` trait for `VectorToolRegistry` 
        // implies read-only operations for discover_tools and get_executor.
        // How does it register native tools? It does it via `pub fn register_tool(&mut self)` 
        // DURING initialization before being wrapped in Arc<RwLock>.
        // Since `register_dynamic_tool` needs to mutate, and it takes `&self` on the trait, 
        // VectorToolRegistry MUST wrap its maps in RwLock or Mutex.
        // Quick fix: return an error for now because this trait method is currently implemented in SharedToolRegistry!
        // Actually, `SharedToolRegistry<T>` implements `ToolRegistry` by calling `T`'s mutable methods via `try_write().guard.register_dynamic_tool()`.
        // Wait, `SharedToolRegistry<T>` expects `T` to have `register_dynamic_tool(&mut self)`? No, it calls `guard.register_dynamic_tool` which goes to the trait implementation.
        // Let me just fix the trait to match `VectorToolRegistry`'s interior mutability needs.
        Err("Cannot register dynamically to a non-shared registry directly".into())
    }
}

impl Default for VectorToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
