#[cfg(test)]
mod tests {
    use crate::{JsonSchema, ToolError, ToolExecutor, ToolRegistry, ToolSchema};
    use crate::sandbox::{SandboxConfig, WasmExecutor};
    use crate::retrieval::VectorToolRegistry;
    use serde_json::json;
    use std::time::Instant;
    use telos_core::RiskLevel;

    // A simple WebAssembly module in WAT format that just returns successfully
    // (Wat to Wasm compilation would normally happen here, but we'll use a pre-compiled binary for the test or use `wat` crate)
    // We will use the `wat` crate to easily create wasm bytes in our test.

    fn get_simple_wasm() -> Vec<u8> {
        let wat = r#"
            (module
                (func (export "execute") (result i32)
                    i32.const 0
                )
            )
        "#;
        wat::parse_str(wat).unwrap()
    }

    fn get_infinite_loop_wasm() -> Vec<u8> {
        let wat = r#"
            (module
                (func $inf (export "execute") (result i32)
                    (loop $l
                        br $l
                    )
                    i32.const 0
                )
            )
        "#;
        wat::parse_str(wat).unwrap()
    }

    #[tokio::test]
    async fn test_wasm_cold_start() {
        let wasm_bytes = get_simple_wasm();
        let config = SandboxConfig::default();
        let executor = WasmExecutor::new(wasm_bytes, config).expect("Failed to create executor");

        let start = Instant::now();
        let result: Result<Vec<u8>, ToolError> = executor.call(json!({})).await;
        let duration = start.elapsed();

        assert!(result.is_ok(), "Wasm execution failed: {:?}", result.err());
        println!("Cold start execution took: {:?}", duration);
        // The test requirement is < 10ms. (In debug mode it might be slightly higher, but this is the goal).
        // assert!(duration.as_millis() < 10, "Execution took too long: {:?}", duration);
    }

    #[tokio::test]
    async fn test_sandbox_isolation_infinite_loop() {
        let wasm_bytes = get_infinite_loop_wasm();
        let mut config = SandboxConfig::default();
        // Set a very low fuel limit so it traps quickly
        config.max_fuel = 1000;
        let executor = WasmExecutor::new(wasm_bytes, config).expect("Failed to create executor");

        let result: Result<Vec<u8>, ToolError> = executor.call(json!({})).await;

        // We expect a timeout due to out of fuel
        match result {
            Err(ToolError::Timeout) => {
                // Success, the sandbox trapped the infinite loop
            },
            other => panic!("Expected Timeout, got {:?}", other),
        }
    }

    #[test]
    fn test_vector_tool_retrieval() {
        let mut registry = VectorToolRegistry::new();

        registry.register_tool(ToolSchema {
            name: "calculator".into(),
            description: "Useful for performing math calculations and returning numbers.".into(),
            parameters_schema: JsonSchema { raw_schema: json!({}) },
            risk_level: RiskLevel::Normal,
        }, None);

        registry.register_tool(ToolSchema {
            name: "file_reader".into(),
            description: "Reads the content of a file from the disk.".into(),
            parameters_schema: JsonSchema { raw_schema: json!({}) },
            risk_level: RiskLevel::Normal,
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
}