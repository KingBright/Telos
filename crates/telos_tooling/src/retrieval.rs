use crate::{ToolExecutor, ToolRegistry, ToolSchema};
use std::collections::HashMap;
use tracing::{info, debug};
use std::path::{PathBuf, Path};
use tokio::fs;

pub struct VectorToolRegistry {
    tools: std::sync::RwLock<HashMap<String, ToolSchema>>,
    executors: std::sync::RwLock<HashMap<String, std::sync::Arc<dyn ToolExecutor>>>,
    plugins_dir: PathBuf,
    #[cfg(feature = "local-embeddings")]
    embeddings_cache: std::sync::RwLock<HashMap<String, Vec<f32>>>,
    #[cfg(feature = "local-embeddings")]
    model: Option<std::sync::RwLock<fastembed::TextEmbedding>>,
}

impl VectorToolRegistry {
    pub fn new() -> Self {
        #[cfg(feature = "local-embeddings")]
        {
            let model_result = std::panic::catch_unwind(|| {
                let cache_dir = dirs::home_dir().map(|h| h.join(".telos").join("models")).unwrap_or_else(|| std::path::PathBuf::from(".fastembed_cache"));
                fastembed::TextEmbedding::try_new(fastembed::InitOptions::new(
                    fastembed::EmbeddingModel::AllMiniLML6V2,
                ).with_cache_dir(cache_dir))
            });

            match model_result {
                Ok(Ok(model)) => {
                    info!("[ToolRegistry] Fastembed model loaded successfully.");
                    let mut registry = Self {
                        tools: std::sync::RwLock::new(HashMap::new()),
                        executors: std::sync::RwLock::new(HashMap::new()),
                        plugins_dir: Self::get_default_plugins_dir(),
                        embeddings_cache: std::sync::RwLock::new(HashMap::new()),
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
            tools: std::sync::RwLock::new(HashMap::new()),
            executors: std::sync::RwLock::new(HashMap::new()),
            plugins_dir: Self::get_default_plugins_dir(),
            #[cfg(feature = "local-embeddings")]
            embeddings_cache: std::sync::RwLock::new(HashMap::new()),
            #[cfg(feature = "local-embeddings")]
            model: None,
        };
        registry.load_saved_plugins();
        registry
    }

    /// Create a registry exclusively for isolated testing without plugin loading.
    #[cfg(test)]
    pub fn new_for_test() -> Self {
        Self {
            tools: std::sync::RwLock::new(HashMap::new()),
            executors: std::sync::RwLock::new(HashMap::new()),
            plugins_dir: std::env::temp_dir().join("telos_test_plugins"),
            #[cfg(feature = "local-embeddings")]
            embeddings_cache: std::sync::RwLock::new(HashMap::new()),
            #[cfg(feature = "local-embeddings")]
            model: None,
        }
    }

    pub fn register_tool(
        &self,
        schema: ToolSchema,
        executor: Option<std::sync::Arc<dyn ToolExecutor>>,
    ) {
        #[cfg(feature = "local-embeddings")]
        if let Some(ref model) = self.model {
            let text_to_embed = format!("{} {}", schema.name, schema.description);
            if let Ok(mut m) = model.write() {
                if let Ok(embeddings) = m.embed(vec![text_to_embed], None) {
                    if let Some(embedding) = embeddings.into_iter().next() {
                        if let Ok(mut cache) = self.embeddings_cache.write() {
                            cache.insert(schema.name.clone(), embedding);
                        }
                    }
                }
            }
        }

        if let Some(exec) = executor {
            if let Ok(mut execs) = self.executors.write() {
                execs.insert(schema.name.clone(), exec);
            }
        }

        if let Ok(mut tools) = self.tools.write() {
            tools.insert(schema.name.clone(), schema);
        }
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

        let mut all_tools = Vec::new();
        if let Ok(guard) = self.tools.read() {
            for (name, schema) in guard.iter() {
                all_tools.push((name.clone(), schema.clone()));
            }
        }

        let mut scored: Vec<(ToolSchema, f32)> = all_tools
            .into_iter()
            .map(|(name, schema)| {
                let haystack = format!("{} {}", name, schema.description).to_lowercase();
                let score: f32 = query_words.iter().filter(|w| haystack.contains(*w)).count() as f32;
                (schema, score)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored
            .into_iter()
            .filter(|&(_, score)| score > 0.0)
            .take(limit)
            .map(|(schema, _)| schema)
            .collect()
    }

    /// List all registered tools with their schemas
    pub fn list_all_tools(&self) -> Vec<ToolSchema> {
        if let Ok(guard) = self.tools.read() {
            guard.values().cloned().collect()
        } else {
            vec![]
        }
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
                        let mut scored_tools: Vec<(String, f32)> = if let Ok(guard) = self.embeddings_cache.read() {
                            guard.iter().map(|(name, emb)| {
                                (name.clone(), Self::cosine_similarity(&query_embedding, emb))
                            }).collect()
                        } else {
                            vec![]
                        };

                        scored_tools.sort_by(|a, b| {
                            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
                        });

                        let mut results = Vec::new();
                        if let Ok(tools_guard) = self.tools.read() {
                            for (name, _) in scored_tools.into_iter().take(limit) {
                                if let Some(schema) = tools_guard.get(&name) {
                                    results.push(schema.clone());
                                }
                            }
                        }
                        return results;
                    }
                }
            }
        }

        self.keyword_discover(intent, limit)
    }

    fn get_executor(&self, tool_name: &str) -> Option<std::sync::Arc<dyn ToolExecutor>> {
        if let Ok(guard) = self.executors.read() {
            guard.get(tool_name).map(|inner| {
                std::sync::Arc::new(crate::InstrumentedToolExecutor {
                    inner: inner.clone(),
                    tool_name: tool_name.to_string(),
                }) as std::sync::Arc<dyn ToolExecutor>
            })
        } else {
            None
        }
    }

    fn get_schema(&self, tool_name: &str) -> Option<ToolSchema> {
        if let Ok(guard) = self.tools.read() {
            guard.get(tool_name).cloned()
        } else {
            None
        }
    }

    fn list_all_tools(&self) -> Vec<ToolSchema> {
        self.list_all_tools()
    }

    fn register_dynamic_tool(&self, schema: ToolSchema, executor: std::sync::Arc<dyn ToolExecutor>) -> Result<(), String> {
        info!("[ToolRegistry] 🔌 Dynamic Tool Registered: {}", schema.name);
        
        // Only persist "real" tools to disk — skip debug/test intermediaries
        // These are created by the CoderAgent during ReactLoop iterations and should be ephemeral
        let is_ephemeral = schema.name.starts_with("debug_") 
            || schema.name.starts_with("test_") 
            || schema.name.starts_with("diag_");
        
        if !is_ephemeral {
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
        } else {
            debug!("[ToolRegistry] Ephemeral tool '{}' registered in-memory only (not persisted to disk)", schema.name);
        }

        // Register it natively using the RwLock (always in-memory for current session)
        self.register_tool(schema, Some(executor));
        Ok(())
    }
}

impl Default for VectorToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script_sandbox::{ScriptSandbox, ScriptExecutor};
    use crate::{JsonSchema, RiskLevel};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_dynamic_tool_registration_and_execution() {
        // 1. Setup a fresh registry
        let registry = VectorToolRegistry::new_keyword_only();
        
        // 2. Define a dummy schema
        let schema = ToolSchema {
            name: "calculate_tax".into(),
            description: "Calculates 20% tax on an amount integer-wise".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "amount": { "type": "integer" }
                    },
                    "required": ["amount"]
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        };

        // 3. Define the Rhai Executor (Integer arithmetic)
        let rhai_code = r#"
            let amt = params.amount;
            return amt / 5;
        "#.to_string();
        
        let sandbox = Arc::new(ScriptSandbox::new());
        let executor = Arc::new(ScriptExecutor::new(rhai_code.clone(), sandbox));

        // 4. Register dynamically
        let res = registry.register_dynamic_tool(schema.clone(), executor);
        assert!(res.is_ok(), "Registration should succeed");

        // 5. Verify it's discoverable
        let discovered = registry.discover_tools("tax", 5);
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].name, "calculate_tax");

        // 6. Verify it's executable via the registry reference
        let retrieved_executor = registry.get_executor("calculate_tax").expect("Executor must exist");
        
        let params = serde_json::json!({ "amount": 100 });
        let exec_result = retrieved_executor.call(params).await.expect("Execution should not fail");
        
        let out_str = String::from_utf8(exec_result).unwrap();
        assert_eq!(out_str, "20");
    }
}
