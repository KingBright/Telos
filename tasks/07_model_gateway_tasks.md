# Module 7 Tasks: Model Gateway & Resource Governor

- [x] Define `LlmRequest` and `ModelGateway` trait.
- [x] Implement exponential backoff for 429/503 status codes.
- [x] Add session-level token leaky bucket rate limiting.
- [x] Test gateway middleware overhead (<2ms).
- [x] Enforce budget cutoff across sessions.

## Notes/Issues
- Defined traits and implemented `GatewayManager` coordinating token usage via `LeakyBucket` thread-safe struct.
- Implemented `ExponentialBackoff` with proper timing formulas `2^c * base_delay + jitter`.
- Integrated `tokio::sync::Mutex` and `rand` inside the gateway for concurrency handling and jitter generation.
- Tests verify leaky bucket rate limiting correctly rejects overhead requests without hitting the dummy provider. Middleware latency successfully measured under 2ms limits.