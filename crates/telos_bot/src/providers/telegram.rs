use std::sync::Arc;
use async_trait::async_trait;
use teloxide::{prelude::*, utils::command::BotCommands};
use reqwest::Client;
use serde_json::json;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;
use futures_util::stream::StreamExt;
use std::collections::HashMap;
use tokio::sync::Mutex;

use crate::traits::{ChatBotProvider, BotCommand};
use telos_core::config::TelosConfig;

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "These commands are supported:")]
enum Command {
    #[command(description = "display this text.")]
    Help,
    #[command(description = "start interacting with Telos.")]
    Start,
    #[command(description = "show the active project or instruct how to use CLI to switch.")]
    Project,
}

pub struct TelegramBotProvider {
    bot: Bot,
    daemon_url: String,
    daemon_ws_url: String,
    active_tasks: Arc<Mutex<HashMap<String, String>>>,
    send_state_changes: bool,
}

impl TelegramBotProvider {
    pub fn new(token: String, daemon_url: String, daemon_ws_url: String, send_state_changes: bool) -> Self {
        Self {
            bot: Bot::new(token),
            daemon_url,
            daemon_ws_url,
            active_tasks: Arc::new(Mutex::new(HashMap::new())),
            send_state_changes,
        }
    }

    async fn handle_message(
        bot: Bot,
        msg: Message,
        _daemon_url: String,
        _active_tasks: Arc<Mutex<HashMap<String, String>>>,
        cmd: Command,
    ) -> ResponseResult<()> {
        match cmd {
            Command::Help | Command::Start => {
                bot.send_message(msg.chat.id, Command::descriptions().to_string()).await?;
            }
            Command::Project => {
                let config = TelosConfig::load();
                let active = config.ok().and_then(|c| c.active_project_id).unwrap_or_else(|| "None".to_string());
                bot.send_message(msg.chat.id, format!("Active Project ID: {}
Use `telos project switch` via CLI to change projects.", active)).await?;
            }
        }
        Ok(())
    }

    async fn handle_text(
        bot: Bot,
        msg: Message,
        daemon_url: String,
        active_tasks: Arc<Mutex<HashMap<String, String>>>,
    ) -> ResponseResult<()> {
        if let Some(text) = msg.text() {
            bot.send_message(msg.chat.id, "Dispatching task to Telos Daemon...").await?;

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
                            active_tasks.lock().await.insert(trace_id.to_string(), msg.chat.id.to_string());
                            bot.send_message(msg.chat.id, format!("Task Dispatched. Trace ID: {}", trace_id)).await?;
                        }
                    }
                }
                _ => {
                    bot.send_message(msg.chat.id, "Failed to dispatch task via HTTP.").await?;
                }
            }
        }
        Ok(())
    }

    async fn listen_to_daemon(
        bot: Bot,
        daemon_ws_url: String,
        active_tasks: Arc<Mutex<HashMap<String, String>>>,
        send_state_changes: bool,
    ) {
        loop {
            match connect_async(&daemon_ws_url).await {
                Ok((ws_stream, _)) => {
                    let (_, mut read) = ws_stream.split();
                    println!("[TelegramBot] Connected to Daemon WebSocket");

                    while let Some(message) = read.next().await {
                        if let Ok(WsMessage::Text(text)) = message {
                            let mut target_chat_id = None;
                            let mut trace_id_to_remove = None;
                            {
                                let active_map = active_tasks.lock().await;
                                for (trace_id, chat_id) in active_map.iter() {
                                    if text.contains(trace_id) {
                                        target_chat_id = Some(chat_id.clone());
                                        trace_id_to_remove = Some(trace_id.clone());
                                        break;
                                    }
                                }
                            }

                            if let Some(chat_id_str) = target_chat_id {
                                if let Ok(chat_id_num) = chat_id_str.parse::<i64>() {
                                    let chat_id = ChatId(chat_id_num);

                                    if text.contains("RequireHumanIntervention") {
                                        let _ = bot.send_message(chat_id, format!("🚨 [HUMAN INTERVENTION REQUIRED] 🚨\n{}", text)).await;
                                        let _ = bot.send_message(chat_id, "Please use CLI to approve for now.").await;
                                    } else if text.contains("Output") {
                                        let _ = bot.send_message(chat_id, format!(">> {}", text)).await;
                                        if text.contains("is_final: true") {
                                            let _ = bot.send_message(chat_id, "Task completed.").await;
                                            if let Some(tid) = trace_id_to_remove {
                                                active_tasks.lock().await.remove(&tid);
                                            }
                                        }
                                    } else if send_state_changes {
                                        let _ = bot.send_message(chat_id, format!("[STATE] {}", text)).await;
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[TelegramBot] Failed to connect to Daemon WebSocket: {}. Retrying in 5s...", e);
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }
}

#[async_trait]
impl ChatBotProvider for TelegramBotProvider {
    async fn start(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let bot = self.bot.clone();
        let daemon_url = self.daemon_url.clone();
        let daemon_ws_url = self.daemon_ws_url.clone();
        let active_tasks = self.active_tasks.clone();
        let send_state_changes = self.send_state_changes;

        let ws_bot = bot.clone();
        let ws_active_tasks = active_tasks.clone();

        tokio::spawn(async move {
            Self::listen_to_daemon(ws_bot, daemon_ws_url, ws_active_tasks, send_state_changes).await;
        });

        // Use a standard Dispatcher for both commands and plain text
        let handler = dptree::entry()
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
                    })
            )
            .branch(
                Update::filter_message()
                    .endpoint({
                        let du = daemon_url.clone();
                        let at = active_tasks.clone();
                        move |b: Bot, msg: Message| {
                            let du = du.clone();
                            let at = at.clone();
                            async move { Self::handle_text(b, msg, du, at).await }
                        }
                    })
            );

        Dispatcher::builder(bot, handler).enable_ctrlc_handler().build().dispatch().await;

        Ok(())
    }

    async fn send_message(&self, session_id: &str, text: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Ok(chat_id_num) = session_id.parse::<i64>() {
            self.bot.send_message(ChatId(chat_id_num), text).await?;
        }
        Ok(())
    }

    async fn register_commands(&self, _commands: Vec<BotCommand>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Here we could dynamically map the abstraction to teloxide
        Ok(())
    }
}
