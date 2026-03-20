# Module 17 Tasks: Codebase Refactoring

This file tracks the implementation of the "High Cohesion, Low Coupling" codebase overhaul.

## Overview
As the Telos repository expands, modules like `telos_daemon` and `telos_tooling` have accrued bloated internal files (like `main.rs` and `native.rs`). The goal is to aggressively dismantle these files into specific, domain-scoped file structures without breaking public behavior.

## Tasks

### Phase 1: Modularizing `telos_tooling`
- [x] Create `src/native/` directory.
- [x] Extract `FsReadTool`, `FsWriteTool`, `GlobTool` into `native/fs_tools.rs`.
- [x] Extract `WebSearchTool`, `WebScrapeTool`, `HttpTool` into `native/web_tools.rs`.
- [x] Extract `ShellExecTool`, `GetTimeTool`, `GetLocationTool` into `native/os_tools.rs`.
- [x] Extract `MemoryRecallTool`, `MemoryStoreTool` into `native/memory_tools.rs`.
- [x] Extract `CalculatorTool`, `LspTool`, `CreateRhaiTool` into `native/dev_tools.rs`.
- [x] Create generic `native/mod.rs` to re-export and load tools.
- [x] Safely delete monolithic `src/native.rs`.

### Phase 2: Dismantling `telos_daemon`
- [ ] Extract API Routes & Models into `src/api/`.
- [ ] Extract AppState, MetricsManager, and adapters into `src/core/`.
- [ ] Extract Custom Graph Nodes and Factory into `src/graph/`.
- [ ] Extract background event loops into `src/workers/`.
- [ ] Strip `main.rs` down to merely loading configs and spinning up the `tokio` threads.

## Notes/Issues
- Compilation via `cargo check` must be continually validated to ensure structural integrity holds up.
