use crate::{JsonSchema, ToolError, ToolExecutor, ToolSchema};
use tracing::{info, debug, warn, error};
use async_trait::async_trait;
use serde_json::Value;
use telos_core::RiskLevel;
use crate::ToolRegistry;
// 4. Calculator Tool
#[derive(Clone)]
pub struct CalculatorTool;

#[async_trait]
impl ToolExecutor for CalculatorTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let expression = params
            .get("expression")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'expression' parameter".into()))?;

        // Use evalexpr for safe mathematical expression evaluation
        // For now, use a simple approach with basic operations
        let result = Self::evaluate_expression(expression)?;

        let output = serde_json::json!({
            "expression": expression,
            "result": result
        });

        Ok(serde_json::to_vec(&output)
            .unwrap_or_else(|_| format!("{{\"result\": {}}}", result).into_bytes()))
    }
}

impl CalculatorTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "calculator".into(),
            description: "Evaluates mathematical expressions. Supports basic operations (+, -, *, /, ^), functions (sqrt, sin, cos, tan, log, exp, abs), and constants (pi, e). Requires an 'expression' parameter.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "expression": {
                            "type": "string",
                            "description": "Mathematical expression to evaluate, e.g., '2+2', 'sqrt(16)', 'sin(pi/2)'"
                        }
                    },
                    "required": ["expression"]
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }

    /// Safely evaluate a mathematical expression
    fn evaluate_expression(expr: &str) -> Result<f64, ToolError> {
        let expr = expr.trim().replace(" ", "");

        // Handle common constants
        let expr = expr.replace("pi", &std::f64::consts::PI.to_string());
        let expr = expr.replace("e", &std::f64::consts::E.to_string());

        // Simple recursive descent parser for basic expressions
        Self::parse_expression(&expr, &mut 0)
    }

    fn parse_expression(expr: &str, pos: &mut usize) -> Result<f64, ToolError> {
        let mut result = Self::parse_term(expr, pos)?;

        while *pos < expr.len() {
            let c = expr.chars().nth(*pos).unwrap();
            if c == '+' {
                *pos += 1;
                let term = Self::parse_term(expr, pos)?;
                result += term;
            } else if c == '-' {
                *pos += 1;
                let term = Self::parse_term(expr, pos)?;
                result -= term;
            } else {
                break;
            }
        }

        Ok(result)
    }

    fn parse_term(expr: &str, pos: &mut usize) -> Result<f64, ToolError> {
        let mut result = Self::parse_factor(expr, pos)?;

        while *pos < expr.len() {
            let c = expr.chars().nth(*pos).unwrap();
            if c == '*' {
                *pos += 1;
                let factor = Self::parse_factor(expr, pos)?;
                result *= factor;
            } else if c == '/' {
                *pos += 1;
                let factor = Self::parse_factor(expr, pos)?;
                if factor == 0.0 {
                    return Err(ToolError::ExecutionFailed("Division by zero".into()));
                }
                result /= factor;
            } else if c == '^' {
                *pos += 1;
                let factor = Self::parse_factor(expr, pos)?;
                result = result.powf(factor);
            } else {
                break;
            }
        }

        Ok(result)
    }

    fn parse_factor(expr: &str, pos: &mut usize) -> Result<f64, ToolError> {
        // Skip whitespace (already removed, but just in case)
        while *pos < expr.len() && expr.chars().nth(*pos).unwrap().is_whitespace() {
            *pos += 1;
        }

        if *pos >= expr.len() {
            return Err(ToolError::ExecutionFailed(
                "Unexpected end of expression".into(),
            ));
        }

        let c = expr.chars().nth(*pos).unwrap();

        // Handle negative numbers
        if c == '-' {
            *pos += 1;
            return Ok(-Self::parse_factor(expr, pos)?);
        }

        // Handle parentheses
        if c == '(' {
            *pos += 1;
            let result = Self::parse_expression(expr, pos)?;
            if *pos >= expr.len() || expr.chars().nth(*pos).unwrap() != ')' {
                return Err(ToolError::ExecutionFailed(
                    "Missing closing parenthesis".into(),
                ));
            }
            *pos += 1;
            return Ok(result);
        }

        // Handle functions
        if c.is_alphabetic() {
            let start = *pos;
            while *pos < expr.len()
                && (expr.chars().nth(*pos).unwrap().is_alphanumeric()
                    || expr.chars().nth(*pos).unwrap() == '_')
            {
                *pos += 1;
            }
            let func_name = &expr[start..*pos];

            if *pos < expr.len() && expr.chars().nth(*pos).unwrap() == '(' {
                *pos += 1;
                let arg = Self::parse_expression(expr, pos)?;
                if *pos >= expr.len() || expr.chars().nth(*pos).unwrap() != ')' {
                    return Err(ToolError::ExecutionFailed(
                        "Missing closing parenthesis for function".into(),
                    ));
                }
                *pos += 1;

                let result = match func_name {
                    "sqrt" => arg.sqrt(),
                    "sin" => arg.sin(),
                    "cos" => arg.cos(),
                    "tan" => arg.tan(),
                    "log" => arg.ln(),
                    "log10" => arg.log10(),
                    "exp" => arg.exp(),
                    "abs" => arg.abs(),
                    "floor" => arg.floor(),
                    "ceil" => arg.ceil(),
                    "round" => arg.round(),
                    _ => {
                        return Err(ToolError::ExecutionFailed(format!(
                            "Unknown function: {}",
                            func_name
                        )))
                    }
                };
                return Ok(result);
            }
        }

        // Handle numbers
        let start = *pos;
        while *pos < expr.len() {
            let c = expr.chars().nth(*pos).unwrap();
            if c.is_ascii_digit() || c == '.' {
                *pos += 1;
            } else {
                break;
            }
        }

        if start == *pos {
            return Err(ToolError::ExecutionFailed(format!(
                "Expected number at position {}",
                *pos
            )));
        }

        let num_str = &expr[start..*pos];
        num_str
            .parse::<f64>()
            .map_err(|e| ToolError::ExecutionFailed(format!("Invalid number '{}': {}", num_str, e)))
    }
}

