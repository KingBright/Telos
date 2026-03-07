# Module 11 Tasks: Daemon Server & CLI Client

- [ ] Initialize `telos_daemon` and `telos_cli` crates in the virtual workspace.
- [ ] Configure `Cargo.toml` dependencies for both crates.
- [ ] Implement Configuration Management in `telos_daemon` (e.g., reading `config.toml` for API keys, DB paths).
- [ ] Implement `telos_daemon` core wiring logic to dynamically instantiate all 10 modules using real dependencies (e.g., `OpenAiProvider`, real `TaskGraph` parsing).
- [ ] Implement `axum` HTTP and WebSocket endpoints in `telos_daemon`.
- [ ] Implement `clap` CLI parsing in `telos_cli` with an initialization prompt wizard if configs are missing.
- [ ] Implement network requests (HTTP POST & WebSocket stream) in `telos_cli`.
- [ ] Implement interactive approval prompting using `inquire` in `telos_cli`.
- [ ] Write integration tests for the real Daemon event flow.

## Notes/Issues
- Needs to verify if `TokioEventBroker` can be cleanly instantiated with other modules.
- Ensure no mock providers are used in the production wiring; prompt users for real API keys during `cli init`.
