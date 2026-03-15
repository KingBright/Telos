use crate::agents::{
    async_trait, AgentInput, AgentOutput, Arc, ExecutableNode, GatewayManager, SystemRegistry,
};
use telos_model_gateway::{Capability, LlmRequest, Message, ModelGateway};
use telos_tooling::ToolRegistry;
use tracing::info;
use std::collections::HashSet;

/// Parsed search result from the structured web_search output
#[derive(Clone, Debug, serde::Deserialize)]
struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

pub struct SearchWorkerAgent {
    pub gateway: Arc<GatewayManager>,
    pub tool_registry: std::sync::Arc<tokio::sync::RwLock<telos_tooling::retrieval::VectorToolRegistry>>,
}

impl SearchWorkerAgent {
    pub fn new(
        gateway: Arc<GatewayManager>,
        tool_registry: std::sync::Arc<tokio::sync::RwLock<telos_tooling::retrieval::VectorToolRegistry>>,
    ) -> Self {
        Self { gateway, tool_registry }
    }

    /// Phase 1: Use LLM to generate multiple search queries from a natural language intent
    async fn generate_search_queries(&self, intent: &str) -> Vec<String> {
        let prompt = format!(
            r#"You are a search keyword engineer. Given the user's search intent, generate 3-5 high-quality search queries.

Rules:
1. Each query targets a different angle (e.g., event name, related entities, technical terms, time-scoped)
2. Include at least 1 Chinese query AND 1 English query
3. Use search-engine-friendly format: concise, precise, with temporal qualifiers where relevant
4. For time-sensitive queries, include year/month in the query
5. Prefer professional terminology over colloquial descriptions
6. Queries should be diverse — avoid near-duplicates

Search Intent: "{}"

Output ONLY a JSON object: {{ "queries": ["query1", "query2", ...] }}"#,
            intent
        );

        let req = LlmRequest {
            session_id: "search_worker_keygen".to_string(),
            messages: vec![
                Message { role: "system".to_string(), content: "You are a search keyword engineer. Output ONLY valid JSON.".to_string() },
                Message { role: "user".to_string(), content: prompt },
            ],
            required_capabilities: Capability { requires_vision: false, strong_reasoning: false },
            budget_limit: 500,
        };

        match self.gateway.generate(req).await {
            Ok(res) => {
                let content = res.content.trim();
                let json_str = if let (Some(s), Some(e)) = (content.find('{'), content.rfind('}')) {
                    if e > s { &content[s..=e] } else { content }
                } else { content };

                if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if let Some(queries) = json.get("queries").and_then(|v| v.as_array()) {
                        let result: Vec<String> = queries.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect();
                        if !result.is_empty() {
                            info!("[SearchWorker] Generated {} search queries: {:?}", result.len(), result);
                            return result;
                        }
                    }
                }
                info!("[SearchWorker] Query generation failed, using intent as fallback");
                vec![intent.to_string()]
            }
            Err(_) => vec![intent.to_string()],
        }
    }

    /// Execute a single web_search call — returns structured SearchResults
    async fn execute_search(&self, query: &str) -> Vec<SearchResult> {
        let registry_guard = self.tool_registry.read().await;
        if let Some(executor) = registry_guard.get_executor("web_search") {
            drop(registry_guard);
            let params = serde_json::json!({ "query": query });
            match executor.call(params).await {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes).to_string();
                    // Parse as structured SearchResult array
                    match serde_json::from_str::<Vec<SearchResult>>(&text) {
                        Ok(results) => {
                            info!("[SearchWorker] Search '{}' returned {} structured results", query, results.len());
                            results
                        }
                        Err(_) => {
                            // Fallback: if it's an old-format Vec<String>, wrap in SearchResult
                            if let Ok(strings) = serde_json::from_str::<Vec<String>>(&text) {
                                info!("[SearchWorker] Search '{}' returned {} legacy results", query, strings.len());
                                strings.into_iter().map(|s| SearchResult { 
                                    title: String::new(), 
                                    url: String::new(), 
                                    snippet: s 
                                }).collect()
                            } else {
                                info!("[SearchWorker] Search '{}' returned unparseable: {} chars", query, text.len());
                                vec![]
                            }
                        }
                    }
                }
                Err(e) => {
                    info!("[SearchWorker] Search '{}' failed: {:?}", query, e);
                    vec![]
                }
            }
        } else {
            drop(registry_guard);
            info!("[SearchWorker] web_search tool not found in registry");
            vec![]
        }
    }

    /// Scrape a URL to get full page content
    async fn scrape_url(&self, url: &str) -> Option<String> {
        let registry_guard = self.tool_registry.read().await;
        if let Some(executor) = registry_guard.get_executor("web_scrape") {
            drop(registry_guard);
            let params = serde_json::json!({ "url": url });
            match executor.call(params).await {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes).to_string();
                    // Parse structured output
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                        let content = json.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let title = json.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        if content.len() > 100 {
                            info!("[SearchWorker] Scraped '{}' -> {} chars (title: '{}')", url, content.len(), title);
                            return Some(format!("**{}**\nSource: {}\n\n{}", title, url, content));
                        }
                    }
                    // Fallback: raw text
                    if text.len() > 100 {
                        info!("[SearchWorker] Scraped '{}' -> {} chars (raw)", url, text.len());
                        return Some(format!("Source: {}\n\n{}", url, text));
                    }
                    None
                }
                Err(e) => {
                    info!("[SearchWorker] Scrape '{}' failed: {:?}", url, e);
                    None
                }
            }
        } else {
            drop(registry_guard);
            None
        }
    }

    /// Format search results as readable text with titles and URLs
    fn format_search_results(results: &[SearchResult]) -> String {
        results.iter().enumerate().map(|(i, r)| {
            if !r.url.is_empty() {
                format!("{}. **{}**\n   URL: {}\n   {}", i + 1, r.title, r.url, r.snippet)
            } else {
                format!("{}. **{}**\n   {}", i + 1, r.title, r.snippet)
            }
        }).collect::<Vec<_>>().join("\n\n")
    }

    /// Batch quality assessment — evaluate all content at once instead of per-query
    async fn batch_assess(&self, intent: &str, content: &str) -> (bool, String) {
        if content.trim().is_empty() {
            return (false, String::new());
        }

        let truncated: String = content.chars().take(5000).collect();
        let prompt = format!(
            r#"You are a search result quality assessor.

Search Intent: "{}"

Collected Content:
{}

Evaluate:
1. Does the content contain information RELEVANT to the search intent?
2. Extract ONLY the relevant, useful information (discard ads, SEO filler, navigation text, unrelated content).
3. Preserve factual details, statistics, dates, and source attributions.

Output JSON:
{{
  "has_useful_content": true/false,
  "relevant_extract": "extracted useful content here (or empty string if nothing useful)"
}}"#,
            intent, truncated
        );

        let req = LlmRequest {
            session_id: "search_worker_batch_assess".to_string(),
            messages: vec![
                Message { role: "system".to_string(), content: "You are a search result quality assessor. Output ONLY valid JSON.".to_string() },
                Message { role: "user".to_string(), content: prompt },
            ],
            required_capabilities: Capability { requires_vision: false, strong_reasoning: false },
            budget_limit: 1500,
        };

        match self.gateway.generate(req).await {
            Ok(res) => {
                let content = res.content.trim();
                let json_str = if let (Some(s), Some(e)) = (content.find('{'), content.rfind('}')) {
                    if e > s { &content[s..=e] } else { content }
                } else { content };

                if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
                    let useful = json.get("has_useful_content").and_then(|v| v.as_bool()).unwrap_or(false);
                    let extract = json.get("relevant_extract").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    return (useful, extract);
                }
                (true, truncated)
            }
            Err(_) => (true, truncated),
        }
    }

    /// Generate corrected search queries based on CorrectionDirective from Critic feedback.
    /// This implements the "correction, not retry" principle — each iteration adjusts
    /// its approach based on what the Critic diagnosed as missing or wrong.
    async fn generate_corrected_queries(
        &self,
        intent: &str,
        correction: &telos_core::CorrectionDirective,
    ) -> Vec<String> {
        let corrections_text = if correction.correction_instructions.is_empty() {
            "No specific corrections provided.".to_string()
        } else {
            correction.correction_instructions.iter()
                .enumerate()
                .map(|(i, c)| format!("{}. {}", i + 1, c))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let prompt = format!(
            r#"You are a search keyword engineer performing a CORRECTED search.

Original Intent: "{}"

Previous Iteration (#{}):
- Score: {:.1}/1.0
- Diagnosis: {}
- Previous Summary: {}

Correction Instructions:
{}

Generate 3-5 NEW search queries that address the corrections above.
Do NOT repeat queries from the previous iteration.
Focus on what was MISSING or WRONG according to the diagnosis.

Output ONLY a JSON object: {{ "queries": ["query1", "query2", ...] }}"#,
            intent,
            correction.iteration,
            correction.satisfaction_score,
            correction.diagnosis,
            correction.previous_summary,
            corrections_text
        );

        let req = LlmRequest {
            session_id: "search_worker_corrected_keygen".to_string(),
            messages: vec![
                Message { role: "system".to_string(), content: "You are a search keyword engineer. Output ONLY valid JSON.".to_string() },
                Message { role: "user".to_string(), content: prompt },
            ],
            required_capabilities: Capability { requires_vision: false, strong_reasoning: false },
            budget_limit: 500,
        };

        match self.gateway.generate(req).await {
            Ok(res) => {
                let content = res.content.trim();
                let json_str = if let (Some(s), Some(e)) = (content.find('{'), content.rfind('}')) {
                    if e > s { &content[s..=e] } else { content }
                } else { content };

                if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if let Some(queries) = json.get("queries").and_then(|v| v.as_array()) {
                        let result: Vec<String> = queries.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect();
                        if !result.is_empty() {
                            info!("[SearchWorker] 🔄 Generated {} corrected queries (iter {}): {:?}",
                                result.len(), correction.iteration, result);
                            return result;
                        }
                    }
                }
                info!("[SearchWorker] Corrected query generation failed, using intent as fallback");
                vec![intent.to_string()]
            }
            Err(_) => vec![intent.to_string()],
        }
    }
}