// 5. Tool Register Tool
// Allows dynamic registration of newly compiled Wasm modules.
// Because it needs to mutate the registry, we'll implement this with a reference or a specific API design later,
// as the registry is behind RwLock and managed globally.
// For now, returning a schema indicator so the DAEMON knows to handle it specifically as a macro-tool.
#[derive(Clone)]
pub struct ToolRegisterTool;

#[async_trait]
impl ToolExecutor for ToolRegisterTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        // This tool is currently a "Macro" intercepted by the daemon or handled via a dedicated channel.
        // It outputs the request for the host to process the actual registry mutation.
        let wasm_path = params
            .get("wasm_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'wasm_path' parameter".into()))?;

        let schema_json = params
            .get("schema")
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'schema' parameter".into()))?;

        let _name = schema_json
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("unknown");

        // We serialize this special JSON so the host (Daemon) can intercept it in the ExecutionEngine
        let out = serde_json::json!({
            "__macro__": "register_tool",
            "wasm_path": wasm_path,
            "schema": schema_json
        });

        Ok(serde_json::to_vec(&out).unwrap())
    }
}

impl ToolRegisterTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "register_tool".into(),
            description: "Registers a newly compiled WebAssembly tool into the system registry. Requires 'wasm_path' and 'schema' parameters.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "wasm_path": { "type": "string", "description": "Absolute or relative path to the .wasm file" },
                        "schema": { "type": "object", "description": "The ToolSchema JSON describing the new tool" }
                    },
                    "required": ["wasm_path", "schema"]
                }),
            },
            risk_level: RiskLevel::HighRisk,
            ..Default::default()
        }
    }
}


// 13. Lsp Tool
#[derive(Clone)]
pub struct LspTool;

