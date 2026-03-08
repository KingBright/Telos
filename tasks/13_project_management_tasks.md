# Module 13 Tasks: Project Management

- [x] Create `telos_project` crate and primitive structs (`Project`, `ProjectConfig`) in `telos_core`.
- [x] Add active project tracking inside the main `TelosConfig`.
- [x] Implement `ProjectManager` and `ProjectRegistry` to manage projects via JSON data in `~/.telos/projects.json`.
- [x] Update `telos_cli` to include a new `project` subcommand for initializing, listing, and switching projects.
- [x] Update `telos_cli` `run` command to pass `project_id`.
- [x] Update `telos_daemon` event payloads (`RunRequest`, `AgentEvent::UserInput`) to include the `project_id`.
- [x] Update `telos_daemon` execution loop to process and log the active project.
- [x] Update `telos_bot` to pass the active `project_id` and allow viewing the active project.

## Notes
- Completed project management infrastructure.
- Added project context correctly across CLI, Bot, and Daemon endpoints.