#[async_trait]
impl ExecutableNode for SearchWorkerAgent {
    async fn execute(&self, input: AgentInput, _registry: &dyn SystemRegistry) -> AgentOutput {
        let intent = &input.task;
        
        // Read search mode from schema_payload (set by Architect at planning time)
        let mode = input.schema_payload
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| v.get("mode").and_then(|m| m.as_str().map(|s| s.to_string())))
            .unwrap_or_default();
        
        let use_deep = mode == "deep";
        
        info!("[SearchWorker] 🔍 Starting search (mode={}): \"{}\"", 
            if use_deep { "deep" } else { "direct" }, intent);

        // === DIRECT MODE: single search + optional auto-scrape ===
        if !use_deep {
            // Extract keyword hints from task if present
            let direct_query = if let Some(kw_pos) = intent.find("— keywords:").or_else(|| intent.find("— 关键词:")).or_else(|| intent.find("keywords:")) {
                let kw_section = &intent[kw_pos..];
                let kw_start = kw_section.find(':').map(|i| i + 1).unwrap_or(0);
                let keywords: Vec<&str> = kw_section[kw_start..].split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
                if !keywords.is_empty() {
                    info!("[SearchWorker] Extracted {} keyword hints, using first: \"{}\"", keywords.len(), keywords[0]);
                    keywords[0].to_string()
                } else {
                    intent.to_string()
                }
            } else {
                intent.to_string()
            };

            let results = self.execute_search(&direct_query).await;
            if !results.is_empty() {
                // Format structured results (with titles & URLs)
                let formatted = Self::format_search_results(&results);
                
                // Check if snippets are substantial enough (>500 chars total = likely good enough)
                let total_snippet_len: usize = results.iter().map(|r| r.snippet.len()).sum();
                
                if total_snippet_len > 500 {
                    // Snippets are rich enough, return directly without scraping
                    info!("[SearchWorker] ✅ Direct search successful ({} results, {} snippet chars)", results.len(), total_snippet_len);
                    return AgentOutput::success(serde_json::json!({
                        "text": formatted,
                        "queries_used": [direct_query],
                        "useful_results": results.len(),
                        "mode": "direct"
                    }));
                }
                
                // Snippets are thin — try auto-scraping top 1-2 URLs for richer content
                let scrape_urls: Vec<&str> = results.iter()
                    .filter(|r| r.url.starts_with("http"))
                    .take(2)
                    .map(|r| r.url.as_str())
                    .collect();
                
                if !scrape_urls.is_empty() {
                    info!("[SearchWorker] Snippets thin ({} chars), auto-scraping {} URLs", total_snippet_len, scrape_urls.len());
                    let mut scraped_content = formatted.clone();
                    for url in &scrape_urls {
                        if let Some(content) = self.scrape_url(url).await {
                            scraped_content.push_str("\n\n--- Full Page Content ---\n\n");
                            scraped_content.push_str(&content);
                        }
                    }
                    info!("[SearchWorker] ✅ Direct search + auto-scrape completed");
                    return AgentOutput::success(serde_json::json!({
                        "text": scraped_content,
                        "queries_used": [direct_query],
                        "useful_results": results.len(),
                        "scraped_urls": scrape_urls,
                        "mode": "direct+scrape"
                    }));
                }
                
                // No scrape-able URLs, return snippet results as-is
                info!("[SearchWorker] ✅ Direct search completed (no URLs to scrape)");
                return AgentOutput::success(serde_json::json!({
                    "text": formatted,
                    "queries_used": [direct_query],
                    "useful_results": results.len(),
                    "mode": "direct"
                }));
            }
            info!("[SearchWorker] Direct search returned no results, escalating to deep pipeline");
        }

