import sys

def main():
    with open('crates/telos_daemon/src/main.rs', 'r') as f:
        content = f.read()

    # 1. Update AgentFeedback::Output to include task_id everywhere
    content = content.replace(
        """broker_bg.publish_feedback(AgentFeedback::Output {
                        session_id,
                        content: format!("Execution Complete. LLM Response: {}", final_result),
                        is_final: true,
                    });""",
        """broker_bg.publish_feedback(AgentFeedback::Output {
                        task_id: trace_id.to_string(),
                        session_id,
                        content: format!("Execution Complete. LLM Response: {}", final_result),
                        is_final: true,
                    });"""
    )
    content = content.replace(
        """broker_bg.publish_feedback(AgentFeedback::Output {
                            session_id: "default".into(),
                            content: "Task Rejected.".into(),
                            is_final: true,
                        });""",
        """broker_bg.publish_feedback(AgentFeedback::Output {
                            task_id: task_id.clone(),
                            session_id: "default".into(),
                            content: "Task Rejected.".into(),
                            is_final: true,
                        });"""
    )
    content = content.replace(
        """broker_bg.publish_feedback(AgentFeedback::Output {
                            session_id: "default".into(),
                            content: "Task Approved. Executing...".into(),
                            is_final: false,
                        });""",
        """broker_bg.publish_feedback(AgentFeedback::Output {
                            task_id: task_id.clone(),
                            session_id: "default".into(),
                            content: "Task Approved. Executing...".into(),
                            is_final: false,
                        });"""
    )
    content = content.replace(
        """broker_bg.publish_feedback(AgentFeedback::Output {
                            session_id: "default".into(),
                            content: format!("Execution Complete. LLM Response: {}", final_result),
                            is_final: true,
                        });""",
        """broker_bg.publish_feedback(AgentFeedback::Output {
                            task_id: task_id.clone(),
                            session_id: "default".into(),
                            content: format!("Execution Complete. LLM Response: {}", final_result),
                            is_final: true,
                        });"""
    )
    content = content.replace(
        """broker_bg.publish_feedback(AgentFeedback::Output {
                            session_id: "default".into(),
                            content: "Task failed to resume: Payload lost or expired.".into(),
                            is_final: true,
                        });""",
        """broker_bg.publish_feedback(AgentFeedback::Output {
                            task_id: task_id.clone(),
                            session_id: "default".into(),
                            content: "Task failed to resume: Payload lost or expired.".into(),
                            is_final: true,
                        });"""
    )

    # 2. Add dynamic DAG generation logic

    # We need to add the DAG structs and replace the classification logic
    # First, let's insert the structs after standard imports

    structs_code = """
// --- Dynamic DAG Deserialization structs ---
#[derive(serde::Deserialize, Debug)]
struct DagEdge {
    from: String,
    to: String,
}

#[derive(serde::Deserialize, Debug)]
struct DagNode {
    id: String,
    task_type: String, // "LLM" or "TOOL"
    prompt: String,
}

#[derive(serde::Deserialize, Debug)]
struct DagPlan {
    nodes: Vec<DagNode>,
    edges: Vec<DagEdge>,
}
"""

    # find the struct LlmPromptNode definition
    pos = content.find("struct LlmPromptNode")
    if pos != -1:
        content = content[:pos] + structs_code + content[pos:]

    # Now replace the dynamic DAG generation code block
    old_classification_logic = """                    // --- DYNAMIC DAG GENERATION VIA LLM ---
                    // 1. Ask the Gateway to classify the task
                    let classification_req = LlmRequest {
                        session_id: "daemon_planner".to_string(),
                        messages: vec![telos_model_gateway::Message {
                            role: "user".to_string(),
                            content: format!("Classify this task as either 'TOOL' or 'LLM': {}", payload),
                        }],
                        required_capabilities: Capability { requires_vision: false, strong_reasoning: false },
                        budget_limit: 1000,
                    };

                    let classification = match gateway_clone.generate(classification_req).await {
                        Ok(res) => res.content,
                        Err(_) => "LLM".to_string(), // Fallback
                    };

                    let mut graph = TaskGraph::new(trace_id.to_string());

                    // 2. Dynamically construct the graph based on the LLM's classification
                    if classification.contains("TOOL") {
                        println!("[Daemon] LLM classified task as TOOL execution.");
                        graph.add_node("tool_node".to_string(), Box::new(WasmToolNode {
                            tool_name: payload.clone()
                        }));
                    } else {
                        println!("[Daemon] LLM classified task as LLM generation.");
                        graph.add_node("llm_node".to_string(), Box::new(LlmPromptNode {
                            prompt: format!("Execute the following user command: {}", payload),
                            gateway: gateway_clone.clone()
                        }));
                    }

                    graph.current_state = GraphState { is_running: true, completed: false };

                    let empty_ctx = telos_context::ScopedContext {
                        budget_tokens: 1000,
                        summary_tree: vec![],
                        precise_facts: vec![],
                    };

                    execution_engine.run_graph(&mut graph, &empty_ctx, registry_clone.as_ref(), broker_bg.as_ref()).await;

                    // Fetch the result from the graph dynamically
                    let node_id = if classification.contains("TOOL") { "tool_node" } else { "llm_node" };
                    let final_result = match graph.node_results.get(node_id) {
                        Some(Ok(res)) => String::from_utf8_lossy(&res.output_data).to_string(),
                        Some(Err(e)) => format!("Error executing node: {:?}", e),
                        None => "No result generated by node".to_string(),
                    };

                    broker_bg.publish_feedback(AgentFeedback::Output {
                        task_id: trace_id.to_string(),
                        session_id,
                        content: format!("Execution Complete. LLM Response: {}", final_result),
                        is_final: true,
                    });"""

    new_classification_logic = """                    // --- DYNAMIC DAG GENERATION VIA LLM ---
                    let prompt = format!(
                        "You are an expert planner. Break down the following task into a directed acyclic graph (DAG) of sub-tasks.\\n\\
                        Task: {}\\n\\n\\
                        Respond strictly with a JSON object matching this schema:\\n\\
                        {{\\n\\
                            \\"nodes\\": [ {{ \\"id\\": \\"string\\", \\"task_type\\": \\"LLM\\" or \\"TOOL\\", \\"prompt\\": \\"Detailed execution instruction for this node\\" }} ],\\n\\
                            \\"edges\\": [ {{ \\"from\\": \\"node_id_1\\", \\"to\\": \\"node_id_2\\" }} ]\\n\\
                        }}\\n\\
                        Do not include markdown blocks, only raw JSON.",
                        payload
                    );

                    let classification_req = LlmRequest {
                        session_id: "daemon_planner".to_string(),
                        messages: vec![telos_model_gateway::Message {
                            role: "user".to_string(),
                            content: prompt,
                        }],
                        required_capabilities: Capability { requires_vision: false, strong_reasoning: false },
                        budget_limit: 2000,
                    };

                    let plan_json = match gateway_clone.generate(classification_req).await {
                        Ok(res) => res.content.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").to_string(),
                        Err(e) => {
                            broker_bg.publish_feedback(AgentFeedback::Output {
                                task_id: trace_id.to_string(),
                                session_id: session_id.clone(),
                                content: format!("Planning Failed: {:?}", e),
                                is_final: true,
                            });
                            continue;
                        }
                    };

                    println!("LLM Plan JSON: {}", plan_json);

                    let dag_plan: DagPlan = match serde_json::from_str(&plan_json) {
                        Ok(plan) => plan,
                        Err(e) => {
                            // Fallback if LLM fails to return valid JSON
                            println!("Failed to parse DAG plan: {}. Using fallback single node.", e);
                            DagPlan {
                                nodes: vec![DagNode { id: "main".to_string(), task_type: "LLM".to_string(), prompt: payload.clone() }],
                                edges: vec![]
                            }
                        }
                    };

                    let mut graph = TaskGraph::new(trace_id.to_string());
                    let mut terminal_nodes = vec![];

                    for node in &dag_plan.nodes {
                        terminal_nodes.push(node.id.clone());
                        if node.task_type == "TOOL" {
                            graph.add_node(node.id.clone(), Box::new(WasmToolNode {
                                tool_name: node.prompt.clone()
                            }));
                        } else {
                            graph.add_node(node.id.clone(), Box::new(LlmPromptNode {
                                prompt: node.prompt.clone(),
                                gateway: gateway_clone.clone()
                            }));
                        }
                    }

                    for edge in &dag_plan.edges {
                        let _ = graph.add_edge(&edge.from, &edge.to);
                        terminal_nodes.retain(|id| id != &edge.from); // Keep only nodes that have no outgoing edges
                    }

                    if terminal_nodes.is_empty() && !dag_plan.nodes.is_empty() {
                         terminal_nodes.push(dag_plan.nodes.last().unwrap().id.clone());
                    }

                    graph.current_state = GraphState { is_running: true, completed: false };

                    let empty_ctx = telos_context::ScopedContext {
                        budget_tokens: 1000,
                        summary_tree: vec![],
                        precise_facts: vec![],
                    };

                    execution_engine.run_graph(&mut graph, &empty_ctx, registry_clone.as_ref(), broker_bg.as_ref()).await;

                    // Fetch the results from the terminal nodes
                    let mut final_results = Vec::new();
                    for node_id in terminal_nodes {
                        if let Some(Ok(res)) = graph.node_results.get(&node_id) {
                            final_results.push(format!("[{}] {}", node_id, String::from_utf8_lossy(&res.output_data)));
                        } else if let Some(Err(e)) = graph.node_results.get(&node_id) {
                             final_results.push(format!("[{}] Failed: {:?}", node_id, e));
                        }
                    }

                    let combined_result = if final_results.is_empty() {
                         "No result generated by graph".to_string()
                    } else {
                         final_results.join("\\n")
                    };

                    broker_bg.publish_feedback(AgentFeedback::Output {
                        task_id: trace_id.to_string(),
                        session_id,
                        content: format!("Execution Complete. Responses:\\n{}", combined_result),
                        is_final: true,
                    });"""

    if old_classification_logic in content:
        content = content.replace(old_classification_logic, new_classification_logic)
    else:
        print("Could not find old classification logic")
        sys.exit(1)


    # 3. Add bot startup logic to daemon main

    bot_startup_code = """
    // Start Bot Provider in background if configured
    if let Some(bot_token) = config.telegram_bot_token.clone() {
        println!("Starting Telegram Bot Provider from Daemon...");
        let daemon_url = "http://127.0.0.1:3000".to_string();
        let daemon_ws_url = "ws://127.0.0.1:3000/api/v1/stream".to_string();
        let send_state_changes = config.bot_send_state_changes;

        tokio::spawn(async move {
            let provider = telos_bot::providers::telegram::TelegramBotProvider::new(
                bot_token, daemon_url, daemon_ws_url, send_state_changes
            );
            if let Err(e) = telos_bot::traits::ChatBotProvider::start(&provider).await {
                eprintln!("Failed to start bot provider: {}", e);
            }
        });
    }

    // --- API SERVER ---"""

    content = content.replace("    // --- API SERVER ---", bot_startup_code)


    with open('crates/telos_daemon/src/main.rs', 'w') as f:
        f.write(content)

if __name__ == "__main__":
    main()
