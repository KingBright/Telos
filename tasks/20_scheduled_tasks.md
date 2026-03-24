# Iteration 20: Scheduled Missions (Cron) Module

## Overview
Enable the Telos Agent to autonomously manage and execute Scheduled Missions. Rather than "simulating" user inputs, Scheduled Missions are explicit, autonomous assignments. The system must verify the capability (by running it once) before scheduling it, ensure restart-resilience, and inherently support continuous self-evolution through existing Workflow/Tool Mutation loops.

## Phase 1: Mission Storage & Resilience (`telos_memory`)
- [x] Define `ScheduledMission` struct: `id`, `project_id`, `cron_expr`, `mission_context` (instruction or bound `workflow_id`), `origin_channel`, `last_run_at`, `next_run_at`, `status`.
- [x] Create `scheduled_missions` table in `MemoryOS` (redb).
- [x] **Restart Recovery:** Implement `load_and_catchup_missions()` on boot to instantly queue missions where `next_run_at < now()`, then reschedule for the future.

## Phase 2: Workflow Mapping & Verification (`telos_tooling` & Agent Runtime)
- [ ] Create `schedule_mission` native tool.
  - **Constraints:** Before `schedule_mission` can be called, the Agent *must* have successfully executed the intent at least once in the current session (Dry Run). If missing tools/workflows exist, the Agent must create/mutate them first.
- [ ] Create tools for `list_scheduled_missions()` and `cancel_mission(id)`.

## Phase 3: The Scheduler Actor & Mission Dispatch (`telos_daemon`)
- [ ] Create `Scheduler` Actor in `telos_daemon/src/workers/scheduler.rs` utilizing `tokio-cron-scheduler` or a lightweight `sleep` loop.
- [ ] Add `InteractionEvent::SystemMission { mission_id, context, origin_channel }` to EventBus.
- [ ] **System Prompt Context:** Update `PromptBuilder` to format `SystemMission` uniquely—"You are executing an autonomous scheduled mission. Fulfill it and dispatch the result to `origin_channel`. If you fail, log the failure for standard evolution."

## Phase 4: UI & Observability (`telos_web`)
- [ ] Add REST endpoints (`/api/v1/schedules` and `/api/v1/schedules/metrics`).
- [ ] Add **[Missions]** Tab in the Dashboard WebUI:
  - List active cron jobs, last run status, and next run countdown.
  - Display aggregate metrics: Total scheduled missions, total executions, execution count per mission, success rate.
  - Support filtering these metrics by different time dimensions (e.g., last 24h, 7d, 30d).
- [ ] Enhance **[Overview]** Tab:
  - Add the most critical scheduled mission metrics to the primary dashboard view.
- [ ] Simplify Dashboard UI:
  - **Top Bar:** Refine to only show `uptime` (or "System Offline" if disconnected). Remove all other redundant statuses.
  - **Bottom Bar:** Completely remove the bottom status bar for a cleaner UI.