        // === DEEP MODE: keyword engineering + multi-query + batch scrape ===
        
        // Phase 1: Keyword Engineering (corrected if CorrectionDirective present)
        let queries = if let Some(ref correction) = input.correction {
            info!("[SearchWorker] 🔄 Iteration {} — applying corrections. Diagnosis: {}",
                correction.iteration, correction.diagnosis);
            self.generate_corrected_queries(intent, correction).await
        } else {
            self.generate_search_queries(intent).await
        };

        // Phase 2: Execute all queries, collect structured results
        let mut all_results: Vec<SearchResult> = Vec::new();
        let mut seen_urls: HashSet<String> = HashSet::new();
        let mut successful_queries: Vec<String> = Vec::new();

        for (i, query) in queries.iter().enumerate() {
            info!("[SearchWorker] Executing query {}/{}: \"{}\"", i + 1, queries.len(), query);
            let results = self.execute_search(query).await;
            
            if results.is_empty() {
                continue;
            }

            // Deduplicate by URL
            let mut new_count = 0;
            for r in results {
                if r.url.is_empty() || seen_urls.insert(r.url.clone()) {
                    all_results.push(r);
                    new_count += 1;
                }
            }
            
            if new_count > 0 {
                successful_queries.push(query.clone());
                info!("[SearchWorker] Query {} added {} new results (total: {})", i + 1, new_count, all_results.len());
            }

            // Stop if we have enough results
            if all_results.len() >= 15 {
                info!("[SearchWorker] Sufficient results gathered ({}), stopping search", all_results.len());
                break;
            }
        }

