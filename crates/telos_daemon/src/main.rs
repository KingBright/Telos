use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
use tracing::{debug, info, error};
mod agents;
pub use workers::*;
pub use api::ws::ws_handler;

// Telemetry Metrics

// Core Traits and Primitives
use telos_context::providers::OpenAiProvider;
use telos_context::RaptorContextManager;
use telos_core::config::TelosConfig;
use telos_hci::{
    global_log_level, LogLevel, TokioEventBroker,
};
use telos_memory::engine::RedbGraphStore;
use telos_model_gateway::gateway::GatewayManager;

// 1. Adapter to convert Context OpenAiProvider to Gateway ModelProvider for the Gateway Manager


pub mod api;
pub mod core;
pub mod graph;
pub mod workers;

pub use crate::api::models::*;
pub use crate::api::handlers::*;
pub use crate::api::routes::*;
pub use crate::core::state::*;
pub use crate::core::metrics::*;
pub use crate::core::adapters::*;
pub use crate::graph::factory::*;
pub use crate::graph::nodes::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Initialize logging with timestamps, rotation, and size limits
    let log_dir = TelosConfig::logs_dir();
    let log_dir_str = log_dir.to_string_lossy();
    let _guard = telos_telemetry::init_standard_logging(
        "debug", // Default level, will be filtered by EnvFilter if TELOS_LOG_LEVEL is set
        Some(&log_dir_str),
        Some("daemon.log")
    );

    debug!("Initializing Telos Daemon...");

    let config = TelosConfig::load().expect(
        "Failed to load configuration. Please run `telos cli` first to complete initialization.",
    );

    // cleanup orphaned memory files from previous PID-suffix fallbacks
    let _ = TelosConfig::cleanup_orphaned_memory_files();

    // Set proxy environment variable from config if configured
    if let Some(ref proxy) = config.proxy {
        std::env::set_var("TELOS_PROXY", proxy);
        info!("[Daemon] Proxy configured: {}", proxy);
    }

    // Initialize SOUL (personality/identity) from SOUL.md
    agents::prompt_builder::init_soul(".");

    // Initialize global log level from config
    let initial_log_level = LogLevel::from_string(&config.log_level);
    global_log_level().set(initial_log_level);
    debug!("Log level set to: {:?}", initial_log_level);

    // --- WIRING ---
    let (broker, event_rx) = TokioEventBroker::new(1000, 1000, 1024);
    let broker = Arc::new(broker);

    let openai_provider = OpenAiProvider::new(
        config.openai_api_key.clone(),
        config.openai_base_url.clone(),
        config.openai_model.clone(),
        config.openai_embedding_model.clone(),
    );
    let gateway_adapter = Arc::new(GatewayAdapter {
        inner: openai_provider.clone(),
    });
    let gateway = Arc::new(GatewayManager::new(
        gateway_adapter,
        config.llm_throttle_ms,
        config.global_concurrency_permits,
    ));

    // Initialize MemoryOS - we no longer use PID-suffix fallbacks to avoid file sprawl.
    let memory_os_instance = match RedbGraphStore::new(&config.db_path) {
        Ok(store) => Arc::new(store),
        Err(e) => {
            error!("[Daemon] Failed to initialize MemoryOS database at {}: {}. If the database is locked by another instance, please close it first.", config.db_path, e);
            panic!("MemoryOS initialization failed");
        }
    };

    let tool_registry = crate::core::setup::build_tool_registry(&config, gateway.clone(), memory_os_instance.clone()).await;
    let system_context = Arc::new(tokio::sync::RwLock::new(telos_core::SystemContext {
        current_time: String::new(),
        location: config.default_location.clone().unwrap_or_else(|| "Unknown Location".to_string()),
    }));

    if config.default_location.is_none() {
        let sys_ctx_clone = system_context.clone();
        tokio::spawn(async move {
            loop {
                if let Ok(resp) = reqwest::get("http://ip-api.com/json/").await {
                    if let Ok(json) = resp.json::<serde_json::Value>().await {
                        let mut city = json.get("city").and_then(|v| v.as_str()).unwrap_or("Unknown City").to_string();
                        let as_str = json.get("as").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
                        let isp_str = json.get("isp").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
                        
                        // Extract finer city details from ASN/ISP if "city" defaults poorly
                        if as_str.contains("suzhou") || isp_str.contains("suzhou") {
                            city = "Suzhou".to_string();
                        } else if as_str.contains("shanghai") || isp_str.contains("shanghai") {
                            city = "Shanghai".to_string();
                        }

                        let loc = format!("{}, {}, {}", 
                            city,
                            json.get("regionName").and_then(|v| v.as_str()).unwrap_or("Unknown Region"),
                            json.get("country").and_then(|v| v.as_str()).unwrap_or("Unknown Country")
                        );
                        let mut w = sys_ctx_clone.write().await;
                        w.location = loc;
                    }
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            }
        });
    }

    let registry = Arc::new(DaemonRegistry {
        gateway: gateway.clone(),
        memory_os: memory_os_instance.clone(),
        system_context,
    });
    // Inject LLM gateway into MemoryOS for reconsolidation
    memory_os_instance.set_gateway(gateway.clone() as Arc<dyn telos_model_gateway::ModelGateway>);

    // Using cloud embeddings as configured
    let context_manager = Arc::new(RaptorContextManager::new(
        Arc::new(openai_provider.clone()),
        Arc::new(openai_provider.clone()),
        Some(memory_os_instance.clone() as Arc<dyn telos_memory::integration::MemoryIntegration>),
    ));

    // Initialize Evolution Evaluator
    let evaluator = Arc::new(
        telos_evolution::evaluator::ActorCriticEvaluator::new()
            .expect("Failed to initialize ActorCriticEvaluator")
            .with_gateway(gateway.clone()),
    );

    // --- BACKGROUND EVENT LOOP ---
    let broker_bg = Arc::clone(&broker);
    let gateway_clone = gateway.clone();
    let registry_clone = registry.clone();
    let tool_registry = tool_registry.clone();
    let loop_config = config.clone();
    let paused_tasks: Arc<TokioMutex<HashMap<String, String>>> =
        Arc::new(TokioMutex::new(HashMap::new()));
    let paused_tasks_bg = paused_tasks.clone();
    let wakeup_map: Arc<
        TokioMutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<(String, String, String)>>>,
    > = Arc::new(TokioMutex::new(HashMap::new()));
    let wakeup_map_bg = wakeup_map.clone();

    let active_tasks: telos_dag::engine::ActiveTaskRegistry = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    let active_tasks_loop = active_tasks.clone();

    // Global short-term session memory for Context/History Injection
    let global_session_logs: Arc<tokio::sync::RwLock<crate::core::state::SessionState>> = Arc::new(tokio::sync::RwLock::new(crate::core::state::SessionState::new()));
    let session_logs_loop = global_session_logs.clone();
    let evaluator_loop = evaluator.clone();

    let (distillation_tx, recent_traces) = crate::workers::spawner::spawn_background_tasks(
        &config, evaluator.clone(), registry.clone(), memory_os_instance.clone(), broker.clone()
    );
    let distillation_tx_bg = distillation_tx.clone();

    // --- PERSISTENT METRICS STORE ---
    let metrics_db_path = dirs::home_dir()
        .map(|h| h.join(".telos/metrics.redb"))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/telos_metrics.redb"));
    let metrics_store = Arc::new(
        crate::core::metrics_store::MetricsStore::new(
            metrics_db_path.to_str().unwrap_or("/tmp/telos_metrics.redb")
        ).expect("Failed to initialize metrics store")
    );
    metrics_store.restore_counters();
    let _ = crate::core::metrics_store::METRICS_STORE.set(metrics_store.clone());
    // Install sink in telos_core so ALL crates can emit metrics via telos_core::metrics::record()
    telos_core::metrics::install_sink(metrics_store.clone() as std::sync::Arc<dyn telos_core::metrics::MetricsSink>);
    info!("Persistent metrics store initialized at {:?}", metrics_db_path);

    // --- API SERVER ---
    let state = AppState {
        broker: broker.clone(),
        recent_traces,
        active_tasks,
        memory_os: memory_os_instance.clone(),
        metrics_store: metrics_store.clone(),
    };

    let app = crate::api::routes::build_router(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8321").await?;
    info!("Telos Daemon listening on ws://0.0.0.0:8321/api/v1/stream");

    let axum_server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // Start Scheduler Actor
    let scheduler = crate::workers::scheduler::SchedulerActor::new(
        memory_os_instance.clone(),
        broker.clone(),
    );
    scheduler.run().await;

    crate::workers::event_loop::run_event_loop(
        event_rx,
        broker_bg,
        gateway_clone,
        registry_clone,
        tool_registry,
        context_manager.clone(),
        Arc::new(loop_config),
        paused_tasks_bg,
        wakeup_map_bg,
        active_tasks_loop,
        distillation_tx_bg,
        session_logs_loop,
        evaluator_loop,
    ).await;

    let _ = tokio::join!(axum_server);

    Ok(())
}

