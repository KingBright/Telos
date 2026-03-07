# Telos Agent: User Guide & E2E Test Cases

Welcome to Telos! This guide will help you compile, configure, and run the stateless daemon and the lightweight CLI client. It also provides five progressively complex end-to-end (E2E) test cases to verify the entire system's execution capabilities.

---

## 1. Compilation & Installation

Telos is built as a pure Rust virtual workspace. To compile the entire system, ensure you have Rust installed and run:

```bash
# Clone the repository
cargo build --release

# The compiled binaries will be located at:
# target/release/telos_cli
# target/release/telos_daemon
```

*For local testing without moving binaries around, you can use `cargo run -p <crate_name>`.*

---

## 2. Initialization Wizard

Telos strictly avoids hardcoded configurations. The system requires a persistent `.telos_config.toml` file in your home directory to load API keys, Base URLs, LLM Models, and Embedding Models dynamically.

If this configuration is missing, the CLI will automatically trigger an interactive setup wizard to guide you.

**Run the CLI to trigger the wizard:**
```bash
cargo run -p telos_cli -- run "Hello"
```

**Example inputs for Zhipu GLM-4 (Cloud Embedding/LLM):**
*   **API Key:** `Your-API-Key-Here`
*   **Base URL:** `https://open.bigmodel.cn/api/paas/v4`
*   **LLM Model:** `glm-4`
*   **Embedding Model:** `Embedding-3`
*   **DB Path:** (Press Enter for default `~/.telos_memory.redb`)

---

## 3. Starting the Daemon

The `telos_daemon` runs as a background service binding to `0.0.0.0:3000`. It dynamically loads the configurations, initiates all 10 core modules (EventBroker, MemoryOS, GatewayManager, TaskGraph Engine, Wasm Sandbox, etc.), and awaits HTTP/WebSocket inputs.

Start it in a dedicated terminal window:
```bash
cargo run -p telos_daemon
```
*You should see logs indicating: `[Daemon] Event loop started.` and `Telos Daemon listening on ws://...`*

---

## 4. End-to-End Delivery Test Cases

Once the daemon is running, you can dispatch tasks using the CLI in another terminal:
`cargo run -p telos_cli -- run "<YOUR_PROMPT>"`

Here are 5 carefully designed E2E scenarios, ranked from simplest to most complex, to test the dynamic workflow logic of the Telos Agent architecture.

### Test Case 1: Simple LLM Intent & Generation
**Objective:** Verify that the system correctly initializes the `ModelGateway`, communicates with the cloud LLM, dynamically builds an `LlmPromptNode` inside the `TaskGraph`, and streams the WebSocket feedback to the CLI.

*   **Command:**
    `cargo run -p telos_cli -- run "Write a short, two-line poem about the rust programming language."`
*   **Expected Behavior:**
    *   Daemon classifies the task as `LLM generation`.
    *   Daemon executes the dynamic DAG.
    *   CLI instantly streams state changes and the final poem text back to the terminal.

### Test Case 2: Zero-Trust Tooling Sandbox Allocation (WASM)
**Objective:** Verify the dynamic conditional routing in the Event Loop. The LLM must correctly parse the user intent as a "Tool Execution" requirement, prompting the daemon to branch away from raw generation and instead construct and execute a `WasmToolNode`.

*   **Command:**
    `cargo run -p telos_cli -- run "Compile and run a python script to calculate fibonacci. (Tool)"`
*   **Expected Behavior:**
    *   Daemon classifies the task as `TOOL execution`.
    *   Daemon injects `WasmToolNode` into the graph.
    *   The `WasmExecutor` compiles the provided secure Wasm blob natively using `wasmtime` fuel allocations.
    *   CLI receives output: `Successfully loaded tool '...' into Wasm Sandbox and verified execution capabilities.`

### Test Case 3: Human-in-the-loop Privileged Escalation (SUDO)
**Objective:** Verify the asynchronous `AgentFeedback::RequireHumanIntervention` pausing mechanic and CLI interactive wizard continuity.

*   **Command:**
    `cargo run -p telos_cli -- run "Please sudo rm -rf /tmp/cache files to clean up space."`
