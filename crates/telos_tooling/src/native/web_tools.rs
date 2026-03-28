use crate::{JsonSchema, ToolError, ToolExecutor, ToolSchema};
use tracing::{info, debug, warn, error};
use async_trait::async_trait;
use serde_json::Value;
use telos_core::RiskLevel;
// 11. Http Tool
#[derive(Clone)]
pub struct HttpTool;

#[async_trait]
impl ToolExecutor for HttpTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'url'".into()))?;

        let output = reqwest::get(url)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("HTTP request failed: {}", e)))?
            .bytes()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read response body: {}", e)))?;

        Ok(output.to_vec())
    }
}

impl HttpTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "http_get".into(),
            description: "Fetches the content of a URL.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string" }
                    },
                    "required": ["url"]
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}

// 12. WebSearch Tool - 多搜索引擎支持 (结构化输出)

/// Structured search result with title, URL, and snippet
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

impl SearchResult {
    fn clean_text(s: &str) -> String {
        s.replace("&amp;", "&")
            .replace("&nbsp;", " ")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .trim()
            .to_string()
    }
}

#[derive(Clone)]
pub struct WebSearchTool;

impl WebSearchTool {
    fn create_client() -> Result<reqwest::Client, ToolError> {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|e| ToolError::ExecutionFailed(format!("Client build failed: {:?}", e)))
    }

    /// Create client with proxy support
    fn create_client_with_proxy(proxy_url: &str) -> Result<reqwest::Client, ToolError> {
        let proxy = reqwest::Proxy::all(proxy_url)
            .map_err(|e| ToolError::ExecutionFailed(format!("Invalid proxy URL: {:?}", e)))?;
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::limited(5))
            .proxy(proxy)
            .build()
            .map_err(|e| ToolError::ExecutionFailed(format!("Client build failed: {:?}", e)))
    }

    /// Get proxy URL from environment (TELOS_PROXY, HTTP_PROXY, or HTTPS_PROXY)
    fn get_proxy_url() -> Option<String> {
        std::env::var("TELOS_PROXY")
            .or_else(|_| std::env::var("HTTPS_PROXY"))
            .or_else(|_| std::env::var("HTTP_PROXY"))
            .ok()
    }

    /// Fetch HTML from a search engine URL
    async fn fetch_html(client: &reqwest::Client, url: &str, accept_lang: &str) -> Result<String, ToolError> {
        client.get(url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .header("Accept-Language", accept_lang)
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Request failed: {:?}", e)))?
            .text()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read response: {:?}", e)))
    }

    /// 使用 DuckDuckGo Lite 搜索 — 纯 POST 防封锁模式
    async fn search_duckduckgo(query: &str, client: &reqwest::Client) -> Result<Vec<SearchResult>, ToolError> {
        let search_url = "https://lite.duckduckgo.com/lite/";
        let body_content = format!("q={}", urlencoding::encode(query));

        let html = client.post(search_url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .header("Accept-Language", "en-US,en;q=0.9,zh-CN;q=0.8")
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body_content)
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("DuckDuckGo request failed: {:?}", e)))?
            .text()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("DuckDuckGo failed to read body: {:?}", e)))?;

        if html.contains("bots use DuckDuckGo") || html.contains("blocked") {
            warn!("[WebSearch] DuckDuckGo bot detection triggered.");
            return Ok(vec![]);
        }

        let document = scraper::Html::parse_document(&html);
        let mut results = Vec::new();
        
        let result_tr_sel = scraper::Selector::parse("tr").unwrap();
        let link_sel = scraper::Selector::parse("a.result-link, a.result-url").unwrap();
        let snippet_sel = scraper::Selector::parse("td.result-snippet").unwrap();

        let mut current_title = String::new();
        let mut current_url = String::new();

        for tr in document.select(&result_tr_sel) {
            if let Some(link) = tr.select(&link_sel).next() {
                current_title = SearchResult::clean_text(&link.text().collect::<String>());
                let href = link.value().attr("href").unwrap_or("").to_string();
                
                // DDG Lite URLs 经常包含 uddg= 参数代理，将其解开拿到直链
                current_url = if let Some(pos) = href.find("uddg=") {
                    urlencoding::decode(&href[pos + 5..]).unwrap_or_default().split('&').next().unwrap_or_default().to_string()
                } else {
                    href
                };
            } else if let Some(snippet_td) = tr.select(&snippet_sel).next() {
                let snippet = SearchResult::clean_text(&snippet_td.text().collect::<String>());
                
                if !current_title.is_empty() && (!snippet.is_empty() || !current_url.is_empty()) {
                    results.push(SearchResult { 
                        title: current_title.clone(), 
                        url: current_url.clone(), 
                        snippet 
                    });
                    current_title.clear();
                    current_url.clear();
                }
            }
        }

        if !results.is_empty() {
            info!("[WebSearch] DDG Lite POST parser found {} results", results.len());
        } else {
            debug!("[WebSearch] DDG Lite no results. HTML len={}", html.len());
        }

        Ok(results)
    }

    /// 使用 Bing 搜索 — CSS selector + fallback
    async fn search_bing(query: &str, client: &reqwest::Client) -> Result<Vec<SearchResult>, ToolError> {
        let encoded_query = urlencoding::encode(query);
        let search_url = format!("https://cn.bing.com/search?q={}&ensearch=0", encoded_query);
        let html = Self::fetch_html(client, &search_url, "zh-CN,zh;q=0.9,en;q=0.8").await?;

        let document = scraper::Html::parse_document(&html);

        // Strategy 1: CSS selectors (try multiple known selectors)
        let result_selectors = [".b_algo", "li.b_algo", ".b_results .b_algo", "li[class*=\"algo\"]", "div[class*=\"algo\"]"];
        let mut results = Vec::new();

        for sel_str in &result_selectors {
            if let Ok(result_sel) = scraper::Selector::parse(sel_str) {
                let title_sel = scraper::Selector::parse("h2 a").unwrap();
                let snippet_sel = scraper::Selector::parse("p, .b_caption p, .b_lineclamp2, .b_algoSlug, div[class*=\"lineclamp\"], div[class*=\"caption\"]").unwrap();

                for result in document.select(&result_sel).take(10) {
                    let title_elem = result.select(&title_sel).next();
                    let title = title_elem
                        .map(|e| SearchResult::clean_text(&e.text().collect::<String>()))
                        .unwrap_or_default();
                    let url = title_elem
                        .and_then(|e| e.value().attr("href"))
                        .unwrap_or("")
                        .to_string();
                    let mut snippet = result.select(&snippet_sel).next()
                        .map(|e| SearchResult::clean_text(&e.text().collect::<String>()))
                        .unwrap_or_default();
                        
                    if snippet.is_empty() {
                        let full_text = SearchResult::clean_text(&result.text().collect::<String>());
                        if full_text.len() > title.len() {
                            // Bing sometimes prepends "Web" or appends the URL
                            snippet = full_text.replace(&title, "").replace("Web", "").trim().to_string();
                        }
                    }

                    if !title.is_empty() && !url.is_empty() {
                        results.push(SearchResult { title, url, snippet });
                    }
                }
                if !results.is_empty() {
                    return Ok(results);
                }
            }
        }

        // Strategy 2: Fallback string matching
        debug!("[WebSearch] Bing CSS selectors found nothing. HTML len={}", html.len());
        for line in html.split('\n') {
            if line.contains("class=\"b_caption\"") || line.contains("class=\"b_algoSlug\"") || line.contains("class=\"b_lineclamp") || line.contains("class=\"algo") || line.contains("caption") {
                let clean = line.replace("<p>", "").replace("</p>", "").replace("<strong>", "").replace("</strong>", "").replace("<span>", "").replace("</span>", "");
                let text: String = clean
                    .split('>')
                    .skip(1)
                    .flat_map(|s| s.split('<').next().unwrap_or("").chars())
                    .collect();
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() && trimmed.len() > 10 {
                    results.push(SearchResult { title: String::new(), url: String::new(), snippet: trimmed });
                }
            }
        }
        if !results.is_empty() {
            info!("[WebSearch] Bing fallback parser found {} results", results.len());
        } else {
            debug!("[WebSearch] Bing no results. HTML preview: {}", &html.chars().take(300).collect::<String>());
        }
        Ok(results)
    }

    /// 使用百度资讯搜索 (Baidu News) — 适合实时天气、新闻、赛事安排
    async fn search_baidu_news(query: &str, client: &reqwest::Client) -> Result<Vec<SearchResult>, ToolError> {
        let encoded_query = urlencoding::encode(query);
        // tn=news: news search, rtt=1: sort by date (newest first), bsst=1: show time, cl=2: search news
        let search_url = format!("https://www.baidu.com/s?tn=news&rtt=1&bsst=1&cl=2&wd={}", encoded_query);
        let html = Self::fetch_html(client, &search_url, "zh-CN,zh;q=0.9").await?;

        let document = scraper::Html::parse_document(&html);

        // Strategy 1: CSS selectors for Baidu News
        let result_selectors = [".result-op", ".result", "div[class*=\"result\"]"];
        let mut results = Vec::new();

        for sel_str in &result_selectors {
            if let Ok(result_sel) = scraper::Selector::parse(sel_str) {
                let title_sel = scraper::Selector::parse("h3 a").unwrap();
                let snippet_sel = scraper::Selector::parse(".c-summary, .news-summary, span.c-font-normal, span.c-color-text, div.c-span-last").unwrap();

                for result in document.select(&result_sel).take(10) {
                    let title_elem = result.select(&title_sel).next();
                    let title = title_elem
                        .map(|e| SearchResult::clean_text(&e.text().collect::<String>()))
                        .unwrap_or_default();
                    let url = title_elem
                        .and_then(|e| e.value().attr("href"))
                        .unwrap_or("")
                        .to_string();
                    let snippet = result.select(&snippet_sel).next()
                        .map(|e| SearchResult::clean_text(&e.text().collect::<String>()))
                        .unwrap_or_default();

                    if !title.is_empty() && !snippet.is_empty() {
                        results.push(SearchResult { title, url, snippet });
                    }
                }
                if !results.is_empty() {
                    return Ok(results);
                }
            }
        }

        // Strategy 2: fallback string matching for news
        debug!("[WebSearch] Baidu News CSS selectors found nothing. HTML len={}", html.len());
        for line in html.split('\n') {
            if line.contains("class=\"c-summary\"") || line.contains("class=\"news-title\"") || line.contains("c-span-last") {
                let clean = line.replace("<em>", "").replace("</em>", "").replace("&nbsp;", " ");
                let text: String = clean.chars()
                    .skip_while(|c| *c != '>')
                    .skip(1)
                    .take_while(|c| *c != '<')
                    .collect();
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() && trimmed.len() > 10 {
                    results.push(SearchResult { title: String::new(), url: String::new(), snippet: trimmed });
                }
            }
        }
        if !results.is_empty() {
            info!("[WebSearch] Baidu News fallback parser found {} results", results.len());
        } else {
            debug!("[WebSearch] Baidu News no results. HTML preview: {}", &html.chars().take(300).collect::<String>());
        }
        Ok(results)
    }

    /// 使用百度搜索 — CSS selector + fallback
    async fn search_baidu(query: &str, client: &reqwest::Client) -> Result<Vec<SearchResult>, ToolError> {
        let encoded_query = urlencoding::encode(query);
        let search_url = format!("https://www.baidu.com/s?wd={}", encoded_query);
        let html = Self::fetch_html(client, &search_url, "zh-CN,zh;q=0.9").await?;

        let document = scraper::Html::parse_document(&html);

        // Strategy 1: CSS selectors
        let result_selectors = [".result.c-container", ".c-container", "div[class*=\"result\"]"];
        let mut results = Vec::new();

        for sel_str in &result_selectors {
            if let Ok(result_sel) = scraper::Selector::parse(sel_str) {
                let title_sel = scraper::Selector::parse("h3 a").unwrap();
                let snippet_sel = scraper::Selector::parse(".c-abstract, .c-span-last, .content-right_8Sakl, div[class*=\"content-right\"], div.c-span18").unwrap();

                for result in document.select(&result_sel).take(10) {
                    let title_elem = result.select(&title_sel).next();
                    let title = title_elem
                        .map(|e| SearchResult::clean_text(&e.text().collect::<String>()))
                        .unwrap_or_default();
                    let url = title_elem
                        .and_then(|e| e.value().attr("href"))
                        .unwrap_or("")
                        .to_string();
                    let snippet = result.select(&snippet_sel).next()
                        .map(|e| SearchResult::clean_text(&e.text().collect::<String>()))
                        .unwrap_or_default();

                    if !title.is_empty() && !snippet.is_empty() {
                        results.push(SearchResult { title, url, snippet });
                    }
                }
                if !results.is_empty() {
                    return Ok(results);
                }
            }
        }

        // Strategy 2: fallback string matching
        debug!("[WebSearch] Baidu CSS selectors found nothing. HTML len={}", html.len());
        for line in html.split('\n') {
            if line.contains("class=\"c-abstract\"") || line.contains("class=\"content-right") || line.contains("c-span-last") {
                let clean = line.replace("<em>", "").replace("</em>", "").replace("&nbsp;", " ");
                let text: String = clean.chars()
                    .skip_while(|c| *c != '>')
                    .skip(1)
                    .take_while(|c| *c != '<')
                    .collect();
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() && trimmed.len() > 10 {
                    results.push(SearchResult { title: String::new(), url: String::new(), snippet: trimmed });
                }
            }
        }
        if !results.is_empty() {
            info!("[WebSearch] Baidu fallback parser found {} results", results.len());
        } else {
            debug!("[WebSearch] Baidu no results. HTML preview: {}", &html.chars().take(300).collect::<String>());
        }
        Ok(results)
    }



    /// 使用 Serper API 搜索 (serper.dev) — 直接返回结构化 JSON
    async fn search_serper(query: &str) -> Result<Vec<SearchResult>, ToolError> {
        let api_key = std::env::var("SERPER_API_KEY")
            .map_err(|_| ToolError::ExecutionFailed("SERPER_API_KEY not set".into()))?;

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| ToolError::ExecutionFailed(format!("Client build failed: {:?}", e)))?;

        let body = serde_json::json!({
            "q": query,
            "gl": "cn",
            "hl": "zh-cn",
            "num": 10
        });

        let resp = client.post("https://google.serper.dev/search")
            .header("X-API-KEY", &api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Serper request failed: {:?}", e)))?;

        if !resp.status().is_success() {
            return Err(ToolError::ExecutionFailed(format!("Serper returned HTTP {}", resp.status())));
        }

        let data: serde_json::Value = resp.json().await
            .map_err(|e| ToolError::ExecutionFailed(format!("Serper JSON decode failed: {:?}", e)))?;

        let mut results = Vec::new();
        if let Some(organic) = data.get("organic").and_then(|v| v.as_array()) {
            for item in organic.iter().take(10) {
                let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let url = item.get("link").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let snippet = item.get("snippet").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if !title.is_empty() {
                    results.push(SearchResult { title, url, snippet });
                }
            }
        }

        // Also check answerBox for quick answers (weather, etc.)
        if let Some(answer_box) = data.get("answerBox") {
            let title = answer_box.get("title").and_then(|v| v.as_str()).unwrap_or("Answer").to_string();
            let snippet = answer_box.get("answer")
                .or_else(|| answer_box.get("snippet"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !snippet.is_empty() {
                results.insert(0, SearchResult { title, url: String::new(), snippet });
            }
        }

        Ok(results)
    }
}

#[async_trait]
impl ToolExecutor for WebSearchTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'query'".into()))?;

        // Tier 1: Serper API (if API key configured) — fastest, most reliable
        let has_serper = std::env::var("SERPER_API_KEY").is_ok();
        if has_serper {
            match Self::search_serper(query).await {
                Ok(results) if !results.is_empty() => {
                    info!("[WebSearch] ✅ Serper API 返回 {} 条结构化结果", results.len());
                    return Ok(serde_json::to_vec(&results).unwrap_or_default());
                }
                Ok(_) => warn!("[WebSearch] ⚠️ Serper API 返回空结果，降级到网页搜索引擎"),
                Err(e) => warn!("[WebSearch] ⚠️ Serper API 失败: {:?}，降级到网页搜索引擎", e),
            }
        }

        // Tier 2: Web scraping fallback
        // KEY INSIGHT: Domestic engines (cn.bing.com, Baidu) MUST connect directly (no proxy) —
        // they are fast and reliable in China. Proxy only helps for international engines (DDG).
        // Google is removed: it returns JS-rendered HTML with no parseable results via HTTP.
        
        let direct_client = Self::create_client()?;
        
        let proxy_url = Self::get_proxy_url();
        let proxy_client = if let Some(ref proxy) = proxy_url {
            match Self::create_client_with_proxy(proxy) {
                Ok(c) => Some(c),
                Err(e) => {
                    warn!("[WebSearch] ⚠️ Proxy client creation failed: {:?}", e);
                    None
                }
            }
        } else {
            None
        };

        let max_retries = 2;
        let mut last_error_msg = String::new();

        // Helper macro for trying a search engine with a specific client
        macro_rules! try_engine {
            ($name:expr, $method:ident, $client:expr) => {
                match Self::$method(query, $client).await {
                    Ok(results) if !results.is_empty() => {
                        info!("[WebSearch] ✅ {} 返回 {} 条结构化结果", $name, results.len());
                        return Ok(serde_json::to_vec(&results).unwrap_or_default());
                    }
                    Ok(_) => warn!("[WebSearch] ⚠️ {} 返回空结果，尝试下一个引擎", $name),
                    Err(e) => {
                        last_error_msg = format!("{}: {:?}", $name, e);
                        warn!("[WebSearch] ❌ {} 失败: {}", $name, last_error_msg);
                    }
                }
            };
        }

        for attempt in 1..=max_retries {
            if attempt > 1 {
                let sleep_time = std::time::Duration::from_secs(2);
                warn!("[WebSearch] 🔄 Retry {}/{}...", attempt, max_retries);
                tokio::time::sleep(sleep_time).await;
            }

            // 首先尝试 DuckDuckGo Lite POST (因为最稳定, 但需要走代理池或者直连如果环境允许)
            if let Some(ref pc) = proxy_client {
                try_engine!("DuckDuckGo", search_duckduckgo, pc);
            } else {
                // 如果没有配置代理，也直接用 direct_client 试一把 DDG
                try_engine!("DuckDuckGo", search_duckduckgo, &direct_client);
            }

            // 备用兜底：Baidu News — 仍保留，用以抓取最新百度国内新闻结构
            try_engine!("Baidu News", search_baidu_news, &direct_client);
            
            // 备用兜底：Baidu (Web) — 作为最后方案
            try_engine!("Baidu", search_baidu, &direct_client);

            // 备用兜底：Bing CN
            try_engine!("Bing CN", search_bing, &direct_client);
        }

        if last_error_msg.is_empty() {
            Err(ToolError::ExecutionFailed("All search engines returned empty results. Try simpler keywords.".into()))
        } else {
            Err(ToolError::ExecutionFailed(format!("All search engines failed after {} retries. Last error: {}", max_retries, last_error_msg)))
        }
    }
}

