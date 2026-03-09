    use crate::{JsonSchema, ScriptSandbox, ScriptExecutor, ToolExecutor, ToolRegistry, ToolSchema};
    use crate::retrieval::VectorToolRegistry;
    use serde_json::json;
    use std::sync::Arc;
    use telos_core::RiskLevel;

    #[tokio::test]
    async fn test_rhai_script_execution() {
        let sandbox = Arc::new(ScriptSandbox::new());
        let script = r#"
            let x = params.a;
            let y = params.b;
            x + y
        "#;
        let executor = ScriptExecutor::new(script.to_string(), sandbox);
        
        let params = json!({ "a": 10, "b": 20 });
        let result = executor.call(params).await.expect("Script failed");
        
        let val: serde_json::Value = serde_json::from_slice(&result).unwrap();
        assert_eq!(val, json!(30));
    }

    #[tokio::test]
    async fn test_rhai_error_handling() {
        let sandbox = Arc::new(ScriptSandbox::new());
        let script = r#"
             throw "something went wrong";
        "#;
        let executor = ScriptExecutor::new(script.to_string(), sandbox);
        
        let result = executor.call(json!({})).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_vector_tool_retrieval() {
        let mut registry = VectorToolRegistry::new();

        registry.register_tool(ToolSchema {
            name: "calculator".into(),
            description: "Useful for performing math calculations and returning numbers.".into(),
            parameters_schema: JsonSchema { raw_schema: json!({}) },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }, None);

        registry.register_tool(ToolSchema {
            name: "file_reader".into(),
            description: "Reads the content of a file from the disk.".into(),
            parameters_schema: JsonSchema { raw_schema: json!({}) },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }, None);

        // Query for math intent
        let math_tools = registry.discover_tools("I need to add two numbers together", 1);
        assert_eq!(math_tools.len(), 1);
        assert_eq!(math_tools[0].name, "calculator");

        // Query for file intent
        let file_tools = registry.discover_tools("Please read the log file for me", 1);
        assert_eq!(file_tools.len(), 1);
        assert_eq!(file_tools[0].name, "file_reader");
    }