#[async_trait]
impl ToolExecutor for LspTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let symbol = params
            .get("symbol")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'symbol'".into()))?;

        let pattern = format!("(fn|struct|enum|trait) {}", symbol);
        let output = tokio::process::Command::new("grep")
            .arg("-rnE")
            .arg(&pattern)
            .arg(".")
            .output()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Command failed: {}", e)))?;

        Ok(output.stdout)
    }
}

impl LspTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "lsp_symbol_search".into(),
            description:
                "Finds definitions of symbols (structs, fns, enums) across the repository.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "symbol": { "type": "string" }
                    },
                    "required": ["symbol"]
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}

// 18. List/Inspect Rhai Tools
#[derive(Clone)]
#[cfg(feature = "full")]
pub struct ListRhaiTools {
    plugins_dir: String,
}

#[cfg(feature = "full")]
impl ListRhaiTools {
    pub fn new(plugins_dir: String) -> Self {
        Self { plugins_dir }
    }

    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "list_rhai_tools".into(),
            description: "Lists all custom Rhai tools, or inspects a specific tool's source code and schema. \
            Use this BEFORE modifying a tool to see its current implementation.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Optional. If provided, returns the full source code and schema of this specific tool. If omitted, lists all custom tools." }
                    }
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}

#[cfg(feature = "full")]
#[async_trait]
impl ToolExecutor for ListRhaiTools {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let plugins_dir = std::path::Path::new(&self.plugins_dir);
        let name_filter = params.get("name").and_then(|v| v.as_str());

        if let Some(name) = name_filter {
            // Specific tool — return full details including source code
            let json_path = plugins_dir.join(format!("{}.json", name));
            let rhai_path = plugins_dir.join(format!("{}.rhai", name));

            if !json_path.exists() {
                return Err(ToolError::ExecutionFailed(format!("Tool '{}' not found in plugins directory", name)));
            }

            let schema_str = std::fs::read_to_string(&json_path)
                .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read schema: {}", e)))?;
            let source_code = if rhai_path.exists() {
                std::fs::read_to_string(&rhai_path)
                    .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read source: {}", e)))?
            } else {
                "(source code not found)".to_string()
            };

            let schema: Value = serde_json::from_str(&schema_str).unwrap_or(Value::Null);

            let out = serde_json::json!({
                "name": name,
                "description": schema.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                "parameters_schema": schema.get("parameters_schema"),
                "rhai_source_code": source_code,
            });
            Ok(serde_json::to_vec_pretty(&out).unwrap())
        } else {
            // List all custom tools
            let mut tools = Vec::new();
            if plugins_dir.exists() {
                if let Ok(entries) = std::fs::read_dir(plugins_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().and_then(|e| e.to_str()) == Some("json") {
                            if let Ok(content) = std::fs::read_to_string(&path) {
                                if let Ok(schema) = serde_json::from_str::<Value>(&content) {
                                    let name = schema.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                                    let desc = schema.get("description").and_then(|v| v.as_str()).unwrap_or("");
                                    tools.push(serde_json::json!({
                                        "name": name,
                                        "description": desc,
                                    }));
                                }
                            }
                        }
                    }
                }
            }

            let out = serde_json::json!({
                "custom_tools_count": tools.len(),
                "tools": tools,
            });
            Ok(serde_json::to_vec_pretty(&out).unwrap())
        }
    }
}


// 17. Create Rhai Tool
#[derive(Clone)]
#[cfg(feature = "full")]
pub struct CreateRhaiTool {
    registry: std::sync::Arc<dyn ToolRegistry>,
}

