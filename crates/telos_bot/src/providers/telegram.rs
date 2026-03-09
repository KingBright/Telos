use async_trait::async_trait;
use tracing::{info, error, warn};
use futures_util::stream::StreamExt;
use reqwest::Client;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use telos_hci::{global_log_level, AgentFeedback, LogLevel};
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};
use teloxide::{prelude::*, utils::command::BotCommands};
use tokio::sync::Mutex;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;

use crate::traits::{BotCommand, ChatBotProvider};
use telos_core::config::TelosConfig;

#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "These commands are supported:"
)]
enum Command {
    #[command(description = "display this text.")]
    Help,
    #[command(description = "start interacting with Telos.")]
    Start,
    #[command(description = "show the active project or instruct how to use CLI to switch.")]
    Project,
    #[command(description = "get current log level.")]
    LogLevel,
    #[command(description = "set log level (quiet, normal, verbose, debug).")]
    SetLogLevel { level: String },
}

/// Telegram Feedback Formatter (respects 4096 char limit)
struct TelegramFeedbackFormatter {
    level: LogLevel,
}

impl TelegramFeedbackFormatter {
    fn new(level: LogLevel) -> Self {
        Self { level }
    }

    /// Truncate string to fit within Telegram's 4096 char limit
    fn truncate_for_telegram(s: &str, max_len: usize) -> String {
        if s.len() > max_len {
            format!("{}... (truncated)", &s[..max_len])
        } else {
            s.to_string()
        }
    }