impl WebSearchTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "web_search".into(),
            description: "Searches the web. Uses Serper API (Google-quality results) when SERPER_API_KEY is configured. Falls back to Bing/Baidu web scraping. Set use_proxy=false to force direct connection.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "The search query" },
                        "use_proxy": { "type": "boolean", "description": "Whether to use proxy for web scraping fallback (default: true when proxy is configured)." }
                    },
                    "required": ["query"]
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}


// 14. Web Scrape Tool — 增强版 (代理/超时/Readability/截断)
#[derive(Clone)]
pub struct WebScrapeTool;

impl WebScrapeTool {
    /// Get proxy URL from environment (reuses same env vars as WebSearchTool)
    fn get_proxy_url() -> Option<String> {
        std::env::var("TELOS_PROXY")
            .or_else(|_| std::env::var("HTTPS_PROXY"))
            .or_else(|_| std::env::var("HTTP_PROXY"))
            .ok()
    }

    fn create_client() -> Result<reqwest::Client, ToolError> {
        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::limited(5));

        if let Some(proxy_url) = Self::get_proxy_url() {
            if let Ok(proxy) = reqwest::Proxy::all(&proxy_url) {
                debug!("[WebScrape] Using proxy: {}", proxy_url);
                builder = builder.proxy(proxy);
            }
        }

