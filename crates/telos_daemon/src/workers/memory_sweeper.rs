use std::sync::Arc;

// Telemetry Metrics

// Core Traits and Primitives
use telos_model_gateway::gateway::{GatewayManager, ModelProvider};
use telos_model_gateway::ModelGateway;

// 1. Adapter to convert Context OpenAiProvider to Gateway ModelProvider for the Gateway Manager

    pub async fn compress_for_session_log(
    response: &str,
    gateway: &Arc<GatewayManager>,
) -> String {
    const MAX_UNCOMPRESSED: usize = 500;

    if response.len() <= MAX_UNCOMPRESSED {
        return response.to_string();
    }

    use telos_model_gateway::{Capability, LlmRequest, Message, ModelGateway};

    let request = LlmRequest {
        session_id: "session_compress".to_string(),
        messages: vec![
            Message {
                role: "system".into(),
                content: "You are a concise summarizer. Compress the assistant's response into a SHORT summary (max 200 chars). \
                         PRESERVE ALL: specific numbers, port numbers, filenames, function names, variable names, URLs, calculations, and key facts. \
                         Drop verbose explanations and formatting. \
                         Output the summary only, no prefix.".into(),
            },
            Message {
                role: "user".into(),
                content: format!("Compress this response:\n{}", response.chars().take(2000).collect::<String>()),
            },
        ],
        required_capabilities: Capability { requires_vision: false, strong_reasoning: false },
        budget_limit: 200,
        tools: None,
    };

    match gateway.generate(request).await {
        Ok(res) => {
            let summary = res.content.trim().to_string();
            tracing::info!("[Compression] Summary length: {}, Content: {}", summary.len(), summary);
            if summary.is_empty() {
                // Fallback: truncate
                format!("[摘要] {}...", response.chars().take(200).collect::<String>())
            } else {
                format!("[摘要] {}", summary)
            }
        }
        Err(e) => {
            tracing::warn!("[Compression] LLM compression failed: {:?}", e);
            // Fallback: simple truncation if LLM fails
            format!("[摘要] {}...", response.chars().take(200).collect::<String>())
        }
    }
}

// --- App State ---
    pub async fn summarize_evicted_logs(gateway: &std::sync::Arc<GatewayManager>, previous_summary: &str, new_logs: &[telos_context::LogEntry]) -> String {
    use telos_model_gateway::{Capability, LlmRequest, Message, ModelGateway};

    if new_logs.is_empty() {
        return previous_summary.to_string();
    }

    let mut logs_text = String::new();
    for log in new_logs {
        logs_text.push_str(&format!("{}: {}\n", log.timestamp, log.message));
    }

    let prompt = format!(
        "Please update the running [SESSION SUMMARY] of the ongoing conversation.\n\
         You are provided with the [PREVIOUS SUMMARY] and a [NEW LOG BATCH].\n\
         Merge the new information into the summary concisely.\n\
         Retain specific facts, user preferences, names, numbers, and decisions.\n\n\
         [PREVIOUS SUMMARY]\n{}\n\n\
         [NEW LOG BATCH]\n{}\n\n\
         Respond with ONLY the updated summary text.",
        if previous_summary.is_empty() { "None." } else { previous_summary },
        logs_text
    );

    let request = LlmRequest {
        session_id: "rolling_summary".to_string(),
        messages: vec![
            Message { role: "system".into(), content: "You are a seamless context window summarizer. Capture continuity logically without omitting key facts. Be as concise as possible.".into() },
            Message { role: "user".into(), content: prompt },
        ],
        required_capabilities: Capability { requires_vision: false, strong_reasoning: false },
        budget_limit: 400,
        tools: None,
    };

    match gateway.generate(request).await {
        Ok(res) => {
            let res_text = res.content.trim().to_string();
            tracing::info!("[RollingSummary] generated, len: {}", res_text.len());
            res_text
        },
        Err(e) => {
            tracing::warn!("[RollingSummary] Failed: {:?}. Keeping previous.", e);
            previous_summary.to_string()
        }
    }
}

