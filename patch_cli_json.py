import sys

with open("crates/telos_cli/src/main.rs", "r") as f:
    code = f.read()

if "use telos_hci::AgentFeedback;" not in code:
    code = code.replace("use telos_core::config::TelosConfig;", "use telos_core::config::TelosConfig;\nuse telos_hci::AgentFeedback;")

old_loop = """    // Listen for incoming events
    while let Some(message) = read.next().await {
        let msg = message?;
        if let Message::Text(text) = msg {
            if text.contains("RequireHumanIntervention") {
                println!("\\n🚨 [HUMAN INTERVENTION REQUIRED] 🚨");
                println!("{}", text);

                print!("Approve this action? [y/N]: ");
                io::stdout().flush().unwrap();
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                let approved = input.trim().eq_ignore_ascii_case("y");

                let res = client
                    .post("http://127.0.0.1:3000/api/v1/approve")
                    .json(&json!({ "task_id": trace_id, "approved": approved }))
                    .send()
                    .await?;

                if res.status().is_success() {
                     println!("-> User Decision sent: Approved={}", approved);
                } else {
                     println!("-> Failed to send decision.");
                }

            } else if text.contains("Output") {
                println!(">> {}", text);
                if text.contains("is_final: true") {
                    println!("Task completed.");
                    break;
                }
            } else {
                 println!("[STATE] {}", text);
            }
        }
    }"""

new_loop = """    // Listen for incoming events
    while let Some(message) = read.next().await {
        let msg = message?;
        if let Message::Text(text) = msg {
            if let Ok(feedback) = serde_json::from_str::<AgentFeedback>(&text) {
                match feedback {
                    AgentFeedback::RequireHumanIntervention { prompt, task_id, .. } => {
                        println!("\\n🚨 [HUMAN INTERVENTION REQUIRED] 🚨");
                        println!("{}", prompt);

                        print!("Approve this action? [y/N]: ");
                        io::stdout().flush().unwrap();
                        let mut input = String::new();
                        io::stdin().read_line(&mut input)?;
                        let approved = input.trim().eq_ignore_ascii_case("y");

                        let res = client
                            .post("http://127.0.0.1:3000/api/v1/approve")
                            .json(&json!({ "task_id": task_id, "approved": approved }))
                            .send()
                            .await?;

                        if res.status().is_success() {
                             println!("-> User Decision sent: Approved={}", approved);
                        } else {
                             println!("-> Failed to send decision.");
                        }
                    }
                    AgentFeedback::Output { content, is_final, .. } => {
                        println!(">> {}", content);
                        if is_final {
                            println!("Task completed.");
                            break;
                        }
                    }
                    AgentFeedback::StateChanged { current_node, status, .. } => {
                        println!("[STATE] {} -> {:?}", current_node, status);
                    }
                }
            }
        }
    }"""

code = code.replace(old_loop, new_loop)

with open("crates/telos_cli/src/main.rs", "w") as f:
    f.write(code)
