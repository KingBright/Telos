# Phase 6: Engineering Closed-Loop Architecture

This document tracks the implementation of the macro-correction CI/CD loop and Git state-machine to transition the Telos DAG into a fully autonomous, self-healing engineering system.

## 1. Global CI/CD Validation Node (`IntegrationTester`)
- [ ] Implement `IntegrationTesterNode` in `telos_daemon/src/agents/bmad/integration.rs`.
- [ ] Utilize `telos_tooling` internal filesystem/shell tools to execute `cargo check` and `cargo test` securely inside the sandboxed `~/.telos/projects/{project_name}`.
- [ ] Plumb the DAG topology to ensure this node acts as a sink/barrier, executing strictly *after* all `WorkerAgent` subgraphs reach `Completed`.

## 2. Macro-Correction & SubGraph Rewiring (Backward Status Update)
- [ ] Upgrade the DAG Engine (`engine.rs`) or `ScrumMaster` to listen for `IntegrationTester` failures.
- [ ] Stream compiler `stderr` output (like "unresolved module") as MVC feedback back to `ScrumMaster`.
- [ ] Enable `ScrumMaster` to issue dynamic "Hotfix" `DevTask` tickets, rewriting the graph to fix compilation dependencies.

## 3. Topological Worker Dispatching (Dependency Injection)
- [x] Modify `ScrumMaster` LLM schema to force the model to output a `dependency_links` array for each component.
- [x] Translate component dependencies into graph `DependencyType::Data` edges, preventing high-level modules from compiling before foundational ones.

## 4. Project Initialization & Git Version Control
- [x] Initialize `git init` automatically inside `ProductAgent` when the sandbox is created.
- [x] Base working drafts tracking iteratively on `ai/wip-{version}` branches across `IntegrationTester` nodes.
- [x] In the core `EventLoop`: Detect pipeline completions, trigger a final `git merge --squash`, and create definitive `Release` tracking commits on `main`.

## 5. Persistent State Machine (Project Kanban)
- [x] `ProductAgent` creates the initial baseline `.telos_project.toml` containing version numbers.
- [x] `ScrumMaster` dynamically appends all dispatch components into the `.telos_project.toml` tracker.
- [x] Dynamically update the overall project `status = "done"` inside the Event Loop upon QA merge.

## Notes/Issues
- Successfully implemented a dual-branch (WIP + Main) Git sync methodology that elegantly satisfies macro-version rhythms.
- The Git structure behaves like an Actor (draft iterations in `ai/wip`) and Critic (only validated squash-merging to `main`).
