#[tokio::main]
async fn main() {
    let client = reqwest::Client::builder().build().unwrap();
    let res = client.get("https://cn.bing.com/search?q=AI+reasoning+2026&ensearch=0")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        .send().await.unwrap().text().await.unwrap();
    
    // Dump first 1000 chars of HTML
    println!("{}", &res[..2000.min(res.len())]);
    
    // Write full HTML to file
    std::fs::write("bing_output.html", res).unwrap();
}
