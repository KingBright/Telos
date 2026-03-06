use std::time::Instant;

#[derive(Debug)]
pub struct LeakyBucket {
    capacity: usize,
    leak_rate: f64, // tokens per second
    tokens: f64,
    last_update: Instant,
}

impl LeakyBucket {
    pub fn new(capacity: usize, leak_rate: f64) -> Self {
        Self {
            capacity,
            leak_rate,
            tokens: capacity as f64,
            last_update: Instant::now(),
        }
    }

    fn update(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update).as_secs_f64();

        self.tokens += elapsed * self.leak_rate;
        if self.tokens > self.capacity as f64 {
            self.tokens = self.capacity as f64;
        }

        self.last_update = now;
    }

    pub fn try_consume(&mut self, amount: usize) -> bool {
        self.update();
        if self.tokens >= amount as f64 {
            self.tokens -= amount as f64;
            true
        } else {
            false
        }
    }
}