        builder.build()
            .map_err(|e| ToolError::ExecutionFailed(format!("Client build failed: {:?}", e)))
    }

    /// Extract main content from HTML using readability heuristics
    fn extract_readable_content(document: &scraper::Html) -> String {
        // Priority order for content extraction:
        // 1. <article> element (most semantic)
        // 2. <main> element
        // 3. Elements with content-like class/id names
        // 4. Largest text block fallback

        let content_selectors = [
            "article",
            "main",
            "[role=\"main\"]",
            ".post-content",
            ".article-content",
            ".entry-content",
            ".content",
            "#content",
            ".post-body",
            ".article-body",
        ];

        for sel_str in &content_selectors {
            if let Ok(sel) = scraper::Selector::parse(sel_str) {
                if let Some(elem) = document.select(&sel).next() {
                    let text = Self::extract_clean_text_from_element(&elem);
                    if text.len() > 200 {
                        return text;
                    }
                }
            }
        }

        // Fallback: extract all body text, filtering out noise
        Self::extract_body_text(document)
    }

    /// Extract clean text from an element, skipping script/style/nav/footer
    fn extract_clean_text_from_element(elem: &scraper::ElementRef) -> String {
        let skip_tags = ["script", "style", "noscript", "nav", "footer", "header", "aside", "form", "iframe"];
        let mut text = String::new();

        for node in elem.descendants() {
            if let Some(text_node) = node.value().as_text() {
                // Check if any ancestor is a skip tag
                let mut should_skip = false;
                for parent in node.ancestors() {
                    if let Some(parent_elem) = scraper::ElementRef::wrap(parent) {
                        if skip_tags.contains(&parent_elem.value().name()) {
                            should_skip = true;
                            break;
                        }
                    }
                }
                if !should_skip {
                    let t = text_node.trim();
                    if !t.is_empty() {
                        text.push_str(t);
                        text.push(' ');
                    }
                }
            }
        }
        text
    }

    /// Extract text from body, filtering common noise elements
    fn extract_body_text(document: &scraper::Html) -> String {
        let mut clean_text = String::new();
        let skip_tags = ["script", "style", "noscript", "head", "nav", "footer", "header", "aside", "form", "iframe"];

        for node in document.tree.nodes() {
            if let Some(text_node) = node.value().as_text() {
                let mut should_ignore = false;
                for parent in node.ancestors() {
                    if let Some(elem) = scraper::ElementRef::wrap(parent) {
                        let tag = elem.value().name();
                        if skip_tags.contains(&tag) {
                            should_ignore = true;
                            break;
                        }
                    }
                }

                if !should_ignore {
                    let ts = text_node.trim();
                    if !ts.is_empty() {
                        clean_text.push_str(ts);
                        clean_text.push(' ');
                    }
                }
            }
        }
        clean_text
    }

    /// Extract page title
    fn extract_title(document: &scraper::Html) -> String {
        if let Ok(sel) = scraper::Selector::parse("title") {
            if let Some(elem) = document.select(&sel).next() {
                return elem.text().collect::<String>().trim().to_string();
            }
        }
        String::new()
    }
}