    /// Format feedback for Telegram display
    fn format(&self, feedback: &AgentFeedback) -> Option<String> {
        if !feedback.should_show(self.level) {
            return None;
        }

        match feedback {
            AgentFeedback::PlanCreated { plan, .. } => {
                let mut output = format!("<b>📋 Plan Created: {} steps</b>\n", plan.total_steps);
                if let Some(ref reply) = plan.reply {
                    output.push_str(&Self::truncate_for_telegram(&format!("{}\n", reply), 500));
                }
                if self.level.should_show(LogLevel::Verbose) {
                    output.push_str("\n<b>Nodes:</b>\n");
                    for node in &plan.nodes {
                        let deps = if node.dependencies.is_empty() {
                            "none".to_string()
                        } else {
                            node.dependencies.join(", ")
                        };
                        output.push_str(&Self::truncate_for_telegram(
                            &format!("• {} ({}) - deps: {}\n", node.id, node.task_type, deps),
                            100,
                        ));
                    }
                }
                Some(output)
            }

            AgentFeedback::NodeStarted {
                node_id, detail, ..
            } => {
                let mut output =
                    format!("▶ <b>Starting [{}]</b> ({})\n", node_id, detail.task_type);
                if self.level.should_show(LogLevel::Verbose) {
                    output.push_str(&Self::truncate_for_telegram(
                        &format!("Task: {}\n", detail.input_preview),
                        200,
                    ));
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
                    output.push_str(&Self::truncate_for_telegram(
                        &format!("Result: {}\n", result_preview),
                        300,
                    ));
                }
                Some(output)
            }

            AgentFeedback::NodeFailed { node_id, error, .. } => {
                let mut output = format!("✗ <b>[{}] FAILED</b>\n", node_id);
                output.push_str(&format!("Type: {}\n", error.error_type));
                output.push_str(&Self::truncate_for_telegram(
                    &format!("Message: {}\n", error.message),
                    300,
                ));
                Some(output)
            }

            AgentFeedback::ProgressUpdate { progress, .. } => {
                let status_icons = format!(
                    "✓ {} ✗ {} ⏳ {}",
                    progress.completed, progress.failed, progress.running
                );
                Some(format!(
                    "📊 Progress: {}/{} ({}%) | {}",
                    progress.completed, progress.total, progress.percentage, status_icons
                ))
            }

            AgentFeedback::TaskCompleted { summary, .. } => {
                let icon = if summary.success { "✅" } else { "⚠️" };
                let status = if summary.success {
                    "Success"
                } else {
                    "Finished with errors"
                };
                let time_str = Self::format_duration(summary.total_time_ms);

                let mut output = format!(
                    "{} <b>Task {}</b>\n{} nodes (✓ {} ✗ {}) | {}\n",
                    icon,
                    status,
                    summary.total_nodes,
                    summary.completed_nodes,
                    summary.failed_nodes,
                    time_str
                );

                if !summary.success && !summary.failed_node_ids.is_empty() {
                    output.push_str(&format!("Failed: {}\n", summary.failed_node_ids.join(", ")));
                }

                output.push_str(&Self::truncate_for_telegram(&summary.summary, 500));
                Some(output)
            }

            AgentFeedback::StateChanged {
                current_node,
                status,
                ..
            } => {
                // Only show state changes in Debug mode
                if self.level.should_show(LogLevel::Debug) {
                    Some(format!(
                        "<code>[DEBUG] {} -> {:?}</code>",
                        current_node, status
                    ))
                } else {
                    None
                }
            }

            AgentFeedback::RequireHumanIntervention { prompt, .. } => Some(format!(
                "🚨 <b>Human Intervention Required</b>\n\n{}",
                Self::truncate_for_telegram(prompt, 3500)
            )),

            AgentFeedback::NodeNeedsHelp { node_id, help, .. } => {
                let suggestions_text = if help.suggestions.is_empty() {
                    String::new()
                } else {
                    format!(
                        "\n\n<b>Suggestions:</b>\n• {}",
                        help.suggestions.join("\n• ")
                    )
                };
                Some(format!(
                    "❓ <b>Node [{}] Needs Help</b>\n\nType: {}\n\n{}{}",
                    node_id,
                    help.help_type,
                    Self::truncate_for_telegram(&help.detail, 3000),
                    suggestions_text
                ))
            }

            AgentFeedback::Output {
                content, is_final, ..
            } => {
                let prefix = if *is_final { "✓" } else { "→" };
                Some(Self::truncate_for_telegram(
                    &format!("{} {}", prefix, content),
                    4000,
                ))
            }

            AgentFeedback::LogLevelChanged {
                old_level,
                new_level,
            } => Some(format!("📝 Log level: {:?} → {:?}", old_level, new_level)),
            AgentFeedback::Trace { .. } => None,
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
}

pub struct TelegramBotProvider {
    bot: Bot,
    daemon_url: String,
    daemon_ws_url: String,
    active_tasks: Arc<Mutex<HashMap<String, String>>>,
    pending_interactions: Arc<Mutex<HashMap<String, (String, String)>>>, // chat_id -> (task_id, node_id)
    send_state_changes: bool,
}

impl TelegramBotProvider {
    pub fn new(
        token: String,
        daemon_url: String,
        daemon_ws_url: String,
        send_state_changes: bool,
    ) -> Self {
        Self {
            bot: Bot::new(token),
            daemon_url,
            daemon_ws_url,
            active_tasks: Arc::new(Mutex::new(HashMap::new())),
            pending_interactions: Arc::new(Mutex::new(HashMap::new())),
            send_state_changes,
        }
    }

    async fn handle_message(
        bot: Bot,
        msg: Message,
        daemon_url: String,
        _active_tasks: Arc<Mutex<HashMap<String, String>>>,
        cmd: Command,
    ) -> ResponseResult<()> {
        match cmd {
            Command::Help | Command::Start => {
                bot.send_message(msg.chat.id, Command::descriptions().to_string())
                    .await?;
            }
            Command::Project => {
                let config = TelosConfig::load();
                let active = config
                    .ok()
                    .and_then(|c| c.active_project_id)
                    .unwrap_or_else(|| "None".to_string());
                bot.send_message(msg.chat.id, format!("Active Project ID: {}\nUse `telos project switch` via CLI to change projects.", active)).await?;
            }
            Command::LogLevel => {
                let client = Client::new();
                let res = client
                    .get(format!("{}/api/v1/log-level", daemon_url))
                    .send()
                    .await;

                match res {
                    Ok(r) if r.status().is_success() => {
                        if let Ok(body) = r.json::<serde_json::Value>().await {
                            bot.send_message(
                                msg.chat.id,
                                format!(
                                    "Current log level: {}\nAvailable: quiet, normal, verbose, debug\nUse /setloglevel <level> to change.",
                                    body["level"].as_str().unwrap_or("unknown")
                                ),
                            ).await?;
                        }
                    }
                    _ => {
                        bot.send_message(
                            msg.chat.id,
                            "Failed to get log level. Is the daemon running?",
                        )
                        .await?;
                    }
                }
            }
            Command::SetLogLevel { level } => {
                let client = Client::new();
                let res = client
                    .post(format!("{}/api/v1/log-level", daemon_url))
                    .json(&json!({ "level": level }))
                    .send()
                    .await;

                match res {
                    Ok(r) if r.status().is_success() => {
                        if let Ok(body) = r.json::<serde_json::Value>().await {
                            bot.send_message(
                                msg.chat.id,
                                format!(
                                    "Log level changed: {} → {}",
                                    body["old_level"].as_str().unwrap_or("?"),
                                    body["new_level"].as_str().unwrap_or("?")
                                ),
                            )
                            .await?;
                        }
                    }
                    _ => {
                        bot.send_message(
                            msg.chat.id,
                            "Failed to set log level. Is the daemon running?",
                        )
                        .await?;
                    }
                }
            }
        }
        Ok(())
    }

    async fn handle_text(
        bot: Bot,
        msg: Message,
        daemon_url: String,
        active_tasks: Arc<Mutex<HashMap<String, String>>>,
        pending_interactions: Arc<Mutex<HashMap<String, (String, String)>>>,
    ) -> ResponseResult<()> {
        if let Some(text) = msg.text() {
            let chat_id_str = msg.chat.id.to_string();

            // Check for pending interaction (intervention)
            let mut pending_map = pending_interactions.lock().await;
            if let Some((task_id, node_id)) = pending_map.remove(&chat_id_str) {
                let client = Client::new();
                let res = client
                    .post(format!("{}/api/v1/intervention", daemon_url))
                    .json(&json!({
                        "task_id": task_id,
                        "node_id": Some(node_id),
                        "instruction": text
                    }))
                    .send()
                    .await;

                match res {
                    Ok(r) if r.status().is_success() => {
                        bot.send_message(msg.chat.id, "→ Response sent to agent.")
                            .await?;
                    }
                    _ => {
                        bot.send_message(msg.chat.id, "❌ Failed to send response to daemon.")
                            .await?;
                    }
                }
                return Ok(());
            }
            drop(pending_map);

            let client = Client::new();
            let config = TelosConfig::load();
            let project_id = config.ok().and_then(|c| c.active_project_id);

            let res = client
                .post(format!("{}/api/v1/run", daemon_url))
                .json(&json!({ "payload": text, "project_id": project_id }))
                .send()
                .await;

            match res {
                Ok(r) if r.status().is_success() => {
                    if let Ok(response_body) = r.json::<serde_json::Value>().await {
                        if let Some(trace_id) = response_body["trace_id"].as_str() {
                            active_tasks
                                .lock()
                                .await
                                .insert(trace_id.to_string(), msg.chat.id.to_string());
                            bot.send_message(
                                msg.chat.id,
                                format!("Task Dispatched. Trace ID: {}", trace_id),
                            )
                            .await?;
                        }
                    }
                }
                _ => {
                    bot.send_message(msg.chat.id, "Failed to dispatch task via HTTP.")
                        .await?;
                }
            }
        }
        Ok(())
    }

    async fn listen_to_daemon(
        bot: Bot,
        daemon_ws_url: String,
        daemon_url: String,
        active_tasks: Arc<Mutex<HashMap<String, String>>>,
        pending_interactions: Arc<Mutex<HashMap<String, (String, String)>>>,
        send_state_changes: bool,
    ) {
        let client = Client::new();
        let initial_level = match client
            .get(format!("{}/api/v1/log-level", daemon_url))
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
        global_log_level().set(initial_level);

        loop {
            let current_level = global_log_level().get();
            let formatter = TelegramFeedbackFormatter::new(current_level);

            match connect_async(&daemon_ws_url).await {
                Ok((ws_stream, _)) => {
                    let (_, mut read) = ws_stream.split();
                    info!("[TelegramBot] Connected to Daemon WebSocket");

                    while let Some(message) = read.next().await {
                        if let Ok(WsMessage::Text(text)) = message {
                            if let Ok(feedback) = serde_json::from_str::<AgentFeedback>(&text) {
                                if let AgentFeedback::LogLevelChanged { new_level, .. } = &feedback
                                {
                                    global_log_level().set(*new_level);
                                    let active_map = active_tasks.lock().await;
                                    for (_, chat_id_str) in active_map.iter() {
                                        if let Ok(chat_id_num) = chat_id_str.parse::<i64>() {
                                            let _ = bot
                                                .send_message(
                                                    ChatId(chat_id_num),
                                                    format!("Log level changed: {:?}", new_level),
                                                )
                                                .await;
                                        }
                                    }
                                    continue;
                                }

                                let mut target_chat_id = None;
                                let mut trace_id_to_remove = None;
                                {
                                    let active_map = active_tasks.lock().await;
                                    let task_id = match &feedback {
                                        AgentFeedback::StateChanged { task_id, .. } => task_id,
                                        AgentFeedback::RequireHumanIntervention {
                                            task_id, ..
                                        } => task_id,
                                        AgentFeedback::NodeNeedsHelp { task_id, .. } => task_id,
                                        AgentFeedback::Output { task_id, .. } => task_id,
                                        AgentFeedback::PlanCreated { task_id, .. } => task_id,
                                        AgentFeedback::NodeStarted { task_id, .. } => task_id,
                                        AgentFeedback::NodeCompleted { task_id, .. } => task_id,
                                        AgentFeedback::NodeFailed { task_id, .. } => task_id,
                                        AgentFeedback::ProgressUpdate { task_id, .. } => task_id,
                                        AgentFeedback::TaskCompleted { task_id, .. } => task_id,
                                        AgentFeedback::LogLevelChanged { .. } => "",
                                        AgentFeedback::Trace { task_id, .. } => task_id,
                                    };

                                    if !task_id.is_empty() {
                                        if let Some(chat_id) = active_map.get(task_id) {
                                            target_chat_id = Some(chat_id.clone());
                                            if feedback.is_final() {
                                                trace_id_to_remove = Some(task_id.to_string());
                                            }
                                        }
                                    }
                                }

                                if let Some(chat_id_str) = target_chat_id {
                                    if let Ok(chat_id_num) = chat_id_str.parse::<i64>() {
                                        let chat_id = ChatId(chat_id_num);

                                        match &feedback {
                                            AgentFeedback::RequireHumanIntervention {
                                                prompt,
                                                task_id,
                                                ..
                                            } => {
                                                let keyboard =
                                                    InlineKeyboardMarkup::new(vec![vec![
                                                        InlineKeyboardButton::callback(
                                                            "✅ Approve",
                                                            format!("approve_{}", task_id),
                                                        ),
                                                        InlineKeyboardButton::callback(
                                                            "❌ Reject",
                                                            format!("reject_{}", task_id),
                                                        ),
                                                    ]]);
                                                let _ = bot.send_message(chat_id, format!("🚨 <b>Human Intervention Required</b>\n\n{}", prompt))
                                                    .parse_mode(teloxide::types::ParseMode::Html)
                                                    .reply_markup(keyboard).await;
                                            }
                                            AgentFeedback::NodeNeedsHelp {
                                                task_id,
                                                node_id,
                                                ..
                                            } => {
                                                pending_interactions.lock().await.insert(
                                                    chat_id_str.clone(),
                                                    (task_id.clone(), node_id.clone()),
                                                );
                                                if let Some(formatted) = formatter.format(&feedback)
                                                {
                                                    let _ = bot
                                                        .send_message(chat_id, &formatted)
                                                        .parse_mode(
                                                            teloxide::types::ParseMode::Html,
                                                        )
                                                        .await;
                                                }
                                            }
                                            _ => {
                                                if let Some(formatted) = formatter.format(&feedback)
                                                {
                                                    if matches!(
                                                        feedback,
                                                        AgentFeedback::StateChanged { .. }
                                                    ) {
                                                        if send_state_changes {
                                                            let _ = bot.send_message(chat_id, format!("<i>{}</i>", formatted)).parse_mode(teloxide::types::ParseMode::Html).await;
                                                        }
                                                    } else {
                                                        let _ = bot
                                                            .send_message(chat_id, &formatted)
                                                            .parse_mode(
                                                                teloxide::types::ParseMode::Html,
                                                            )
                                                            .await;
                                                    }
                                                }
                                                if feedback.is_final() {
                                                    pending_interactions
                                                        .lock()
                                                        .await
                                                        .remove(&chat_id_str);
                                                    if let Some(tid) = trace_id_to_remove {
                                                        active_tasks.lock().await.remove(&tid);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("[TelegramBot] WebSocket error: {}. Retrying in 5s...", e);
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }

    async fn handle_callback(bot: Bot, q: CallbackQuery, daemon_url: String) -> ResponseResult<()> {
        if let Some(data) = &q.data {
            let parts: Vec<&str> = data.split('_').collect();
            if parts.len() == 2 {
                let action = parts[0];
                let task_id = parts[1];
                let approved = action == "approve";

                let client = Client::new();
                let _res = client
                    .post(format!("{}/api/v1/approve", daemon_url))
                    .json(&json!({ "task_id": task_id, "approved": approved }))
                    .send()
                    .await;

                let ack_text = if approved {
                    "Approved execution."
                } else {
                    "Rejected execution."
                };
                bot.answer_callback_query(q.id.clone())
                    .text(ack_text)
                    .await?;

                if let Some(teloxide::types::MaybeInaccessibleMessage::Regular(regular_msg)) =
                    q.message
                {
                    bot.edit_message_text(
                        regular_msg.chat.id,
                        regular_msg.id,
                        format!("{} {}", ack_text, task_id),
                    )
                    .await?;
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl ChatBotProvider for TelegramBotProvider {
    async fn start(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let bot = self.bot.clone();
        let daemon_url = self.daemon_url.clone();
        let daemon_ws_url = self.daemon_ws_url.clone();
        let active_tasks = self.active_tasks.clone();
        let pending_interactions = self.pending_interactions.clone();
        let send_state_changes = self.send_state_changes;

        let ws_bot = bot.clone();
        let ws_active_tasks = active_tasks.clone();
        let ws_pending_interactions = pending_interactions.clone();
        let ws_daemon_url = daemon_url.clone();

        tokio::spawn(async move {
            Self::listen_to_daemon(
                ws_bot,
                daemon_ws_url,
                ws_daemon_url,
                ws_active_tasks,
                ws_pending_interactions,
                send_state_changes,
            )
            .await;
        });

        let handler = dptree::entry()
            .branch(Update::filter_callback_query().endpoint({
                let du = daemon_url.clone();
                move |b: Bot, q: CallbackQuery| {
                    let du = du.clone();
                    async move { Self::handle_callback(b, q, du).await }
                }
            }))
            .branch(
                Update::filter_message()
                    .filter_command::<Command>()
                    .endpoint({
                        let du = daemon_url.clone();
                        let at = active_tasks.clone();
                        move |b: Bot, msg: Message, cmd: Command| {
                            let du = du.clone();
                            let at = at.clone();
                            async move { Self::handle_message(b, msg, du, at, cmd).await }
                        }
                    }),
            )
            .branch(Update::filter_message().endpoint({
                let du = daemon_url.clone();
                let at = active_tasks.clone();
                let pi = pending_interactions.clone();
                move |b: Bot, msg: Message| {
                    let du = du.clone();
                    let at = at.clone();
                    let pi = pi.clone();
                    async move { Self::handle_text(b, msg, du, at, pi).await }
                }
            }));

        let mut dispatcher = Dispatcher::builder(bot.clone(), handler)
            .enable_ctrlc_handler()
            .build();

        // Prevent teloxide from panicking the daemon if Telegram's API is unreachable
        tokio::spawn(async move {
            let mut attempts = 0;
            let max_attempts = 10;
            let mut delay = 2; // start with 2 seconds
            
            loop {
                match bot.get_me().await {
                    Ok(_) => {
                        info!("[TelegramBot] Successfully connected to Telegram API. Starting dispatcher.");
                        dispatcher.dispatch().await;
                        break;
                    }
                    Err(e) => {
                        attempts += 1;
                        if attempts >= max_attempts {
                            error!("[TelegramBot] Failed to connect to Telegram API after {} attempts: {}. Giving up.", max_attempts, e);
                            break;
                        }
                        warn!("[TelegramBot] Telegram API unreachable (attempt {}/{}): {}. Retrying in {}s...", attempts, max_attempts, e, delay);
                        tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                        delay = std::cmp::min(delay * 2, 60); // Cap at 60s
                    }
                }
            }
        });

        Ok(())
    }

    async fn send_message(
        &self,
        session_id: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Ok(chat_id_num) = session_id.parse::<i64>() {
            self.bot.send_message(ChatId(chat_id_num), text).await?;
        }
        Ok(())
    }

    async fn register_commands(
        &self,
        _commands: Vec<BotCommand>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
}
