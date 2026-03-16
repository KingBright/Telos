use async_trait::async_trait;
use tracing::{info, debug, error};

/// Provider 错误类型
#[derive(Debug, Clone)]
pub struct ProviderError {
    /// 用户友好的错误消息
    pub message: String,
    /// 错误分类
    pub kind: ProviderErrorKind,
}

/// Provider 错误分类
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorKind {
    /// 网络错误（可重试）
    NetworkError,
    /// 认证错误（永久性）
    AuthenticationError,
    /// 速率限制（可重试）
    RateLimited,
    /// 服务不可用（可重试）
    ServiceUnavailable,
    /// 配额超限（永久性）
    QuotaExceeded,
    /// 内容过滤（永久性）
    ContentFiltered,
    /// 其他错误
    Other,
}

impl ProviderError {
    pub fn new(message: impl Into<String>, kind: ProviderErrorKind) -> Self {
        Self {
            message: message.into(),
            kind,
        }
    }

    /// 从 HTTP 状态码创建
    pub fn from_http_status(status: u16, body: &str) -> Self {
        let kind = match status {
            401 | 403 => ProviderErrorKind::AuthenticationError,
            429 => ProviderErrorKind::RateLimited,
            503 => ProviderErrorKind::ServiceUnavailable,
            _ if status >= 500 => ProviderErrorKind::ServiceUnavailable,
            _ => ProviderErrorKind::Other,
        };
        Self {
            message: format!("HTTP {}: {}", status, body),
            kind,
        }
    }

    /// 从网络错误创建
    pub fn from_network_error(error: &str) -> Self {
        Self {
            message: error.to_string(),
            kind: ProviderErrorKind::NetworkError,
        }
    }

    /// 是否可重试
    pub fn is_retryable(&self) -> bool {
        matches!(
            self.kind,
            ProviderErrorKind::NetworkError
                | ProviderErrorKind::RateLimited
                | ProviderErrorKind::ServiceUnavailable
        )
    }

    /// 转换为用户友好消息
    pub fn to_user_message(&self) -> String {
        match self.kind {
            ProviderErrorKind::NetworkError => "网络连接失败，请检查网络".to_string(),
            ProviderErrorKind::AuthenticationError => "认证失败，请检查 API 密钥".to_string(),
            ProviderErrorKind::RateLimited => "请求过于频繁，请稍后重试".to_string(),
            ProviderErrorKind::ServiceUnavailable => "服务暂时不可用".to_string(),
            ProviderErrorKind::QuotaExceeded => "配额已用尽".to_string(),
            ProviderErrorKind::ContentFiltered => "内容被过滤".to_string(),
            ProviderErrorKind::Other => self.message.clone(),
        }
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ProviderError {}

// 向后兼容：保留旧的构造方式
impl From<String> for ProviderError {
    fn from(message: String) -> Self {
        Self::new(message, ProviderErrorKind::Other)
    }
}

impl From<&str> for ProviderError {
    fn from(message: &str) -> Self {
        Self::new(message, ProviderErrorKind::Other)
    }
}

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Returns a vector of f32 embeddings for the given text
    async fn embed(&self, text: &str) -> Result<Vec<f32>, ProviderError>;
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Summarizes the given text into a shorter, concise representation
    async fn summarize(&self, text: &str) -> Result<String, ProviderError>;
}

/// A Mock Provider that simulates embeddings and LLM responses locally
/// This is crucial for avoiding heavy ML weights during V1 architecture
/// development and allowing fast, offline `cargo test`.
#[derive(Clone)]
pub struct MockApiProvider {
    // In a real implementation, this might hold an API key or client
    // client: reqwest::Client,
}

impl MockApiProvider {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl EmbeddingProvider for MockApiProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, ProviderError> {
        // Mock embedding: a simple deterministic vector based on text length and some hash-like logic
        // This is purely for testing K-Means and RAPTOR retrieval logic.
        let mut vec = vec![0.0; 32]; // 32-dimensional dummy embedding
        let len = text.len() as f32;
        let char_sum: u32 = text.chars().map(|c| c as u32).sum();

