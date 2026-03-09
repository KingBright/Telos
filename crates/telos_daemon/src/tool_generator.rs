use std::fs;
use std::path::Path;
use std::process::Command;

/// Tool generator that can create and compile new tools dynamically
pub struct ToolGenerator {
    tools_dir: String,
}

impl ToolGenerator {
    pub fn new(tools_dir: &str) -> Self {
        Self {
            tools_dir: tools_dir.to_string(),
        }
    }

    /// Generate a prompt for the LLM to create tool code
    pub fn generate_tool_creation_prompt(
        &self,
        tool_intent: &str,
        task_description: &str,
    ) -> String {
        format!(
            r#"You are a Rust tool generator. Create a tool that can handle the following intent:

**Intent**: {}
**Task Context**: {}

Generate a Rust tool that implements the `ToolExecutor` trait. The tool should:
1. Have a descriptive name (snake_case)
2. Accept relevant parameters as JSON
3. Return the result as bytes

Available dependencies:
- `telos_tooling::{{ToolError, ToolExecutor, ToolSchema, JsonSchema}}`
- `async_trait::async_trait`
- `serde_json::Value`

Respond with ONLY the Rust code (no markdown blocks). The code should follow this template:

```rust
use telos_tooling::{{ToolError, ToolExecutor, ToolSchema, JsonSchema}};
use async_trait::async_trait;
use serde_json::Value;

#[derive(Clone)]
pub struct MyTool;

#[async_trait]
impl ToolExecutor for MyTool {{
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {{
        // Extract parameters
        let param1 = params.get("param1")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing param1".into()))?;

        // Execute tool logic
        let result = format!("Result: {{}}", param1);
        Ok(result.into_bytes())
    }}
}}

impl MyTool {{
    pub fn schema() -> ToolSchema {{
        ToolSchema {{
            name: "my_tool".into(),
            description: "Tool description".into(),
            parameters_schema: JsonSchema {{
                raw_schema: serde_json::json!({{
                    "type": "object",
                    "properties": {{
                        "param1": {{ "type": "string" }}
                    }},
                    "required": ["param1"]
                }}),
            }},
            risk_level: telos_core::RiskLevel::Normal,
        }}
    }}
}}
```

Now generate the tool code:"#,
            tool_intent, task_description
        )
    }

    /// Parse the tool name from generated code
    pub fn extract_tool_name(code: &str) -> Option<String> {
        // Find "pub struct XxxTool" pattern
        if let Some(start) = code.find("pub struct ") {
            let rest = &code[start + 11..];
            if let Some(end) = rest.find("Tool") {
                let name = rest[..end].to_string();
                // Convert to snake_case
                let snake = name
                    .chars()
                    .enumerate()
                    .map(|(i, c)| {
                        if c.is_uppercase() && i > 0 {
                            format!("_{}", c.to_lowercase())
                        } else {
                            c.to_lowercase().to_string()
                        }
                    })
                    .collect::<String>();
                return Some(snake.trim_start_matches('_').to_string());
            }
        }
        None
    }

    /// Create a Cargo.toml for the new tool
    pub fn create_cargo_toml(&self, tool_name: &str, telos_path: &str) -> Result<String, String> {
        Ok(format!(
            r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
telos-tooling = {{ path = "{}/crates/telos_tooling", default-features = false }}
telos-core = {{ path = "{}/crates/telos_core", default-features = false }}
async-trait = "0.1"
serde_json = "1.0"

[profile.release]
opt-level = "s"
lto = true
"#,
            tool_name, telos_path, telos_path
        ))
    }

    /// Save tool code to disk for compilation
    pub fn save_tool_code(
        &self,
        tool_name: &str,
        code: &str,
        telos_path: &str,
    ) -> Result<String, String> {
        let tool_dir = format!("{}/gen_{}", self.tools_dir, tool_name);
        let src_dir = format!("{}/src", tool_dir);

        fs::create_dir_all(&src_dir)
            .map_err(|e| format!("Failed to create tool directory: {}", e))?;

        // Write Cargo.toml
        let cargo_toml = self.create_cargo_toml(tool_name, telos_path)?;
        fs::write(format!("{}/Cargo.toml", tool_dir), cargo_toml)
            .map_err(|e| format!("Failed to write Cargo.toml: {}", e))?;

        // Write lib.rs
        fs::write(format!("{}/lib.rs", src_dir), code)
            .map_err(|e| format!("Failed to write lib.rs: {}", e))?;

        Ok(tool_dir)
    }

    /// Compile tool to Wasm (requires wasm32-unknown-unknown target)
    pub fn compile_to_wasm(&self, tool_dir: &str, tool_name: &str) -> Result<String, String> {
        println!("[ToolGenerator] Compiling tool '{}' to Wasm...", tool_name);

        let output = Command::new("cargo")
            .args(["build", "--release", "--target", "wasm32-unknown-unknown"])
            .current_dir(tool_dir)
            .output()
            .map_err(|e| format!("Failed to run cargo: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Compilation failed:\n{}", stderr));
        }

        // Find the compiled wasm
        let wasm_name = tool_name.replace("-", "_");
        let wasm_path = format!(
            "{}/target/wasm32-unknown-unknown/release/{}.wasm",
            tool_dir, wasm_name
        );

        if !Path::new(&wasm_path).exists() {
            return Err(format!("Compiled wasm not found at: {}", wasm_path));
        }

        // Copy to tools directory
        let dest_path = format!("{}/{}.wasm", self.tools_dir, tool_name);
        fs::copy(&wasm_path, &dest_path)
            .map_err(|e| format!("Failed to copy wasm: {}", e))?;

        // Cleanup build directory
        let _ = fs::remove_dir_all(tool_dir);

        println!("[ToolGenerator] Successfully compiled: {}", dest_path);
        Ok(dest_path)
    }

    /// Get the Telos project path
    pub fn get_telos_path() -> Option<String> {
        // Try common locations
        let home = dirs::home_dir()?;
        let telos_path = format!("{}/Workspace/Telos", home.display());
        if Path::new(&telos_path).exists() {
            return Some(telos_path);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_tool_name() {
        let code = r#"pub struct WeatherFetchTool"#;
        let name = ToolGenerator::extract_tool_name(code);
        assert_eq!(name, Some("weather_fetch".to_string()));
    }

    #[test]
    fn test_generate_tool_creation_prompt() {
        let gen = ToolGenerator::new("/tmp/tools");
        let prompt = gen.generate_tool_creation_prompt(
            "fetch weather",
            "Get current weather for a location",
        );
        assert!(prompt.contains("fetch weather"));
        assert!(prompt.contains("ToolExecutor"));
    }
}
