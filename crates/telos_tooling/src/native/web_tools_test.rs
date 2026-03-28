#[cfg(test)]
mod tests {
    use crate::native::web_tools::*;
    use crate::ToolExecutor;
    use serde_json::json;

    #[tokio::test]
    async fn test_web_search_fallback() {
        println!("Testing WebSearchTool fallback (no api key)...");
        std::env::remove_var("SERPER_API_KEY");

        let tool = WebSearchTool;
        let params = json!({
            "query": "大模型 推理 2026年3月",
            "use_proxy": false
        });

        match tool.call(params).await {
            Ok(bytes) => {
                let results: Vec<SearchResult> = serde_json::from_slice(&bytes).unwrap();
                println!("Got {} results: {:#?}", results.len(), results);
                assert!(!results.is_empty(), "Fallback search returned 0 results!");
            }
            Err(e) => {
                println!("Search failed: {:?}", e);
                panic!("Search failed: {:?}", e);
            }
        }
    }
}