        if all_results.is_empty() {
            // Last resort: try a simpler fallback search
            info!("[SearchWorker] No results from initial queries. Attempting fallback...");
            let fallback_query = format!("{} site:wikipedia.org OR site:arxiv.org", intent);
            let results = self.execute_search(&fallback_query).await;
            for r in results {
                all_results.push(r);
            }
        }

        if all_results.is_empty() {
            info!("[SearchWorker] ⚠️ All search strategies exhausted with no results");
            return AgentOutput::success(serde_json::json!({
                "text": format!("[SearchWorker] Exhausted {} search queries for intent '{}' but found no results.", queries.len(), intent),
                "queries_tried": queries,
                "useful_results": 0
            }));
        }

        // Phase 3: Auto-scrape top URLs for richer content
        let scrape_urls: Vec<String> = all_results.iter()
            .filter(|r| r.url.starts_with("http"))
            .take(3) // Scrape top 3 URLs in deep mode
            .map(|r| r.url.clone())
            .collect();

        let mut combined_content = Self::format_search_results(&all_results);
        let mut scraped_urls_success = Vec::new();

        if !scrape_urls.is_empty() {
            info!("[SearchWorker] Deep mode: scraping top {} URLs for full content", scrape_urls.len());
            for url in &scrape_urls {
                if let Some(content) = self.scrape_url(url).await {
                    combined_content.push_str("\n\n--- Full Page Content ---\n\n");
                    combined_content.push_str(&content);
                    scraped_urls_success.push(url.clone());
                }
            }
        }

        // Phase 4: Batch quality assessment (single LLM call for ALL content)
        let (useful, extract) = self.batch_assess(intent, &combined_content).await;

        if useful && !extract.is_empty() {
            info!("[SearchWorker] ✅ Deep search completed: {} results, {} scraped pages", all_results.len(), scraped_urls_success.len());
            AgentOutput::success(serde_json::json!({
                "text": extract,
                "queries_used": successful_queries,
                "useful_results": all_results.len(),
                "scraped_urls": scraped_urls_success,
                "mode": "deep"
            }))
        } else {
            // Even if assessment says not useful, return the raw data — let the summarizer decide
            info!("[SearchWorker] ⚠️ Quality assessment negative, returning raw results anyway");
            AgentOutput::success(serde_json::json!({
                "text": combined_content,
                "queries_used": successful_queries,
                "useful_results": all_results.len(),
                "scraped_urls": scraped_urls_success,
                "mode": "deep",
                "quality_warning": "Assessment found no directly relevant content"
            }))
        }
    }
}
