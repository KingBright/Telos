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

## D1: Corrective LoopNode (2026-03-16)
- [x] LoopState runtime tracker (stagnation detection, best-output tracking)
- [x] SubGraph injection loop_config detection + LoopState registration
- [x] Corrective loop handling after Critic node completion (Actor-Critic pattern)
- [x] CorrectionDirective injection into AgentInput for loop iterations
- [x] Integration with SearchWorker (generate_corrected_queries)
- Key design: loops carry CorrectionDirective with diagnosis + corrective instructions — never blind retry
- Three exit conditions: SatisfactionThreshold, stagnation detection (score variance < 0.05), max_iterations hard cap