use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex as TokioMutex;
use std::collections::HashMap;
use std::time::Instant;
use tracing::{debug, error};
use telos_hci::{AgentEvent, AgentFeedback, EventBroker, TaskSummary, global_log_level};
use telos_core::{config::TelosConfig, SystemRegistry};
use telos_memory::engine::{MemoryOS, RedbGraphStore};
use telos_model_gateway::ModelGateway;

use telos_dag::{TaskGraph, GraphState, ExecutableNode, NodeMetadata, ExecutionEngine};
use telos_context::ContextManager;
use telos_tooling::ToolRegistry;

use crate::{
    graph::nodes::{truncate_for_preview, parse_clarification_options, LlmPromptNode},
    core::adapters::DaemonRegistry,
    graph::factory::DaemonNodeFactory,
    agents,
    compress_for_session_log, summarize_evicted_logs, extract_and_store_user_profile
};

pub async fn run_event_loop(
    mut event_rx: mpsc::Receiver<AgentEvent>,
    broker_bg: Arc<dyn EventBroker>,
    gateway_clone: Arc<telos_model_gateway::gateway::GatewayManager>,
    registry_clone: Arc<DaemonRegistry>,
    tool_registry: Arc<tokio::sync::RwLock<telos_tooling::retrieval::VectorToolRegistry>>,
    context_manager: Arc<telos_context::RaptorContextManager>,
    loop_config: Arc<TelosConfig>,
    paused_tasks_bg: Arc<TokioMutex<HashMap<String, String>>>,
    wakeup_map_bg: Arc<TokioMutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<(String, String, String)>>>>,
    active_tasks_loop: telos_dag::engine::ActiveTaskRegistry,
    distillation_tx_bg: tokio::sync::mpsc::UnboundedSender<telos_evolution::ExecutionTrace>,
    session_logs_loop: Arc<tokio::sync::RwLock<crate::core::state::SessionState>>,
    evaluator_loop: Arc<telos_evolution::evaluator::ActorCriticEvaluator>,
) {
    debug!("[Daemon] Event loop started.");

    // --- RESUME CRASHED/STOPPED TASKS ---
    let cp_mgr = std::env::var("HOME").ok()
        .map(|h| std::path::PathBuf::from(h).join(".telos").join("checkpoints.redb"))
        .and_then(|p| telos_dag::checkpoint::CheckpointManager::new(&p).ok());
        
    if let Some(mgr) = cp_mgr {
        if let Ok(checkpoints) = mgr.get_all_checkpoints() {
            let mut resumed_count = 0;
            for (graph_id, json) in checkpoints {
                match serde_json::from_str::<TaskGraph>(&json) {
                    Ok(mut graph) => {
                        if graph.current_state.completed || graph.schema_version < 1 {
                            tracing::info!("[Daemon] 🗑️ Cleaning up outdated/completed checkpoint: {}", graph_id);
                            let _ = mgr.delete_checkpoint(&graph_id);
                        } else {
                            // Recreate node logic
                            let node_factory = std::sync::Arc::new(DaemonNodeFactory {
                                gateway: gateway_clone.clone(),
                                tool_registry: tool_registry.clone(),
                                tools_dir: loop_config.tools_dir.clone(),
                            });
                            
                            if let Err(e) = graph.rebuild_nodes(&*node_factory) {
                                tracing::warn!("[Daemon] Failed to rebuild nodes for {}: {}", graph_id, e);
                                continue;
                            }

                            // Register into active tasks
                            {
                                let mut w = active_tasks_loop.write().await;
                                w.insert(
                                    graph_id.clone(),
                                    telos_hci::ActiveTaskInfo {
                                        task_id: graph_id.clone(),
                                        task_name: graph_id.clone(),
                                        progress: telos_hci::ProgressInfo::new(0, graph.nodes.len(), 0, 0, graph.nodes.len(), None),
                                        running_nodes: vec![],
                                        started_at_ms: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
                                    },
                                );
                            }
                            
                            let mut execution_engine = telos_dag::engine::TokioExecutionEngine::new()
                                .with_node_factory(node_factory)
                                .with_active_tasks(active_tasks_loop.clone())
                                .with_evaluator(evaluator_loop.clone());
                                
                            let registry_clone = registry_clone.clone();
                            let broker_bg = broker_bg.clone();
                                
                            tokio::spawn(async move {
                                tracing::info!("[Daemon] 🔄 Resuming active Graph: {}", graph_id);
                                let system_registry_arc: Arc<dyn telos_core::SystemRegistry> = registry_clone;
                                 execution_engine.run_graph(&mut graph, &telos_context::ScopedContext {
                                    budget_tokens: 4000,
                                    summary_tree: vec![],
                                    precise_facts: vec![],
                                }, system_registry_arc.as_ref(), broker_bg.as_ref()).await;
                                tracing::info!("[Daemon] ✅ Resumed graph {} finished.", graph_id);
                            });
                            
                            resumed_count += 1;
                        }
                    }
                    Err(_) => {
                        tracing::info!("[Daemon] 🗑️ Cleaning up invalid/legacy checkpoint: {}", graph_id);
                        let _ = mgr.delete_checkpoint(&graph_id);
                    }
                }
            }
            if resumed_count > 0 {
                tracing::info!("[Daemon] 🔄 Resumed {} active tasks from checkpoints.", resumed_count);
            }
        }
    }
    // ------------------------------------

    while let Some(event) = event_rx.recv().await {
            match event {
                AgentEvent::SetLogLevel { level } => {
                    let old_level = global_log_level().get();
                    global_log_level().set(level);
                    broker_bg.publish_feedback(AgentFeedback::LogLevelChanged {
                        old_level,
                        new_level: level,
                    });
                    debug!("[Daemon] Log level changed: {:?} -> {:?}", old_level, level);
                }
                event @ AgentEvent::UserInput { .. } | event @ AgentEvent::SystemMission { .. } => {
                    let (session_id, payload, trace_id, project_id, system_mission_channel) = match event {
                        AgentEvent::UserInput { session_id, payload, trace_id, project_id } => (session_id, payload, trace_id, project_id, None),
                        AgentEvent::SystemMission { mission_id, context, origin_channel, trace_id } => (mission_id, context, trace_id, None, Some(origin_channel)),
                        _ => unreachable!(),
                    };
                    let broker_bg = broker_bg.clone();
                    let gateway_clone = gateway_clone.clone();
                    let registry_clone = registry_clone.clone();
                    let tool_registry = tool_registry.clone();
                    let config = loop_config.clone();
                    let paused_tasks_bg = paused_tasks_bg.clone();
                    let distillation_tx_spawn = distillation_tx_bg.clone();
                    let session_logs_loop = session_logs_loop.clone();
                    let evaluator_spawn = evaluator_loop.clone();

                    let node_factory = std::sync::Arc::new(DaemonNodeFactory {
                        gateway: gateway_clone.clone(),
                        tool_registry: tool_registry.clone(),
                        tools_dir: config.tools_dir.clone(),
                    });

                    let mut execution_engine = telos_dag::engine::TokioExecutionEngine::new()
                        .with_node_factory(node_factory)
                        .with_active_tasks(active_tasks_loop.clone())
                        .with_evaluator(evaluator_spawn);

                    let context_manager_spawn = context_manager.clone();
                    let active_tasks_spawn = active_tasks_loop.clone();
                    tokio::spawn(async move {
                        let task_start_time = Instant::now();
                        debug!(
                            "[Daemon] Received UserInput: {} (trace: {})",
                            payload, trace_id
                        );
                        let current_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;

                        // -- GLOBAL SESSION HISTORY INJECTION --
                        // -- GLOBAL SESSION HISTORY INJECTION (Native Working Memory) --
                        let mut conversation_history_vec = Vec::new();
                        let mut memory_context_text = String::new();
                        {
                            let mut logs_w = session_logs_loop.write().await;
                            
                            // Maintain max 20 turns, capture evicted items into buffer
                            let mut evicted = Vec::new();
                            while logs_w.logs.len() > 20 {
                                if let Some(front) = logs_w.logs.pop_front() {
                                    logs_w.evicted_buffer.push(front);
                                }
                            }
                            if logs_w.evicted_buffer.len() >= 6 {
                                evicted = std::mem::take(&mut logs_w.evicted_buffer);
                            }

                            if !logs_w.logs.is_empty() {
                                for log in logs_w.logs.iter() {
                                    let (role, content) = if log.message.starts_with("User: ") {
                                        ("user", log.message.trim_start_matches("User: "))
                                    } else if log.message.starts_with("Assistant: ") {
                                        ("assistant", log.message.trim_start_matches("Assistant: "))
                                    } else {
                                        ("system", log.message.as_str())
                                    };
                                    conversation_history_vec.push(telos_core::ConversationMessage {
                                        role: role.to_string(),
                                        content: content.to_string(),
                                    });
                                }
                            } else {
                                    // Session memory is empty — preload from persistent memory (telos_memory)
                                    // This bridges headless CLI calls and daemon restarts
                                    let twenty_four_hours_ago = current_ms.saturating_sub(86_400_000);
                                    if let Ok(results) = registry_clone.memory_os.retrieve(telos_memory::MemoryQuery::TimeRange {
                                        start: twenty_four_hours_ago,
                                        end: current_ms,
                                    }).await {
                                        let mut interaction_entries: Vec<&telos_memory::MemoryEntry> = results.iter()
                                            .filter(|e| e.memory_type == telos_memory::MemoryType::InteractionEvent)
                                            .collect();
                                        // Sort newest first to easily grab the most recent interactions
                                        interaction_entries.sort_by_key(|e| std::cmp::Reverse(e.created_at));
                                        
                                        // Take the 15 most recent turns (30 messages total)
                                        interaction_entries.truncate(15);
                                        
                                        // Reverse back to chronological order (oldest -> newest) for the LLM context
                                        interaction_entries.reverse();
                                        
                                        if !interaction_entries.is_empty() {
                                            for entry in &interaction_entries {
                                                if entry.content.starts_with("User: ") && entry.content.contains("\nAssistant: ") {
                                                    let parts: Vec<&str> = entry.content.splitn(2, "\nAssistant: ").collect();
                                                    let user_part = parts[0].trim_start_matches("User: ");
                                                    let assistant_part = parts[1];
                                                    
                                                    conversation_history_vec.push(telos_core::ConversationMessage {
                                                        role: "user".to_string(),
                                                        content: user_part.to_string(),
                                                    });
                                                    conversation_history_vec.push(telos_core::ConversationMessage {
                                                        role: "assistant".to_string(),
                                                        content: assistant_part.to_string(),
                                                    });
                                                } else {
                                                    let (role, content) = if entry.content.starts_with("User: ") {
                                                        ("user", entry.content.trim_start_matches("User: "))
                                                    } else if entry.content.starts_with("Assistant: ") {
                                                        ("assistant", entry.content.trim_start_matches("Assistant: "))
                                                    } else {
                                                        ("system", entry.content.as_str())
                                                    };
                                                    conversation_history_vec.push(telos_core::ConversationMessage {
                                                        role: role.to_string(),
                                                        content: content.to_string(),
                                                    });
                                                }
                                            }
                                            debug!("[Daemon] Preloaded {} native interaction events from persistent memory (24h window)", interaction_entries.len());
                                        }
                                    }
                            }
                            
                            // Immediately append the user's new query so it's logged
                            logs_w.logs.push_back(telos_context::LogEntry {
                                timestamp: current_ms,
                                message: format!("User: {}", payload),
                            });

                            if !logs_w.rolling_summary.is_empty() {
                                memory_context_text.push_str("\n[SESSION CONTEXT SUMMARY — 更早的对话概要]\n");
                                memory_context_text.push_str(&logs_w.rolling_summary);
                                memory_context_text.push_str("\n[END SESSION CONTEXT SUMMARY]\n\n");
                            }

                            if !evicted.is_empty() {
                                let gw_for_sum = gateway_clone.clone();
                                let state_for_sum = session_logs_loop.clone();
                                let prev_sum = logs_w.rolling_summary.clone();
                                tokio::spawn(async move {
                                    let new_sum = summarize_evicted_logs(&gw_for_sum, &prev_sum, &evicted).await;
                                    let mut state_w = state_for_sum.write().await;
                                    state_w.rolling_summary = new_sum;
                                });
                            }
                        }
                        // ----------------------------------------

                        // --- USER PROFILE INJECTION (Dual-Layer: Static + Dynamic) ---
                        {
                            let mem_os = registry_clone.memory_os.clone();
                            let profile_text = telos_memory::build_and_format_profile(&*mem_os).await;
                            if !profile_text.is_empty() {
                                memory_context_text.push_str(&profile_text);
                                debug!("[Daemon] Injected dual-layer user profile into memory context");
                            }
                        }
                        // ----------------------------------------

                        // --- PROCEDURAL MEMORY INJECTION (Learned Strategies) ---
                        {
                            let mem_os = registry_clone.memory_os.clone();
                            if let Ok(results) = mem_os.retrieve(telos_memory::MemoryQuery::TimeRange {
                                start: 0,
                                end: u64::MAX,
                            }).await {
                                let procedural_entries: Vec<&telos_memory::MemoryEntry> = results.iter()
                                    .filter(|e| e.memory_type == telos_memory::MemoryType::Procedural)
                                    .collect();
                                if !procedural_entries.is_empty() {
                                    memory_context_text.push_str("[LEARNED STRATEGIES — distilled from past successful task executions]\n");
                                    for entry in procedural_entries.iter().take(10) { // Cap at 10 most recent
                                        memory_context_text.push_str(&format!("• {}\n", entry.content));
                                    }
                                    memory_context_text.push('\n');
                                    debug!("[Daemon] Injected {} Procedural skills into memory context", procedural_entries.len());
                                }
                            }
                        }
                        // ----------------------------------------

                        {
                            let mut w = active_tasks_spawn.write().await;
                            w.insert(
                                trace_id.to_string(),
                                telos_hci::ActiveTaskInfo {
                                    task_id: trace_id.to_string(),
                                    task_name: trace_id.to_string(),
                                    progress: telos_hci::ProgressInfo::new(0, 1, 1, 0, 0, None),
                                    running_nodes: vec!["router".to_string()],
                                    started_at_ms: current_ms,
                                }
                            );
                        }

                        // --- CONTEXTUAL BYPASS ---
                        let is_resume = paused_tasks_bg
                            .lock()
                            .await
                            .contains_key(&trace_id.to_string());

                        if is_resume {
                            debug!("[Daemon] Contextual Bypass: Task {} is active, injecting Architect for replan.", trace_id);
                            let _original_payload =
                                paused_tasks_bg.lock().await.remove(&trace_id.to_string());

                            broker_bg.publish_feedback(AgentFeedback::StateChanged {
                                task_id: trace_id.to_string(),
                                current_node: "replan_architect".into(),
                                status: telos_core::NodeStatus::Running,
                            });

                            let mut graph = TaskGraph::new(trace_id.to_string());
                            graph.conversation_history = conversation_history_vec.clone();
                            graph.add_node_with_metadata(
                                "replan_architect".to_string(),
                                Box::new(agents::architect::ArchitectAgent::new(
                                    gateway_clone.clone(),
                                )),
                                NodeMetadata {
                                    task_type: "architect".to_string(),
                                    prompt_preview: truncate_for_preview(&payload, 100),
                                    full_task: payload.clone(),
                                    tool_name: None,
                                    schema_payload: None,
                                },
                            );

                            graph.current_state = GraphState {
                                is_running: true,
                                completed: false,
                            };

                            let scoped_ctx = telos_context::ScopedContext {
                                budget_tokens: 128_000,
                                summary_tree: vec![],
                                precise_facts: vec![],
                            };

                            execution_engine
                                .run_graph(
                                    &mut graph,
                                    &scoped_ctx,
                                    registry_clone.as_ref(),
                                    broker_bg.as_ref(),
                                )
                                .await;

                            let mut completed_nodes = 0;
                            let mut failed_nodes = 0;
                            let mut failed_node_ids = Vec::new();
                            for (id, status) in &graph.node_statuses {
                                if *status == telos_core::NodeStatus::Completed {
                                    completed_nodes += 1;
                                } else if *status == telos_core::NodeStatus::Failed {
                                    failed_nodes += 1;
                                    failed_node_ids.push(id.clone());
                                }
                            }
                            let task_success = failed_nodes == 0;
                            let summary = if task_success {
                                "Task Replan Completed".to_string()
                            } else {
                                "Task Replan Failed".to_string()
                            };

                            let combined_result = graph
                                .node_results
                                .iter()
                                .filter_map(|(id, out)| {
                                    out.output.as_ref().map(|v| format!("{}:\n{}", id, v))
                                })
                                .collect::<Vec<_>>()
                                .join("\n\n");

                            broker_bg.publish_feedback(AgentFeedback::Output {
                                task_id: trace_id.to_string(),
                                session_id: session_id.clone(),
                                content: format!("{}\n\n{}", summary, combined_result),
                                is_final: true,
                                silent: false,
                            });

                            broker_bg.publish_feedback(AgentFeedback::TaskCompleted {
                                task_id: trace_id.to_string(),
                                summary: TaskSummary {
                                    fulfilled: task_success,
                                    completed: true,
                                    total_nodes: graph.node_statuses.len(),
                                    completed_nodes,
                                    failed_nodes,
                                    total_time_ms: task_start_time.elapsed().as_millis() as u64,
                                    summary: summary.clone(),
                                    failed_node_ids,
                                },
                            });
                        } else {
                            // --- NORMAL PLANNING & EXECUTION ---
                            let mut enriched_payload = payload.clone();

                            if let Some(channel) = &system_mission_channel {
                                enriched_payload = format!("[SYSTEM MISSION]\nYou are executing an autonomous scheduled mission. Fulfill it and dispatch the final result back to `origin_channel` ({}). Keep logs of any failures for standard evolution.\n\nMission Context/Instruction:\n{}", channel, enriched_payload);
                            }

                            if let Some(pid) = &project_id {
                                debug!("[Daemon] Active Project ID: {}", pid);
                                if let Ok(Some(project)) =
                                    telos_project::manager::ProjectRegistry::new().get_project(pid)
                                {
                                    let working_dir = project.path.clone();
                                    debug!("[Daemon] Project working directory: {:?}", working_dir);

                                    // Load custom project instructions
                                    let project_config =
                                        telos_core::project::ProjectConfig::load(&working_dir);

                                    // Dynamically inject project context into the payload for the agent
                                    enriched_payload = format!(
                                        "Context:\n- Active Project: {}\n- Description: {}\n- Working Directory: {:?}\n- Custom Instructions: {}\n\nTask:\n{}",
                                        project.name,
                                        project.description.unwrap_or_else(|| "None".to_string()),
                                        working_dir,
                                        project_config.custom_instructions.unwrap_or_else(|| "None".to_string()),
                                        payload
                                    );
                                    
                                    debug!(
                                        "[Daemon] Dynamically injected project context into payload."
                                    );
                                }
                            }

                            // --- ACTIVE TASK INJECTION FOR ROUTER OMNISCIENCE ---
                            let active_tasks_snapshot = {
                                let w = active_tasks_spawn.read().await;
                                if w.is_empty() {
                                    String::new()
                                } else {
                                    let mut tasks_desc = Vec::new();
                                    for (id, state) in w.iter() {
                                        if id != &trace_id.to_string() {
                                            let nodes_str = if state.running_nodes.is_empty() { "Pending".to_string() } else { state.running_nodes.join(", ") };
                                            tasks_desc.push(format!("- Task ID [{}]: {}", id, nodes_str));
                                        }
                                    }
                                    if tasks_desc.is_empty() {
                                        String::new()
                                    } else {
                                        format!("[SYSTEM: Active Background Tasks]\nThe system is currently executing the following tasks in the background:\n{}\n\n", tasks_desc.join("\n"))
                                    }
                                }
                            };
                            
                            if !active_tasks_snapshot.is_empty() {
                                enriched_payload = format!("{}{}", active_tasks_snapshot, enriched_payload);
                            }

                            // --- DISCOVER CUSTOM TOOLS for Router awareness ---
                            let custom_tools_context = {
                                let guard = tool_registry.read().await;
                                let matching_tools = guard.discover_tools(&payload, 5);
                                // Filter out core/native tools — only show user-created custom tools
                                let core_names = ["file_edit", "fs_read", "fs_write", "shell_exec", "lsp_tool", "glob",
                                    "create_rhai_tool", "list_rhai_tools", "web_search", "web_scrape", "http",
                                    "schedule_mission", "list_scheduled_missions", "cancel_mission"];
                                let custom: Vec<_> = matching_tools.iter()
                                    .filter(|t| !core_names.contains(&t.name.as_str()))
                                    .collect();
                                if custom.is_empty() {
                                    String::new()
                                } else {
                                    let tool_list: Vec<String> = custom.iter()
                                        .map(|t| format!("- {} : {}", t.name, t.description))
                                        .collect();
                                    // Detect if user explicitly wants to CREATE/MODIFY a tool
                                    let payload_lower = payload.to_lowercase();
                                    let wants_creation = payload_lower.contains("创建") || payload_lower.contains("制作")
                                        || payload_lower.contains("做一个") || payload_lower.contains("写一个工具")
                                        || payload_lower.contains("create") || payload_lower.contains("build a tool")
                                        || payload_lower.contains("make a tool") || payload_lower.contains("修改工具")
                                        || payload_lower.contains("更新工具") || payload_lower.contains("update tool")
                                        || payload_lower.contains("改写") || payload_lower.contains("换成");
                                    let routing_hint = if wants_creation {
                                        "IMPORTANT: The user explicitly wants to CREATE or MODIFY a tool. Even though a matching tool already exists, you MUST route to \"general_expert\" so it can use create_rhai_tool to create/overwrite the tool. DO NOT skip tool creation just because a tool with that name already exists."
                                    } else {
                                        "If a relevant custom tool exists, prefer routing to \"general_expert\" so it can use the tool directly instead of searching."
                                    };
                                    format!(
                                        "[AVAILABLE CUSTOM TOOLS]\nThe following previously-created tools are available and match this query:\n{}\n{}\n\n",
                                        tool_list.join("\n"),
                                        routing_hint,
                                    )
                                }
                            };

                            // --- TIER 1: ROUTER AGENT DISPATCH ---
                            broker_bg.publish_feedback(AgentFeedback::StateChanged {
                                task_id: trace_id.to_string(),
                                current_node: "routing".into(),
                                status: telos_core::NodeStatus::Running,
                            });

                            let router = agents::router::RouterAgent::new(gateway_clone.clone(), config.router_persona_name.clone(), config.router_persona_trait.clone(), tool_registry.clone());
                            let combined_memory_context = {
                                let mut ctx = String::new();
                                if !custom_tools_context.is_empty() {
                                    ctx.push_str(&custom_tools_context);
                                }
                                if !memory_context_text.is_empty() {
                                    ctx.push_str(&memory_context_text);
                                }
                                if ctx.is_empty() { None } else { Some(ctx) }
                            };
                            let mut router_input = telos_core::AgentInput {
                                node_id: "router_main".to_string(),
                                task: enriched_payload.clone(),
                                dependencies: Default::default(),
                                schema_payload: None,
                                conversation_history: conversation_history_vec.clone(),
                                memory_context: combined_memory_context,
                                correction: None,
                            };

                            let mut route_result = telos_core::AgentOutput::failure("Init", "Not started");
                            let mut tool_used = false;

                           // --- ROUTER REACT LOOP for TOOL ACCESS ---
                            let mut tool_attempts_used = 0u32;
                            let mut memory_write_dedup: std::collections::HashSet<String> = std::collections::HashSet::new();
                            let mut memory_read_dedup: std::collections::HashSet<String> = std::collections::HashSet::new();
                            for attempt in 0..3 {
                                route_result = router.execute(router_input.clone(), registry_clone.as_ref()).await;
                                
                                if !route_result.success {
                                    break;
                                }

                                if let Some(route_data) = route_result.output.as_ref() {
                                    // Intercept "tool"
                                    if let Some(tool_name) = route_data.get("tool").and_then(|v| v.as_str()) {
                                        if tool_name == "memory_read" {
                                            tool_attempts_used = attempt + 1;
                                            let query_val = route_data.get("query").and_then(|v| v.as_str()).unwrap_or("");
                                            
                                            // Dedup: skip if identical query was already made
                                            if memory_read_dedup.contains(query_val) {
                                                debug!("[Daemon] Skipping duplicate memory_read for query: {}", query_val);
                                                let existing_mem = router_input.memory_context.unwrap_or_default();
                                                router_input.memory_context = Some(format!("{}[TOOL RESULT: memory_read]\nYou already searched for '{}'. The results are listed above. Do NOT search for this again! Review the retrieved data and make a final direct_reply or route decision.\n\n", existing_mem, query_val));
                                                tool_used = true;
                                                continue;
                                            }
                                            memory_read_dedup.insert(query_val.to_string());
                                            
                                            debug!("[Daemon] Router triggered memory_read tool for query: {}", query_val);
                                            
                                            // Provide feedback to UI that we are searching memory
                                            broker_bg.publish_feedback(AgentFeedback::Output {
                                                task_id: trace_id.to_string(),
                                                session_id: session_id.clone(),
                                                content: format!("*(Router is recalling memory for: {})*", query_val),
                                                is_final: false,
                                                silent: false,
                                            });

                                            let mut memory_findings = String::new();
                                            {
                                                let mem_os = registry_clone.memory_os.clone();
                                                // Dual-strategy query: EntityLookup (keyword) + TimeRange (recency)
                                                let mut all_entries: Vec<(u64, String)> = Vec::new(); // (created_at, content)
                                                
                                                // Strategy 1: Keyword-based EntityLookup
                                                if let Ok(results) = mem_os.retrieve(telos_memory::MemoryQuery::EntityLookup { entity: query_val.to_string() }).await {
                                                    for e in results.iter().filter(|e| e.memory_type == telos_memory::MemoryType::Semantic || e.memory_type == telos_memory::MemoryType::InteractionEvent) {
                                                        all_entries.push((e.created_at, e.content.clone()));
                                                    }
                                                }
                                                
                                                // Strategy 2: Recent interactions (last 1 hour)
                                                let one_hour_ago = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                                                let one_hour_ago = one_hour_ago.saturating_sub(3600_000);
                                                if let Ok(results) = mem_os.retrieve(telos_memory::MemoryQuery::TimeRange { start: one_hour_ago, end: u64::MAX }).await {
                                                    for e in results.iter().filter(|e| e.memory_type == telos_memory::MemoryType::InteractionEvent) {
                                                        if !all_entries.iter().any(|(_, c)| c == &e.content) {
                                                            all_entries.push((e.created_at, e.content.clone()));
                                                        }
                                                    }
                                                }
                                                
                                                // Sort chronologically (oldest first) and format with ordinals
                                                all_entries.sort_by_key(|(ts, _)| *ts);
                                                
                                                if !all_entries.is_empty() {
                                                    let formatted: Vec<String> = all_entries.iter().enumerate()
                                                        .map(|(i, (_, content))| format!("#{}: {}", i + 1, content))
                                                        .collect();
                                                    let merged = formatted.join("\n");
                                                    let truncated: String = if merged.chars().count() > 2000 { merged.chars().take(2000).collect::<String>() + "..." } else { merged };
                                                    memory_findings = format!("[TOOL RESULT: memory_read] STATUS=SUCCESS\nFound relevant memories (chronological order, #1 = earliest):\n{}\n\n", truncated);
                                                } else {
                                                    memory_findings = format!("[TOOL RESULT: memory_read] STATUS=SUCCESS\nNo results for '{}'.\n\n", query_val);
                                                }
                                            }

                                            // Append findings back to Router memory context and loop
                                            let existing_mem = router_input.memory_context.unwrap_or_default();
                                            router_input.memory_context = Some(format!("{}{}", existing_mem, memory_findings));
                                            tool_used = true;
                                            continue; 
                                        }

                                        // Intercept memory_write tool
                                        if tool_name == "memory_write" {
                                            tool_attempts_used = attempt + 1;
                                            let content_val = route_data.get("content").and_then(|v| v.as_str()).unwrap_or("");

                                            // Dedup: skip if same content already written in this conversation turn
                                            if memory_write_dedup.contains(content_val) {
                                                debug!("[Daemon] Skipping duplicate memory_write for content: {}", content_val);
                                                let existing_mem = router_input.memory_context.unwrap_or_default();
                                                router_input.memory_context = Some(format!("{}[TOOL RESULT: memory_write]\nThis information has already been stored. No need to write again. Proceed to confirm to the user.\n\n", existing_mem));
                                                tool_used = true;
                                                continue;
                                            }
                                            memory_write_dedup.insert(content_val.to_string());
                                            debug!("[Daemon] Router triggered memory_write tool for content: {}", content_val);

                                            broker_bg.publish_feedback(AgentFeedback::Output {
                                                task_id: trace_id.to_string(),
                                                session_id: session_id.clone(),
                                                content: format!("*(正在记录您的信息: {})*", content_val),
                                                is_final: false,
                                                silent: false,
                                            });

                                            let mut write_result = String::new();
                                            if !content_val.is_empty() {
                                                let mem_os = registry_clone.memory_os.clone();
                                                let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                                                let entry = telos_memory::MemoryEntry::new(
                                                    format!("profile_explicit_{}", timestamp),
                                                    telos_memory::MemoryType::UserProfileStatic,
                                                    content_val.to_string(),
                                                    timestamp,
                                                    None,
                                                );
                                                match mem_os.store(entry).await {
                                                    Ok(_) => {
                                                        debug!("[Daemon] Successfully stored user fact via memory_write: {}", content_val);
                                                        write_result = format!("[TOOL RESULT: memory_write] STATUS=SUCCESS\nStored: \"{}\"\n\n", content_val);
                                                    }
                                                    Err(e) => {
                                                        error!("[Daemon] Failed to store via memory_write: {:?}", e);
                                                        write_result = format!("[TOOL RESULT: memory_write] STATUS=FAILURE\nError: {:?}\n\n", e);
                                                    }
                                                }
                                            } else {
                                                write_result = format!("[TOOL RESULT: memory_write] STATUS=FAILURE\nError: empty content\n\n");
                                            }

                                            let existing_mem = router_input.memory_context.unwrap_or_default();
                                            router_input.memory_context = Some(format!("{}{}", existing_mem, write_result));
                                            tool_used = true;
                                            continue;
                                        }
                                    }
                                    
                                    // If we reach here, it's either "direct_reply" or "route", routing is finished
                                    break;
                                }
                            }
                            
                            // --- GRACEFUL DEGRADATION: Final synthesis pass ---
                            // If the loop exhausted all 3 tool attempts without converging on
                            // a direct_reply or route decision, give the Router ONE final chance
                            // to synthesize an answer from whatever it gathered, or escalate.
                            if tool_attempts_used >= 3 && route_result.success {
                                if let Some(route_data) = route_result.output.as_ref() {
                                    if route_data.get("tool").is_some() && route_data.get("direct_reply").is_none() && route_data.get("route").is_none() {
                                        debug!("[Daemon] Router exhausted all tool attempts without converging. Triggering final synthesis pass.");
                                        let existing_mem = router_input.memory_context.unwrap_or_default();
                                        let system_note = if !memory_write_dedup.is_empty() {
                                            "[SYSTEM: All memory operations completed. Produce a direct_reply now.]"
                                        } else {
                                            "[SYSTEM NOTE: You have used all your tool attempts. You MUST now make a final decision: either provide a direct_reply with your best answer based on whatever information you found, or route to an appropriate expert. Do NOT request any more tools.]"
                                        };
                                        router_input.memory_context = Some(format!(
                                            "{}{}\n\n",
                                            existing_mem, system_note
                                        ));
                                        route_result = router.execute(router_input.clone(), registry_clone.as_ref()).await;
                                    }
                                }
                            }
                            
                            // --- MEMORY-WRITE SHORT-CIRCUIT ---
                            // If memory was written during this router turn but the router didn't
                            // produce a direct_reply (it routed to an expert instead), short-circuit:
                            // The expert would just fail trying to "plan" a confirmation message anyway.
                            if !memory_write_dedup.is_empty() && route_result.success {
                                if let Some(ref rd) = route_result.output {
                                    let has_route = rd.get("route").is_some();
                                    let has_direct = rd.get("direct_reply").is_some();
                                    
                                    if has_route || has_direct {
                                        debug!("[Daemon] Memory-write short-circuit: memory was written but router dispatched to expert. Generating direct confirmation.");
                                        let facts_stored: Vec<&String> = memory_write_dedup.iter().collect();
                                        let soul_content = crate::agents::prompt_builder::get_soul();
                                        let confirm_prompt = format!(
                                            "[IDENTITY & VALUES]\n{}\n\nYou just stored the following facts in memory: {:?}.\n\
                                            Generate a brief, warm confirmation to the user that you've remembered this information.\n\
                                            Be natural and conversational. Use the persona described above. Reply in the user's language.",
                                            soul_content, facts_stored
                                        );

                                        let confirm_req = telos_model_gateway::LlmRequest {
                                            session_id: format!("memory_confirm_{}", trace_id),
                                            messages: vec![
                                                telos_model_gateway::Message { role: "system".to_string(), content: confirm_prompt },
                                                telos_model_gateway::Message { role: "user".to_string(), content: payload.to_string() },
                                            ],
                                            required_capabilities: telos_model_gateway::Capability { requires_vision: false, strong_reasoning: false },
                                            budget_limit: 500,
                                            tools: None,
                                        };

                                        let confirm_text = match gateway_clone.generate(confirm_req).await {
                                            Ok(res) => res.content,
                                            Err(_) => format!("好的，我已经记住了：{}", facts_stored.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("、")),
                                        };

                                        // Emit as direct reply
                                        broker_bg.publish_feedback(AgentFeedback::Output {
                                            task_id: trace_id.to_string(),
                                            session_id: session_id.clone(),
                                            content: confirm_text.clone(),
                                            is_final: true,
                                            silent: false,
                                        });

                                        // Push to session logs
                                        {
                                            let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                                            {
                                                let mut state_w = session_logs_loop.write().await;
                                                state_w.logs.push_back(telos_context::LogEntry {
                                                    timestamp,
                                                    message: format!("Assistant: {}", confirm_text),
                                                });
                                            }

                                            let gw_for_compress = gateway_clone.clone();
                                            let logs_for_push = session_logs_loop.clone();
                                            let confirm_for_compress = confirm_text.clone();
                                            tokio::spawn(async move {
                                                let compressed = compress_for_session_log(&confirm_for_compress, &gw_for_compress).await;
                                                let mut state_w = logs_for_push.write().await;
                                                for entry in state_w.logs.iter_mut().rev() {
                                                    if entry.timestamp == timestamp {
                                                        entry.message = format!("Assistant: {}", compressed);
                                                        break;
                                                    }
                                                }
                                            });
                                        }

                                        // Persist interaction to long-term memory
                                        {
                                            let mem_os = registry_clone.memory_os.clone();
                                            let conversation = format!("[User]: {}\n[Assistant ({} Persona)]: {}", payload, config.router_persona_name, confirm_text);
                                            let conv_for_profile = conversation.clone();
                                            let current_time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
                                            let entry = telos_memory::MemoryEntry::new(
                                                uuid::Uuid::new_v4().to_string(),
                                                telos_memory::MemoryType::InteractionEvent,
                                                conversation,
                                                current_time,
                                                None,
                                            );
                                            if let Err(e) = mem_os.store(entry).await {
                                                error!("[Daemon] Failed to store memory-write confirmation interaction: {:?}", e);
                                            }
                                            let gw_for_profile = gateway_clone.clone();
                                            let mem_for_profile = registry_clone.memory_os.clone();
                                            tokio::spawn(async move {
                                                extract_and_store_user_profile(&conv_for_profile, gw_for_profile, mem_for_profile).await;
                                            });
                                        }

                                        broker_bg.publish_feedback(AgentFeedback::TaskCompleted {
                                            task_id: trace_id.to_string(),
                                            summary: TaskSummary {
                                                fulfilled: true,
                                                completed: true,
                                                total_nodes: 1,
                                                completed_nodes: 1,
                                                failed_nodes: 0,
                                                total_time_ms: task_start_time.elapsed().as_millis() as u64,
                                                summary: "Memory Write Confirmed (Short-Circuit)".to_string(),
                                                failed_node_ids: vec![],
                                            },
                                        });

                                        {
                                            let mut w = active_tasks_spawn.write().await;
                                            w.remove(&trace_id.to_string());
                                        }
                                        return;
                                    }
                                }
                            }

                            if !route_result.success {
                                let error_msg = route_result.error.map(|e| e.message).unwrap_or_else(|| "Unknown routing error".to_string());
                                // P2: Detect content filter and provide user-friendly message
                                let user_msg = if error_msg.to_lowercase().contains("contentfilter")
                                    || error_msg.to_lowercase().contains("content_filter")
                                    || error_msg.to_lowercase().contains("content filter")
                                    || error_msg.contains("ResponsibleAIPolicyViolation") {
                                    "抱歉，这个话题的讨论可能涉及敏感内容，模型端对此有一定限制。我可以尝试从不同角度来回答这个问题，或者请您换一种方式提问。".to_string()
                                } else {
                                    format!("Routing Failed: {}", error_msg)
                                };
                                broker_bg.publish_feedback(AgentFeedback::Output {
                                    task_id: trace_id.to_string(),
                                    session_id: session_id.clone(),
                                    content: user_msg,
                                    is_final: true,
                                    silent: false,
                                });
                                broker_bg.publish_feedback(AgentFeedback::TaskCompleted {
                                    task_id: trace_id.to_string(),
                                    summary: TaskSummary {
                                        fulfilled: false,
                                        completed: true,
                                        total_nodes: 1,
                                        completed_nodes: 0,
                                        failed_nodes: 1,
                                        total_time_ms: task_start_time.elapsed().as_millis() as u64,
                                        summary: "Router Task Failed".to_string(),
                                        failed_node_ids: vec!["router_main".to_string()],
                                    },
                                });
                                // Remove from active tasks before return
                                {
                                    let mut w = active_tasks_spawn.write().await;
                                    w.remove(&trace_id.clone().to_string());
                                }
                                return;
                            }

                            // Parse router output
                            let route_data = route_result.output.unwrap_or_default();
                            
                            // --- DIRECT REPLY SHORT-CIRCUIT (with QA Gate) ---
                            if let Some(direct_reply) = route_data.get("direct_reply").and_then(|v| v.as_str()) {
                                debug!("[Daemon] Router generated a direct reply. Running QA verification...");
                                // Record route decision metric: direct_reply
                                crate::core::metrics_store::record(crate::core::metrics_store::MetricEvent::RouteDecision {
                                    timestamp_ms: crate::core::metrics_store::now_ms(),
                                    task_id: trace_id.to_string(),
                                    route: "direct_reply".to_string(),
                                    reason: "Router chose direct reply".to_string(),
                                });
                                // QA Gate: evaluate direct_reply quality before accepting
                                let qa_result = router.evaluate(&payload, direct_reply, registry_clone.as_ref()).await;
                                let qa_accepted = if qa_result.success {
                                    let json = qa_result.output.as_ref();
                                    let is_acceptable = json
                                        .and_then(|j| j.get("is_acceptable").and_then(|v| v.as_bool()))
                                        .unwrap_or(true);
                                    let is_clarification = json
                                        .and_then(|j| j.get("is_clarification").and_then(|v| v.as_bool()))
                                        .unwrap_or(false);
                                    is_acceptable || is_clarification
                                } else {
                                    true // default accept if QA call itself fails
                                };

                                if qa_accepted {
                                    debug!("[Daemon] QA Gate approved direct reply.");
                                    broker_bg.publish_feedback(AgentFeedback::Output {
                                        task_id: trace_id.to_string(),
                                        session_id: session_id.clone(),
                                        content: direct_reply.to_string(),
                                        is_final: true,
                                        silent: false,
                                    });

                                    // --- PUSH ASSISTANT RESPONSE TO SESSION LOGS (two-phase to avoid race conditions) ---
                                    {
                                        let reply_text = direct_reply.to_string();
                                        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                                        
                                        // Phase 1: Synchronously push the uncompressed (or placeholder) response
                                        // This ensures the VERY NEXT request sees exactly what the assistant said immediately
                                        {
                                            let mut state_w = session_logs_loop.write().await;
                                            state_w.logs.push_back(telos_context::LogEntry {
                                                timestamp,
                                                message: format!("Assistant: {}", reply_text),
                                            });
                                        }

                                        // Phase 2: Asynchronously compress older responses to preserve immediate context
                                        let gw_for_compress = gateway_clone.clone();
                                        let logs_for_push = session_logs_loop.clone();
                                        tokio::spawn(async move {
                                            let target_info = {
                                                let state_r = logs_for_push.read().await;
                                                let len = state_r.logs.len();
                                                if len > 4 {
                                                    let mut found = None;
                                                    for i in 0..(len - 4) {
                                                        let msg = &state_r.logs[i].message;
                                                        if msg.starts_with("Assistant: ") && !msg.starts_with("Assistant: [摘要]") && msg.len() > 500 {
                                                            found = Some((state_r.logs[i].timestamp, msg.clone()));
                                                            break;
                                                        }
                                                    }
                                                    found
                                                } else {
                                                    None
                                                }
                                            };

                                            if let Some((target_ts, target_msg)) = target_info {
                                                let text_to_compress = target_msg.strip_prefix("Assistant: ").unwrap_or(&target_msg);
                                                let compressed = compress_for_session_log(text_to_compress, &gw_for_compress).await;
                                                
                                                let mut state_w = logs_for_push.write().await;
                                                for entry in state_w.logs.iter_mut() {
                                                    if entry.timestamp == target_ts {
                                                        entry.message = format!("Assistant: {}", compressed);
                                                        break;
                                                    }
                                                }
                                            }
                                        });
                                    }

                                    broker_bg.publish_feedback(AgentFeedback::TaskCompleted {
                                        task_id: trace_id.to_string(),
                                        summary: TaskSummary {
                                            fulfilled: true,
                                            completed: true,
                                            total_nodes: 1,
                                            completed_nodes: 1,
                                            failed_nodes: 0,
                                            total_time_ms: task_start_time.elapsed().as_millis() as u64,
                                            summary: "Direct Router Reply".to_string(),
                                            failed_node_ids: vec![],
                                        },
                                    });

                                    // Persist to long-term memory
                                    {
                                        let mem_os = registry_clone.memory_os.clone();
                                        let conversation = format!("[User]: {}\n[Assistant ({} Persona)]: {}", payload, config.router_persona_name, direct_reply);
                                        let conv_for_profile = conversation.clone();
                                        let current_time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
                                        let entry = telos_memory::MemoryEntry::new(
                                            uuid::Uuid::new_v4().to_string(),
                                            telos_memory::MemoryType::InteractionEvent,
                                            conversation,
                                            current_time,
                                            None,
                                        );
                                        if let Err(e) = mem_os.store(entry).await {
                                            error!("[Daemon] Failed to store direct reply interaction memory: {:?}", e);
                                        }
                                        // Background: Extract user preferences from this conversation
                                        let gw_for_profile = gateway_clone.clone();
                                        let mem_for_profile = registry_clone.memory_os.clone();
                                        tokio::spawn(async move {
                                            extract_and_store_user_profile(&conv_for_profile, gw_for_profile, mem_for_profile).await;
                                        });
                                    }

                                    // Remove from active tasks
                                    {
                                        let mut w = active_tasks_spawn.write().await;
                                        w.remove(&trace_id.to_string());
                                    }
                                    return; // QA passed, skip DAG
                                } else {
                                    // QA rejected direct_reply — fallthrough to Expert routing
                                    let critique = qa_result.output.as_ref()
                                        .and_then(|json| json.get("critique").and_then(|v| v.as_str()))
                                        .unwrap_or("Direct reply did not adequately answer the user's question.");
                                    debug!("[Daemon] QA Gate REJECTED direct reply. Falling through to Expert routing. Critique: {}", critique);
                                    broker_bg.publish_feedback(AgentFeedback::Output {
                                        task_id: trace_id.to_string(),
                                        session_id: session_id.clone(),
                                        content: format!("🔄 Direct reply rejected by QA. Routing to expert. Critique: {}", critique),
                                        is_final: false,
                                        silent: false,
                                    });
                                    // Don't return — fall through to expert routing below
                                }
                            }

                            let expert_route = route_data.get("route").and_then(|v| v.as_str()).unwrap_or("general_expert");
                            let route_reason = route_data.get("reason").and_then(|v| v.as_str()).unwrap_or("Fallback to general expert.");
                            let enriched_task = route_data.get("enriched_task").and_then(|v| v.as_str()).unwrap_or(&enriched_payload);
                            enriched_payload = enriched_task.to_string();

                            // Record route decision metric: expert route
                            crate::core::metrics_store::record(crate::core::metrics_store::MetricEvent::RouteDecision {
                                timestamp_ms: crate::core::metrics_store::now_ms(),
                                task_id: trace_id.to_string(),
                                route: expert_route.to_string(),
                                reason: route_reason.to_string(),
                            });

                            broker_bg.publish_feedback(AgentFeedback::Output {
                                task_id: trace_id.to_string(),
                                session_id: session_id.clone(),
                                content: format!("Router Decision: Dispatching to `{}`. Reason: {}", expert_route, route_reason),
                                is_final: false, // Not final, we are just starting
                                silent: false,
                            });

                            // Build context using the context manager with memory integration for the Expert
                            let session_history: Vec<telos_context::LogEntry> = {
                                let state = session_logs_loop.read().await;
                                state.logs.iter().cloned().collect()
                            };
                            let raw_ctx = telos_context::RawContext {
                                history_logs: session_history.clone(),
                                retrieved_docs: vec![],
                            };
                            let ctx_req = telos_context::NodeRequirement {
                                required_tokens: 2000,
                                query: enriched_payload.clone(),
                            };
                            let ctx_start = std::time::Instant::now();
                            let actual_ctx = context_manager_spawn.compress_for_node(&raw_ctx, &ctx_req).await
                            .unwrap_or_else(|e| {
                                debug!("[Daemon] Context compression failed: {:?}, using empty context", e);
                                telos_context::ScopedContext {
                                    budget_tokens: 2000,
                                    summary_tree: vec![],
                                    precise_facts: vec![],
                                }
                            });
                            let ctx_elapsed = ctx_start.elapsed().as_millis() as u64;
                            crate::core::metrics_store::record(crate::core::metrics_store::MetricEvent::ContextCompression {
                                timestamp_ms: crate::core::metrics_store::now_ms(),
                                task_id: trace_id.to_string(),
                                elapsed_ms: ctx_elapsed,
                                facts_count: actual_ctx.precise_facts.len(),
                                summary_count: actual_ctx.summary_tree.len(),
                            });

                            debug!(
                                "[Daemon] Context prepared with {} summary nodes and {} precise facts",
                                actual_ctx.summary_tree.len(),
                                actual_ctx.precise_facts.len()
                            );

                            // --- TIER 2: EXPERT AGENT PLANNING & DAG EXECUTION ---
                            let mut attempt = 0;
                            const MAX_ATTEMPTS: usize = 3;
                            let mut loop_final_response = String::new();
                            let mut loop_qa_accepted = false;
                            let mut clarification_sent = false;
                            let mut loop_completed_nodes = 0;
                            let mut loop_failed_nodes = 0;
                            let mut loop_failed_node_ids = vec![];
                            let mut loop_summary = String::new();
                            let mut total_time_ms = 0;
                            let mut loop_final_trace_steps = Vec::new();
                            let mut loop_final_sub_graph = None;

                            while attempt < MAX_ATTEMPTS {
                                attempt += 1;
                                debug!("[Daemon] Starting execution attempt {}/{}", attempt, MAX_ATTEMPTS);

                                let mut graph = TaskGraph::new(trace_id.to_string());
                                graph.conversation_history = conversation_history_vec.clone();
                                let mut terminal_nodes = vec![];

                                // Instantiate the specific expert dynamically.
                                let expert_node: Box<dyn ExecutableNode> = match expert_route {
                                    "software_expert" => Box::new(agents::architect::ArchitectAgent::new(gateway_clone.clone())) as Box<dyn ExecutableNode>,
                                    "research_expert" => Box::new(agents::researcher::DeepResearchAgent::new(gateway_clone.clone(), tool_registry.clone())) as Box<dyn ExecutableNode>,
                                    "qa_expert" => Box::new(agents::tester::TestingAgent::new(gateway_clone.clone())) as Box<dyn ExecutableNode>,
                                    _ => Box::new(agents::general::GeneralAgent::new(
                                        gateway_clone.clone(),
                                        tool_registry.clone(),
                                        config.tools_dir.clone(),
                                    )) as Box<dyn ExecutableNode>,
                                };

                                graph.add_node_with_metadata(
                                    "expert_execution".to_string(),
                                    expert_node,
                                    NodeMetadata {
                                        task_type: expert_route.to_string(),
                                        prompt_preview: truncate_for_preview(&enriched_payload, 100),
                                        full_task: enriched_payload.clone(),
                                        tool_name: None,
                                        schema_payload: route_data.get("auto_discovered_tools").map(|v| v.to_string()),
                                    },
                                );
                                graph.current_state = GraphState {
                                    is_running: true,
                                    completed: false,
                                };
                                terminal_nodes.push("expert_execution".to_string());


                                // Build context using the context manager with memory integration
                                let session_history: Vec<telos_context::LogEntry> = {
                                    let state = session_logs_loop.read().await;
                                    state.logs.iter().cloned().collect()
                                };
                                let raw_ctx = telos_context::RawContext {
                                    history_logs: session_history,
                                    retrieved_docs: vec![],
                                };
                                let ctx_req = telos_context::NodeRequirement {
                                    required_tokens: 2000,
                                    query: enriched_payload.clone(),
                                };
                                let ctx_start2 = std::time::Instant::now();
                                let actual_ctx = context_manager_spawn.compress_for_node(&raw_ctx, &ctx_req).await
                                .unwrap_or_else(|e| {
                                    debug!("[Daemon] Context compression failed: {:?}, using empty context", e);
                                    telos_context::ScopedContext {
                                        budget_tokens: 1000,
                                        summary_tree: vec![],
                                        precise_facts: vec![],
                                    }
                                });
                                let ctx_elapsed2 = ctx_start2.elapsed().as_millis() as u64;
                                crate::core::metrics_store::record(crate::core::metrics_store::MetricEvent::ContextCompression {
                                    timestamp_ms: crate::core::metrics_store::now_ms(),
                                    task_id: trace_id.to_string(),
                                    elapsed_ms: ctx_elapsed2,
                                    facts_count: actual_ctx.precise_facts.len(),
                                    summary_count: actual_ctx.summary_tree.len(),
                                });

                                debug!(
                                    "[Daemon] Context prepared with {} summary nodes and {} precise facts",
                                    actual_ctx.summary_tree.len(),
                                    actual_ctx.precise_facts.len()
                                );

                                let graph_result = tokio::time::timeout(
                                    std::time::Duration::from_secs(180),
                                    execution_engine.run_graph(
                                        &mut graph,
                                        &actual_ctx,
                                        registry_clone.as_ref(),
                                        broker_bg.as_ref(),
                                    )
                                ).await;

                                if graph_result.is_err() {
                                    tracing::warn!("[Daemon] DAG execution timed out after 180s (single attempt {}/{})", attempt, MAX_ATTEMPTS);
                                    broker_bg.publish_feedback(AgentFeedback::Output {
                                        task_id: trace_id.to_string(),
                                        session_id: session_id.clone(),
                                        content: format!("⚠️ Execution attempt {}/{} timed out after 180s", attempt, MAX_ATTEMPTS),
                                        is_final: false,
                                        silent: false,
                                    });
                                    loop_failed_nodes += 1;
                                    loop_summary = format!("DAG execution timed out on attempt {}", attempt);
                                    break;
                                }

                                // Calculate task summary
                                total_time_ms = task_start_time.elapsed().as_millis() as u64;
                                let mut completed_nodes = 0;
                                let mut failed_nodes = 0;
                                let mut failed_node_ids: Vec<String> = Vec::new();

                                for (node_id, status) in &graph.node_statuses {
                                    if *status == telos_core::NodeStatus::Completed {
                                        completed_nodes += 1;
                                    } else if *status == telos_core::NodeStatus::Failed {
                                        failed_nodes += 1;
                                        failed_node_ids.push(node_id.clone());
                                    }
                                }

                                // Fetch the results from the terminal nodes dynamically
                                let mut final_results = Vec::new();
                                use petgraph::Direction;
                                
                                for (node_id, &node_idx) in &graph.node_indices {
                                    // A terminal node has no outgoing edges
                                    if graph.edges.neighbors_directed(node_idx, Direction::Outgoing).count() == 0 {
                                        // Ignore dummy nodes whose output is just "Research plan generated" if they spawned a subgraph
                                        if let Some(res) = graph.node_results.get(node_id) {
                                            let output_str = res
                                                .output
                                                .as_ref()
                                                .map(|v| v.to_string())
                                                .unwrap_or_else(|| "No output".to_string());
                                                
                                            if !output_str.contains("Research plan generated")
                                                && !output_str.contains("execution stub")
                                                && !output_str.contains("SubGraph decomposition complete")
                                                && !output_str.contains("Plan generated")
                                            {
                                                if res.success {
                                                    final_results.push(format!("[{}] {}", node_id, output_str));
                                                } else {
                                                    let error_str = res
                                                        .error
                                                        .as_ref()
                                                        .map(|e| {
                                                            let mut s = format!("{}: {}", e.error_type, e.message);
                                                            if let Some(ref td) = e.technical_detail {
                                                                s.push_str(&format!("\nDetail: {}", td));
                                                            }
                                                            s
                                                        })
                                                        .unwrap_or_else(|| "Unknown error".to_string());
                                                    final_results
                                                        .push(format!("[{}] Failed: {}", node_id, error_str));
                                                }
                                            }
                                        }
                                    }
                                }

                                let combined_result = if final_results.is_empty() {
                                    "No result generated by graph".to_string()
                                } else {
                                    final_results.join("\n")
                                };

                                let task_success = failed_nodes == 0;

                                // Build summary message
                                let summary = if task_success {
                                    format!(
                                        "Task completed successfully. {} node(s) executed in {:.1}s.",
                                        completed_nodes,
                                        total_time_ms as f64 / 1000.0
                                    )
                                } else {
                                    format!(
                                        "Task finished with errors. {} succeeded, {} failed. Node(s) failed: {}",
                                        completed_nodes,
                                        failed_nodes,
                                        failed_node_ids.join(", ")
                                    )
                                };

                                // --- ROUTE AGENT SYNTHESIS ---
                                // Delegate the final summary to the ExpertAgent that planned the execution
                                let expert_agent_for_summary: Box<dyn telos_core::agent_traits::ExpertAgent> = match expert_route {
                                    "software_expert" => Box::new(agents::architect::ArchitectAgent::new(gateway_clone.clone())) as Box<dyn telos_core::agent_traits::ExpertAgent>,
                                    "research_expert" => Box::new(agents::researcher::DeepResearchAgent::new(gateway_clone.clone(), tool_registry.clone())) as Box<dyn telos_core::agent_traits::ExpertAgent>,
                                    "qa_expert" => Box::new(agents::tester::TestingAgent::new(gateway_clone.clone())) as Box<dyn telos_core::agent_traits::ExpertAgent>,
                                    _ => Box::new(agents::general::GeneralAgent::new(
                                        gateway_clone.clone(),
                                        tool_registry.clone(),
                                        config.tools_dir.clone(),
                                    )) as Box<dyn telos_core::agent_traits::ExpertAgent>,
                                };

                                let summary_input = telos_core::AgentInput {
                                    node_id: "expert_summary".to_string(),
                                    task: payload.to_string(),
                                    dependencies: {
                                        let mut deps = std::collections::HashMap::new();
                                        deps.insert("dag_results".to_string(), telos_core::AgentOutput::success(
                                            serde_json::json!({"text": combined_result})
                                        ));
                                        deps
                                    },
                                    schema_payload: None,
                                    conversation_history: router_input.conversation_history.clone(),
                                    memory_context: router_input.memory_context.clone(),
                                    correction: None,
                                };

                                let summary_output = expert_agent_for_summary
                                    .summarize(&summary_input, registry_clone.as_ref())
                                    .await;

                                let final_response = if summary_output.success {
                                    summary_output
                                        .output
                                        .as_ref()
                                        .and_then(|json| json.get("text").and_then(|t| t.as_str()))
                                        .unwrap_or("No summary provided by expert.")
                                        .to_string()
                                } else {
                                    format!(
                                        "{}\n\n(Note: Failed to generate final summary: {:?})",
                                        combined_result, summary_output.error
                                    )
                                };

                                loop_final_response = final_response.clone();

                                loop_completed_nodes = completed_nodes;
                                loop_failed_nodes = failed_nodes;
                                loop_failed_node_ids = failed_node_ids.clone();
                                loop_summary = summary.clone();

                                loop_final_trace_steps.clear();
                                for (node_id, output) in &graph.node_results {
                                    let input_data = graph.node_metadata.get(node_id).map(|m| m.prompt_preview.clone()).unwrap_or_default();
                                    let error_opt = output.error.as_ref().map(|e| {
                                        let mut msg = e.message.clone();
                                        if let Some(ref td) = e.technical_detail {
                                            msg.push_str(&format!(" | Detail: {}", td));
                                        }
                                        telos_core::NodeError::ExecutionFailed(msg)
                                    });
                                    loop_final_trace_steps.push(telos_evolution::TraceStep {
                                        node_id: node_id.clone(),
                                        input_data,
                                        output_data: output.output.as_ref().map(|v| v.to_string()),
                                        error: error_opt,
                                    });
                                }
                                loop_final_sub_graph = Some(graph.to_subgraph());

                                // --- ROUTER EVALUATION ---
                                let eval_output = router.evaluate(&payload, &final_response, registry_clone.as_ref()).await;
                                if eval_output.success {
                                    if let Some(json) = eval_output.output {
                                        let is_acceptable = json.get("is_acceptable").and_then(|v| v.as_bool()).unwrap_or(false);
                                        let is_clarification = json.get("is_clarification").and_then(|v| v.as_bool()).unwrap_or(false);
                                        let critique = json.get("critique").and_then(|v| v.as_str()).unwrap_or("");
                                        
                                        if is_clarification {
                                            // Clarification is a valid response — send ClarificationNeeded and complete
                                            debug!("[Daemon] QA identified clarification response — delivering to user.");
                                            loop_qa_accepted = true;
                                            clarification_sent = true;
                                            
                                            // Parse options from the response text — build structured options
                                            let options = parse_clarification_options(&final_response);
                                            broker_bg.publish_feedback(AgentFeedback::ClarificationNeeded {
                                                task_id: trace_id.to_string(),
                                                session_id: session_id.clone(),
                                                prompt: final_response.clone(),
                                                options,
                                            });
                                            break;
                                        } else if is_acceptable || attempt == MAX_ATTEMPTS {
                                            loop_qa_accepted = is_acceptable;
                                            if !is_acceptable {
                                                debug!("[Daemon] Max attempts reached despite router rejection. Proceeding anyway.");
                                            } else {
                                                debug!("[Daemon] Router evaluated result as acceptable.");
                                            }
                                            break;
                                        } else {
                                            debug!("[Daemon] Router rejected the output. Critique: {}", critique);
                                            broker_bg.publish_feedback(AgentFeedback::Output {
                                                task_id: trace_id.to_string(),
                                                session_id: session_id.clone(),
                                                content: format!("🔄 Router QA rejected output.\nCritique: {}", critique),
                                                is_final: false,
                                                silent: false,
                                            });
                                            enriched_payload = format!(
                                                "Task:\n{}\n\n[PERSONA CONTEXT]\n{}\n\n[SYSTEM DIRECTIVE — MANDATORY]\n\
                                                 Your previous attempt was REJECTED by the QA evaluator.\n\
                                                 Critique: {}\n\n\
                                                 You MUST autonomously retry with an improved strategy.\n\
                                                 DO NOT ask the user for permission, clarification, or confirmation.\n\
                                                 DO NOT say 'if you want me to continue' or similar phrases.\n\
                                                 Execute the corrected approach IMMEDIATELY and deliver the result.",
                                                payload, agents::prompt_builder::get_soul(), critique
                                            );
                                        }
                                    } else {
                                        break;
                                    }
                                } else {
                                    debug!("[Daemon] Router evaluation failed, continuing with generated output.");
                                    break;
                                }
                            }

                            // Send Output ONLY if we didn't already send ClarificationNeeded
                            if !clarification_sent {
                                broker_bg.publish_feedback(AgentFeedback::Output {
                                    task_id: trace_id.to_string(),
                                    session_id: session_id.clone(),
                                    content: loop_final_response.clone(),
                                    is_final: true,
                                    silent: false,
                                });
                            }

                            // --- PUSH ASSISTANT RESPONSE TO SESSION LOGS (two-phase to avoid race conditions) ---
                            {
                                let response_text = loop_final_response.clone();
                                let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;

                                // Phase 1: Synchronously push the uncompressed response
                                {
                                    let mut state_w = session_logs_loop.write().await;
                                    state_w.logs.push_back(telos_context::LogEntry {
                                        timestamp,
                                        message: format!("Assistant: {}", response_text),
                                    });
                                }

                                // Phase 2: Asynchronously compress older responses to preserve immediate context
                                let gw_for_compress = gateway_clone.clone();
                                let logs_for_push = session_logs_loop.clone();
                                tokio::spawn(async move {
                                    let target_info = {
                                        let state_r = logs_for_push.read().await;
                                        let len = state_r.logs.len();
                                        if len > 4 {
                                            let mut found = None;
                                            for i in 0..(len - 4) {
                                                let msg = &state_r.logs[i].message;
                                                if msg.starts_with("Assistant: ") && !msg.starts_with("Assistant: [摘要]") && msg.len() > 500 {
                                                    found = Some((state_r.logs[i].timestamp, msg.clone()));
                                                    break;
                                                }
                                            }
                                            found
                                        } else {
                                            None
                                        }
                                    };

                                    if let Some((target_ts, target_msg)) = target_info {
                                        let text_to_compress = target_msg.strip_prefix("Assistant: ").unwrap_or(&target_msg);
                                        let compressed = compress_for_session_log(text_to_compress, &gw_for_compress).await;
                                        
                                        let mut state_w = logs_for_push.write().await;
                                        for entry in state_w.logs.iter_mut() {
                                            if entry.timestamp == target_ts {
                                                entry.message = format!("Assistant: {}", compressed);
                                                break;
                                            }
                                        }
                                    }
                                });
                            }

                            // Publish TaskCompleted feedback LAST, which breaks the CLI stream
                            broker_bg.publish_feedback(AgentFeedback::TaskCompleted {
                                task_id: trace_id.to_string(),
                                summary: TaskSummary {
                                    fulfilled: loop_qa_accepted,
                                    completed: true,
                                    total_nodes: loop_completed_nodes + loop_failed_nodes,
                                    completed_nodes: loop_completed_nodes,
                                    failed_nodes: loop_failed_nodes,
                                    total_time_ms,
                                    summary: loop_summary,
                                    failed_node_ids: loop_failed_node_ids,
                                },
                            });
                            // Persist task result to metrics store
                            crate::core::metrics_store::record(crate::core::metrics_store::MetricEvent::TaskResult {
                                timestamp_ms: crate::core::metrics_store::now_ms(),
                                task_id: trace_id.to_string(),
                                fulfilled: loop_qa_accepted,
                                total_time_ms,
                            });
                            // --- INTERACTION EVENT PERSISTENCE (Global Long-Term Memory) ---
                            let interaction_content = format!("User: {}\nAssistant: {}", payload, loop_final_response);
                            let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
                            let interaction_entry = telos_memory::MemoryEntry::new(
                                format!("interaction_{}_{}", session_id, timestamp),
                                telos_memory::MemoryType::InteractionEvent,
                                interaction_content.clone(),
                                timestamp,
                                None,
                            );

                            let memory_clone = registry_clone.memory_os.clone();
                            let session_id_for_mem = session_id.clone();
                            tokio::spawn(async move {
                                if let Err(e) = memory_clone.store(interaction_entry).await {
                                    debug!("[Daemon] ⚠️ Failed to store InteractionEvent for session {}: {}", session_id_for_mem, e);
                                } else {
                                    debug!("[Daemon] 📥 Successfully archived global InteractionEvent.");
                                }
                            });

                            // --- USER PROFILE EXTRACTION (Background) ---
                            let gw_for_profile = gateway_clone.clone();
                            let mem_for_profile = registry_clone.memory_os.clone();
                            let conv_for_profile = interaction_content;
                            tokio::spawn(async move {
                                extract_and_store_user_profile(&conv_for_profile, gw_for_profile, mem_for_profile).await;
                            });

                            // --- EVOLUTION & MEMORY INTEGRATION LOOP ---
                            // Detect workflow reuse from architect's explicit adopted_templates declaration
                            let mut reused_wf_ids: Vec<String> = Vec::new();
                            for step in &loop_final_trace_steps {
                                if let Some(ref output_str) = step.output_data {
                                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(output_str) {
                                        // New: read adopted_templates (Architect explicitly declares which templates it used)
                                        if let Some(adopted) = json.get("adopted_templates").and_then(|v| v.as_array()) {
                                            for template_desc in adopted {
                                                if let Some(desc) = template_desc.as_str() {
                                                    reused_wf_ids.push(desc.to_string());
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            // Fallback: check route_data for adopted_templates from software_expert
                            if reused_wf_ids.is_empty() && expert_route == "software_expert" {
                                if let Some(adopted) = route_data.get("adopted_templates").and_then(|v| v.as_array()) {
                                    for template_desc in adopted {
                                        if let Some(desc) = template_desc.as_str() {
                                            reused_wf_ids.push(desc.to_string());
                                        }
                                    }
                                }
                            }

                            let trace = telos_evolution::ExecutionTrace {
                                task_id: trace_id.to_string(),
                                steps: loop_final_trace_steps,
                                errors_encountered: vec![],
                                success: loop_qa_accepted,
                                sub_graph: loop_final_sub_graph,
                                reused_workflow_ids: reused_wf_ids,
                            };
                            
                                // Send trace to asynchronous Evolution worker for Skill Distillation
                                if let Err(e) = distillation_tx_spawn.send(trace) {
                                    debug!("[Daemon] ⚠️ Failed to send trace {} to evolution queue: {}", trace_id, e);
                                }
                            
                            // Cleanup task from registry
                            {
                                let mut w = active_tasks_spawn.write().await;
                                w.remove(&trace_id.clone().to_string());
                            }
                            }
                        });
                    }
                AgentEvent::UserIntervention {
                    task_id,
                    node_id,
                    instruction,
                    trace_id: _,
                } => {
                    debug!(
                        "[Daemon] UserIntervention for task {}: {}",
                        task_id, instruction
                    );
                    if let Some(node) = node_id {
                        let lock = wakeup_map_bg.lock().await;
                        if let Some(tx) = lock.get(&task_id) {
                            let _ = tx.send((task_id.clone(), node, instruction));
                        }
                    } else {
                        // Default to first waiting node if we can't be sure, but usually targeted.
                        debug!(
                            "[Daemon] Warning: Targeted intervention missing node_id for task {}",
                            task_id
                        );
                    }
                }
                AgentEvent::UserApproval {
                    task_id,
                    node_id: _,
                    approved,
                    supplement_info: _,
                    trace_id: _,
                } => {
                    let broker_bg = broker_bg.clone();
                    let gateway_clone = gateway_clone.clone();
                    let registry_clone = registry_clone.clone();
                    let paused_tasks_bg = paused_tasks_bg.clone();
                    let tool_registry = tool_registry.clone();
                    let config = loop_config.clone();

                    let node_factory = std::sync::Arc::new(DaemonNodeFactory {
                        gateway: gateway_clone.clone(),
                        tool_registry: tool_registry.clone(),
                        tools_dir: config.tools_dir.clone(),
                    });

                    let mut execution_engine = telos_dag::engine::TokioExecutionEngine::new()
                        .with_node_factory(node_factory)
                        .with_active_tasks(active_tasks_loop.clone())
                        .with_evaluator(evaluator_loop.clone());

                    let context_manager_approval = context_manager.clone();
                    let session_logs_loop = session_logs_loop.clone();
                    tokio::spawn(async move {
                        let task_start_time = Instant::now();
                        debug!(
                            "[Daemon] Received UserApproval for task {} (approved: {})",
                            task_id, approved
                        );

                        if !approved {
                            broker_bg.publish_feedback(AgentFeedback::Output {
                                task_id: task_id.clone(),
                                session_id: "default".into(),
                                content: "Task Rejected.".into(),
                                is_final: true,
                                silent: false,
                            });
                            paused_tasks_bg.lock().await.remove(&task_id);

                            // Publish TaskCompleted for rejected task
                            broker_bg.publish_feedback(AgentFeedback::TaskCompleted {
                                task_id: task_id.clone(),
                                summary: TaskSummary {
                                    fulfilled: false,
                                    completed: true,
                                    total_nodes: 0,
                                    completed_nodes: 0,
                                    failed_nodes: 0,
                                    total_time_ms: 0,
                                    summary: "Task was rejected by user".to_string(),
                                    failed_node_ids: vec![],
                                },
                            });
                            return;
                        }

                        // User approved. Retrieve the paused task payload and execute it.
                        let payload_opt = paused_tasks_bg.lock().await.remove(&task_id);
                        if let Some(payload) = payload_opt {
                            broker_bg.publish_feedback(AgentFeedback::Output {
                                task_id: task_id.clone(),
                                session_id: "default".into(),
                                content: "Task Approved. Executing...".into(),
                                is_final: false,
                                silent: false,
                            });

                            let mut graph = TaskGraph::new(task_id.clone());
                            graph.add_node_with_metadata(
                                "llm_node".to_string(),
                                Box::new(LlmPromptNode {
                                    prompt: format!(
                                        "Execute the following elevated user command: {}",
                                        payload
                                    ),
                                    gateway: gateway_clone.clone(),
                                }),
                                NodeMetadata {
                                    task_type: "LLM".to_string(),
                                    prompt_preview: truncate_for_preview(&payload, 100),
                                    full_task: format!("Execute the following elevated user command: {}", payload),
                                    tool_name: None,
                                    schema_payload: None,
                                },
                            );
                            graph.current_state = GraphState {
                                is_running: true,
                                completed: false,
                            };

                            let session_history: Vec<telos_context::LogEntry> = {
                                let state = session_logs_loop.read().await;
                                state.logs.iter().cloned().collect()
                            };
                            let raw_ctx = telos_context::RawContext {
                                history_logs: session_history,
                                retrieved_docs: vec![telos_context::Document {
                                    doc_id: "user_input".to_string(),
                                    content: payload.clone(),
                                }],
                            };
                            let req = telos_context::NodeRequirement {
                                required_tokens: 1000,
                                query: payload.clone(),
                            };
                            let actual_ctx = context_manager_approval
                                .compress_for_node(&raw_ctx, &req)
                                .await
                                .unwrap_or_else(|_e| telos_context::ScopedContext {
                                    budget_tokens: 1000,
                                    summary_tree: vec![],
                                    precise_facts: vec![],
                                });

                            execution_engine
                                .run_graph(
                                    &mut graph,
                                    &actual_ctx,
                                    registry_clone.as_ref(),
                                    broker_bg.as_ref(),
                                )
                                .await;

                            let _final_result = match graph.node_results.get("llm_node") {
                                Some(res) if res.success => res
                                    .output
                                    .as_ref()
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| "No output".to_string()),
                                Some(res) => res
                                    .error
                                    .as_ref()
                                    .map(|e| format!("Error: {}", e.message))
                                    .unwrap_or_else(|| "Unknown error".to_string()),
                                None => "No result generated by node".to_string(),
                            };

                            let _total_time_ms = task_start_time.elapsed().as_millis() as u64;
                            let failed_nodes = graph.node_statuses.values().filter(|s| **s == telos_core::NodeStatus::Failed).count();
                            let task_success = failed_nodes == 0;
                            
                            if task_success {
                                crate::METRICS.task_total_success.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            } else {
                                crate::METRICS.task_total_failures.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                            crate::core::metrics_store::record(crate::core::metrics_store::MetricEvent::TaskResult {
                                timestamp_ms: crate::core::metrics_store::now_ms(),
                                task_id: task_id.clone(),
                                fulfilled: task_success,
                                total_time_ms: _total_time_ms,
                            });
                        }
                    });
                }
                _ => {}
            }
        }
}
