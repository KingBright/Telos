# Module 10 Tasks: System Integration

This file tracks the integration points between different modules to ensure the entire Telos architecture works cohesively.

## 03 Context Compression (telos_context) Integration

- [x] **DAG Engine (Module 2) -> Context Compression (Module 3)**:
  - DAG engine nodes (`ExecutableNode`) must be able to request compressed context based on token budgets (`ScopedContext`).
  - Upon node completion, `NodeResult` output data and extracted knowledge must be ingested into the `ContextManager` to update the global/session context (`ingest_new_info`).

- [x] **Memory OS (Module 4) -> Context Compression (Module 3)**:
  - (Future) Context manager needs to flush aged context into the Memory OS, or retrieve facts from Semantic Memory to supplement `ScopedContext`.

- [x] **DAG Engine (Module 2) -> Memory OS (Module 4)**:
  - Task results and execution steps (`NodeResult`) are automatically archived into the Episodic Memory using vector embeddings to enable future retrieval and semantic promotion.

- [ ] **Evolution Evaluator (Module 6) -> Memory OS (Module 4)**:
  - (Future) The Actor-Critic system evaluates completed sub-tasks and extracts successful trajectory patterns. It promotes relevant `Episodic` and `Semantic` facts into highly structured `Procedural` (skill) memories.

- [x] **Tooling & Wasm Sandbox (Module 5) -> Memory OS (Module 4)**:
  - Tool retrieval API implemented via `VectorToolRegistry`. Wasm Sandbox execution API implemented (`ToolExecutor`). The Wasm engine can now run procedural tools extracted from memory templates with tight fuel bounds.

- [ ] **HCI Event Bus (Module 1) -> Memory OS (Module 4)**:
  - (Future) Direct, high-priority user feedback triggers immediate write operations to Semantic Memory with maximum strength, ensuring user preferences override default behaviors.

- [ ] **Model Gateway (Module 7) -> Memory OS (Module 4)**:
  - (Future) Memory OS background workers (reconsolidation) utilize the Model Gateway API for summarization, applying exponential backoff and rate limiting during background compression.

## Notes/Issues
- *Added as part of Module 3 planning to track inter-module dependencies.*
- Context Compression API implemented. DAG nodes can now inject NodeRequirement (tokens and query string) and receive ScopedContext arrays via the RAPTOR manager.
