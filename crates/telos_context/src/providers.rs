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

        for i in 0..32 {
            vec[i] = ((char_sum + i as u32) % 100) as f32 / 100.0 * (len / 100.0).sin();
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
