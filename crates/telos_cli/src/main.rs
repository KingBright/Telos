use clap::{Parser, Subcommand};
use futures_util::stream::StreamExt;
use inquire::{Text, Confirm};
use reqwest::Client;
use serde_json::json;
use std::io::{self, Write};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message;

use telos_core::config::TelosConfig;
use telos_hci::AgentFeedback;
use telos_bot::providers::telegram::TelegramBotProvider;
use telos_bot::traits::ChatBotProvider;
use telos_project::manager::ProjectRegistry;

#[derive(Parser)]
#[command(name = "telos")]
#[command(about = "Telos Agent CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize and configure the Telos environment
    Init,
    /// Start the telos daemon server
    Daemon {
        #[arg(short, long)]
        start: bool,
    },
    /// Run a task against the daemon
    Run {
        /// The natural language task description
        task: String,
    },
    /// Manage and start chatbots
    Bot {
        #[arg(long)]
        telegram: bool,
    },
    /// Manage projects
    Project {
        #[command(subcommand)]
        action: ProjectCommands,
    },
}

#[derive(Subcommand)]
enum ProjectCommands {
    /// Initialize a new project in the current or specified directory
    Init {
        name: String,
        path: Option<String>,
    },
    /// List all registered projects
    List,
    /// Switch active project context
    Switch {
        id_or_name: String,
    },
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Init => {
            if check_and_init_config(true) {
                start_daemon();
            }
        }
        Commands::Daemon { start } => {
            if *start {
                start_daemon();
            } else {
                println!("Please use `telos daemon --start`");
            }
        }
        Commands::Run { task } => {
            if check_and_init_config(false) {
                start_daemon();
            }
            println!("Dispatching Task: {}", task);
            handle_run(task).await?;
        }
        Commands::Bot { telegram } => {
            if check_and_init_config(false) {
                start_daemon();
            }
            if *telegram {
                handle_telegram_bot().await?;
            } else {
                println!("Please specify a bot platform, e.g., `telos bot --telegram`");
            }
        }
        Commands::Project { action } => {
            if check_and_init_config(false) {
                start_daemon();
            }
            handle_project(action).await?;
        }
    }

    Ok(())
}

async fn handle_telegram_bot() -> Result<(), Box<dyn std::error::Error>> {
    let config = TelosConfig::load().expect("Could not load config");
    let token = config.telegram_bot_token.expect("Telegram bot token not found in config. Please re-run config or add it manually.");

    println!("Starting Telegram Bot Adapter...");

    let daemon_url = "http://127.0.0.1:3000".to_string();
    let daemon_ws_url = "ws://127.0.0.1:3000/api/v1/stream".to_string();

    let provider = TelegramBotProvider::new(token, daemon_url, daemon_ws_url, config.bot_send_state_changes);
    provider.start().await.map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;

    Ok(())
}

fn check_and_init_config(force: bool) -> bool {
    if !force && TelosConfig::load().is_ok() {
        return false; // Valid config already exists
    }

    println!("Welcome to Telos! Let's set up your environment.");

    let mut api_key = String::new();
    while api_key.is_empty() {
        api_key = Text::new("Please enter your API Key (e.g. OpenAI or Zhipu GLM key):")
            .prompt()
            .unwrap_or_default()
            .trim()
            .to_string();
    }

    let mut base_url = String::new();
    while base_url.is_empty() {
        base_url = Text::new("Please enter the API Base URL (e.g. https://open.bigmodel.cn/api/paas/v4 for GLM, or https://api.openai.com/v1 for OpenAI):")
            .prompt()
            .unwrap_or_default()
            .trim()
            .to_string();
    }

    let mut model = String::new();
    while model.is_empty() {
        model = Text::new("Please enter the LLM Model name (e.g. glm-4.7 for Zhipu, or gpt-4o-mini for OpenAI):")
            .prompt()
            .unwrap_or_default()
            .trim()
            .to_string();
    }

    let mut embedding_model = String::new();
    while embedding_model.is_empty() {
        embedding_model = Text::new("Please enter the Embedding Model name (e.g. Embedding-3 for Zhipu, or text-embedding-3-small for OpenAI):")
            .prompt()
            .unwrap_or_default()
            .trim()
            .to_string();
    }

    let default_db_path = {
        let mut path = dirs::home_dir().expect("Could not find home directory");
        path.push(".telos_memory.redb");
        path.to_string_lossy().into_owned()
    };

    let db_path = Text::new("Where should we store the memory database?")
        .with_default(&default_db_path)
        .prompt()
        .unwrap_or(default_db_path);

    let wants_telegram = Confirm::new("Would you like to configure a Telegram bot integration?")
        .with_default(false)
        .prompt()
        .unwrap_or(false);

    let mut telegram_bot_token = None;
    if wants_telegram {
        let token = Text::new("Please enter your Telegram Bot Token:")
            .prompt()
            .unwrap_or_default()
            .trim()
            .to_string();
        if !token.is_empty() {
            telegram_bot_token = Some(token);
        }
    }

    let bot_send_state_changes = Confirm::new("Should the Telegram bot send intermediate state changes (sub-task progress)?")
        .with_default(false)
        .prompt()
        .unwrap_or(false);

    let config = TelosConfig {
        openai_api_key: api_key,
        openai_base_url: base_url,
        openai_model: model,
        openai_embedding_model: embedding_model,
        db_path,
        tools_dir: default_tools_dir(),
        telegram_bot_token,
        bot_send_state_changes,
        active_project_id: None,
    };

    match config.save() {
        Ok(_) => {
            println!("Configuration saved successfully to {:?}", TelosConfig::config_file_path());
            true
        }
        Err(e) => {
            eprintln!("Failed to save config: {}", e);
            false
        }
    }
}

