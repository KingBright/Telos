# Telemetry Dashboard Tasks

This document tracks the progress of the Telemetry Dashboard web module, which provides observability into the Telos system's reliable execution and AI agent behaviors.

## Core Objectives
1. **Metrics Collection**: Aggregate single metrics (LLM requests, errors, 429s, tool executions, dynamic tool creation stats, QA rates) and composite metrics (Task success/failure rates).
2. **Web Module (`telos_web`)**: An independent crate serving a lightweight, clean, and responsive frontend dashboard.
3. **Daemon Integration**: The web server must start alongside the `telos_daemon` and share the same lifecycle.

## Tasks

- [ ] **16.1 Dashboard UI Design (Stitch)**
  - [ ] Generate modern UI layout for observability metrics.
  - [ ] Export HTML/JS/CSS frontend assets.
- [ ] **16.2 Create `telos_web` Crate**
  - [ ] Initialize `telos_web` crate in the workspace.
  - [ ] Set up Axum web server to serve static frontend files.
  - [ ] Implement REST/SSE API endpoints to supply metrics to the frontend.
- [ ] **16.3 Metrics Aggregation Logic**
  - [ ] Tie into `telos_telemetry` or global EventBus/Database to calculate:
    - [ ] LLM stats (requests, 429s, other errors).
    - [ ] Tool stats (success, failure).
    - [ ] Dynamic tool stats (creation success/fail, iteration success/fail, execution success/fail).
    - [ ] Agent interactions (QA pass/fail, proactive interactions).
    - [ ] Task execution (overall success/failure from user perspective).
- [ ] **16.4 Daemon Integration**
  - [ ] Modify `telos_daemon` startup to launch `telos_web` server concurrently.
  - [ ] Pass necessary database / broker arcs to the web server.

## Notes/Issues
* Phase 1 focuses strictly on observability metrics. Future phases will expand the UI to include task traces, memory logs, and configurations.
