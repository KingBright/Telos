use std::time::Duration;
use rand::Rng;
use tokio::time::sleep;

#[derive(Debug)]
pub struct ExponentialBackoff {
    base_delay: u64,
    max_retries: u32,
    max_delay: u64,
}

impl ExponentialBackoff {
    pub fn new(base_delay: u64, max_retries: u32, max_delay: u64) -> Self {
        Self {
            base_delay,
            max_retries,
            max_delay,
        }
    }

    pub fn get_max_retries(&self) -> u32 {
        self.max_retries
    }

    pub async fn wait(&self, attempt: u32) {
        if attempt == 0 {
            return;
        }

        let final_delay_ms = {
            let mut rng = rand::thread_rng();

            // T_wait = 2^c * base_delay + jitter
            let power = 2u64.pow(attempt.min(31)); // Avoid overflow
            let delay_ms = power.saturating_mul(self.base_delay);
            let capped_delay_ms = delay_ms.min(self.max_delay);

            let jitter = rng.gen_range(0..=(capped_delay_ms / 4).max(1));
            capped_delay_ms + jitter
        };

        sleep(Duration::from_millis(final_delay_ms)).await;
    }
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        Self::new(100, 5, 30_000) // 100ms base, 5 retries, 30s max delay
    }
}