#[cfg(feature = "full")]
impl CreateRhaiTool {
    pub fn new(registry: std::sync::Arc<dyn ToolRegistry>) -> Self {
        Self { registry }
    }

    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "create_rhai_tool".into(),
            description: "Creates, tests, and permanently registers a new Rhai script tool. \
            If a tool with the same name already exists, it will be OVERWRITTEN (use this for tool iteration/updates). \
            \n\
            DESIGN PRINCIPLE: Tools should ONLY FETCH data. Keep scripts simple — the LLM will interpret the output. \
            Do NOT write complex parsing/formatting logic; just return raw data. \
            \n\
            CRITICAL: Rhai is a sandboxed scripting language. It is NOT Rust, NOT JavaScript, NOT Python! \
            - NO explicit types, NO imports/crates, NO `use` statements. \
            - ONLY these built-in functions are available: \
              • `http_get(url)` → String (single HTTP GET, 10s timeout) \
              • `http_get_with_fallback(urls_json_array)` → String (tries each URL until one succeeds) \
              • `parse_json(text)` → Object (throws error if not valid JSON) \
              • `try_parse_json(text)` → Object|String (returns parsed object OR original string if parsing fails — PREFERRED for safety) \
              • `to_json(obj)` → String \
            - `params` is the input object map based on your parameters_schema. \
            - Last expression without `;` is the return value. \
            \n\
            RHAI SYNTAX ESSENTIALS: \
            - Every statement MUST end with `;` \
            - String interpolation: backtick `Hello ${name}` \
            - NO ternary `? :` → use `if expr { a } else { b }` \
            - NO `null` → use `()` \
            - Check key exists: `if \"key\" in map { map[\"key\"] } else { \"default\" }` \
            - Arrays: `let arr = [1, 2]; arr[0]` \
            - For loops: `for item in array { ... }` \
            \n\
            WORKING EXAMPLE (weather tool — uses open-meteo.com, reliable globally and in China, no API key): \n\
            ```rhai\n\
            let city = params[\"city\"];\n\
            // Coordinates must be resolved separately; here we show a direct example for Suzhou\n\
            let url = `https://api.open-meteo.com/v1/forecast?latitude=31.30&longitude=120.62&current=temperature_2m,weather_code,wind_speed_10m,relative_humidity_2m&timezone=Asia/Shanghai`;\n\
            let body = http_get(url);\n\
            let data = try_parse_json(body);\n\
            if data.is_string() {\n\
              data\n\
            } else {\n\
              let current = data[\"current\"];\n\
              `${city}: ${current[\"temperature_2m\"]}°C, humidity ${current[\"relative_humidity_2m\"]}%, wind ${current[\"wind_speed_10m\"]}km/h`\n\
            }\n\
            ```\n\
            The script is tested with `test_params` FIRST. Test passes if the script executes and returns any non-empty value (raw text is acceptable). \
            If test fails, re-create with simpler code.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Snake_case name of the new tool, e.g. 'get_weather'. If a tool with this name exists, it will be overwritten." },
                        "description": { "type": "string", "description": "Detailed description of what the tool does." },
                        "parameters_schema": { "type": "string", "description": "JSON string of the tool's input schema." },
                        "rhai_code": { "type": "string", "description": "The actual Rhai script code. Keep it SIMPLE: fetch data, optionally try_parse_json, return result." },
                        "test_params": { "type": "string", "description": "JSON string of REAL parameters to test the script with before registering. Use realistic values." }
                    },
                    "required": ["name", "description", "parameters_schema", "rhai_code", "test_params"]
                }),
            },
            risk_level: RiskLevel::HighRisk,
            ..Default::default()
        }
    }
}

