use telos_tooling::native::WebSearchTool;
use telos_tooling::ToolExecutor;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tool = WebSearchTool;
    let params = json!({
        "query": "西湖马拉松 2026 比赛时间",
        "use_proxy": false
    });
    
    match tool.call(params).await {
        Ok(bytes) => {
            let s = String::from_utf8_lossy(&bytes);
            println!("Results:\n{}", s);
        }
        Err(e) => {
            println!("Error: {:?}", e);
        }
    }
    Ok(())
}
