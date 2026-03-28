use serde::{Deserialize, Serialize};

// ============================================================================
// L1: Product & Business Layer
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProductFeature {
    pub id: String,
    pub title: String,
    pub description: String,
    /// Vertical depth: what must be true for this to be accepted
    pub acceptance_criteria: Vec<String>,
    /// Horizontal breadth: how this interacts with other features
    pub user_journey_flows: Vec<String>,
    pub status: MetaStatus,
}

// ============================================================================
// L2: Tech & Contract Layer (The Absolute Guardrails)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TechModule {
    pub id: String,
    /// Vertical mapping: which product feature this supports
    pub mapped_feature_id: String,
    pub name: String,
    /// Vertical mapping: Physical directory boundary
    pub directory_path: String,
    pub status: MetaStatus,
}

/// A rigid schema boundary negotiated between modules.
/// It acts as the immutable law that the LLM must follow.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Contract {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Who provides this interface
    pub provider_module_id: String,
    /// Who consumes this interface
    pub consumer_module_ids: Vec<String>,
    /// The strict schema definition (e.g. OpenAPI, Protobuf, or strict JSON Schema)
    pub schema_definition: serde_json::Value,
    pub status: ContractStatus,
}

// ============================================================================
// L3: Task & Execution Layer
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DevTask {
    pub id: String,
    pub title: String,
    pub belong_to_module: String,
    pub target_file: String,
    /// Specific instructions for the WorkerAgent
    pub instruction: String,
    /// The most critical field: execution of this task MUST comply with these contracts
    pub enforced_contracts: Vec<String>,
    pub status: TaskStatus,
    /// Harness feedback on failures, passed back as context progressively
    #[serde(default)]
    pub harness_feedback: Vec<String>,
}

// ============================================================================
// Unified Graph Enums
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum MetaStatus {
    #[default]
    Proposed,
    Approved,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum ContractStatus {
    #[default]
    Draft,
    /// A Locked contract CANNOT be mutated by the WorkerAgent.
    Locked,
    Deprecated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum TaskStatus {
    #[default]
    Todo,
    InProgress,
    /// Validating against harness contracts
    InQA,
    Done,
    Blocked,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_product_feature_serialization() {
        let feature = ProductFeature {
            id: "feat_123".to_string(),
            title: "User Login".to_string(),
            description: "Allows users to log into the platform".to_string(),
            acceptance_criteria: vec!["Must support SSO".to_string(), "Requires MFA".to_string()],
            user_journey_flows: vec!["Login -> Dashboard".to_string()],
            status: MetaStatus::Proposed,
        };

        let json = serde_json::to_string(&feature).unwrap();
        let deserialized: ProductFeature = serde_json::from_str(&json).unwrap();
        assert_eq!(feature, deserialized);
    }

    #[test]
    fn test_contract_serialization() {
        let contract = Contract {
            id: "contract_auth".to_string(),
            name: "AuthInterface".to_string(),
            description: "Login schema".to_string(),
            provider_module_id: "mod_auth".to_string(),
            consumer_module_ids: vec!["mod_frontend".to_string()],
            schema_definition: serde_json::json!({
                "type": "object",
                "properties": {
                    "token": { "type": "string" }
                }
            }),
            status: ContractStatus::Locked,
        };

        let json = serde_json::to_string(&contract).unwrap();
        let deserialized: Contract = serde_json::from_str(&json).unwrap();
        assert_eq!(contract, deserialized);
        assert_eq!(deserialized.schema_definition["type"], "object");
    }

    #[test]
    fn test_dev_task_serialization() {
        let task = DevTask {
            id: "task_1".to_string(),
            title: "Implement Login Handler".to_string(),
            belong_to_module: "mod_auth".to_string(),
            target_file: "src/auth/handler.rs".to_string(),
            instruction: "Implement JWT generation".to_string(),
            enforced_contracts: vec!["contract_auth".to_string()],
            status: TaskStatus::Todo,
            harness_feedback: vec![],
        };

        let json = serde_json::to_string(&task).unwrap();
        let deserialized: DevTask = serde_json::from_str(&json).unwrap();
        assert_eq!(task, deserialized);
        assert_eq!(task.enforced_contracts.len(), 1);
    }
}
