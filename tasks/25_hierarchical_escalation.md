# AI Agent Onboarding Guide (Telos) - Module 25: Hierarchical Failure Escalation (HFE)

## 1. Goal
Implement a progressive, multi-layered Error Escalation System. When a lower-level Worker Agent completely fails a task (exhausts all retries), the system should **NOT** immediately crash out to the human user.
Instead, it should trigger an atomic rollback (`git reset --hard`) and escalate the failure to the upper-level manager (e.g., `ScrumMaster`), allowing the manager to reconsider the architectural plan and reissue a new set of instructions. Only when the top-level Agent (`ProductAgent` or `Architect`) exhausts its structural retries should it halt and report to the Human Administrator.

## 2. Core Mechanism (The Escalation Ladder)
1. **L1 Failure (Syntax/Implementation level):**
   - The `WorkerAgent` receives compiler errors from the `IntegrationTester` (or `HarnessValidator`).
   - It attempts to read the code and fix the syntax/logic up to its `max_iterations` (e.g., 3 times).
   - If it fails 3 times, it triggers a **L1 Exhaustion**.

2. **L2 Escalation & Rollback (Design level):**
   - Upon L1 Exhaustion, the `DAG Engine` intercepts the SubGraph failure.
   - It executes `git reset --hard HEAD` and `git clean -fd` to revert the repository to the last known working state.
   - The failure context (the specific module that couldn't be implemented and the final compiler dead-end) is escalated back to the original `ScrumMasterAgent`.
   - `ScrumMaster` re-evaluates its `DevTask` breakdown. It realizes its previous API design was fundamentally flawed or impossible to implement, and dynamically generates a *new* set of `DevTask` dependencies to try a different approach.

3. **L3 Escalation (Requirements level):**
   - If `ScrumMaster` generates 3 entirely different architectures and *all* of them fail at the L1 Worker level, it triggers a **L2 Exhaustion**.
   - The issue is escalated to the `ProductAgent`, which might decide that the feature itself is too complex and requires breaking down into even smaller sprints, or it asks the human user for a clarification on the exact requirements.

## 3. Tasks

- [x] **DAG Engine Interceptor**: Hooked into `TokioExecutionEngine` / `EventLoop` to catch fundamental `Failed` status and trigger `git reset --hard HEAD` and clean the workspace.
- [x] **SubGraph Upward Return**: When a SubGraph fails, package the failure reason into the loop context and inject specifically into `attempt + 1` execution (`EventLoop` handles this via `enriched_payload`).
- [x] **ScrumMaster Re-planning**: Intercepted at the MACRO loop level with hardcoded contextual directives ("massive dead-end compiler errors") to FORCE new structural DevTasks instead of repeating.
- [x] **Human Circuit Breaker**: Evaluated the `EventLoop`'s native CLI termination sequence. It only propagates `TaskCompleted(fulfilled=false)` back to the user shell after the MACRO attempt loop (`MAX_ATTEMPTS=5`) completely exhausts all structural variations, correctly shunting minor Worker failures.
