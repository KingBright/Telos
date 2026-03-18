pub mod tui;

use clap::{Parser, Subcommand};
use futures_util::stream::StreamExt;
use inquire::{Confirm, Text};
use reqwest::Client;
use serde_json::json;
use std::io::{self, Write};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message;

use telos_bot::providers::telegram::TelegramBotProvider;
use telos_bot::traits::ChatBotProvider;
use telos_core::config::TelosConfig;
use telos_hci::{global_log_level, AgentFeedback, LogLevel};
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
    /// Get or set the log level (quiet, normal, verbose, debug)
    LogLevel {
        /// The log level to set (optional, if not provided shows current level)
        level: Option<String>,
    },
}

#[derive(Subcommand)]
enum ProjectCommands {
    /// Initialize a new project in the current or specified directory
    Init { name: String, path: Option<String> },
    /// List all registered projects
    List,
    /// Switch active project context
    Switch { id_or_name: String },
}

/// CLI Feedback Formatter
struct CliFeedbackFormatter {
    level: LogLevel,
}

impl CliFeedbackFormatter {
    fn new(level: LogLevel) -> Self {
        Self { level }
    }

    /// Format and print feedback based on current log level
    fn format(&self, feedback: &AgentFeedback) -> Option<String> {
        if !feedback.should_show(self.level) {
            return None;
        }

        match feedback {
            AgentFeedback::PlanCreated { plan, .. } => {
                let mut output = format!("\n📋 Plan Created: {} steps\n", plan.total_steps);
                if let Some(ref reply) = plan.reply {
                    output.push_str(&format!("  {}\n", reply));
                }
                if self.level.should_show(LogLevel::Verbose) {
                    output.push_str("  Nodes:\n");
                    for node in &plan.nodes {
                        let deps = if node.dependencies.is_empty() {
                            "none".to_string()
                        } else {
                            node.dependencies.join(", ")
                        };
                        output.push_str(&format!(
                            "    • {} ({}) - deps: {}\n",
                            node.id, node.task_type, deps
                        ));
                    }
                }
                Some(output)
            }

            AgentFeedback::NodeStarted {
                node_id, detail, ..
            } => {
                let mut output = format!("\n▶ Starting [{}] ({})\n", node_id, detail.task_type);
                if self.level.should_show(LogLevel::Verbose) {
                    output.push_str(&format!("  Task: {}\n", detail.input_preview));
                }
                Some(output)
            }

            AgentFeedback::NodeCompleted {
                node_id,
                result_preview,
                execution_time_ms,
                ..
            } => {
                let mut output = format!("✓ [{}] Completed ({}ms)\n", node_id, execution_time_ms);
                if self.level.should_show(LogLevel::Verbose) {
                    output.push_str(&format!("  Result: {}\n", result_preview));
                }
                Some(output)
            }

            AgentFeedback::NodeFailed { node_id, error, .. } => {
                let mut output = format!("✗ [{}] FAILED\n", node_id);
                output.push_str(&format!("  Type: {}\n", error.error_type));
                output.push_str(&format!("  Message: {}\n", error.message));
                if self.level.should_show(LogLevel::Debug) {
                    if let Some(ref stack) = error.stack_trace {
                        output.push_str(&format!("  Stack: {}\n", stack));
                    }
                }
                Some(output)
            }

            AgentFeedback::ProgressUpdate { progress, .. } => {
                let status_icons = format!(
                    "✓ {} ✗ {} ⏳ {}",
                    progress.completed, progress.failed, progress.running
                );
                Some(format!(
                    "\n📊 Progress: {}/{} ({}%) | {}\n",
                    progress.completed, progress.total, progress.percentage, status_icons
                ))
            }

            AgentFeedback::TaskCompleted { summary, .. } => {
                let icon = if summary.fulfilled { "✅" } else { "⚠️" };
                let status = if summary.fulfilled {
                    "Success"
                } else {
                    "Finished with errors"
                };
                let time_str = format_duration(summary.total_time_ms);

                let mut output = format!(
                    "\n{} Task {} | {} nodes (✓ {} ✗ {}) | {}\n",
                    icon,
                    status,
                    summary.total_nodes,
                    summary.completed_nodes,
                    summary.failed_nodes,
                    time_str
                );

                if !summary.fulfilled && !summary.failed_node_ids.is_empty() {
                    output.push_str(&format!(
                        "  Failed nodes: {}\n",
                        summary.failed_node_ids.join(", ")
                    ));
                }

                if self.level.should_show(LogLevel::Normal) {
                    output.push_str(&format!("  {}\n", summary.summary));
                }

                Some(output)
            }

            AgentFeedback::StateChanged {
                current_node,
                status,
                ..
            } => {
                // Only show state changes in Debug mode
                if self.level.should_show(LogLevel::Debug) {
                    Some(format!("[DEBUG] {} -> {:?}\n", current_node, status))
                } else {
                    None
                }
            }

            AgentFeedback::RequireHumanIntervention { prompt, .. } => Some(format!(
                "\n🚨 [HUMAN INTERVENTION REQUIRED] 🚨\n{}\n",
                prompt
            )),

            AgentFeedback::Output {
                content, is_final, silent, ..
            } => {
                if *silent { return None; }
                let prefix = if *is_final { "✓" } else { ">>" };
                Some(format!("{} {}\n", prefix, content))
            }

            AgentFeedback::LogLevelChanged {
                old_level,
                new_level,
            } => Some(format!(
                "Log level changed: {:?} → {:?}\n",
                old_level, new_level
            )),

            AgentFeedback::NodeNeedsHelp { node_id, help, .. } => {
                let suggestions_text = if help.suggestions.is_empty() {
                    String::new()
                } else {
                    format!("\n  Suggestions:\n  • {}", help.suggestions.join("\n  • "))
                };
                Some(format!(
                    "\n❓ [{}] Needs Help ({})\n  {}\n{}\n",
                    node_id, help.help_type, help.detail, suggestions_text
                ))
            }
            AgentFeedback::Trace { .. } => None,

            AgentFeedback::ClarificationNeeded { prompt, options, .. } => {
                let mut output = format!("\n❓ [CLARIFICATION NEEDED]\n{}\n", prompt);
                for (i, opt) in options.iter().enumerate() {
                    output.push_str(&format!("  {}) {} - {}\n", i + 1, opt.label, opt.description));
                }
                output.push_str("  Enter a number or type your response:\n");
                Some(output)
            }
        }
    }
}

fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        let secs = ms / 1000;
        let mins = secs / 60;
        let remaining_secs = secs % 60;
        format!("{}m {}s", mins, remaining_secs)
    }
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
            let config = TelosConfig::load().unwrap_or_else(|_| panic!("Config should exist"));
            use std::io::IsTerminal;
            if std::io::stdout().is_terminal() {
                tui::run_tui(config, Some(task.clone())).await?;
            } else {
                handle_run_headless(&config, task).await?;
            }
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
        Commands::LogLevel { level } => {
            handle_log_level(level).await?;
        }
    }

    Ok(())
}

async fn handle_log_level(level: &Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new();

    match level {
        Some(level_str) => {
            // Set log level
            let res = client
                .post("http://127.0.0.1:8321/api/v1/log-level")
                .json(&json!({ "level": level_str }))
                .send()
                .await?;

            if res.status().is_success() {
                let body: serde_json::Value = res.json().await?;
                println!(
                    "Log level changed: {} → {}",
                    body["old_level"].as_str().unwrap_or("?"),
                    body["new_level"].as_str().unwrap_or("?")
                );
            } else {
                println!("Failed to set log level. Is the daemon running?");
            }
        }
        None => {
            // Get current log level
            let res = client
                .get("http://127.0.0.1:8321/api/v1/log-level")
                .send()
                .await?;

            if res.status().is_success() {
                let body: serde_json::Value = res.json().await?;
                println!(
                    "Current log level: {}",
                    body["level"].as_str().unwrap_or("unknown")
                );
                println!("Available levels: quiet, normal, verbose, debug");
            } else {
                println!("Failed to get log level. Is the daemon running?");
            }
        }
    }

    Ok(())
}

