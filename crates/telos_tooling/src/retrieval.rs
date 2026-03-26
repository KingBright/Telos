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
    pub fn new(tools_dir: PathBuf) -> Self {
        // Migrate legacy plugins/ dir to the canonical tools/ dir
        Self::migrate_legacy_plugins(&tools_dir);
        let _ = std::fs::create_dir_all(&tools_dir);

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
                        plugins_dir: tools_dir,
                        embeddings_cache: std::sync::RwLock::new(HashMap::new()),
                        model: Some(std::sync::RwLock::new(model)),
                    };
                    registry.load_saved_tools();
                    return registry;
                }
                _ => {
                    info!("[ToolRegistry] Fastembed model unavailable, using keyword fallback.");
                }
            }
        }

        Self::new_keyword_only(tools_dir)
    }

    /// Migrate legacy `~/.telos/plugins/` files to the canonical `tools_dir`.
    /// Only moves files that don't already exist in the target directory.
    fn migrate_legacy_plugins(tools_dir: &Path) {
        let legacy_dir = dirs::home_dir()
            .map(|h| h.join(".telos").join("plugins"))
            .unwrap_or_else(|| PathBuf::from("./plugins"));
        
        if !legacy_dir.exists() || legacy_dir == *tools_dir {
            return;
        }

        let _ = std::fs::create_dir_all(tools_dir);
        let mut migrated = 0u32;
        
        if let Ok(entries) = std::fs::read_dir(&legacy_dir) {
            for entry in entries.flatten() {
                let src = entry.path();
                let filename = match src.file_name() {
                    Some(f) => f.to_owned(),
                    None => continue,
                };
                let dst = tools_dir.join(&filename);
                
                if !dst.exists() {
                    if let Err(e) = std::fs::copy(&src, &dst) {
                        tracing::warn!("[ToolRegistry] Failed to migrate {:?}: {}", filename, e);
                        continue;
                    }
                    migrated += 1;
                }
                // Remove source file after copy (or if dest already exists)
                let _ = std::fs::remove_file(&src);
            }
        }
        
        // Try to remove the legacy dir if empty
        let _ = std::fs::remove_dir(&legacy_dir);
        
        if migrated > 0 {
            info!("[ToolRegistry] Migrated {} files from legacy plugins/ to tools/", migrated);
        }
    }

    /// Load persisted tools from disk on registry initialization
    fn load_saved_tools(&mut self) {
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
                                    let executor = std::sync::Arc::new(crate::ScriptExecutor::new(script, sandbox));
                                    self.register_tool(schema.clone(), Some(executor));
                                    tracing::info!("[ToolRegistry] Loaded tool from disk: {}", schema.name);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Create a registry without attempting to load fastembed (instant startup).
    pub fn new_keyword_only(tools_dir: PathBuf) -> Self {
        Self::migrate_legacy_plugins(&tools_dir);
        let _ = std::fs::create_dir_all(&tools_dir);
        info!("[ToolRegistry] Using keyword-only tool discovery. Tools dir: {:?}", tools_dir);
        let mut registry = Self {
            tools: std::sync::RwLock::new(HashMap::new()),
            executors: std::sync::RwLock::new(HashMap::new()),
            plugins_dir: tools_dir,
            #[cfg(feature = "local-embeddings")]
            embeddings_cache: std::sync::RwLock::new(HashMap::new()),
            #[cfg(feature = "local-embeddings")]
            model: None,
        };
        registry.load_saved_tools();
        registry
    }

    /// Create a registry exclusively for isolated testing without plugin loading.
    #[cfg(test)]
    pub fn new_for_test() -> Self {
        Self {
            tools: std::sync::RwLock::new(HashMap::new()),
            executors: std::sync::RwLock::new(HashMap::new()),
            plugins_dir: std::env::temp_dir().join("telos_test_tools"),
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

    /// Keyword-based tool discovery with health-weighted ranking.
    /// Archived tools are excluded. Broken tools are heavily penalized.
    fn keyword_discover(&self, intent: &str, limit: usize) -> Vec<ToolSchema> {
        let query_lower = intent.to_lowercase().replace('_', " ").replace('-', " ");
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();

        let mut all_tools = Vec::new();
        if let Ok(guard) = self.tools.read() {
            for (name, schema) in guard.iter() {
                // Skip archived tools — they are hidden from discovery
                if schema.health_status == "archived" {
                    continue;
                }
                all_tools.push((name.clone(), schema.clone()));
            }
        }

        let mut scored: Vec<(ToolSchema, f32)> = all_tools
            .into_iter()
            .map(|(name, schema)| {
                let haystack = format!("{} {}", name, schema.description).to_lowercase();
                let keyword_score: f32 = query_words.iter().filter(|w| haystack.contains(*w)).count() as f32;
                let health_w = Self::health_weight(&schema);
                (schema, keyword_score * health_w)
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

    /// Calculate a health weight multiplier for a tool (0.0 to 1.0).
    /// - active tools with high success rates get ~1.0
    /// - dormant tools get 0.6-0.8 (still valuable, just not used recently)
    /// - broken tools get 0.1 (heavily penalized but still discoverable if explicitly matched)
    fn health_weight(schema: &ToolSchema) -> f32 {
        // Archived tools should never reach here but guard anyway
        if schema.health_status == "archived" {
            return 0.0;
        }
        if schema.health_status == "broken" {
            return 0.1;
        }

        let total = schema.success_count + schema.failure_count;
        if total == 0 {
            // Brand new tool, no usage data — neutral weight
            return 0.8;
        }

        let success_rate = schema.success_count as f32 / total as f32;
        
        // Recency boost: recently used tools get a small bonus
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let last_used = schema.last_success_at.max(schema.last_failure_at);
        let age_days = if last_used > 0 { (now_ms.saturating_sub(last_used)) / 86_400_000 } else { 30 };
        let recency = if age_days < 7 { 1.0 } else if age_days < 30 { 0.9 } else { 0.7 };

        // Dormant bonus: tools that worked before but are just not used recently
        // shouldn't be overly penalized — they are still valuable
        if schema.health_status == "dormant" {
            return 0.7 * recency;
        }

        success_rate * recency
    }

    /// Persist a tool's updated schema to disk (for health tracking updates).
    fn persist_schema(&self, schema: &ToolSchema) {
        let is_ephemeral = schema.name.starts_with("debug_") 
            || schema.name.starts_with("test_") 
            || schema.name.starts_with("diag_");
        if is_ephemeral { return; }
        
        let meta_path = self.plugins_dir.join(format!("{}.json", schema.name));
        if let Ok(meta_json) = serde_json::to_string_pretty(schema) {
            let _ = std::fs::write(&meta_path, meta_json);
        }
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
        
        let mut results = Vec::new();
        let mut exact_matches = std::collections::HashSet::new();
        let intent_lower = intent.to_lowercase();
        
        // 1. EXACT-MATCH OVERRIDE
        // If the intent contains the EXACT name of a tool (e.g., "get_weather"), pin it to the top!
        if let Ok(guard) = self.tools.read() {
            for (name, schema) in guard.iter() {
                if schema.health_status == "archived" {
                    continue;
                }
                // Avoid too brief names matching accidentaly (e.g. "fs" or "io")
                if name.len() >= 4 && intent_lower.contains(&name.to_lowercase()) {
                    results.push(schema.clone());
                    exact_matches.insert(name.clone());
                }
            }
        }
        
        if results.len() >= limit {
            results.truncate(limit);
            return results;
        }

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

                        if let Ok(tools_guard) = self.tools.read() {
                            for (name, _) in scored_tools.into_iter() {
                                if !exact_matches.contains(&name) {
                                    if let Some(schema) = tools_guard.get(&name) {
                                        if schema.health_status != "archived" {
                                            results.push(schema.clone());
                                        }
                                    }
                                }
                                if results.len() >= limit {
                                    break;
                                }
                            }
                        }
                        return results;
                    }
                }
            }
        }
        
        // If no embeddings model is available, use keyword fallback
        let mut keyword_results = self.keyword_discover(intent, limit);
        for k in keyword_results.into_iter() {
            if !exact_matches.contains(&k.name) {
                results.push(k);
            }
            if results.len() >= limit {
                break;
            }
        }
        
        results
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

    fn attach_tool_note(&self, tool_name: &str, note: String) -> Result<(), String> {
        let mut schema = {
            let guard = self.tools.read().map_err(|_| "Failed to acquire tools read lock")?;
            guard.get(tool_name).cloned().ok_or_else(|| format!("Tool not found: {}", tool_name))?
        };

        schema.experience_notes.push(note);

        if let Ok(mut tools) = self.tools.write() {
            tools.insert(schema.name.clone(), schema.clone());
        }

        self.persist_schema(&schema);
        
        info!("[ToolRegistry] 📝 Attached note to tool '{}'", schema.name);
        Ok(())
    }

    fn record_tool_usage(&self, tool_name: &str, success: bool) {
        let updated_schema = {
            let guard = match self.tools.read() {
                Ok(g) => g,
                Err(_) => return,
            };
            let schema = match guard.get(tool_name) {
                Some(s) => s.clone(),
                None => return, // Native tools don't have schemas in our map, skip silently
            };
            drop(guard);

            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);

            let mut s = schema;
            if success {
                s.success_count += 1;
                s.last_success_at = now_ms;
            } else {
                s.failure_count += 1;
                s.last_failure_at = now_ms;
            }

            // Auto-classify health status (only for custom/dynamic tools)
            // Key distinction per user requirement:
            // - dormant = has succeeded before but not used recently (still valuable!)
            // - broken = has NEVER succeeded, or total_success=0 with failures (garbage)
            let total = s.success_count + s.failure_count;
            if s.health_status != "archived" {
                if s.success_count == 0 && s.failure_count >= 3 {
                    // Never succeeded with 3+ failures → broken
                    s.health_status = "broken".to_string();
                } else if total > 0 && s.success_count > 0 {
                    // Has succeeded at least once
                    let age_days = if s.last_success_at > 0 {
                        now_ms.saturating_sub(s.last_success_at) / 86_400_000
                    } else {
                        0
                    };
                    if age_days > 30 {
                        s.health_status = "dormant".to_string();
                    } else {
                        s.health_status = "active".to_string();
                    }
                }
            }
            s
        };

        // Write back to the in-memory registry
        if let Ok(mut tools) = self.tools.write() {
            tools.insert(updated_schema.name.clone(), updated_schema.clone());
        }

        // Persist health update to disk
        self.persist_schema(&updated_schema);
    }

    fn archive_tool(&self, tool_name: &str) -> Result<(), String> {
        let mut schema = {
            let guard = self.tools.read().map_err(|_| "Lock error")?;
            guard.get(tool_name).cloned().ok_or_else(|| format!("Tool '{}' not found", tool_name))?
        };

        schema.health_status = "archived".to_string();

        if let Ok(mut tools) = self.tools.write() {
            tools.insert(schema.name.clone(), schema.clone());
        }

        self.persist_schema(&schema);
        info!("[ToolRegistry] 📦 Archived tool '{}'", tool_name);
        Ok(())
    }

    fn delete_tool(&self, tool_name: &str) -> Result<(), String> {
        // Remove from memory
        if let Ok(mut tools) = self.tools.write() {
            tools.remove(tool_name);
        }
        if let Ok(mut execs) = self.executors.write() {
            execs.remove(tool_name);
        }

        // Remove from disk
        let json_path = self.plugins_dir.join(format!("{}.json", tool_name));
        let rhai_path = self.plugins_dir.join(format!("{}.rhai", tool_name));
        let _ = std::fs::remove_file(&json_path);
        let _ = std::fs::remove_file(&rhai_path);

        info!("[ToolRegistry] 🗑️ Deleted tool '{}' permanently", tool_name);
        Ok(())
    }
}

impl Default for VectorToolRegistry {
    fn default() -> Self {
        let dir = dirs::home_dir().map(|h| h.join(".telos").join("tools")).unwrap_or_else(|| PathBuf::from("./tools"));
        Self::new(dir)
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
        let registry = VectorToolRegistry::new_for_test();
        
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