        for (i, v) in vec.iter_mut().enumerate().take(32) {
            *v = ((char_sum + i as u32) % 100) as f32 / 100.0 * (len / 100.0).sin();
        }

        // Normalize the mock vector
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in vec.iter_mut() {
                *v /= norm;
            }
        }

        Ok(vec)
    }
}

#[async_trait]
impl LlmProvider for MockApiProvider {
    async fn summarize(&self, text: &str) -> Result<String, ProviderError> {
        // Mock summarization: Take the first few words and append a suffix
        let words: Vec<&str> = text.split_whitespace().collect();
        let limit = if words.len() > 10 { 10 } else { words.len() };
        let summary = words[..limit].join(" ");
        Ok(format!("{}... [Summarized by Mock API]", summary))
    }
}

/// A real Provider that uses standard OpenAI-compatible HTTP APIs.
/// This fulfills the requirement for a truly "working" compression engine.
#[derive(Clone)]
pub struct OpenAiProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    llm_model: String,
    embedding_model: String,
}

impl OpenAiProvider {
    pub fn new(
        api_key: String,
        base_url: String,
        llm_model: String,
        embedding_model: String,
    ) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))  // 2 minutes for LLM API calls
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            api_key,
            base_url,
            llm_model,
            embedding_model,
        }
    }

    pub async fn chat_completion(&self, prompt: &str) -> Result<String, ProviderError> {
        let messages = vec![
            serde_json::json!({
                "role": "user",
                "content": prompt
            })
        ];
        self.generate_chat(messages).await
    }

    pub async fn generate_chat(&self, messages: Vec<serde_json::Value>) -> Result<String, ProviderError> {
        let response = self.generate_chat_with_tools(messages, None).await?;
        Ok(response.content)
    }

    /// Extended chat completion that supports tool calling.
    /// Returns a structured response including content, tool_calls, and finish_reason.
    pub async fn generate_chat_with_tools(
        &self,
        messages: Vec<serde_json::Value>,
        tools: Option<Vec<serde_json::Value>>,
    ) -> Result<ChatCompletionResponse, ProviderError> {
        let url = format!("{}/chat/completions", self.base_url);

        let mut payload = serde_json::json!({
            "model": self.llm_model,
            "messages": messages,
            "temperature": 0.7
        });

        // Add tools to payload if provided
        if let Some(ref tool_defs) = tools {
            if !tool_defs.is_empty() {
                payload["tools"] = serde_json::json!(tool_defs);
            }
        }

        // === DIAGNOSTIC: Request details ===
        info!("[OpenAiProvider] API Request: {} (Model: {})", url, self.llm_model);
        debug!("[OpenAiProvider] Messages count: {}", messages.len());
        if let Some(ref t) = tools {
            debug!("[OpenAiProvider] Tools count: {}", t.len());
        }
        debug!("[OpenAiProvider] API Key: {}...{}",
            &self.api_key.chars().take(8).collect::<String>(),
            &self.api_key.chars().rev().take(4).collect::<String>().chars().rev().collect::<String>());

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                error!("[OpenAiProvider] ❌ REQUEST FAILED:\n{:#?}", e);
                ProviderError::from_network_error(&format!("{:?}", e))
            })?;

        let status = response.status();
        let status_code = status.as_u16();

        // === DIAGNOSTIC: Response status ===
        info!("[OpenAiProvider] API Response Status: {} ({})", status_code, status.canonical_reason().unwrap_or("Unknown"));

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            error!("[OpenAiProvider] ❌ ERROR RESPONSE BODY: {}", body);
            return Err(ProviderError::from_http_status(status_code, &body));
        }

        // Read response body as text first for debugging
        let response_text = response.text().await.map_err(|e| {
            error!("[OpenAiProvider] ❌ FAILED TO READ RESPONSE: {}", e);
            ProviderError::from_network_error(&format!("Failed to read response: {}", e))
        })?;

        let json: serde_json::Value = serde_json::from_str(&response_text).map_err(|e| {
            error!("[OpenAiProvider] ❌ JSON PARSE ERROR");
            debug!("[OpenAiProvider] Response text (first 500 chars): {}",
                &response_text.chars().take(500).collect::<String>());
            ProviderError::new(
                format!("JSON Parse Error: {} | Response: {}", e,
                    &response_text.chars().take(200).collect::<String>()),
                ProviderErrorKind::Other,
            )
        })?;

        let message = &json["choices"][0]["message"];
        let finish_reason = json["choices"][0]["finish_reason"]
            .as_str()
            .map(|s| s.to_string());

        // Extract text content (may be null when tool_calls are present)
        let content = message["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        // Extract tool calls if present
        let tool_calls = if let Some(tc_array) = message["tool_calls"].as_array() {
            tc_array.iter().filter_map(|tc| {
                let id = tc["id"].as_str()?.to_string();
                let name = tc["function"]["name"].as_str()?.to_string();
                let arguments = tc["function"]["arguments"].as_str()?.to_string();
                Some(ToolCallResponse { id, name, arguments })
            }).collect()
        } else {
            vec![]
        };

        info!("[OpenAiProvider] ✅ SUCCESS - Content: {} bytes, tool_calls: {}, finish_reason: {:?}",
            content.len(), tool_calls.len(), finish_reason);
        debug!("[OpenAiProvider] Response Content: {}", content);

        Ok(ChatCompletionResponse {
            content,
            tool_calls,
            finish_reason,
        })
    }
}

