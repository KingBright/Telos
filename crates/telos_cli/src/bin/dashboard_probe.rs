use std::io::Write;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("┌──────────────────────────────────────────────────────────────┐");
    println!("│            Telos Dashboard Diagnostic Probe                 │");
    println!("│  Tests the same API endpoints the frontend dashboard uses   │");
    println!("│  All services run on a single port: 8321                    │");
    println!("└──────────────────────────────────────────────────────────────┘\n");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    
    // ====== 1. Test Dashboard index.html ======
    print!("🌐 [1/4] GET http://127.0.0.1:8321/ (dashboard UI) ... ");
    std::io::stdout().flush().ok();
    match client.get("http://127.0.0.1:8321/").send().await {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status.is_success() && body.contains("Telos Dashboard") {
                println!("✅ {} ({} bytes, title found)", status, body.len());
            } else if status.is_success() {
                println!("⚠️ {} ({} bytes, but title 'Telos Dashboard' NOT found)", status, body.len());
            } else {
                println!("❌ HTTP {}", status);
            }
        }
        Err(e) => println!("❌ CONNECTION FAILED: {}", e),
    }

    // ====== 2. Test /api/v1/metrics ======
    print!("\n📊 [2/4] GET http://127.0.0.1:8321/api/v1/metrics ... ");
    std::io::stdout().flush().ok();
    match client.get("http://127.0.0.1:8321/api/v1/metrics").send().await {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status.is_success() {
                println!("✅ {} ({} bytes)", status, body.len());
                match serde_json::from_str::<serde_json::Value>(&body) {
                    Ok(json) => {
                        println!("   ┌─ memory_os:");
                        if let Some(mem) = json.get("memory_os") {
                            println!("   │  episodic_nodes:   {}", mem.get("episodic_nodes").unwrap_or(&serde_json::Value::Null));
                            println!("   │  semantic_nodes:   {}", mem.get("semantic_nodes").unwrap_or(&serde_json::Value::Null));
                            println!("   │  procedural_nodes: {}", mem.get("procedural_nodes").unwrap_or(&serde_json::Value::Null));
                        }
                        println!("   ├─ dynamic_tooling:");
                        if let Some(dt) = json.get("dynamic_tooling") {
                            println!("   │  execution_success: {}", dt.get("execution_success").unwrap_or(&serde_json::Value::Null));
                            println!("   │  execution_failure: {}", dt.get("execution_failure").unwrap_or(&serde_json::Value::Null));
                        }
                        println!("   ├─ task_flow:");
                        if let Some(tf) = json.get("task_flow") {
                            println!("   │  total_success:          {}", tf.get("total_success").unwrap_or(&serde_json::Value::Null));
                            println!("   │  total_failures:         {}", tf.get("total_failures").unwrap_or(&serde_json::Value::Null));
                            println!("   │  active_concurrent_tasks: {}", tf.get("active_concurrent_tasks").unwrap_or(&serde_json::Value::Null));
                        }
                        println!("   ├─ agent:");
                        if let Some(agent) = json.get("agent") {
                            println!("   │  qa_passes:              {}", agent.get("qa_passes").unwrap_or(&serde_json::Value::Null));
                            println!("   │  qa_failures:            {}", agent.get("qa_failures").unwrap_or(&serde_json::Value::Null));
                            println!("   │  proactive_interactions: {}", agent.get("proactive_interactions").unwrap_or(&serde_json::Value::Null));
                        }
                        println!("   ├─ llm:");
                        if let Some(llm) = json.get("llm") {
                            println!("   │  total_requests:  {}", llm.get("total_requests").unwrap_or(&serde_json::Value::Null));
                            println!("   │  http_429_errors: {}", llm.get("http_429_errors").unwrap_or(&serde_json::Value::Null));
                            println!("   │  other_api_errors: {}", llm.get("other_api_errors").unwrap_or(&serde_json::Value::Null));
                        }
                        println!("   └─ uptime_seconds: {}", json.get("uptime_seconds").unwrap_or(&serde_json::Value::Null));
                    }
                    Err(e) => println!("   ⚠️ Invalid JSON: {}", e),
                }
            } else {
                println!("❌ HTTP {}", status);
            }
        }
        Err(e) => println!("❌ CONNECTION FAILED: {}", e),
    }

    // ====== 3. Test /api/v1/traces ======
    print!("\n🔍 [3/4] GET http://127.0.0.1:8321/api/v1/traces ... ");
    std::io::stdout().flush().ok();
    match client.get("http://127.0.0.1:8321/api/v1/traces").send().await {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status.is_success() {
                let count = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| v.get("traces")?.as_array().map(|a| a.len()));
                println!("✅ {} ({} bytes, {} traces)", status, body.len(), count.unwrap_or(0));
            } else {
                println!("❌ HTTP {}", status);
            }
        }
        Err(e) => println!("❌ CONNECTION FAILED: {}", e),
    }

    // ====== 4. Test WebSocket ======
    print!("\n🔌 [4/4] WS ws://127.0.0.1:8321/api/v1/stream ... ");
    std::io::stdout().flush().ok();
    match tokio_tungstenite::connect_async("ws://127.0.0.1:8321/api/v1/stream").await {
        Ok((ws_stream, resp)) => {
            println!("✅ Connected (HTTP {})", resp.status());
            use futures_util::StreamExt;
            let (_, mut read) = ws_stream.split();
            println!("   Listening for 3 seconds...");
            let timeout = tokio::time::timeout(std::time::Duration::from_secs(3), async {
                let mut count = 0;
                while let Some(msg) = read.next().await {
                    match msg {
                        Ok(m) => {
                            let text = m.to_text().unwrap_or("(binary)");
                            let preview = if text.len() > 120 { &text[..120] } else { text };
                            println!("   📩 MSG {}: {}", count + 1, preview);
                            count += 1;
                            if count >= 3 { break; }
                        }
                        Err(e) => { println!("   ⚠️ WS Error: {}", e); break; }
                    }
                }
                count
            }).await;
            match timeout {
                Ok(n) => println!("   Received {} messages in 3s", n),
                Err(_) => println!("   ℹ️ No messages in 3s (idle — normal when no tasks running)"),
            }
        }
        Err(e) => println!("❌ CONNECTION FAILED: {}", e),
    }

    println!("\n────────────────────────────────────────────────────────");
    println!("Diagnosis complete. All services on port 8321.");
    
    Ok(())
}