#[async_trait]
impl ToolExecutor for WebScrapeTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'url'".into()))?;

        let client = Self::create_client()?;
        let html_content = client.get(url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to fetch URL: {}", e)))?
            .text()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read HTML: {}", e)))?;

        let document = scraper::Html::parse_document(&html_content);
        let title = Self::extract_title(&document);
        let content = Self::extract_readable_content(&document);

        // Truncate to max 5000 chars to avoid overwhelming LLM
        let max_chars = 5000;
        let truncated: String = content.chars().take(max_chars).collect();
        let was_truncated = content.len() > max_chars;
        let word_count = truncated.split_whitespace().count();

        let result = serde_json::json!({
            "title": title,
            "url": url,
            "content": truncated,
            "word_count": word_count,
            "truncated": was_truncated,
        });

        info!("[WebScrape] Extracted {} chars from '{}' (title: '{}')", truncated.len(), url, title);
        Ok(serde_json::to_vec(&result).unwrap_or_else(|_| truncated.into_bytes()))
    }
}

impl WebScrapeTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "web_scrape".into(),
            description: "Fetches a webpage and extracts clean readable content using smart content extraction. Returns structured JSON with title, content, and word count. Supports proxy. Keywords: scrape, web_scrape, fetch, html, text, extraction.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "The URL to scrape" }
                    },
                    "required": ["url"]
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}

