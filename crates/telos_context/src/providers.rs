use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct ProviderError(pub String);

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
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            api_key,
            base_url,
            llm_model,
            embedding_model,
        }
    }

    pub async fn chat_completion(&self, prompt: &str) -> Result<String, ProviderError> {
        let url = format!("{}/chat/completions", self.base_url);

        let payload = serde_json::json!({
            "model": self.llm_model,
            "messages": [
                {
                    "role": "user",
                    "content": prompt
                }
            ],
            "temperature": 0.7
        });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&payload)
            .send()
            .await
            .map_err(|e| ProviderError(format!("HTTP Error: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError(format!("API Error {}: {}", status, body)));
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| ProviderError(format!("JSON Parse Error: {}", e)))?;

        let reply = json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| ProviderError("Invalid reply format from API".to_string()))?
            .to_string();

        Ok(reply)
    }
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
            .map_err(|e| ProviderError(format!("HTTP Error: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError(format!("API Error {}: {}", status, body)));
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| ProviderError(format!("JSON Parse Error: {}", e)))?;

        let embedding = json["data"][0]["embedding"]
            .as_array()
            .ok_or_else(|| ProviderError("Invalid embedding format from API".to_string()))?
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
            .map_err(|e| ProviderError(format!("HTTP Error: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError(format!("API Error {}: {}", status, body)));
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| ProviderError(format!("JSON Parse Error: {}", e)))?;

        let summary = json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| ProviderError("Invalid summary format from API".to_string()))?
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
            ProviderError(format!("Failed to initialize local embedding model: {}", e))
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
                .map_err(|e| ProviderError(format!("Local embedding failed: {}", e)))?;

        if embeddings.is_empty() {
            return Err(ProviderError(
                "Model returned empty embedding array".to_string(),
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
        };

        let response = self
            .gateway
            .generate(req)
            .await
            .map_err(|e| ProviderError(format!("Gateway Error: {:?}", e)))?;

        Ok(response.content)
    }
}