#[cfg(feature = "full")]
#[async_trait]
impl ToolExecutor for CreateRhaiTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let name = params.get("name").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'name'".into()))?.to_string();
        
        // --- PRODUCTION NAME VALIDATION ---
        // Enforce clean, production-ready tool names at system level
        if let Err(reason) = validate_tool_name(&name) {
            let out = serde_json::json!({
                "status": "invalid_name",
                "message": format!("Tool name '{}' rejected: {}. Choose a clean, descriptive snake_case name like 'get_weather', 'calculate_tax', 'fetch_stock_price'.", name, reason),
            });
            return Ok(serde_json::to_vec(&out).unwrap());
        }
        
        let description = params.get("description").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'description'".into()))?.to_string();
        let schema_str = params.get("parameters_schema").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'parameters_schema'".into()))?;
        let rhai_code = params.get("rhai_code").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'rhai_code'".into()))?.to_string();
        let test_params_str = params.get("test_params").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'test_params'".into()))?;

        // Parse schemas
        let mut raw_schema: Value = serde_json::from_str(schema_str)
            .map_err(|e| ToolError::ExecutionFailed(format!("Invalid parameter schema JSON: {}", e)))?;
            
        // Fortification: Handle LLM double-encoding (stringified JSON)
        if let Value::String(s) = &raw_schema {
            raw_schema = serde_json::from_str(s)
                .map_err(|e| ToolError::ExecutionFailed(format!("Double-encoded schema is invalid JSON: {}", e)))?;
        }
        
        if !raw_schema.is_object() {
            return Err(ToolError::ExecutionFailed("parameters_schema must evaluate to a JSON Object, not a scalar.".into()));
        }

        let test_params: Value = serde_json::from_str(test_params_str)
            .map_err(|e| ToolError::ExecutionFailed(format!("Invalid test params JSON: {}", e)))?;

        // 1. Run the test in Sandbox — this IS the first real invocation
        use crate::script_sandbox::{ScriptSandbox, ScriptExecutor};
        use std::sync::Arc;
        
        let sandbox = Arc::new(ScriptSandbox::new());
        match sandbox.execute(&rhai_code, test_params) {
            Ok(test_result) => {
                // Format the test output as a readable preview for the Agent
                let test_output_str = serde_json::to_string_pretty(&test_result).unwrap_or_else(|_| test_result.to_string());
                let test_preview = if test_output_str.len() > 500 {
                    format!("{}... (truncated, {} total chars)", &test_output_str[..500], test_output_str.len())
                } else {
                    test_output_str.clone()
                };

                // --- TEST OUTPUT QUALITY GATE ---
                // Reject test outputs that indicate the script ran but got no useful data
                let trimmed = test_output_str.trim();
                let is_empty_output = test_result.is_null()
                    || trimmed == "{}"
                    || trimmed == "[]"
                    || trimmed == "\"\""
                    || trimmed.is_empty()
                    || trimmed == "\"()\"";
                
                if is_empty_output {
                    warn!("[CreateRhaiTool] Test for '{}' returned empty/null data: {}. Tool NOT registered.", name, test_preview);
                    crate::fire_tool_creation_hook(&name, false, false);
                    let out = serde_json::json!({
                        "status": "test_empty",
                        "message": format!(
                            "Tool '{}' script executed without errors, but returned EMPTY or null data ({}). \
                            The tool was NOT registered because it would produce useless results in production.",
                            name, test_preview
                        ),
                        "diagnosis_context": {
                            "rhai_code": rhai_code,
                            "test_params_used": test_params_str,
                            "raw_test_output": test_result,
                        },
                        "generic_debugging_hints": [
                            "Inspect the raw API/data source response: call http_get(url) separately and examine what it returns",
                            "The data source may be temporarily unavailable, returning empty responses, or blocked in this network region",
                            "Check if the response structure matches what the script expects (field names, nesting)",
                            "Simplify the script to just return the raw http_get response, verify data is coming through, then add parsing",
                            "Consider using http_get_with_fallback with multiple alternative URLs if the primary source is unreliable",
                        ],
                    });
                    return Ok(serde_json::to_vec(&out).unwrap());
                }

                info!("[CreateRhaiTool] Test passed for '{}'. Output preview: {}", name, test_preview);
                
                // --- VERSION LIFECYCLE MANAGEMENT ---
                // Check if tool already exists to determine version and iteration
                let existing = self.registry.get_schema(&name);
                let (version, iteration, change_reason) = if let Some(ref old) = existing {
                    // UPDATE: bump patch version, increment iteration
                    let new_version = bump_patch_version(&old.version);
                    let iter = old.iteration + 1;
                    let reason = format!("Updated (v{} → v{}): description or implementation changed", old.version, new_version);
                    info!("[CreateRhaiTool] Updating existing tool '{}': {} → {}, iteration {}", name, old.version, new_version, iter);
                    (new_version, iter, reason)
                } else {
                    // NEW: initial version
                    ("1.0.0".to_string(), 0, "Initial creation".to_string())
                };
                
                let new_schema = ToolSchema {
                    name: name.clone(),
                    description,
                    parameters_schema: JsonSchema { raw_schema },
                    risk_level: RiskLevel::Normal,
                    version: version.clone(),
                    iteration,
                    parent_tool: existing.as_ref().map(|_| name.clone()),
                    change_reason: Some(change_reason.clone()),
                    experience_notes: existing.as_ref().map(|e| e.experience_notes.clone()).unwrap_or_default(),
                };
                
                let rhai_code_for_response = rhai_code.clone();
                let new_executor = Arc::new(ScriptExecutor::new(rhai_code, sandbox));
                
                if let Err(e) = self.registry.register_dynamic_tool(new_schema, new_executor) {
                    crate::fire_tool_creation_hook(&name, false, existing.is_some());
                    return Err(ToolError::ExecutionFailed(format!("Failed to register tool into VectorToolRegistry: {}", e)));
                }
                crate::fire_tool_creation_hook(&name, true, existing.is_some());
                let is_update = existing.is_some();
                let out = serde_json::json!({
                    "status": "success",
                    "message": format!("Tool '{}' {} and permanently registered.", name, if is_update { "updated" } else { "created" }),
                    "version_info": {
                        "version": version,
                        "iteration": iteration,
                        "change_reason": change_reason,
                        "is_update": is_update,
                    },
                    "test_output": test_result,
                    "test_output_preview": test_preview,
                    "registered_tool_context": {
                        "rhai_code": rhai_code_for_response,
                        "test_params_used": test_params_str,
                    },
                    "review_checklist": "REVIEW the test_output above carefully: (1) Does it contain the expected data or just raw/unparsed text? (2) Is the output format useful for the end user? (3) If the test_output looks like HTML, ASCII art, or an error page instead of structured data, the script may need to add format parameters to the URL or use a different endpoint."
                });
                return Ok(serde_json::to_vec(&out).unwrap());
            }
            Err(e) => {
                let error_msg = format!("{:?}", e);
                warn!("[CreateRhaiTool] Script test FAILED for '{}': {}", name, error_msg);
                crate::fire_tool_creation_hook(&name, false, false);
                let out = serde_json::json!({
                    "status": "test_failed",
                    "message": format!(
                        "Script test FAILED for tool '{}'. The tool was NOT registered. \
                        Review the diagnosis_context below to understand what went wrong and fix the rhai_code.",
                        name
                    ),
                    "error": error_msg,
                    "diagnosis_context": {
                        "rhai_code": rhai_code,
                        "test_params_used": test_params_str,
                    },
                    "generic_debugging_hints": [
                        "Check if the error is a Rhai syntax issue (missing semicolons, wrong operators, invalid method calls)",
                        "Rhai is NOT Rust/JS/Python: no explicit types, no imports, no ternary ?: operator, no null (use ())",
                        "If the error is about accessing a field that doesn't exist, verify the data structure with a simpler script first",
                        "Network errors suggest the URL is unreachable — verify with http_get(url) separately",
                        "For timeout errors, try http_get_with_fallback with alternative URLs",
                    ],
                });
                return Ok(serde_json::to_vec(&out).unwrap());
            }
        }
    }
}

