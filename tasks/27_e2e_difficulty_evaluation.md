# E2E System Evaluation: Tiered Difficulty Projects

This document tracks the execution and evaluation of the 3 test projects designed to empirically validate the Telos Agent Architecture (specifically evaluating the success of the new Atomic PR Escalation, DAG Planning, and TechLead loops).

## 1. Project 1: Simple (Basic Logic & Single Module)
**Goal:** Verify that the system can properly plan, write, compile, and merge a foundational Rust application without over-complicating dependencies.
**Description:** A CLI-based RPN (Reverse Polish Notation) Calculator.
- **Features:** Supports `+`, `-`, `*`, `/`. Must handle division by zero safely using `Result`.
- **Expected System Behavior:** 
  - `ScrumMaster` should create 1 or 2 Worker nodes (e.g., `parser` and `evaluator`).
  - System shouldn't need massive external dependencies.
  - Harness should compile easily on the first or second attempt.

## 2. Project 2: Medium (Async IO & External Dependencies)
**Goal:** Verify that the system can handle external crates, async runtimes, and slightly larger context windows across multiple interacting modules.
**Description:** A Concurrent File Downloader (like a mini `wget` / `aria2`).
- **Features:** Takes a URL, fetches the `Content-Length`, spins up `N` asynchronous Tokio tasks to fetch explicit byte ranges (`Range: bytes=...`) using `reqwest`, and reconstructs the final binary file on disk.
- **Expected System Behavior:** 
  - The LLM MUST recognize the need for `tokio` and `reqwest`. (Will it automatically modify `Cargo.toml`? This is the test for the Dependency defect mentioned earlier).
  - Multiple DAG nodes must execute chronologically (e.g., `network_client` -> `file_writer` -> `coordinator`).
  - High chance of initial `cargo check` failures due to `async` lifetimes and `Send/Sync` bounds, stress-testing the `TechLeadAgent` surgical patches.

## 3. Project 3: Complex (Networking, Concurrency, State & Custom Protocols)
**Goal:** Stress-test the DAG's topological merge logic, Context limit, and the Macro Circuit Breaker's ability to abandon catastrophic rabbit holes.
**Description:** A Local In-Memory Key-Value Database over TCP (Mini Redis).
- **Features:** 
  - Custom TCP server using `tokio::net::TcpListener`.
  - A thread-safe, concurrent `RwLock<HashMap<String, String>>` storage engine.
  - Custom RESP-like text protocol parser (e.g., `SET <key> <value>`, `GET <key>`, `DEL <key>`).
  - Graceful shutdown signals using `tokio::signal`.
- **Expected System Behavior:** 
  - Massive architectural planning required by `ScrumMaster`.
  - Intricate dependencies between the `TCP Handler`, `Protocol Parser`, and `Storage Engine`.
  - Evaluates if the `IntegrationTesterNode` can correctly fail an integrated DAG run if the generic protocol parser panics when interacting with the storage layer.

---

## 3. Tasks

- [ ] **Run Project 1 (Simple) & Document Metrics**
- [ ] **Run Project 2 (Medium) & Document Metrics**
- [ ] **Run Project 3 (Complex) & Document Metrics**
- [ ] **Identify & Resolve System Deficiencies (e.g., Cargo Manager)**
