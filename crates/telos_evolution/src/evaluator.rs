use std::sync::Arc;
use async_trait::async_trait;
use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};
use tokio::task;

use crate::{DriftWarning, Evaluator, ExecutionTrace, SynthesizedSkill};

pub struct ActorCriticEvaluator {
    embedder: Option<Arc<tokio::sync::Mutex<TextEmbedding>>>,
    gateway: Option<Arc<dyn telos_model_gateway::ModelGateway>>,
    similarity_threshold: f32,
}

impl ActorCriticEvaluator {
    pub fn new() -> anyhow::Result<Self> {
        let model = std::panic::catch_unwind(|| {
            let mut options = InitOptions::new(EmbeddingModel::AllMiniLML6V2);
            let cache_dir = dirs::home_dir().map(|h| h.join(".telos").join("models")).unwrap_or_else(|| std::path::PathBuf::from(".fastembed_cache"));
            options = options.with_cache_dir(cache_dir);
            options.show_download_progress = false;
            TextEmbedding::try_new(options)
        });

        let embedder_opt = match model {
            Ok(Ok(m)) => Some(Arc::new(tokio::sync::Mutex::new(m))),
            _ => {
                eprintln!("[ActorCritic] Fastembed init failed. Semantic loop detection disabled.");
                None
            }
        };

        Ok(Self {
            embedder: embedder_opt,
            gateway: None,
            similarity_threshold: 0.85,
        })
    }

    /// Set an LLM gateway for LLM-powered distillation
    pub fn with_gateway(mut self, gw: Arc<dyn telos_model_gateway::ModelGateway>) -> Self {
        self.gateway = Some(gw);
        self
    }

    fn calculate_cosine_similarity(v1: &[f32], v2: &[f32]) -> f32 {
        let dot_product: f32 = v1.iter().zip(v2.iter()).map(|(a, b)| a * b).sum();
        let norm_v1: f32 = v1.iter().map(|a| a * a).sum::<f32>().sqrt();
        let norm_v2: f32 = v2.iter().map(|b| b * b).sum::<f32>().sqrt();

        if norm_v1 == 0.0 || norm_v2 == 0.0 {
            0.0
        } else {
            dot_product / (norm_v1 * norm_v2)
        }
    }

    pub async fn evaluate_from_source(&self, trace_id: &str, source: &dyn crate::TraceExport) -> Result<Option<crate::SynthesizedSkill>, String> {
        if let Some(trace) = source.export_trace(trace_id) {
            Ok(self.distill_experience(&trace).await)
        } else {
            Err("Trace not found".to_string())
        }
    }
}

#[async_trait]
impl Evaluator for ActorCriticEvaluator {
    async fn detect_drift(&self, trace: &ExecutionTrace) -> Result<(), DriftWarning> {
        if trace.steps.len() < 2 {
            return Ok(());
        }

        let embedder = match &self.embedder {
            Some(e) => e.clone(),
            None => return Ok(()),
        };

        // 5-step sliding window: compare all recent pairs for semantic loops
        let window_size = 5.min(trace.steps.len());
        let steps_text: Vec<String> = trace.steps.iter().rev().take(window_size).map(|step| {
            format!("Node: {}, Input: {}, Output: {:?}",
                step.node_id,
                step.input_data,
                step.output_data.as_deref().unwrap_or("None")
            )
        }).collect();

        // Wrap CPU-bound synchronous fastembed execution in spawn_blocking
        let embeddings = task::spawn_blocking(move || {
            let mut embedder_guard = embedder.blocking_lock();
            embedder_guard.embed(steps_text, None)
        }).await.map_err(|_| DriftWarning::TargetDrift)?
        .map_err(|_| DriftWarning::TargetDrift)?;

        // Pairwise comparison: any two steps too similar = semantic loop
        for i in 0..embeddings.len() {
            for j in (i+1)..embeddings.len() {
                let sim = Self::calculate_cosine_similarity(&embeddings[i], &embeddings[j]);
                if sim >= self.similarity_threshold {
                    return Err(DriftWarning::SemanticLoop);
                }
            }
        }

        Ok(())
    }

