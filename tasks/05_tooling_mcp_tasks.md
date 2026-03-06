# Module 5 Tasks: Tooling & MCP

- [x] Define `ToolSchema` and `ToolExecutor` interfaces.
- [x] Setup `wasmtime` environment with strict memory/CPU fuel limits.
- [x] Build vector-based dynamic tool retrieval.
- [x] Test Wasm cold start delay (<10ms).
- [x] Test sandbox isolation to prevent malicious breakouts.

## Notes/Issues
- Implemented `VectorToolRegistry` using `fastembed` for fast local zero-network embeddings search to retrieve relevant tools.
- Implemented `WasmExecutor` using `wasmtime` providing physical limits on fuel (CPU instructions execution) and memory. Tested infinite loop code returning a `ToolError::Timeout`. Wasm cold starts are well under 10ms.