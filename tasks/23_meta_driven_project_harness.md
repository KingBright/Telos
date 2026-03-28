# Module 14 Tasks: Meta-Driven Project Harness

## Core Philosophy
- **Meta-Graph as Truth**: System output is structured metadata (JSON/YAML) mapped to a DAG.
- **Progressive Disclosure**: LLMs receive a Minimal Viable Context (MVC) comprising current targets and rigid contracts, not the whole codebase.
- **Harness Dictates, LLM Fills**: ReAct is abandoned. Control flow (loops, retries) is executed by the Harness (Rust state machine). LLM acts as a pure function: `f(Context) -> Metadata`.

## The Matrix Data Models (L1-L3)
- [x] Implement `ProductFeature` (L1): Tracks acceptance criteria (depth) and user journeys (breadth).
- [x] Implement `TechModule` & `Contract` (L2): Maps to features. Contracts define strict schemas (OpenAPI/Protobuf) for provider-consumer relationships.
- [x] Implement `DevTask` (L3): Actionable units assigning a specific file with enforced contracts.

## System Core Components
- [x] **Meta-Graph Store**: Database for DAG topology storing `Depends_On`, `Implements`, `Tested_By` relationships.
- [x] **BMAD Persona Pool**:
  - `Product_Agent`: Outputs business graphs.
  - `Architect_Agent`: Outputs tech graphs & exact contracts.
  - `ScrumMaster_Agent`: Deconstructs modules into `DevTask`s.
  - `Worker_Agent`: Executes specific `DevTask`s strictly bound by context.
- [x] **Progressive Context Assembler**: Filters the DAG to provide only 1st-degree relevant context (objective + contracts + target file).
- [x] **Harness Validator**: Strict QA gateway executing static analysis, AST checks, or mock tests. Rejects invalid output to trigger harness-level retry.

## Execution Orchestrator Phases
- [x] **Phase 1: Ideation**: `Product_Agent` generates `ProductFeature`s.
- [x] **Phase 2: Architecture & Contracts**: `Architect_Agent` locks in `Contract`s.
- [x] **Phase 3: WBS Scaffolding**: `ScrumMaster_Agent` generates `DevTask` queue.
- [x] **Phase 4: Controlled Execution**: `Worker_Agent` writes concrete files via MVC.
- [x] **Phase 5: Contract-Driven QA**: Automated mock testing based on L2 contracts.

## Notes/Issues
- *This is the ultimate generalization of the Telos DAG. It replaces unstructured text generation with a strict deterministic metadata pipeline.*
