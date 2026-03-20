use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query,
        State,
    },
    response::IntoResponse,
};
use tracing::{debug, warn};

// Telemetry Metrics

// Core Traits and Primitives
use telos_hci::{
    AgentFeedback, EventBroker,
};

// 1. Adapter to convert Context OpenAiProvider to Gateway ModelProvider for the Gateway Manager

use crate::core::state::*;
use crate::api::models::*;
    pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state, query.trace_id))
}

/// 创建系统取消通知（当通道关闭时发送）
    pub fn create_cancellation_feedback(task_id: &str) -> AgentFeedback {
    AgentFeedback::TaskCompleted {
        task_id: task_id.to_string(),
        summary: telos_hci::TaskSummary {
            fulfilled: false,
            completed: true,
            total_nodes: 0,
            completed_nodes: 0,
            failed_nodes: 0,
            total_time_ms: 0,
            summary: "任务已取消：系统连接断开".to_string(),
            failed_node_ids: vec![],
        },
    }
}
    pub async fn handle_socket(mut socket: WebSocket, state: AppState, filter_trace_id: Option<String>) {
    let mut rx = state.broker.subscribe_feedback();
    let mut current_task_id: Option<String> = None;

    loop {
        tokio::select! {
            // 处理来自 broker 的反馈
            result = rx.recv() => {
                match result {
                    Ok(feedback) => {
                        // 跟踪当前任务ID
                        if let Some(task_id) = feedback.task_id() {
                            current_task_id = Some(task_id.to_string());
                        }

                        // Apply trace_id filter if it was requested via query parameter
                        if let Some(expected_trace_id) = &filter_trace_id {
                            // Since task_id is identical to trace_id for CLI runs, we filter on task_id
                            if let Some(t_id) = feedback.task_id() {
                                if t_id != expected_trace_id {
                                    continue;
                                }
                            } else {
                                // If a message has no task_id, we probably don't want to blindly forward it 
                                // to a trace-specific socket, except maybe LogLevelChanged which is global.
                                if !matches!(feedback, telos_hci::AgentFeedback::LogLevelChanged { .. }) {
                                    continue;
                                }
                            }
                        }

                        let msg_str = serde_json::to_string(&feedback).unwrap_or_else(|_| "{}".to_string());

                        if socket.send(Message::Text(msg_str)).await.is_err() {
                            debug!("[WebSocket] Failed to send message, client disconnected");
                            break;
                        }

                        // 如果是最终消息，正常退出
                        if feedback.is_final() {
                            debug!("[WebSocket] Task completed, closing connection");
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        // 通道关闭，发送系统通知
                        debug!("[WebSocket] Broker channel closed, sending cancellation notice");
                        if let Some(task_id) = &current_task_id {
                            let cancellation = create_cancellation_feedback(task_id);
                            let msg_str = serde_json::to_string(&cancellation).unwrap_or_else(|_| "{}".to_string());
                            let _ = socket.send(Message::Text(msg_str)).await;
                        }
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        // 消息积压，继续运行但记录警告
                        warn!("[WebSocket] Warning: Lagged {} messages, continuing...", n);
                        continue;
                    }
                }
            }
            // 处理来自客户端的消息（心跳、关闭请求等）
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Ping(data))) => {
                        if socket.send(Message::Pong(data)).await.is_err() {
                            debug!("[WebSocket] Failed to send pong");
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {
                        // Pong received, continue
                    }
                    Some(Ok(Message::Close(_))) => {
                        debug!("[WebSocket] Client requested close");
                        break;
                    }
                    Some(Err(e)) => {
                        debug!("[WebSocket] WebSocket error: {:?}", e);
                        break;
                    }
                    None => {
                        debug!("[WebSocket] WebSocket stream ended");
                        break;
                    }
                    _ => {
                        // Ignore other message types
                    }
                }
            }
        }
    }

    // 清理：发送关闭帧
    let _ = socket.close().await;
}
