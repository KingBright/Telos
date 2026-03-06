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

- [x] **Evolution Evaluator (Module 6) -> Memory OS (Module 4)**:
  - (Future) The Actor-Critic system evaluates completed sub-tasks and extracts successful trajectory patterns. It promotes relevant `Episodic` and `Semantic` facts into highly structured `Procedural` (skill) memories.

- [x] **DAG Engine (Module 2) -> Evolution Evaluator (Module 6)**:
  - The DAG engine provides `ExecutionTrace` to the Evolution Evaluator.

- [x] **Tooling & Wasm Sandbox (Module 5) -> Memory OS (Module 4)**:
  - Tool retrieval API implemented via `VectorToolRegistry`. Wasm Sandbox execution API implemented (`ToolExecutor`). The Wasm engine can now run procedural tools extracted from memory templates with tight fuel bounds.

- [ ] **HCI Event Bus (Module 1) -> Memory OS (Module 4)**:
  - (Future) Direct, high-priority user feedback triggers immediate write operations to Semantic Memory with maximum strength, ensuring user preferences override default behaviors.

- [ ] **Model Gateway (Module 7) -> Memory OS (Module 4)**:
  - (Future) Memory OS background workers (reconsolidation) utilize the Model Gateway API for summarization, applying exponential backoff and rate limiting during background compression.

## 07 Model Gateway & Resource Governor (telos_model_gateway) Integration

- [ ] **DAG Engine (Module 2) -> Model Gateway (Module 7)**:
  - DAG engine nodes (`ExecutableNode`) invoke the LLM via the Model Gateway, passing specific prompt messages and capability requirements, while obeying session-level budget constraints.

- [ ] **Context Compression (Module 3) -> Model Gateway (Module 7)**:
  - Context manager's RAPTOR clustering and summarizing routines invoke the LLM via the Model Gateway to construct tree-based hierarchical summaries, with requests properly handled by backoff mechanisms on 429/503 errors.

## Notes/Issues
- *Added as part of Module 3 planning to track inter-module dependencies.*
- Context Compression API implemented. DAG nodes can now inject NodeRequirement (tokens and query string) and receive ScopedContext arrays via the RAPTOR manager.

## 08 Zero-Trust Security & Vault (telos_security) Integration

- [x] **Tooling & Wasm Sandbox (Module 5) -> Security Vault (Module 8)**:
  - Wasm ToolExecutor must validate the tool call against the Vault's ABAC policy (`validate_tool_call`).
  - Wasm Sandbox needs to temporarily lease credentials (`lease_temporary_credential`) with tight TTL before spawning tools.

## 09 Observability & Telemetry (telos_telemetry) Integration

- [ ] **DAG Engine (Module 2) -> Telemetry (Module 9)**:
  - (Future) Ensure `tracing` spans properly encapsulate `ExecutionEngine::run_graph` passing trace IDs through the state machine correctly.

- [ ] **Evolution Evaluator (Module 6) -> Telemetry (Module 9)**:
  - (Future) Telemetry system stores outputs that can be fed natively into the Evolution system avoiding circular dependencies.
