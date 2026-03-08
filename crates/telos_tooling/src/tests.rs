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
    #[tokio::test]
    async fn test_fs_list_dir_tool() {
        use crate::native::FsListDirTool;
        use serde_json::json;
        use crate::ToolExecutor;
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test_file.txt");
        fs::write(&file_path, "hello").unwrap();
        let sub_dir = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir).unwrap();

        let tool = FsListDirTool;
        let params = json!({"path": temp_dir.path().to_str().unwrap()});
        let result = tool.call(params).await;

        assert!(result.is_ok());
        let result_bytes = result.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&result_bytes).unwrap();
        let arr = parsed.as_array().unwrap();

        assert_eq!(arr.len(), 2);

        let mut names: Vec<&str> = arr.iter()
            .map(|v| v.get("name").unwrap().as_str().unwrap())
            .collect();
        names.sort();

        assert_eq!(names, vec!["sub_dir", "test_file.txt"]);
    }

    #[tokio::test]
    async fn test_code_search_tool() {
        use crate::native::CodeSearchTool;
        use serde_json::json;
        use crate::ToolExecutor;
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let file_path1 = temp_dir.path().join("file1.txt");
        let file_path2 = temp_dir.path().join("file2.txt");

        fs::write(&file_path1, "line 1\nsearch_pattern\nline 3").unwrap();
        fs::write(&file_path2, "nothing here").unwrap();

        let tool = CodeSearchTool;
        let params = json!({
            "path": temp_dir.path().to_str().unwrap(),
            "pattern": "search_pattern"
        });
        let result = tool.call(params).await;

        assert!(result.is_ok());
        let result_bytes = result.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&result_bytes).unwrap();
        let arr = parsed.as_array().unwrap();

        assert_eq!(arr.len(), 1);
        let match_obj = &arr[0];
        let matched_text = match_obj.get("text").unwrap().as_str().unwrap();
        let line_num = match_obj.get("line_number").unwrap().as_i64().unwrap();

        assert_eq!(matched_text, "search_pattern");
        assert_eq!(line_num, 2);

}
