use std::time::Instant;

/// Simple rate limiter with fixed token consumption
/// - Maximum capacity (e.g., 20 concurrent requests)
/// - Each request consumes exactly 1 token
/// - Recovers 1 token per second
#[derive(Debug)]
pub struct SimpleRateLimiter {
    max_tokens: usize,
    tokens: usize,
    last_update: Instant,
}

impl SimpleRateLimiter {
    /// Create a new rate limiter
    /// max_tokens = 0 means unlimited
    pub fn new(max_tokens: usize) -> Self {
        Self {
            max_tokens,
            tokens: max_tokens, // Start at full capacity
            last_update: Instant::now(),
        }
    }

    /// Try to consume 1 token for a request
    /// Returns true if allowed, false if rate limited
    pub fn try_consume(&mut self) -> bool {
        // Unlimited mode
        if self.max_tokens == 0 {
            return true;
        }

        // Update tokens based on time elapsed (recover 1 per second)
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update).as_secs_f64();

        // Recover tokens (up to max)
        let recovered = elapsed as usize;
        self.tokens = (self.tokens + recovered).min(self.max_tokens);
        self.last_update = now;

        // Try to consume 1 token
        if self.tokens >= 1 {
            self.tokens -= 1;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unlimited_mode() {
        let mut limiter = SimpleRateLimiter::new(0);
        assert!(limiter.try_consume());
        assert!(limiter.try_consume());
        assert!(limiter.try_consume());
    }

    #[test]
    fn test_basic_limiting() {
        let mut limiter = SimpleRateLimiter::new(3);

        // Should start with 3 tokens
        assert!(limiter.try_consume()); // 2 left
        assert!(limiter.try_consume()); // 1 left
        assert!(limiter.try_consume()); // 0 left
        assert!(!limiter.try_consume()); // rate limited
    }

    #[test]
    fn test_recovery() {
        let mut limiter = SimpleRateLimiter::new(2);

        assert!(limiter.try_consume()); // 1 left
        assert!(limiter.try_consume()); // 0 left
        assert!(!limiter.try_consume()); // rate limited

        // Wait 1 second for recovery
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(limiter.try_consume()); // recovered 1
    }
}
