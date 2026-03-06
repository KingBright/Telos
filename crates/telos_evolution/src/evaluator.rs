use std::sync::Arc;
use async_trait::async_trait;
use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};
use tokio::task;

use crate::{DriftWarning, Evaluator, ExecutionTrace, SynthesizedSkill};

pub struct ActorCriticEvaluator {
    embedder: Arc<tokio::sync::Mutex<TextEmbedding>>,
    similarity_threshold: f32,
}

impl ActorCriticEvaluator {
    pub fn new() -> anyhow::Result<Self> {
        let mut options = InitOptions::new(EmbeddingModel::AllMiniLML6V2);
        options.show_download_progress = false;

        let embedder = TextEmbedding::try_new(options)?;

        Ok(Self {
            embedder: Arc::new(tokio::sync::Mutex::new(embedder)),
            similarity_threshold: 0.85, // Adjust threshold to allow slight variations in testing
        })
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
        };

        let skill = evaluator.distill_experience(&trace).await.expect("Should distill successfully");

        assert_eq!(skill.trigger_condition, "Matches input intent similar to: SELECT * FROM users");
        assert_eq!(skill.executable_code, "Execute sequence: [fetch_db -> filter_admins]");
        assert_eq!(skill.success_rate, 1.0);
    }
}

#[async_trait]
impl Evaluator for ActorCriticEvaluator {
    async fn detect_drift(&self, trace: &ExecutionTrace) -> Result<(), DriftWarning> {
        if trace.steps.len() < 2 {
            return Ok(());
        }

        // Extract text representations of the last two steps
        let steps_text: Vec<String> = trace.steps.iter().rev().take(2).map(|step| {
            format!("Node: {}, Input: {}, Output: {:?}",
                step.node_id,
                step.input_data,
                step.output_data.as_deref().unwrap_or("None")
            )
        }).collect();

        let embedder = self.embedder.clone();

        // Wrap CPU-bound synchronous fastembed execution in spawn_blocking
        let embeddings = task::spawn_blocking(move || {
            let mut embedder_guard = embedder.blocking_lock();
            embedder_guard.embed(steps_text, None)
        }).await.map_err(|_| DriftWarning::TargetDrift)? // Treat spawn_blocking panic as TargetDrift for simplicity
        .map_err(|_| DriftWarning::TargetDrift)?;

        if embeddings.len() >= 2 {
            let sim = Self::calculate_cosine_similarity(&embeddings[0], &embeddings[1]);
            if sim >= self.similarity_threshold {
                return Err(DriftWarning::SemanticLoop);
            }
        }

        Ok(())
    }

    async fn distill_experience(&self, trace: &ExecutionTrace) -> Option<SynthesizedSkill> {
        if !trace.success {
            return None; // Only distill successful traces
        }

        // Extremely naive form of distillation for now
        // A full actor-critic would parse multiple traces, map inputs to tools, etc.
        // We distill the first step as a trigger, and the sequence of node_ids as code.

        let first_step = trace.steps.first()?;
        let trigger_condition = format!("Matches input intent similar to: {}", first_step.input_data);

        let node_sequence: Vec<String> = trace.steps.iter().map(|s| s.node_id.clone()).collect();
        let executable_code = format!("Execute sequence: [{}]", node_sequence.join(" -> "));

        Some(SynthesizedSkill {
            trigger_condition,
            executable_code,
            success_rate: 1.0, // Initial success rate
        })
    }
}
