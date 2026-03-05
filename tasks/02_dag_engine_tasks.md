# Module 2 Tasks: DAG Engine

- [x] Define `ExecutableNode` trait and `NodeResult` structure.
- [x] Implement `TaskGraph` with `petgraph` dependency.
- [x] Implement Kahn's topological sort for node execution.
- [x] Add state machine execution logic in `ExecutionEngine`.
- [x] Add checkpointing via `redb`.
- [x] Test DAG execution, parallelism, and recovery times.

## Notes/Issues
- Completed DAG engine implementation utilizing `petgraph` and `tokio`.
- Enhanced `TaskGraph` structure with node indexing and precise result/state tracking.
- Successfully implemented a parallel execution engine utilizing Kahn's algorithm dynamically updating in-degrees and spawning parallel tasks based on dependencies.
- Added `redb` local embedded storage mechanism for state checkpointing and recovery.
- Developed specific tests validating DAG execution order, parallel execution capabilities, and database checkpoint restoration.