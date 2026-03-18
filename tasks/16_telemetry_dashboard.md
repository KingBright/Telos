# Telemetry Dashboard Tasks

This document tracks the progress of the Telemetry Dashboard web module, which provides observability into the Telos system's reliable execution and AI agent behaviors.

## Core Objectives
1. **Metrics Collection**: Aggregate single metrics (LLM requests, errors, 429s, tool executions, dynamic tool creation stats, QA rates) and composite metrics (Task success/failure rates).
2. **Web Module (`telos_web`)**: An independent crate serving a lightweight, clean, and responsive frontend dashboard.
3. **Daemon Integration**: The web server must start alongside the `telos_daemon` and share the same lifecycle.

## Tasks

- [x] **16.1 Dashboard UI Design (Stitch)**
  - [x] Generate modern UI layout for observability metrics.
  - [x] Export HTML/JS/CSS frontend assets.
- [x] **16.2 Create `telos_web` Crate**
  - [x] Initialize `telos_web` crate in the workspace.
  - [x] Set up Axum web server to serve static frontend files.
  - [x] Implement REST/SSE API endpoints to supply metrics to the frontend.
- [x] **16.3 Metrics Aggregation Logic**
  - [x] Initialize a `GlobalMetrics` struct (managed via `Arc<RwLock<..>>`) in `telos_daemon`.
  - [x] Implement event-based counting hooks for:
    - [x] **LLM Gateway** (Total requests, 429 rate limits, other API errors, cumulative Token consumption, estimated USD cost).
    - [x] **Task & Control Flow** (Overall Task Success, Task Failure, Active concurrent tasks, Paused tasks, Semantic Loop interventions).
    - [x] **Agent & Evolution** (QA Passed intercepts, QA Failed intercepts, Proactive human interactions).
    - [x] **Dynamic Tool Sandbox** (Creation success, Creation failure, Iteration success, Iteration failure, Execution success, Execution failure).
    - [x] **Memory OS** (Total entries by type: Episodic/Semantic/Procedural, Proceedural Distillation count).
- [x] **16.4 Daemon Integration**
  - [x] Modify `telos_daemon` startup to launch `telos_web` server concurrently.
  - [x] Pass necessary database / broker arcs to the web server.

## Notes/Issues
* Phase 1 focuses strictly on observability metrics. Future phases will expand the UI to include task traces, memory logs, and configurations.
* **Deployment Update (2026-03)**: Frontend UI is dynamically verified from `~/.telos/web` to permit global daemon execution. The `install.sh` script actively provisions and migrates these assets during install.
