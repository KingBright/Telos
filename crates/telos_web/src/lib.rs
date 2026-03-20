pub mod metrics;

use axum::{
    routing::get,
    Router,
};
use tower_http::services::ServeDir;
use tower_http::cors::{CorsLayer, Any};
use tracing::{info, error};
use std::net::SocketAddr;

pub async fn start_web_server(port: u16) {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let home_web_dir = dirs::home_dir().map(|h| h.join(".telos/web"));
    
    let serve_dir = if let Some(p) = &home_web_dir {
        if p.exists() {
            ServeDir::new(p)
        } else {
            ServeDir::new("crates/telos_web/static")
        }
    } else {
        ServeDir::new("crates/telos_web/static")
    };

    let app = Router::new()
        // Proxy API endpoints to the daemon (port 8321)
        .route("/api/metrics", get(proxy_metrics))
        .route("/api/traces", get(proxy_traces))
        // Static Files (Frontend UI)
        .fallback_service(serve_dir)
        .layer(cors);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("[Telos Dashboard] Web server listening on http://{}", addr);

    if let Err(e) = axum::serve(tokio::net::TcpListener::bind(&addr).await.unwrap(), app).await {
        error!("[Telos Dashboard] Server error: {}", e);
    }
}

/// Proxy metrics from the daemon API (port 8321) to the dashboard
async fn proxy_metrics() -> axum::response::Response {
    proxy_daemon_api("http://127.0.0.1:8321/api/v1/metrics").await
}

/// Proxy traces from the daemon API (port 8321) to the dashboard
async fn proxy_traces() -> axum::response::Response {
    proxy_daemon_api("http://127.0.0.1:8321/api/v1/traces").await
}

async fn proxy_daemon_api(url: &str) -> axum::response::Response {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();
    
    match client.get(url).send().await {
        Ok(resp) => {
            let status = axum::http::StatusCode::from_u16(resp.status().as_u16()).unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
            let body = resp.text().await.unwrap_or_else(|_| "{}".to_string());
            axum::response::Response::builder()
                .status(status)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap_or_else(|_| {
                    axum::response::Response::builder()
                        .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                        .body(axum::body::Body::from("{\"error\":\"proxy error\"}"))
                        .unwrap()
                })
        }
        Err(e) => {
            error!("[Telos Dashboard] Failed to proxy {}: {}", url, e);
            axum::response::Response::builder()
                .status(axum::http::StatusCode::BAD_GATEWAY)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(format!("{{\"error\":\"daemon unreachable: {}\"}}", e)))
                .unwrap()
        }
    }
}
