# Module 1 Tasks: HCI & Event Bus

- [x] Define `AgentEvent` and `AgentFeedback` enums.
- [x] Implement `EventBroker` trait and underlying `mpsc` channels.
- [x] Add backpressure logic (bounded channels, drop non-critical events).
- [x] Implement event idempotency with UUID tracking.
- [x] Write unit tests to check event dispatch latency and backpressure drops.

## Notes/Issues
- Completed core event bus functionality in `src/event_bus.rs`.
- Leveraged `lru` crate to keep a bounded LRU cache for trace_id tracking preventing memory leaks over time.
- Integrated `tokio::sync::mpsc` and `tokio::sync::broadcast` to handle event and feedback decoupling.