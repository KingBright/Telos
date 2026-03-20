use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use std::collections::VecDeque;
use tracing::{debug, info, warn, error};

use telos_core::config::TelosConfig;
use telos_memory::engine::{RedbGraphStore, MemoryOS};
use telos_evolution::evaluator::ActorCriticEvaluator;
use telos_evolution::Evaluator;
use telos_memory::integration::MemoryIntegration;
use telos_hci::{EventBroker, TokioEventBroker};
use crate::DaemonRegistry;

pub fn spawn_background_tasks(
    config: &TelosConfig,
    evaluator: Arc<ActorCriticEvaluator>,
    registry: Arc<DaemonRegistry>,
    memory_os_instance: Arc<RedbGraphStore>,
    broker: Arc<TokioEventBroker>,
) -> (
    mpsc::UnboundedSender<telos_evolution::ExecutionTrace>,
    Arc<RwLock<VecDeque<telos_hci::AgentFeedback>>>
) {
    // --- EVOLUTION EVALUATION WORKER ---
    let (distillation_tx, mut distillation_rx) = mpsc::unbounded_channel::<telos_evolution::ExecutionTrace>();
    let evaluator_worker = evaluator.clone();
    let registry_worker = registry.clone();
    
    tokio::spawn(async move {
        use telos_model_gateway::ModelGateway; // needed for .generate() on GatewayManager
        debug!("[Daemon] 🧵 Evolution worker thread started, listening for traces...");
        while let Some(trace) = distillation_rx.recv().await {
            let trace_id = trace.task_id.clone();
            debug!("[Daemon] 🧠 Evolution worker processing trace {}...", trace_id);
            
            let has_reused_workflows = !trace.reused_workflow_ids.is_empty();
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default().as_millis() as u64;

            if trace.success {
                // --- WORKFLOW TEMPLATE STORAGE / UPGRADE ---
                if let Some(sub_graph) = &trace.sub_graph {
                    if let Ok(json_str) = serde_json::to_string_pretty(sub_graph) {
                        let desc = trace.steps.first()
                            .map(|s| s.input_data.clone())
                            .unwrap_or_else(|| "Unknown Execution".to_string());
                        
                        let mut required_tools = Vec::new();
                        for step in &trace.steps {
                            if let Some(out) = &step.output_data {
                                if let Ok(val) = serde_json::from_str::<serde_json::Value>(out) {
                                    if let Some(tool_name) = val.get("tool").and_then(|t| t.as_str()) {
                                        if !required_tools.contains(&tool_name.to_string()) {
                                            required_tools.push(tool_name.to_string());
                                        }
                                    }
                                }
                            }
                        }
                        
                        if has_reused_workflows {
                            // Reused a workflow and succeeded → UPGRADE the template
                            let json_str_clone = json_str.clone();
                            match registry_worker.memory_os.upgrade_workflow_template(desc.clone(), json_str, required_tools.clone()).await {
                                Ok(upgraded) => {
                                    if upgraded {
                                        info!("[Daemon] 🔄 Workflow template upgraded for task {}.", trace_id);
                                    } else {
                                        info!("[Daemon] 📥 No matching template found to upgrade, stored as new for task {}.", trace_id);
                                    }
                                }
                                Err(e) => {
                                    warn!("[Daemon] ⚠️ Workflow upgrade failed: {}, storing as new.", e);
                                    let _ = registry_worker.memory_os.store_workflow_template(desc.clone(), json_str_clone, required_tools.clone()).await;
                                }
                            }
                        } else {
                            // Brand new workflow → store as new template
                            let _ = registry_worker.memory_os.store_workflow_template(desc.clone(), json_str, required_tools.clone()).await;
                            info!("[Daemon] 📥 Graph Topology archived as Procedural Workflow Template for task {}.", trace_id);
                            
                            // Emit WorkflowStore metric event (only for new templates)
                            crate::core::metrics_store::record(
                                crate::core::metrics_store::MetricEvent::WorkflowStore {
                                    timestamp_ms: ts,
                                    workflow_id: trace_id.clone(),
                                    description: desc,
                                }
                            );
                        }
                    }
                }

                // --- WORKFLOW REUSE SUCCESS EVENT ---
                if has_reused_workflows {
                    for wf_id in &trace.reused_workflow_ids {
                        crate::core::metrics_store::record(
                            crate::core::metrics_store::MetricEvent::WorkflowReuse {
                                timestamp_ms: ts,
                                workflow_id: wf_id.clone(),
                                task_id: trace_id.clone(),
                                success: true,
                            }
                        );
                    }
                    info!("[Daemon] ✅ Workflow reuse SUCCESS recorded for task {} ({} template(s))", trace_id, trace.reused_workflow_ids.len());
                }
                
                // --- SEMANTIC FACT EXTRACTION ---
                let nodes_used: Vec<String> = trace.steps.iter()
                    .map(|s| s.node_id.clone())
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter().collect();
                let step_count = trace.steps.len();
                let first_input = trace.steps.first()
                    .map(|s| s.input_data.clone())
                    .unwrap_or_default();
                
                let summary = format!(
                    "[Task Insight] Task '{}' completed successfully in {} steps. \
                     Nodes used: [{}]. Original objective: {}{}",
                    trace_id,
                    step_count,
                    if nodes_used.is_empty() { "none".to_string() } else { nodes_used.join(", ") },
                    if first_input.len() > 200 { format!("{}...", &first_input[..200]) } else { first_input },
                    if has_reused_workflows { " (reused existing workflow template)" } else { "" },
                );
                let _ = registry_worker.memory_os.store_semantic_fact(summary).await;
                debug!("[Daemon] 💡 Semantic fact extracted from successful task {}.", trace_id);
            } else if has_reused_workflows {
                // --- WORKFLOW REUSE FAILURE: ATTACH NOTES + PENALTY ---
                // Build failure note from trace errors
                let failure_note = if !trace.errors_encountered.is_empty() {
                    trace.errors_encountered.iter()
                        .map(|e| format!("{:?}", e))
                        .collect::<Vec<_>>()
                        .join("; ")
                } else {
                    let failed_steps: Vec<String> = trace.steps.iter()
                        .filter(|s| s.error.is_some())
                        .map(|s| format!("[{}] {:?}", s.node_id, s.error.as_ref().unwrap()))
                        .collect();
                    if failed_steps.is_empty() {
                        "Task rejected by QA evaluator — output did not meet quality standards.".to_string()
                    } else {
                        failed_steps.join("; ")
                    }
                };
                
                let task_desc = trace.steps.first()
                    .map(|s| s.input_data.clone())
                    .unwrap_or_else(|| "Unknown task".to_string());
                let note_with_context = format!("Failed on task '{}': {}", 
                    &task_desc[..task_desc.len().min(100)], failure_note);
                
                for wf_id in &trace.reused_workflow_ids {
                    crate::core::metrics_store::record(
                        crate::core::metrics_store::MetricEvent::WorkflowReuse {
                            timestamp_ms: ts,
                            workflow_id: wf_id.clone(),
                            task_id: trace_id.clone(),
                            success: false,
                        }
                    );
                    
                    // Attach failure note to the template so the Architect can see warnings
                    let failure_count = match registry_worker.memory_os.attach_failure_note(
                        wf_id.clone(), note_with_context.clone()
                    ).await {
                        Ok(count) => count,
                        Err(e) => {
                            warn!("[Daemon] ⚠️ Failed to attach failure note: {}", e);
                            0
                        }
                    };
                    
                    // Apply mild strength penalty (-0.3, floored at 1.0)
                    if let Err(e) = registry_worker.memory_os.penalize_workflow_template(
                        wf_id.clone()
                    ).await {
                        warn!("[Daemon] ⚠️ Failed to penalize template: {}", e);
                    }
                    
                    // --- SPECIES DIVERGENCE: ≥2 cumulative failures triggers variant generation ---
                    if failure_count >= 2 {
                        info!("[Daemon] 🧬 Template '{}' has {} failures — triggering species divergence.", 
                            &wf_id[..wf_id.len().min(60)], failure_count);
                        
                        // Retrieve the original template to pass to LLM
                        if let Ok(templates) = registry_worker.memory_os.retrieve_procedural_memories(wf_id.clone()).await {
                            if let Some(original_template) = templates.first() {
                                let diverge_prompt = format!(
                                    "You are a workflow evolution engine. A workflow template has failed {} times.\n\n\
                                     ORIGINAL TEMPLATE:\n{}\n\n\
                                     The template contains [FailureNote] lines describing what went wrong.\n\
                                     Generate an ADAPTED VARIANT that avoids the failure patterns.\n\n\
                                     Rules:\n\
                                     1. Keep the same general structure but adjust the failing parts\n\
                                     2. Add error handling or fallback nodes where failures occurred\n\
                                     3. If a node type consistently fails, replace it with an alternative approach\n\
                                     4. Output ONLY the [Description] line and the [TemplateJSON] block\n\n\
                                     Format:\n[Description] <new adapted description>\n[TemplateJSON]\n<adapted JSON>",
                                    failure_count, original_template
                                );
                                
                                let req = telos_model_gateway::LlmRequest {
                                    session_id: format!("diverge_{}", trace_id),
                                    messages: vec![
                                        telos_model_gateway::Message {
                                            role: "system".to_string(),
                                            content: "You are a workflow template evolution system. Output only the requested format.".to_string(),
                                        },
                                        telos_model_gateway::Message {
                                            role: "user".to_string(),
                                            content: diverge_prompt,
                                        },
                                    ],
                                    required_capabilities: telos_model_gateway::Capability {
                                        requires_vision: false,
                                        strong_reasoning: true,
                                    },
                                    budget_limit: 2000,
                                    tools: None,
                                };
                                
                                match registry_worker.gateway.generate(req).await {
                                    Ok(res) => {
                                        let content = res.content.trim()
                                            .trim_start_matches("```json").trim_start_matches("```")
                                            .trim_end_matches("```").trim();
                                        
                                        // Parse out [Description] and [TemplateJSON]
                                        let desc = content.lines()
                                            .find(|l: &&str| l.starts_with("[Description] "))
                                            .map(|l: &str| l.trim_start_matches("[Description] ").to_string())
                                            .unwrap_or_else(|| format!("Variant of: {}", &wf_id[..wf_id.len().min(60)]));
                                        
                                        let template_json = if let Some(pos) = content.find("[TemplateJSON]") {
                                            content[pos + "[TemplateJSON]".len()..].trim().to_string()
                                        } else {
                                            content.to_string()
                                        };
                                        
                                        if !template_json.is_empty() {
                                            let variant_desc = format!("[Variant] {} (diverged from failures)", desc);
                                            match registry_worker.memory_os.store_workflow_template(
                                                variant_desc.clone(), template_json, Vec::new()
                                            ).await {
                                                Ok(()) => {
                                                    info!("[Daemon] 🧬 Species divergence complete — new variant stored: {}", 
                                                        &variant_desc[..variant_desc.len().min(80)]);
                                                }
                                                Err(e) => {
                                                    warn!("[Daemon] ⚠️ Failed to store diverged variant: {}", e);
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!("[Daemon] ⚠️ Species divergence LLM call failed: {:?}", e);
                                    }
                                }
                            }
                        }
                    }
                }
                warn!("[Daemon] ⚠️ Workflow reuse FAILED for task {} ({} template(s)). Failure notes attached, strength penalized.", trace_id, trace.reused_workflow_ids.len());
            }

            if let Some(skill) = evaluator_worker.distill_experience(&trace).await {
                info!("[Daemon] 🧠 Telos distilled a new SynthesizedSkill from task {}!", trace_id);
                let _ = registry_worker.memory_os.store_procedural_skill(
                    skill.trigger_condition,
                    skill.executable_code,
                ).await;
                debug!("[Daemon] 📥 Distilled skill archived as Procedural Memory (permanent).");
            }
        }
    });

    // --- BACKGROUND MEMORY MAINTENANCE WORKER ---
    let memory_maintenance = memory_os_instance.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600)); 
        loop {
            interval.tick().await;
            if let Err(e) = memory_maintenance.trigger_fade_consolidation().await {
                warn!("[MemoryWorker] Fade sweep failed: {}", e);
            }
            if let Err(e) = memory_maintenance.consolidate().await {
                warn!("[MemoryWorker] Reconsolidation failed: {}", e);
            }
            info!("[MemoryWorker] ✅ Hourly memory maintenance completed");
        }
    });

    // --- PERSONALITY REFLECTION WORKER (every 6 hours) ---
    let reflection_memory = memory_os_instance.clone();
    let reflection_gateway = registry.gateway.clone();
    tokio::spawn(async move {
        use telos_model_gateway::{Capability, LlmRequest, Message, ModelGateway};
        // Initial delay: 30 minutes (let interactions accumulate first)
        tokio::time::sleep(std::time::Duration::from_secs(1800)).await;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(21600)); // 6 hours
        loop {
            interval.tick().await;
            debug!("[PersonalityReflection] 🪞 Starting periodic self-reflection...");
            
            // Gather recent interactions (last 24h)
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
            let day_ago = now.saturating_sub(86400);
            
            let recent_interactions = match reflection_memory.retrieve(
                telos_memory::MemoryQuery::TimeRange { start: day_ago, end: now }
            ).await {
                Ok(entries) => entries.into_iter()
                    .filter(|e| matches!(e.memory_type, 
                        telos_memory::MemoryType::InteractionEvent | telos_memory::MemoryType::UserProfile))
                    .take(20) // Cap to avoid huge prompts
                    .map(|e| e.content)
                    .collect::<Vec<_>>(),
                Err(_) => continue,
            };
            
            if recent_interactions.len() < 3 {
                debug!("[PersonalityReflection] Too few interactions ({}) for reflection, skipping.", recent_interactions.len());
                continue;
            }
            
            let reflection_prompt = format!(
                r#"You are a meta-cognitive reflection system for an AI agent named Telos.
Review the following recent interaction summaries and user profile facts.
Synthesize insights about how the agent should adapt its personality and behavior.

Focus on these dimensions:
1. **Communication Adaptation**: How should the agent adjust its tone, language, and formality?
2. **Expertise Growth**: What new domains has the agent gained experience in? What patterns recur?
3. **Trust Evolution**: Is the user giving more autonomy or requesting more oversight? How to adapt?
4. **Workflow Optimization**: What repetitive patterns could be streamlined? What task types dominate?
5. **Personality Traits**: What personality traits should the agent strengthen or develop?

Output ONLY a valid JSON object:
{{"insights": ["[Dimension] insight1", "[Dimension] insight2", ...]}}

Recent Data:
{}
"#,
                recent_interactions.join("\n---\n")
            );
            
            let request = LlmRequest {
                session_id: "personality_reflection".to_string(),
                messages: vec![
                    Message { role: "system".into(), content: "You are a precise behavioral analysis system. Output only valid JSON.".into() },
                    Message { role: "user".into(), content: reflection_prompt },
                ],
                required_capabilities: Capability { requires_vision: false, strong_reasoning: false },
                budget_limit: 2000,
                tools: None,
            };
            
            match reflection_gateway.generate(request).await {
                Ok(response) => {
                    let cleaned = response.content.trim()
                        .trim_start_matches("```json").trim_end_matches("```").trim();
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(cleaned) {
                        if let Some(insights) = parsed.get("insights").and_then(|i| i.as_array()) {
                            let mut stored = 0;
                            for insight in insights {
                                if let Some(text) = insight.as_str() {
                                    let fact = format!("[Agent Reflection] {}", text);
                                    if let Ok(()) = reflection_memory.store_semantic_fact(fact).await {
                                        stored += 1;
                                    }
                                }
                            }
                            if stored > 0 {
                                info!("[PersonalityReflection] 🪞 Stored {} self-reflection insights", stored);
                            }
                        }
                    }
                }
                Err(e) => {
                    debug!("[PersonalityReflection] LLM reflection failed: {:?}", e);
                }
            }
        }
    });

    // NOTE: Dashboard UI is now served directly from the daemon (port 8321) via ServeDir fallback.
    // No separate telos_web server needed — single port architecture.

    // --- TELEGRAM BOT PROVIDER ---
    if let Some(bot_token) = config.telegram_bot_token.clone() {
        info!("Starting Telegram Bot Provider from Daemon...");
        let daemon_url = "http://127.0.0.1:8321".to_string();
        let daemon_ws_url = "ws://127.0.0.1:8321/api/v1/stream".to_string();
        let send_state_changes = config.bot_send_state_changes;
        tokio::spawn(async move {
            let provider = telos_bot::providers::telegram::TelegramBotProvider::new(
                bot_token, daemon_url, daemon_ws_url, send_state_changes,
            );
            if let Err(e) = telos_bot::traits::ChatBotProvider::start(&provider).await {
                error!("Failed to start bot provider: {}", e);
            }
        });
    }

    // --- RECENT TRACES COLLECTOR LOOP (with disk persistence) ---
    let traces_file = dirs::home_dir()
        .map(|h| h.join(".telos").join("traces.json"))
        .unwrap_or_else(|| std::path::PathBuf::from("traces.json"));
    
    // Load persisted traces on startup
    let mut initial_traces = VecDeque::with_capacity(100);
    if traces_file.exists() {
        if let Ok(data) = std::fs::read_to_string(&traces_file) {
            if let Ok(loaded) = serde_json::from_str::<Vec<telos_hci::AgentFeedback>>(&data) {
                info!("[Traces] Restored {} traces from disk", loaded.len());
                for t in loaded.into_iter().rev().take(100).rev() {
                    initial_traces.push_back(t);
                }
            }
        }
    }
    let recent_traces: Arc<RwLock<VecDeque<telos_hci::AgentFeedback>>> = Arc::new(RwLock::new(initial_traces));
    
    let mut rx = broker.subscribe_feedback();
    let traces_bg = recent_traces.clone();
    let traces_file_bg = traces_file.clone();
    tokio::spawn(async move {
        while let Ok(feedback) = rx.recv().await {
            if let telos_hci::AgentFeedback::Trace { .. } = &feedback {
                let mut q = traces_bg.write().await;
                if q.len() >= 100 { q.pop_front(); }
                q.push_back(feedback);
                
                // Persist to disk (best-effort, non-blocking for the collector)
                let traces_vec: Vec<_> = q.iter().cloned().collect();
                let path = traces_file_bg.clone();
                tokio::spawn(async move {
                    if let Ok(json) = serde_json::to_string(&traces_vec) {
                        let _ = tokio::fs::write(&path, json).await;
                    }
                });
            }
        }
    });

    (distillation_tx, recent_traces)
}
