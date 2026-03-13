# Module 11 Tasks: Daemon Server & CLI Client

- [x] Initialize `telos_daemon` and `telos_cli` crates in the virtual workspace.
- [x] Configure `Cargo.toml` dependencies for both crates.
- [x] Implement Configuration Management in `telos_daemon` (e.g., reading `config.toml` for API keys, DB paths).
- [x] Implement `telos_daemon` core wiring logic to dynamically instantiate all 10 modules using real dependencies (e.g., `OpenAiProvider`, real `TaskGraph` parsing).
- [x] Implement `axum` HTTP and WebSocket endpoints in `telos_daemon`.
- [x] Implement `clap` CLI parsing in `telos_cli` with an initialization prompt wizard if configs are missing.
- [x] Implement network requests (HTTP POST & WebSocket stream) in `telos_cli`.
- [x] Implement interactive approval prompting using `inquire` in `telos_cli`.
- [x] Write integration tests for the real Daemon event flow.

## Notes/Issues
- Needs to verify if `TokioEventBroker` can be cleanly instantiated with other modules.
- **Iteration 6**: Integrated configurable `RouterAgent` persona (name and trait) via `config.toml`, routing logic dynamically includes Memory-OS user profiles prior to task delegation.
  - Implemented `ActiveTaskRegistry` within `TokioExecutionEngine` exposed by `GET /api/v1/tasks/active`.
  - Upgraded Client from basic CLI output to powerful `ratatui` TUI featuring background web-socket listeners and async REST polling.
- Ensure no mock providers are used in the production wiring; prompt users for real API keys during `cli init`.