    async fn distill_experience(&self, trace: &ExecutionTrace) -> Option<SynthesizedSkill> {
        if !trace.success {
            return None; // Only distill successful traces
        }
        if trace.steps.is_empty() {
            return None;
        }

        let first_step = trace.steps.first()?;

        // Try LLM-powered distillation if gateway is available
        if let Some(ref gw) = self.gateway {
            let trace_summary: String = trace.steps.iter().enumerate().map(|(i, s)| {
                let output_preview = s.output_data.as_deref()
                    .map(|o| o.chars().take(200).collect::<String>())
                    .unwrap_or_else(|| "None".to_string());
                format!("Step {}: [{}] input='{}' output='{}'", i+1, s.node_id, s.input_data, output_preview)
            }).collect::<Vec<_>>().join("\n");

            let req = telos_model_gateway::LlmRequest {
                session_id: format!("distill_{}", trace.task_id),
                messages: vec![
                    telos_model_gateway::Message {
                        role: "system".to_string(),
                        content: "You are a skill distillation engine. Analyze the execution trace and extract a reusable skill pattern. Output ONLY valid JSON:\n{\"trigger\": \"When the user asks about X or wants to Y\", \"procedure\": \"Step-by-step procedure extracted from the trace\"}\nBe concise. Focus on the PATTERN, not the specific data.".to_string(),
                    },
                    telos_model_gateway::Message {
                        role: "user".to_string(),
                        content: format!("Task: {}\n\nExecution Trace:\n{}", first_step.input_data, trace_summary),
                    },
                ],
                required_capabilities: telos_model_gateway::Capability {
                    requires_vision: false,
                    strong_reasoning: false,
                },
                budget_limit: 500,
                tools: None,
            };

            if let Ok(res) = gw.generate(req).await {
                let content = res.content.trim();
                if let (Some(s), Some(e)) = (content.find('{'), content.rfind('}')) {
                    if e > s {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content[s..=e]) {
                            let trigger = json.get("trigger").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let procedure = json.get("procedure").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            if !trigger.is_empty() && !procedure.is_empty() {
                                return Some(SynthesizedSkill {
                                    trigger_condition: trigger,
                                    executable_code: procedure,
                                    success_rate: 1.0,
                                });
                            }
                        }
                    }
                }
            }
        }

        // Fallback: naive distillation
        let trigger_condition = format!("Matches input intent similar to: {}", first_step.input_data);
        let node_sequence: Vec<String> = trace.steps.iter().map(|s| s.node_id.clone()).collect();
        let executable_code = format!("Execute sequence: [{}]", node_sequence.join(" -> "));

        Some(SynthesizedSkill {
            trigger_condition,
            executable_code,
            success_rate: 1.0,
        })
    }
}



#[cfg(test)]
mod tests {
    use super::*;
    use crate::TraceStep;

    #[tokio::test]
    async fn test_semantic_loop_detection() {
        let evaluator = ActorCriticEvaluator::new().expect("Failed to initialize embedder");

        // Two identical steps to force semantic loop detection
        let step1 = TraceStep {
            node_id: "node_a".to_string(),
            input_data: "Searching for weather in Paris".to_string(),
            output_data: Some("Cannot find data".to_string()),
            error: None,
        };

        let step2 = TraceStep {
            node_id: "node_b".to_string(), // different node
            input_data: "Lookup weather for Paris, France".to_string(), // semantically identical
            output_data: Some("Failed to retrieve data".to_string()), // semantically similar
            error: None,
        };

        let trace = ExecutionTrace {
            task_id: "task_123".to_string(),
            steps: vec![step1, step2],
            errors_encountered: vec![],
            success: false,
            sub_graph: None,
        };

        let result = evaluator.detect_drift(&trace).await;

        // Assert that the semantic loop was caught.
        assert!(matches!(result, Err(DriftWarning::SemanticLoop)));
    }

    #[tokio::test]
    async fn test_distill_experience() {
        let evaluator = ActorCriticEvaluator::new().expect("Failed to initialize embedder");

        let step1 = TraceStep {
            node_id: "fetch_db".to_string(),
            input_data: "SELECT * FROM users".to_string(),
            output_data: Some("user list".to_string()),
            error: None,
        };

        let step2 = TraceStep {
            node_id: "filter_admins".to_string(),
            input_data: "user list".to_string(),
            output_data: Some("admin list".to_string()),
            error: None,
        };

        let trace = ExecutionTrace {
            task_id: "task_456".to_string(),
            steps: vec![step1, step2],
            errors_encountered: vec![],
            success: true,
            sub_graph: None,
        };

        let skill = evaluator.distill_experience(&trace).await.expect("Should distill successfully");

        assert_eq!(skill.trigger_condition, "Matches input intent similar to: SELECT * FROM users");
        assert_eq!(skill.executable_code, "Execute sequence: [fetch_db -> filter_admins]");
        assert_eq!(skill.success_rate, 1.0);
    }
}
