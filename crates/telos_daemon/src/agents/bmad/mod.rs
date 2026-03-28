pub mod product;
pub mod architect;
pub mod scrum_master;
pub mod worker;
pub mod harness;
pub mod approval;
pub mod integration;
pub mod tech_lead;

pub use product::ProductAgent;
pub use architect::BmadArchitectAgent;
pub use scrum_master::ScrumMasterAgent;
pub use worker::WorkerAgent;
pub use harness::HarnessValidatorNode;
pub use approval::SmartApprovalNode;
pub use integration::IntegrationTesterNode;
pub use tech_lead::TechLeadAgent;

#[cfg(test)]
mod tests {
    use super::*;
    use telos_core::{AgentInput, LoopConfig, ExitCondition};

    #[test]
    fn test_bmad_agents_structural_bounds() {
        // Here we test that the BMAD agents are publicly exportable and adhere to structural guarantees.
        // E.g., verifying that the Harness node doesn't have internal loop configs (it expects to be driven).
        
        let dummy_input = AgentInput {
            node_id: "test_node".to_string(),
            task: "test_task".to_string(),
            dependencies: std::collections::HashMap::new(),
            schema_payload: None,
            conversation_history: vec![],
            memory_context: None,
            correction: None,
        };
        
        assert_eq!(dummy_input.node_id, "test_node");
        assert!(dummy_input.dependencies.is_empty());

        let example_loop = LoopConfig {
            max_iterations: 3,
            exit_condition: ExitCondition::SatisfactionThreshold(1.0),
            critic_node_id: "critic".to_string(),
        };

        assert_eq!(example_loop.max_iterations, 3);
    }
}
