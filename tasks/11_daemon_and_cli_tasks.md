# Module 11 Tasks: Daemon Server & CLI Client

- [ ] Initialize `telos_daemon` and `telos_cli` crates in the virtual workspace.
- [ ] Configure `Cargo.toml` dependencies for both crates.
- [ ] Implement `telos_daemon` core wiring logic to instantiate all 10 modules.
- [ ] Implement `axum` HTTP and WebSocket endpoints in `telos_daemon`.
- [ ] Implement `clap` CLI parsing in `telos_cli`.
- [ ] Implement network requests (HTTP POST & WebSocket stream) in `telos_cli`.
- [ ] Implement interactive approval prompting using `inquire` in `telos_cli`.
- [ ] Write integration tests for the Daemon event flow.

## Notes/Issues
- Needs to verify if `TokioEventBroker` can be cleanly instantiated with other modules.
