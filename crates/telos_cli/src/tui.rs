use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::stream::StreamExt;
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap, List, ListItem, Gauge},
    Terminal,
};
use reqwest::Client;
use serde_json::json;
use std::{
    io,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message;
use tui_textarea::{Input, Key, TextArea};

use telos_core::config::TelosConfig;
use telos_hci::{AgentFeedback, LogLevel, global_log_level};

#[derive(Debug, Clone)]
enum TuiEvent {
    Feedback(AgentFeedback),
    ActiveTasksUpdate(Vec<telos_hci::ActiveTaskInfo>),
    Tick,
}

struct App<'a> {
    textarea: TextArea<'a>,
    chat_history: Vec<String>,
    active_tasks: Vec<telos_hci::ActiveTaskInfo>,
    is_running: bool,
}

impl<'a> Default for App<'a> {
    fn default() -> Self {
        let textarea = TextArea::default();
        Self {
            textarea,
            chat_history: Vec::new(),
            active_tasks: Vec::new(),
            is_running: true,
        }
    }
}

pub async fn run_tui(config: TelosConfig, initial_task: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // 2. Channels
    let (tx, mut rx) = mpsc::unbounded_channel::<TuiEvent>();

    // 3. Spawns
    let tx_ws = tx.clone();
    let ws_url = "ws://127.0.0.1:3000/api/v1/stream";
    tokio::spawn(async move {
        // Try to connect repeatedly
        loop {
            if let Ok((ws_stream, _)) = connect_async(ws_url).await {
                let (_, mut read) = ws_stream.split();
                while let Some(msg) = read.next().await {
                    if let Ok(Message::Text(text)) = msg {
                        if let Ok(feedback) = serde_json::from_str::<AgentFeedback>(&text) {
                            let _ = tx_ws.send(TuiEvent::Feedback(feedback));
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(2000)).await;
        }
    });

    let tx_http = tx.clone();
    tokio::spawn(async move {
        let client = Client::new();
        loop {
            if let Ok(res) = client.get("http://127.0.0.1:3000/api/v1/tasks/active").send().await {
                if let Ok(body) = res.json::<serde_json::Value>().await {
                    if let Some(tasks) = body.get("active_tasks").and_then(|v| v.as_array()) {
                        let mut parsed_tasks = Vec::new();
                        for t in tasks {
                            if let Ok(info) = serde_json::from_value::<telos_hci::ActiveTaskInfo>(t.clone()) {
                                parsed_tasks.push(info);
                            }
                        }
                        let _ = tx_http.send(TuiEvent::ActiveTasksUpdate(parsed_tasks));
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
            let _ = tx_http.send(TuiEvent::Tick);
        }
    });

    // 4. Main Event Loop
    let mut app = App::default();
    let client = Client::new();
    let project_id = config.active_project_id.clone();
    let mut _last_tick = Instant::now();
    let _tick_rate = Duration::from_millis(100);

    app.chat_history.push("Connected to Telos Background Daemon. Ready for tasks.".to_string());

    if let Some(task) = initial_task {
        let trace_id = uuid::Uuid::new_v4().to_string();
        let payload = json!({
            "payload": task,
            "project_id": project_id,
            "trace_id": trace_id
        });
        
        app.chat_history.push(format!(">> {}", task));

        let req_client = client.clone();
        tokio::spawn(async move {
            let _ = req_client
                .post("http://127.0.0.1:3000/api/v1/run")
                .json(&payload)
                .send()
                .await;
        });
    }

    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        if crossterm::event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match (key.code, key.modifiers) {
                        (KeyCode::Char('c'), event::KeyModifiers::CONTROL) => {
                            app.is_running = false;
                        }
                        (KeyCode::Enter, event::KeyModifiers::NONE) => {
                            let text = app.textarea.lines().join("\n");
                            app.textarea.delete_line_by_head();
                            app.textarea.delete_line_by_end();
                            let trimmed = text.trim();
                            
                            if !trimmed.is_empty() {
                                // Dispatch Task
                                let trace_id = uuid::Uuid::new_v4().to_string();
                                let payload = json!({
                                    "payload": trimmed,
                                    "project_id": project_id,
                                    "trace_id": trace_id
                                });
                                
                                app.chat_history.push(format!(">> {}", trimmed));

                                let req_client = client.clone();
                                tokio::spawn(async move {
                                    let _ = req_client
                                        .post("http://127.0.0.1:3000/api/v1/run")
                                        .json(&payload)
                                        .send()
                                        .await;
                                });
                            }
                        }
                        _ => {
                            app.textarea.input(Input::from(key));
                        }
                    }
                }
            }
        }

        while let Ok(tui_event) = rx.try_recv() {
            match tui_event {
                TuiEvent::Feedback(fb) => {
                    let log_level = global_log_level().get();
                    if fb.should_show(log_level) {
                        match fb {
                            AgentFeedback::Output { content, task_id, is_final, .. } => {
                                let prefix = if is_final { "✓" } else { ">>" };
                                app.chat_history.push(format!("[{}] {} {}", task_id.chars().take(8).collect::<String>(), prefix, content));
                            }
                            AgentFeedback::TaskCompleted { summary, task_id } => {
                                let icon = if summary.success { "✅" } else { "⚠️" };
                                app.chat_history.push(format!("[{}] {} Task finished: {}", task_id.chars().take(8).collect::<String>(), icon, summary.summary));
                            }
                            AgentFeedback::NodeStarted { node_id, detail, task_id } => {
                                app.chat_history.push(format!("[{}] ▶ Starting node: {} ({})", task_id.chars().take(8).collect::<String>(), node_id, detail.task_type));
                            }
                            AgentFeedback::NodeFailed { node_id, error, task_id } => {
                                app.chat_history.push(format!("[{}] ✗ Node {} Failed: {}", task_id.chars().take(8).collect::<String>(), node_id, error.message));
                            }
                            AgentFeedback::RequireHumanIntervention { prompt, task_id, .. } => {
                                app.chat_history.push(format!("\n🚨 [HUMAN INTERVENTION REQUIRED] Task: {}\n{}\n", task_id.chars().take(8).collect::<String>(), prompt));
                            }
                            _ => {}
                        }
                    }
                }
                TuiEvent::ActiveTasksUpdate(tasks) => {
                    app.active_tasks = tasks;
                }
                TuiEvent::Tick => {}
            }
        }

        if !app.is_running {
            break;
        }
    }

    // 5. Cleanup
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn ui(f: &mut ratatui::Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(10),   // Active Tasks Monitor
            Constraint::Min(10),      // Chat History
            Constraint::Length(4),    // Input Region
        ])
        .split(f.size());

    // 1. Active Tasks Block
    let tasks_block = Block::default()
        .borders(Borders::ALL)
        .title(" Active Tasks Board (Real-Time) ");
    
    let mut task_items = Vec::new();
    for task in &app.active_tasks {
        let task_id_short = task.task_id.chars().take(8).collect::<String>();
        let nodes_str = if task.running_nodes.is_empty() {
            "Planning...".to_string()
        } else {
            task.running_nodes.join(", ")
        };
        
        let content = format!(
            "[{}] {} | Progress: {}% | Nodes: {}", 
            task_id_short,
            task.task_name,
            task.progress.percentage,
            nodes_str
        );
        task_items.push(ListItem::new(content));
    }
    if task_items.is_empty() {
        task_items.push(ListItem::new("No active tasks running."));
    }
    let tasks_list = List::new(task_items).block(tasks_block);
    f.render_widget(tasks_list, chunks[0]);

    // 2. Chat History
    let history_block = Block::default()
        .borders(Borders::ALL)
        .title(" Execution History & Multi-Agent Outputs ");
    
    // Auto-scroll logic: take only the lines that fit
    let visible_history_lines = (chunks[1].height as usize).saturating_sub(2); // Subtract borders
    let start_idx = app.chat_history.len().saturating_sub(visible_history_lines);
    
    let history_text: String = app.chat_history[start_idx..].join("\n");
    let history_paragraph = Paragraph::new(history_text)
        .block(history_block)
        .wrap(Wrap { trim: true });
    
    f.render_widget(history_paragraph, chunks[1]);

    // 3. Text Area Background Block
    let input_block = Block::default()
        .borders(Borders::ALL)
        .title(" Input (Press Enter to submit, Ctrl+C to quit) ");
    
    // We render the block, and the textarea in the inner space
    f.render_widget(input_block, chunks[2]);
    let inner_area = chunks[2].inner(ratatui::layout::Margin { vertical: 1, horizontal: 1 });
    f.render_widget(&app.textarea, inner_area);
}
