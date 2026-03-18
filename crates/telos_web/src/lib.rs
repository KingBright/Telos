pub mod metrics;

use axum::{
    routing::get,
    Router,
    response::Json,
    extract::State,
};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::services::ServeDir;
use tower_http::cors::{CorsLayer, Any};
use tracing::{info, error};
use std::net::SocketAddr;

pub type MetricsState = Arc<RwLock<metrics::GlobalTelemetryMetrics>>;

pub async fn start_web_server(port: u16, metrics: MetricsState) {
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
        // API Endpoints
        .route("/api/metrics", get(get_metrics))
        // Static Files (Frontend UI)
        .fallback_service(serve_dir)
        .layer(cors)
        .with_state(metrics);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("[Telemetry Dashboard] Web server listening on http://{}", addr);

    if let Err(e) = axum::serve(tokio::net::TcpListener::bind(&addr).await.unwrap(), app).await {
        error!("[Telemetry Dashboard] Server error: {}", e);
    }
}

async fn get_metrics(State(metrics): State<MetricsState>) -> Json<metrics::GlobalTelemetryMetrics> {
    let data = metrics.read().await.clone();
    Json(data)
}
