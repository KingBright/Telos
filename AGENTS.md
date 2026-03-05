# AI Agent Onboarding Guide (Telos)

Welcome to the **Telos** repository. Telos is a pure Rust, high-performance autonomous agent architecture designed for complex, long-running tasks.

When starting a new session, **you must read and understand this document** before making changes to the codebase.

---

## 1. Project Philosophy & Core Architecture

* **Headless Daemon & Actor Model:** The system operates as a stateless daemon. Core modules communicate asynchronously via `tokio::sync::mpsc` channels, functioning as independent Actors.
* **Pure Rust Performance Stack:** We strictly avoid external database processes (no Redis, no Postgres). State storage and vector retrieval must be embedded within the process (using libraries like `redb` or `lance`) to achieve microsecond, zero-network-overhead data exchange.
* **DAG-Driven Control Flow:** LLMs do **not** govern the global control flow. They act purely as computation kernels. A strict Directed Acyclic Graph (DAG) built in Rust dictates task execution, allowing for dynamic re-planning via Kahn's algorithm.
* **Defensive Programming & Zero-Trust Sandbox:** LLM outputs are inherently untrusted. All generated dynamic code must be isolated and executed within a WebAssembly (Wasm) or micro-container sandbox with strict, temporarily-leased permissions.

---

## 2. Codebase Structure (Cargo Virtual Workspace)

The project is structured as a **Cargo Virtual Workspace** located in the `crates/` directory.

### Modules Breakdown
* **`telos_core`**: Contains shared primitive structs and enums (`NodeResult`, `NodeStatus`, `RiskLevel`, `NodeError`). **Do not introduce heavy dependencies here.** Other crates rely on this to avoid circular dependencies.
* **`telos_hci`**: (Module 1) Handles Human-Computer Interaction and the Event Bus. Manages UUID idempotency and channel backpressure.
* **`telos_dag`**: (Module 2) The DAG Engine. Manages `ExecutableNode`, task topologies (`petgraph`), and execution routing.
* **`telos_context`**: (Module 3) Context Compression. Manages AST/EDU decomposition and RAPTOR-based soft clustering to provide LLMs with Scoped Contexts.
* **`telos_memory`**: (Module 4) Hierarchical Memory OS. Handles episodic, semantic, and procedural memory using `lance` (vector) and `redb` (graph). Implements Ebbinghaus forgetting curves.
* **`telos_tooling`**: (Module 5) MCP Tools & Wasm Sandboxing. Enforces fuel/memory limits via `wasmtime`.
* **`telos_evolution`**: (Module 6) Evaluator. Actor-Critic system detecting semantic loops and distilling successful trace experiences into procedural skills.
* **`telos_model_gateway`**: (Module 7) Model Routing. Handles exponential backoff, leaky bucket token throttling, and model failovers.
* **`telos_security`**: (Module 8) Zero-Trust Vault. Manages short-lived JWT credentials and ABAC validation using Casbin policies.
* **`telos_telemetry`**: (Module 9) Observability. OTLP-compatible distributed span tracking for long-running processes.

---

## 3. Task Tracking & Management Protocol

We meticulously track the implementation progress of every module inside the `tasks/` directory.

### Your workflow for every new task:
1. **Locate the Task File:** Look inside the `tasks/` directory for the module you are working on (e.g., `tasks/02_dag_engine_tasks.md`).
2. **Review Status:** Read the file to understand what has been completed (`- [x]`) and what is pending (`- [ ]`).
3. **Execute Work:** Write the Rust code, adhering strictly to the architecture philosophies mentioned above. Use modern libraries.
4. **Write Tests:** **Every module must have detailed test cases.** Do not mark a feature as complete until tests are written and passing (`cargo test`).
5. **Update Task File:** Check off the completed sub-tasks in the Markdown file and write a brief summary of the changes, issues encountered, or architectural decisions made under the `## Notes/Issues` section.

*Before submitting your final code, ensure `cargo check` and `cargo test --workspace` run cleanly without major regressions.*