fn start_daemon() {
    println!("Starting Telos Daemon...");
    let exe_dir = std::env::current_exe()
        .map(|p| p.parent().unwrap().to_path_buf())
        .unwrap_or_else(|_| std::path::PathBuf::from("."));

    // In release/debug builds, telos_daemon should be next to telos cli
    let daemon_path = exe_dir.join("telos_daemon");

    if daemon_path.exists() {
        #[allow(clippy::zombie_processes)]
        std::process::Command::new(daemon_path)
            .spawn()
            .expect("Failed to start telos_daemon");
    } else {
        // Fallback for local dev
        #[allow(clippy::zombie_processes)]
        std::process::Command::new("cargo")
            .args(["run", "-p", "telos_daemon"])
            .spawn()
            .expect("Failed to start telos_daemon via cargo");
    }

    // Brief sleep to allow server to start
    std::thread::sleep(std::time::Duration::from_millis(1500));
    println!("Daemon started in the background.");
}

async fn handle_run(task: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ws_url = "ws://127.0.0.1:3000/api/v1/stream";
    println!("Connecting to Feedback Stream at {} ...", ws_url);

    // Connect to WebSocket FIRST to prevent race condition
    let (ws_stream, _) = match connect_async(ws_url).await {
        Ok(ws) => ws,
        Err(e) => {
            println!("Failed to connect to daemon WebSocket: {}. Is the daemon running?", e);
            return Ok(());
        }
    };
    let (_, mut read) = ws_stream.split();

    let config = TelosConfig::load().unwrap_or_else(|_| panic!("Config should exist"));
    let project_id = config.active_project_id;

    // Now send the HTTP POST request to trigger the execution
    let client = Client::new();
    let payload = json!({
        "payload": task,
        "project_id": project_id
    });

    let res = client
        .post("http://127.0.0.1:3000/api/v1/run")
        .json(&payload)
        .send()
        .await?;

    if !res.status().is_success() {
        println!("Failed to dispatch task via HTTP.");
        return Ok(());
    }

    let response_body: serde_json::Value = res.json().await?;
    let trace_id = response_body["trace_id"].as_str().unwrap_or("unknown");
    println!("Task Dispatched. Trace ID: {}", trace_id);

    // Listen for incoming events
    while let Some(message) = read.next().await {
        let msg = message?;
        if let Message::Text(text) = msg {
            if let Ok(feedback) = serde_json::from_str::<AgentFeedback>(&text) {
                match feedback {
                    AgentFeedback::RequireHumanIntervention { prompt, task_id, .. } => {
                        println!("\n🚨 [HUMAN INTERVENTION REQUIRED] 🚨");
                        println!("{}", prompt);

                        print!("Approve this action? [y/N]: ");
                        io::stdout().flush().unwrap();
                        let mut input = String::new();
                        io::stdin().read_line(&mut input)?;
                        let approved = input.trim().eq_ignore_ascii_case("y");

                        let res = client
                            .post("http://127.0.0.1:3000/api/v1/approve")
                            .json(&json!({ "task_id": task_id, "approved": approved }))
                            .send()
                            .await?;

                        if res.status().is_success() {
                             println!("-> User Decision sent: Approved={}", approved);
                        } else {
                             println!("-> Failed to send decision.");
                        }
                    }
                    AgentFeedback::Output { content, is_final, .. } => {
                        println!(">> {}", content);
                        if is_final {
                            println!("Task completed.");
                            break;
                        }
                    }
                    AgentFeedback::StateChanged { current_node, status, .. } => {
                        println!("[STATE] {} -> {:?}", current_node, status);
                    }
                }
            }
        }
    }

    Ok(())
}

fn default_tools_dir() -> String {
    let mut path = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    path.push(".telos/tools");
    path.to_string_lossy().into_owned()
}

async fn handle_project(action: &ProjectCommands) -> Result<(), Box<dyn std::error::Error>> {
    let registry = ProjectRegistry::new();

    match action {
        ProjectCommands::Init { name, path } => {
            match registry.create_project(name.clone(), path.clone(), None) {
                Ok(project) => {
                    println!("Project '{}' created successfully at: {:?}", project.name, project.path);
                    registry.set_active_project(&project.id)?;
                    println!("Switched active project to: {}", project.name);
                }
                Err(e) => eprintln!("Failed to create project: {}", e),
            }
        }
        ProjectCommands::List => {
            match registry.list_projects() {
                Ok(projects) => {
                    if projects.is_empty() {
                        println!("No projects found. Use `telos project init <name>` to create one.");
                    } else {
                        let config = TelosConfig::load().unwrap_or_else(|_| panic!("Failed to load config"));
                        let active_id = config.active_project_id.unwrap_or_default();
                        println!("Registered Projects:");
                        for p in projects {
                            let active_marker = if p.id == active_id { "*" } else { " " };
                            println!("{} {} ({}) - {:?}", active_marker, p.name, p.id, p.path);
                        }
                    }
                }
                Err(e) => eprintln!("Failed to list projects: {}", e),
            }
        }
        ProjectCommands::Switch { id_or_name } => {
            match registry.set_active_project(id_or_name) {
                Ok(project) => println!("Switched active project to: {}", project.name),
                Err(e) => eprintln!("Failed to switch project: {}", e),
            }
        }
    }
    Ok(())
}