/// Bump the patch component of a semver version string (e.g. "1.0.2" → "1.0.3")
#[cfg(feature = "full")]
fn bump_patch_version(version: &str) -> String {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() == 3 {
        let major = parts[0];
        let minor = parts[1];
        let patch: u32 = parts[2].parse().unwrap_or(0);
        format!("{}.{}.{}", major, minor, patch + 1)
    } else {
        // Fallback: if version is malformed, start fresh
        "1.0.1".to_string()
    }
}

/// Validate tool names for production readiness.
/// Enforces: snake_case, 3-50 chars, starts with letter, no reserved prefixes.
#[cfg(feature = "full")]
fn validate_tool_name(name: &str) -> Result<(), String> {
    // Length check
    if name.len() < 3 {
        return Err("name too short (minimum 3 characters)".into());
    }
    if name.len() > 50 {
        return Err("name too long (maximum 50 characters)".into());
    }
    
    // Must start with a letter
    if !name.chars().next().map_or(false, |c| c.is_ascii_lowercase()) {
        return Err("must start with a lowercase letter".into());
    }
    
    // Only allow lowercase letters, digits, and underscores (snake_case)
    if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        return Err("must be snake_case (only lowercase letters, digits, underscores)".into());
    }
    
    // No double underscores
    if name.contains("__") {
        return Err("no double underscores allowed".into());
    }
    
    // No trailing underscore
    if name.ends_with('_') {
        return Err("must not end with underscore".into());
    }
    
    // Reserved prefixes — these are ephemeral/debug artifacts, not production tools
    const RESERVED_PREFIXES: &[&str] = &["debug_", "test_", "diag_", "simple_", "tmp_", "temp_", "scratch_"];
    for prefix in RESERVED_PREFIXES {
        if name.starts_with(prefix) {
            return Err(format!("prefix '{}' is reserved for ephemeral tools and cannot be used for production tools", prefix));
        }
    }
    
    Ok(())
}