*   **Expected Behavior:**
    *   Daemon intercepts the high-risk "sudo" keyword immediately.
    *   Daemon pauses execution and fires a `RequireHumanIntervention` event via WebSocket.
    *   CLI pauses and renders `🚨 [HUMAN INTERVENTION REQUIRED] 🚨`.
    *   CLI prompts: `Approve this action? [y/N]:`.
    *   User types `y` and hits enter. CLI fires a POST `/approve` back to the server.
    *   Daemon resumes processing and prints: `Task Approved and Continuing...`.

### Test Case 4: Human-in-the-loop Rejection
**Objective:** Verify the opposite flow of Test Case 3 to ensure system safety when the user explicitly aborts a high-risk operation.

*   **Command:**
    `cargo run -p telos_cli -- run "Use sudo to format the primary drive."`
*   **Expected Behavior:**
    *   System halts and prompts for intervention.
    *   User inputs `N` or presses enter without typing `y`.
    *   Daemon receives rejection event and aborts the graph execution, terminating safely.
    *   CLI receives: `Task Rejected.`

### Test Case 5: Complex Cross-Module Integration Stress Test
**Objective:** Verify the robust async capabilities of the `TokioExecutionEngine` dealing with heavy back-to-back requests, triggering MemoryOS vectorization initialization and Context Manager instantiations concurrently without crashing.

*   **Command:** Open three different terminal windows and fire the following commands simultaneously:
    *   Terminal 1: `cargo run -p telos_cli -- run "What is the capital of France?"`
    *   Terminal 2: `cargo run -p telos_cli -- run "TOOL execution test"`
    *   Terminal 3: `cargo run -p telos_cli -- run "sudo restart daemon"`
*   **Expected Behavior:**
    *   The system handles backpressure gracefully via the `TokioEventBroker` `mpsc` limits.
    *   Terminal 1 outputs "Paris".
    *   Terminal 2 outputs "Successfully loaded tool into Wasm Sandbox".
    *   Terminal 3 prompts for human intervention.
    *   The Daemon does not panic, and the thread-safe `Arc` pointers (e.g., `GatewayManager`, `RaptorContextManager`) effectively serialize access safely.

### Test Case 6: Complex Multi-Step Programming Task
**Objective:** Verify the ModelGateway and DAG engine's ability to handle complex, multi-step code generation and validation prompts sequentially.

*   **Command:**
    `cargo run -p telos_cli -- run "Write a Python script that fetches the weather using an open API, parses the JSON to get the temperature, and then prints a recommendation for clothing. Make sure it uses robust error handling."`
*   **Expected Behavior:**
    *   Daemon analyzes the complex intent.
    *   The DAG engine triggers the LLM to generate the script.
    *   The final output streams cleanly without truncation, proving the backpressure limits and context windows handle large payloads robustly.

### Test Case 7: Internet Search & Context Compression
**Objective:** Verify the Context Compression engine's ability to abstract and summarize dense knowledge.

*   **Command:**
    `cargo run -p telos_cli -- run "Search for the latest Rust 2024 features and summarize the 3 most important points into a markdown list."`
*   **Expected Behavior:**
    *   Daemon handles the query via LLM.
    *   The LLM successfully formats the abstracted knowledge into precise Markdown.
    *   The streaming pipeline handles newlines and markdown characters cleanly without UI breakage in the terminal.

### Test Case 8: Cross-Turn Memory & Semantic Retrieval
**Objective:** Verify that the system can maintain coherence across separate execution boundaries, requiring the backend to invoke MemoryOS retrieval strategies.

*   **Command:**
    `cargo run -p telos_cli -- run "Rewrite the Python weather script from earlier into Rust, and evaluate its Big-O time complexity."`
*   **Expected Behavior:**
    *   Since the daemon architecture routes standard generation through a stateless `LlmPromptNode` by default for simple tests, this case tests the boundary of the V1 MVP.
    *   In a fully evolved Telos state, the MemoryOS will inject previous outputs into `ScopedContext`.
    *   Currently, the LLM will generate a standalone Rust script attempting to satisfy the prompt, validating that the routing and response structures do not panic on ambiguous contextual references.
