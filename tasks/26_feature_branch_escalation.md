# AI Agent Onboarding Guide (Telos) - Module 26: Atomic PR Escalation Model

## 1. Goal
Evolve the Hierarchical Failure Escalation (HFE) from a "Macro Hard Reset" into an **"Atomic Feature Branch & PR Review"** mechanism. This fundamentally mimics a high-performing Silicon Valley engineering team.
When the `ScrumMaster` dispatches a `DevTask` to a `Worker`, the DAG should sandbox the changes into a temporary Git branch (`task/{node_id}`). If the worker fails compilation, the broken code is preserved, and a `SeniorCoder` (Tech Lead) is dispatched to perform Code Review and write patches. We only abandon the branch and ask the `Architect` to re-plan if the Senior confirms the architecture is fundamentally broken.

## 2. Core Mechanism (The PR Model)
1. **Feature Branch Isolation (L0):**
   - The `WorkerAgent` receives a task (e.g., `calc.rs`).
   - The DAG Engine executes `git checkout -b task/{node_id} ai/wip-vX.X.X`.
   - The Worker generates the code.
2. **Integration Tester (CI/CD Gates):**
   - `HarnessValidatorNode` runs `cargo check`.
   - If **Passed**: The `IntegrationTesterNode` triggers `git checkout ai/wip-vX.X.X && git merge task/{node_id}`.
   - If **Failed**: System triggers L1 Loop. The Worker tries to fix it itself up to 3 times.
3. **Tech Lead Intervention (L1.5 Level):**
   - If L1 exhausts retries, it **does not** `git reset --hard`.
   - Instead, the uncompilable branch is passed to a new `SeniorCoderAgent` (or `ReviewerAgent`).
   - The Senior Agent reads `git diff ai/wip-vX.X.X..task/{node_id}` and the compiler `stderr`.
   - It patches the code syntax (e.g., fixing lifetime issues or import errors) without rewriting the entire file.
4. **Architect Pivot (L2 Level):**
   - If even the Senior Agent can't fix it (compilation still fails), the Senior Agent returns an `ArchitectureDeadEnd` signal.
   - The DAG Engine intercepts this signal, abandons the branch (`git checkout ai/wip-vX.X.X`), and alerts the `ScrumMaster`.
   - The `ScrumMaster` re-issues a new structural breakdown.

## 3. Tasks

- [x] **Feature Branch Shell Logic**: Update `HarnessValidatorNode` inside a global project Mutex to safely checkout a local `task/{node_id}` branch before spawning disk IO and compiler tests.
- [x] **Integration Node Merge Logic**: Update `HarnessValidatorNode` to automatically fast-forward/merge the task branch into the `wip` base branch upon successful `cargo check`, or leave it as a snapshot on failure.
- [x] **L1.5 Senior Agent**: Create `SeniorCoderAgent` (`telos_daemon/src/agents/bmad/tech_lead.rs`) configured to read diffs and write precise string-replacement patches exactly when the AST Critic iteration is exhausted.
- [x] **Circuit Breaker Upgrade**: Abandoned the brutal `git reset --hard` macro logic. Substituted with localized feature-branch isolation, where failures are simply abandoned without destroying the wider build environment.