// 19. Discover Tools (Progressive Exposure)
#[derive(Clone)]
#[cfg(feature = "full")]
pub struct DiscoverTools {
    registry: std::sync::Arc<dyn ToolRegistry>,
}

#[cfg(feature = "full")]
impl DiscoverTools {
    pub fn new(registry: std::sync::Arc<dyn ToolRegistry>) -> Self {
        Self { registry }
    }

    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "discover_tools".into(),
            description: "Discovers available tools in the system based on semantic search of your query. \
            Use this when you need a capability but don't have the right tool in your context. \
            It returns the ToolSchema including parameters and any experience_notes on how to use them.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "What you want to achieve, e.g. 'check weather in Hangzhou' or 'search files'" },
                        "top_k": { "type": "integer", "description": "Number of tools to return. Default is 5. Max is 10." }
                    },
                    "required": ["query"]
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}

#[cfg(feature = "full")]
#[async_trait]
impl ToolExecutor for DiscoverTools {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let query = params.get("query").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'query'".into()))?;
        
        let top_k = params.get("top_k").and_then(|v| v.as_u64()).unwrap_or(8).min(20) as usize; // Increased default to 8 as per user request
        
        let tools = self.registry.discover_tools(query, top_k);
        
        let out = serde_json::json!({
            "discovered_tools_count": tools.len(),
            "tools": tools,
        });
        
        Ok(serde_json::to_vec_pretty(&out).unwrap())
    }
}

// 20. Attach Tool Note
#[derive(Clone)]
#[cfg(feature = "full")]
pub struct AttachToolNote {
    registry: std::sync::Arc<dyn ToolRegistry>,
}

#[cfg(feature = "full")]
impl AttachToolNote {
    pub fn new(registry: std::sync::Arc<dyn ToolRegistry>) -> Self {
        Self { registry }
    }

    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "attach_tool_note".into(),
            description: "Appends a lesson learned or usage warning to a tool's experience notes. \
            These notes will be read by future agents discovering the tool to avoid repeating mistakes or to use it more effectively.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "tool_name": { "type": "string", "description": "The exact name of the tool to attach the note to" },
                        "note": { "type": "string", "description": "The experience note, clearly describing a caveat, a constraint, or a proven best practice" }
                    },
                    "required": ["tool_name", "note"]
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}

#[cfg(feature = "full")]
#[async_trait]
impl ToolExecutor for AttachToolNote {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let tool_name = params.get("tool_name").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'tool_name'".into()))?;
        
        let note = params.get("note").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'note'".into()))?;
        
        match self.registry.attach_tool_note(tool_name, note.to_string()) {
            Ok(_) => {
                let out = serde_json::json!({
                    "status": "success",
                    "message": format!("Successfully attached note to '{}'", tool_name)
                });
                Ok(serde_json::to_vec_pretty(&out).unwrap())
            }
            Err(e) => {
                Err(ToolError::ExecutionFailed(format!("Failed to attach note to tool: {}", e)))
            }
        }
    }
}