/// Structured response from chat completion API, supporting both text and tool calls.
#[derive(Debug, Clone)]
pub struct ChatCompletionResponse {
    /// Text content from the LLM (may be empty when tool_calls are present)
    pub content: String,
    /// Tool calls requested by the LLM
    pub tool_calls: Vec<ToolCallResponse>,
    /// Finish reason: "stop", "tool_calls", "length", etc.
    pub finish_reason: Option<String>,
}

/// A tool call from the LLM response.
#[derive(Debug, Clone)]
pub struct ToolCallResponse {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[async_trait]
impl EmbeddingProvider for OpenAiProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, ProviderError> {
        let url = format!("{}/embeddings", self.base_url);

        let payload = serde_json::json!({
            "model": self.embedding_model,
            "input": text,
        });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&payload)
            .send()
            .await
            .map_err(|e| ProviderError::from_network_error(&format!("{:?}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::from_http_status(status.as_u16(), &body));
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| ProviderError::new(format!("JSON Parse Error: {}", e), ProviderErrorKind::Other))?;

        let embedding = json["data"][0]["embedding"]
            .as_array()
            .ok_or_else(|| ProviderError::new("Invalid embedding format from API", ProviderErrorKind::Other))?
            .iter()
            .map(|v: &serde_json::Value| v.as_f64().unwrap_or(0.0) as f32)
            .collect();

        Ok(embedding)
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn summarize(&self, text: &str) -> Result<String, ProviderError> {
        let url = format!("{}/chat/completions", self.base_url);

        let payload = serde_json::json!({
            "model": self.llm_model,
            "messages": [
                {
                    "role": "system",
                    "content": "You are a context compressor. Summarize the following text cluster into a single, concise paragraph that captures all key facts and relationships. Do not add conversational filler."
                },
                {
                    "role": "user",
                    "content": text
                }
            ],
            "temperature": 0.3
        });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&payload)
            .send()
            .await
            .map_err(|e| ProviderError::from_network_error(&format!("{:?}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::from_http_status(status.as_u16(), &body));
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| ProviderError::new(format!("JSON Parse Error: {}", e), ProviderErrorKind::Other))?;

        let summary = json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| ProviderError::new("Invalid summary format from API", ProviderErrorKind::Other))?
            .to_string();

        Ok(summary)
    }
}

use std::sync::Arc;


/// A Local Provider that uses ONNX Runtime (`fastembed`) for high-performance,
/// in-process embeddings with zero network overhead.
#[cfg(feature = "local-embeddings")]
#[derive(Clone)]
pub struct LocalEmbeddingProvider {
    model: Arc<Mutex<fastembed::TextEmbedding>>,
}

#[cfg(feature = "local-embeddings")]
impl LocalEmbeddingProvider {
    pub fn new() -> Result<Self, ProviderError> {
        // InitTextEmbedding defaults to BGE-small-en-v1.5 or similar highly efficient models
        let model = fastembed::TextEmbedding::try_new(Default::default()).map_err(|e| {
            ProviderError::new(
                format!("Failed to initialize local embedding model: {}", e),
                ProviderErrorKind::Other,
            )
        })?;

        Ok(Self {
            model: Arc::new(Mutex::new(model)),
        })
    }
}

#[cfg(feature = "local-embeddings")]
#[async_trait]
impl EmbeddingProvider for LocalEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, ProviderError> {
        // fastembed requires a Vec<String> or slice of strings
        let documents = vec![text.to_string()];

        let model = self.model.clone();

        let mut model_lock = model.lock().await;

        let mut embeddings =
            tokio::task::block_in_place(|| model_lock.embed(documents, None::<usize>))
                .map_err(|e| {
                    ProviderError::new(
                        format!("Local embedding failed: {}", e),
                        ProviderErrorKind::Other,
                    )
                })?;

        if embeddings.is_empty() {
            return Err(ProviderError::new(
                "Model returned empty embedding array",
                ProviderErrorKind::Other,
            ));
        }

        // Return the first (and only) embedding
        Ok(embeddings.remove(0))
    }
}

// We can optionally create a Mock/Fallback Local LLM Provider here if desired using Candle,
// but fastembed is primarily focused on embeddings. For a pure local LLM, integrating `candle-core`
// would be the next step, though `OpenAiProvider` targeting a local `vLLM` instance
// is generally preferred for production "local" LLM setups due to continuous batching.

impl Default for MockApiProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// A Provider that acts as a wrapper for the Model Gateway.
/// It routes LLM summarization calls through the central rate-limiting and backoff mechanisms.
#[derive(Clone)]
pub struct GatewayLlmProvider {
    gateway: Arc<dyn telos_model_gateway::ModelGateway>,
}

impl GatewayLlmProvider {
    pub fn new(gateway: Arc<dyn telos_model_gateway::ModelGateway>) -> Self {
        Self { gateway }
    }
}

#[async_trait]
impl LlmProvider for GatewayLlmProvider {
    async fn summarize(&self, text: &str) -> Result<String, ProviderError> {
        let req = telos_model_gateway::LlmRequest {
            session_id: "context_compression".to_string(), // In reality, fetch from context
            messages: vec![
                telos_model_gateway::Message {
                    role: "system".to_string(),
                    content: "You are a context compressor. Summarize the following text cluster into a single, concise paragraph that captures all key facts and relationships. Do not add conversational filler.".to_string(),
                },
                telos_model_gateway::Message {
                    role: "user".to_string(),
                    content: text.to_string(),
                }
            ],
            required_capabilities: telos_model_gateway::Capability {
                requires_vision: false,
                strong_reasoning: false,
            },
            budget_limit: 1000,
            tools: None,
        };

        let response = self
            .gateway
            .generate(req)
            .await
            .map_err(|e| {
                ProviderError::new(
                    e.to_user_message(),
                    if e.is_retryable() { ProviderErrorKind::ServiceUnavailable } else { ProviderErrorKind::Other },
                )
            })?;

        Ok(response.content)
    }
}
