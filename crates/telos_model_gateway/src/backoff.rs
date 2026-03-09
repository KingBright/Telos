use std::time::Duration;
use tracing::warn;
use rand::Rng;
use tokio::time::sleep;

/// Exponential backoff configuration for handling external API rate limits
/// More robust retry strategy for 429 (TooManyRequests) errors
#[derive(Debug)]
pub struct ExponentialBackoff {
    /// Base delay in milliseconds (starts at 500ms for better handling of API limits)
    base_delay_ms: u64,
    /// Maximum number of retry attempts
    max_retries: u32,
    /// Maximum delay cap in milliseconds
    max_delay_ms: u64,
}

impl ExponentialBackoff {
    pub fn new(base_delay_ms: u64, max_retries: u32, max_delay_ms: u64) -> Self {
        Self {
            base_delay_ms,
            max_retries,
            max_delay_ms,
        }
    }

    pub fn get_max_retries(&self) -> u32 {
        self.max_retries
    }

    /// Calculate delay with exponential backoff and jitter
    /// Formula: min(base * 2^attempt + jitter, max_delay)
    pub async fn wait(&self, attempt: u32) {
        if attempt == 0 {
            return;
        }

        let final_delay_ms = {
            let mut rng = rand::thread_rng();

            // Exponential backoff: base * 2^attempt
            let power = 2u64.pow(attempt.min(30)); // Avoid overflow
            let delay_ms = self.base_delay_ms.saturating_mul(power);

            // Cap at max delay
            let capped_delay_ms = delay_ms.min(self.max_delay_ms);

            // Add jitter (10-30% of delay)
            let jitter_max = (capped_delay_ms as f64 * 0.3).max(100.0) as u64;
            let jitter = if jitter_max > 0 {
                rng.gen_range(0..=jitter_max)
            } else {
                0
            };

            capped_delay_ms + jitter
        };

        warn!("[Backoff] Waiting {}ms before retry (attempt {})", final_delay_ms, attempt);
        sleep(Duration::from_millis(final_delay_ms)).await;
    }
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        // More robust defaults for external API rate limits:
        // - 500ms base delay (longer than before)
        // - 5 max retries
        // - 30 seconds max delay
        Self::new(500, 5, 30_000)
    }
}