async fn handle_telegram_bot() -> Result<(), Box<dyn std::error::Error>> {
    let config = TelosConfig::load().expect("Could not load config");
    let token = config
        .telegram_bot_token
        .expect("Telegram bot token not found in config. Please re-run config or add it manually.");

    println!("Starting Telegram Bot Adapter...");

    let daemon_url = "http://127.0.0.1:8321".to_string();
    let daemon_ws_url = "ws://127.0.0.1:8321/api/v1/stream".to_string();

    let provider = TelegramBotProvider::new(
        token,
        daemon_url,
        daemon_ws_url,
        config.bot_send_state_changes,
    );
    provider
        .start()
        .await
        .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;

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
        model = Text::new(
            "Please enter the LLM Model name (e.g. glm-4.7 for Zhipu, or gpt-4o-mini for OpenAI):",
        )
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

    let default_db_path = TelosConfig::memory_db_path().to_string_lossy().into_owned();

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

    let bot_send_state_changes = Confirm::new(
        "Should the Telegram bot send intermediate state changes (sub-task progress)?",
    )
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
        global_concurrency_permits: 3,
        log_level: "normal".to_string(),
        llm_throttle_ms: 2000,
        global_prompt: None,
        proxy: None,
        router_persona_name: "小特".to_string(),
        router_persona_trait: "聪明、活泼且不失风趣".to_string(),
        default_location: None,
        openai_audio_base_url: None,
        openai_audio_api_key: None,
        tts_voice_id: telos_core::config::default_tts_voice_id(),
    };

    match config.save() {
        Ok(_) => {
            println!(
                "Configuration saved successfully to {:?}",
                TelosConfig::config_file_path()
            );
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
    let trace_id = uuid::Uuid::new_v4().to_string();
    let ws_url = format!("ws://127.0.0.1:8321/api/v1/stream?trace_id={}", trace_id);
    println!("Connecting to Feedback Stream at {} ...", ws_url);

    // Get current log level from daemon
    let client = Client::new();
    let current_level = match client
        .get("http://127.0.0.1:8321/api/v1/log-level")
        .send()
        .await
    {
        Ok(res) if res.status().is_success() => {
            if let Ok(body) = res.json::<serde_json::Value>().await {
                LogLevel::from_string(body["level"].as_str().unwrap_or("normal"))
            } else {
                LogLevel::Normal
            }
        }
        _ => LogLevel::Normal,
    };

    // Update local global log level
    global_log_level().set(current_level);
    let formatter = CliFeedbackFormatter::new(current_level);

    // Connect to WebSocket FIRST to prevent race condition
    let (ws_stream, _) = match connect_async(ws_url).await {
        Ok(ws) => ws,
        Err(e) => {
            println!(
                "Failed to connect to daemon WebSocket: {}. Is the daemon running?",
                e
            );
            return Ok(());
        }
    };
    let (_, mut read) = ws_stream.split();

    let config = TelosConfig::load().unwrap_or_else(|_| panic!("Config should exist"));
    let project_id = config.active_project_id;

    // Now send the HTTP POST request to trigger the execution
    let payload = json!({
        "payload": task,
        "project_id": project_id,
        "trace_id": trace_id
    });

    let res = client
        .post("http://127.0.0.1:8321/api/v1/run")
        .json(&payload)
        .send()
        .await?;

    if !res.status().is_success() {
        println!("Failed to dispatch task via HTTP.");
        return Ok(());
    }

    let response_body: serde_json::Value = res.json().await?;
    let _server_trace_id = response_body["trace_id"].as_str().unwrap_or("unknown");
    println!("Task Dispatched. Trace ID: {}", trace_id);

    // Listen for incoming events
    while let Some(message) = read.next().await {
        let msg = message?;
        if let Message::Text(text) = msg {
            if let Ok(feedback) = serde_json::from_str::<AgentFeedback>(&text) {
                // Check for log level changes
                if let AgentFeedback::LogLevelChanged { new_level, .. } = &feedback {
                    global_log_level().set(*new_level);
                    println!("Log level changed: {:?}", new_level);
                    continue;
                }

                // Format and print feedback
                if let Some(formatted) = formatter.format(&feedback) {
                    print!("{}", formatted);
                    io::stdout().flush().unwrap();
                }

                // Handle human intervention (Approval)
                if let AgentFeedback::RequireHumanIntervention { task_id, .. } = &feedback {
                    print!("Approve this action? [y/N]: ");
                    io::stdout().flush().unwrap();
                    let mut input = String::new();
                    io::stdin().read_line(&mut input)?;
                    let approved = input.trim().eq_ignore_ascii_case("y");

                    let res = client
                        .post("http://127.0.0.1:8321/api/v1/approve")
                        .json(&json!({ "task_id": task_id, "approved": approved }))
                        .send()
                        .await?;

                    if res.status().is_success() {
                        println!("-> User Decision sent: Approved={}", approved);
                    } else {
                        println!("-> Failed to send decision.");
                    }
                }

                // Handle Node Needs Help (Intervention)
                if let AgentFeedback::NodeNeedsHelp {
                    task_id, node_id, ..
                } = &feedback
                {
                    print!("\nProvide input for node [{}]: ", node_id);
                    io::stdout().flush().unwrap();
                    let mut input = String::new();
                    io::stdin().read_line(&mut input)?;
                    let instruction = input.trim().to_string();

                    if !instruction.is_empty() {
                        let res = client
                            .post("http://127.0.0.1:8321/api/v1/intervention")
                            .json(&json!({
                                "task_id": task_id,
                                "node_id": Some(node_id),
                                "instruction": instruction
                            }))
                            .send()
                            .await?;

                        if res.status().is_success() {
                            println!("-> Response sent to agent.");
                        } else {
                            println!("-> Failed to send response.");
                        }
                    }
                }

                // Check for task completion
                if feedback.is_final() {
                    break;
                }
            }
        }
    }

    Ok(())
}

async fn handle_run_headless(config: &TelosConfig, task: &str) -> Result<(), Box<dyn std::error::Error>> {
    use futures_util::StreamExt;

    let client = Client::new();
    let project_id = config.active_project_id.clone();
    let trace_id = uuid::Uuid::new_v4().to_string();

    let payload = json!({
        "payload": task,
        "project_id": project_id,
        "trace_id": trace_id
    });

    println!("Dispatching task headlessly (Trace ID: {})...", trace_id);
    
    // No hard timeout — we use idle-based timeout instead.
    // The SSE stream will send heartbeat events as the system works.
    let res = client
        .post("http://127.0.0.1:8321/api/v1/run_sync")
        .json(&payload)
        .send()
        .await?;

    if !res.status().is_success() {
        println!("Failed to run task. HTTP Status: {}", res.status());
        return Ok(());
    }

    // Consume SSE stream with idle-based timeout
    let idle_timeout = std::time::Duration::from_secs(120);
    let mut byte_stream = res.bytes_stream();
    let mut buffer = String::new();
    let mut last_event_time = std::time::Instant::now();
    let mut final_output = String::new();

    loop {
        let chunk = tokio::time::timeout(idle_timeout, byte_stream.next()).await;

        match chunk {
            Ok(Some(Ok(bytes))) => {
                buffer.push_str(&String::from_utf8_lossy(&bytes));

                // Parse SSE events from the buffer
                while let Some(event_end) = buffer.find("\n\n") {
                    let event_block = buffer[..event_end].to_string();
                    buffer = buffer[event_end + 2..].to_string();

                    let mut event_type = String::new();
                    let mut data = String::new();

                    for line in event_block.lines() {
                        if let Some(val) = line.strip_prefix("event: ") {
                            event_type = val.trim().to_string();
                        } else if let Some(val) = line.strip_prefix("data: ") {
                            data = val.to_string();
                        } else if line.starts_with(":") {
                            // SSE comment (keep-alive), do NOT reset idle timer
                            continue;
                        }
                    }

                    // Only reset idle timer when a real data event is received
                    if !data.is_empty() || !event_type.is_empty() {
                        last_event_time = std::time::Instant::now();
                    }

                    match event_type.as_str() {
                        "started" => {
                            println!("[Started] Task accepted by daemon.");
                        }
                        "heartbeat" => {
                            println!("[Progress] {}", data);
                        }
                        "output" => {
                            final_output = data.clone();
                            println!("\n[Final Output]\n{}", data);
                        }
                        "completed" => {
                            println!("\n[Summary]\n{}", data);
                            return Ok(());
                        }
                        _ => {}
                    }
                }
            }
            Ok(Some(Err(e))) => {
                println!("Stream error: {}", e);
                break;
            }
            Ok(None) => {
                // Stream ended without a completed event
                if !final_output.is_empty() {
                    println!("\nStream ended. Output was received.");
                } else {
                    println!("\nStream ended prematurely without final output.");
                }
                break;
            }
            Err(_) => {
                // Idle timeout triggered
                println!("\n[Idle Timeout] No activity from daemon for {}s. The task may still be running on the server.", idle_timeout.as_secs());
                break;
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
                    println!(
                        "Project '{}' created successfully at: {:?}",
                        project.name, project.path
                    );
                    registry.set_active_project(&project.id)?;
                    println!("Switched active project to: {}", project.name);
                }
                Err(e) => eprintln!("Failed to create project: {}", e),
            }
        }
        ProjectCommands::List => match registry.list_projects() {
            Ok(projects) => {
                if projects.is_empty() {
                    println!("No projects found. Use `telos project init <name>` to create one.");
                } else {
                    let config =
                        TelosConfig::load().unwrap_or_else(|_| panic!("Failed to load config"));
                    let active_id = config.active_project_id.unwrap_or_default();
                    println!("Registered Projects:");
                    for p in projects {
                        let active_marker = if p.id == active_id { "*" } else { " " };
                        println!("{} {} ({}) - {:?}", active_marker, p.name, p.id, p.path);
                    }
                }
            }
            Err(e) => eprintln!("Failed to list projects: {}", e),
        },
        ProjectCommands::Switch { id_or_name } => match registry.set_active_project(id_or_name) {
            Ok(project) => println!("Switched active project to: {}", project.name),
            Err(e) => eprintln!("Failed to switch project: {}", e),
        },
    }
    Ok(())
}
