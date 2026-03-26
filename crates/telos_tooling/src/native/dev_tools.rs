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
            If a tool with the same name already exists, it will be OVERWRITTEN. \
            \n\
            DESIGN PRINCIPLE: Keep scripts simple — fetch data and return it. The LLM will interpret the output. \
            \n\
            ══════════════════════════════════════════\n\
            RHAI COMPLETE SYNTAX REFERENCE\n\
            ══════════════════════════════════════════\n\
            Rhai is a SANDBOXED scripting language. It is NOT Rust, NOT JavaScript, NOT Python.\n\
            \n\
            ▸ VARIABLES: `let x = 42;` — No explicit types, no `mut` keyword.\n\
            ▸ SEMICOLONS: Every statement MUST end with `;` — the LAST expression (the return value) has NO `;`\n\
            ▸ STRINGS: Double quotes `\"hello\"` for literals. Backtick `` `hello ${name}` `` for interpolation.\n\
            ▸ MAP ACCESS: ALWAYS use bracket notation: `map[\"key\"]` — NEVER use dot notation `map.key` on parsed JSON.\n\
            ▸ ARRAYS: `let arr = [1, 2, 3]; arr[0];` — Zero-indexed.\n\
            ▸ IF/ELSE: `if condition { a } else { b }` — NO ternary `? :`.\n\
            ▸ FOR LOOP: `for item in array { ... }` \n\
            ▸ WHILE LOOP: `while condition { ... }` \n\
            ▸ STRING CONCAT: `\"hello\" + \" \" + \"world\"` or use backtick interpolation.\n\
            ▸ COMPARISON: `==`, `!=`, `<`, `>`, `<=`, `>=` — standard operators.\n\
            ▸ ARITHMETIC: `+`, `-`, `*`, `/`, `%` — integers supported natively, floats via arithmetic only.\n\
            ▸ BOOLEAN: `true`, `false`, `&&`, `||`, `!`.\n\
            ▸ NO `null` — use `()` for empty/nothing.\n\
            ▸ KEY CHECK: `if \"key\" in map { map[\"key\"] } else { \"default\" }` \n\
            ▸ TYPE CHECK: `value.is_string()`, `value.is_int()` etc. for type inspection.\n\
            ▸ LET RESULT: `let result = \"\"; if x { result = \"a\"; } else { result = \"b\"; } result` \n\
            \n\
            ▸ FORBIDDEN: No imports, no `use`, no `crate`, no `mod`, no explicit type annotations, \
              no `struct`, no `enum`, no `match`, no `null`, no ternary `? :`.\n\
            \n\
            ══════════════════════════════════════════\n\
            AVAILABLE BUILT-IN FUNCTIONS\n\
            ══════════════════════════════════════════\n\
            • `http_get(url)` → String — Single HTTP GET request (10s timeout).\n\
            • `http_get_with_fallback(urls_json_array)` → String — Tries each URL in the JSON array until one succeeds.\n\
            • `parse_json(text)` → Object — Parse JSON string. Throws error if invalid.\n\
            • `try_parse_json(text)` → Object|String — Safe parse: returns parsed object OR original string if invalid. PREFERRED.\n\
            • `to_json(obj)` → String — Convert object back to JSON string.\n\
            \n\
            INPUT: `params` is a map with keys from your `parameters_schema`. Access via `params[\"key\"]`.\n\
            OUTPUT: The last expression without `;` is the return value.\n\
            \n\
            ══════════════════════════════════════════\n\
            ANTI-PATTERNS (DO NOT DO THESE)\n\
            ══════════════════════════════════════════\n\
            ✗ `data.current` → Use `data[\"current\"]` instead (bracket notation for parsed JSON).\n\
            ✗ `let x: i64 = 5;` → Use `let x = 5;` (no type annotations).\n\
            ✗ `use std::collections;` → No imports allowed.\n\
            ✗ `x ? a : b` → Use `if x { a } else { b }`.\n\
            ✗ `null` → Use `()`.\n\
            ✗ `value.to_float()` → Not available. Do arithmetic directly: `value * 1`.\n\
            \n\
            ══════════════════════════════════════════\n\
            DEMO 1: Pure Computation (unit conversion)\n\
            ══════════════════════════════════════════\n\
            ```rhai\n\
            let unit_type = params[\"unit_type\"];\n\
            let value = params[\"value\"];\n\
            let result = \"\";\n\
            if unit_type == \"cm_to_ft\" {\n\
              let ft = value * 100 / 3048;\n\
              let remainder = value * 100 % 3048;\n\
              result = `${value}cm = ${ft}.${remainder / 30} feet`;\n\
            } else if unit_type == \"f_to_c\" {\n\
              let c = (value - 32) * 5 / 9;\n\
              result = `${value}°F = ${c}°C`;\n\
            } else if unit_type == \"kg_to_lb\" {\n\
              let lb = value * 220462 / 100000;\n\
              result = `${value}kg = ${lb} lbs`;\n\
            } else {\n\
              result = `Unknown unit_type: ${unit_type}`;\n\
            }\n\
            result\n\
            ```\n\
            test_params: `{\"unit_type\": \"cm_to_ft\", \"value\": 180}` — NOTE: use INTEGER values in test_params.\n\
            \n\
            ══════════════════════════════════════════\n\
            DEMO 2: HTTP Fetch (weather query)\n\
            ══════════════════════════════════════════\n\
            ```rhai\n\
            let city = params[\"city\"];\n\
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
            \n\
            CRITICAL RULES:\n\
            1. test_params values MUST be integers (not floats). E.g. `{\"value\": 180}` not `{\"value\": 1.8}`.\n\
            2. ALWAYS use `map[\"key\"]` bracket notation to access parsed JSON fields.\n\
            3. If test fails, SIMPLIFY the script — fewer branches, simpler logic.\n\
            4. The script is tested with `test_params` FIRST. Test passes if it returns any non-empty value.".into(),
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
                    // Preserve health stats from previous version if updating
                    success_count: existing.as_ref().map(|e| e.success_count).unwrap_or(0),
                    failure_count: existing.as_ref().map(|e| e.failure_count).unwrap_or(0),
                    last_success_at: existing.as_ref().map(|e| e.last_success_at).unwrap_or(0),
                    last_failure_at: existing.as_ref().map(|e| e.last_failure_at).unwrap_or(0),
                    health_status: existing.as_ref().map(|e| e.health_status.clone()).unwrap_or_else(|| "active".to_string()),
                };
                
                let rhai_code_for_response = rhai_code.clone();
                let new_executor = Arc::new(ScriptExecutor::new(rhai_code, sandbox));
                
                if let Err(e) = self.registry.register_dynamic_tool(new_schema, new_executor) {
                    crate::fire_tool_creation_hook(&name, false, existing.is_some());
                    return Err(ToolError::ExecutionFailed(format!("Failed to register tool into VectorToolRegistry: {}", e)));
                }
                crate::fire_tool_creation_hook(&name, true, existing.is_some());
                let is_update = existing.is_some();
                // Note: register_dynamic_tool() handles file persistence to the canonical tools/ dir.
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
                
                // Extract failing line content for better LLM debugging
                let mut failing_line_info = String::new();
                let error_str = format!("{}", e);
                // Parse "line N, position M" from error messages
                if let Some(line_idx) = error_str.find("line ") {
                    let after_line = &error_str[line_idx+5..];
                    if let Some(comma_idx) = after_line.find(',') {
                        if let Ok(line_num) = after_line[..comma_idx].trim().parse::<usize>() {
                            let lines: Vec<&str> = rhai_code.lines().collect();
                            if line_num > 0 && line_num <= lines.len() {
                                let start = if line_num > 2 { line_num - 2 } else { 0 };
                                let end = (line_num + 1).min(lines.len());
                                let context_lines: Vec<String> = (start..end)
                                    .map(|i| format!("  {} | {}{}", i+1, lines[i], if i+1 == line_num { " ← ERROR HERE" } else { "" }))
                                    .collect();
                                failing_line_info = format!("\n\nFailing code context:\n{}", context_lines.join("\n"));
                            }
                        }
                    }
                }
                
                let out = serde_json::json!({
                    "status": "test_failed",
                    "message": format!(
                        "Script test FAILED for tool '{}'. The tool was NOT registered.{}",
                        name, failing_line_info
                    ),
                    "error": error_msg,
                    "diagnosis_context": {
                        "rhai_code": rhai_code,
                        "test_params_used": test_params_str,
                    },
                    "fix_instructions": [
                        "LOOK at the 'Failing code context' above to see the EXACT line causing the error.",
                        "Common fix: Replace `data.property` with `data[\"property\"]` (bracket notation for parsed JSON).",
                        "Common fix: Remove type annotations like `: i64`, `: String` — just use `let x = value;`.",
                        "Common fix: Use integers in test_params, not floats (e.g. 180 not 1.8).",
                        "If stuck after 2 failed attempts, SIMPLIFY drastically: fewer branches, no nested conditions.",
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


// 21. Manage Tools — lifecycle management with health dashboard
#[derive(Clone)]
#[cfg(feature = "full")]
pub struct ManageToolsTool {
    registry: std::sync::Arc<dyn ToolRegistry>,
}

#[cfg(feature = "full")]
impl ManageToolsTool {
    pub fn new(registry: std::sync::Arc<dyn ToolRegistry>) -> Self {
        Self { registry }
    }

    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "manage_tools".into(),
            description: r#"Manage custom Rhai tool lifecycle. Actions:
- "health": Show all custom tools with health status, success rate, and last-used time.
- "archive": Hide a tool from discovery (it stays on disk but won't be found by discover_tools).
- "unarchive": Restore a previously archived tool back to discovery.
- "delete": PERMANENTLY delete a tool from memory AND disk (irreversible!).

⚠️ MANAGEMENT POLICY — READ CAREFULLY:
• "dormant" tools have SUCCEEDED BEFORE but haven't been used recently. They are valuable and should NOT be deleted. Only archive if clutter is a concern.
• "broken" tools have NEVER SUCCEEDED (0 successes with 3+ failures). They are garbage — safe to delete or archive.
• "active" tools are healthy and recently used. Do NOT touch them.
• Only delete a tool if it is BROKEN and has never provided value. Dormant ≠ Broken."#.into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "action": { 
                            "type": "string", 
                            "enum": ["health", "archive", "unarchive", "delete"],
                            "description": "Management action to perform" 
                        },
                        "tool_name": { 
                            "type": "string", 
                            "description": "Name of the tool to manage (required for archive/unarchive/delete)" 
                        }
                    },
                    "required": ["action"]
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}

#[cfg(feature = "full")]
#[async_trait]
impl ToolExecutor for ManageToolsTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let action = params.get("action").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'action'".into()))?;
        let tool_name = params.get("tool_name").and_then(|v| v.as_str());

        match action {
            "health" => {
                let all_tools = self.registry.list_all_tools();
                // Only show custom/dynamic tools (not native ones — they have no health data)
                let custom_tools: Vec<_> = all_tools.iter()
                    .filter(|t| t.iteration > 0 || t.success_count > 0 || t.failure_count > 0 
                            || t.health_status != "active" || t.version != "1.0.0")
                    .collect();

                if custom_tools.is_empty() {
                    let out = serde_json::json!({
                        "status": "success",
                        "message": "No custom tools found. Only native tools are registered.",
                        "tools": []
                    });
                    return Ok(serde_json::to_vec_pretty(&out).unwrap());
                }

                let tool_reports: Vec<Value> = custom_tools.iter().map(|t| {
                    let total = t.success_count + t.failure_count;
                    let success_rate = if total > 0 { 
                        format!("{:.0}%", (t.success_count as f64 / total as f64) * 100.0)
                    } else { 
                        "N/A (never used)".to_string() 
                    };

                    let last_used = if t.last_success_at > 0 || t.last_failure_at > 0 {
                        let ts = t.last_success_at.max(t.last_failure_at);
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64).unwrap_or(0);
                        let age_hours = now.saturating_sub(ts) / 3_600_000;
                        if age_hours < 24 { format!("{}h ago", age_hours) }
                        else { format!("{}d ago", age_hours / 24) }
                    } else {
                        "never".to_string()
                    };

                    serde_json::json!({
                        "name": t.name,
                        "health_status": t.health_status,
                        "success_rate": success_rate,
                        "success_count": t.success_count,
                        "failure_count": t.failure_count,
                        "last_used": last_used,
                        "version": t.version,
                        "iteration": t.iteration,
                        "description_preview": if t.description.len() > 80 { 
                            format!("{}...", &t.description[..80]) 
                        } else { 
                            t.description.clone() 
                        },
                    })
                }).collect();

                let summary = format!("{} custom tools: {} active, {} dormant, {} broken, {} archived",
                    custom_tools.len(),
                    custom_tools.iter().filter(|t| t.health_status == "active").count(),
                    custom_tools.iter().filter(|t| t.health_status == "dormant").count(),
                    custom_tools.iter().filter(|t| t.health_status == "broken").count(),
                    custom_tools.iter().filter(|t| t.health_status == "archived").count(),
                );

                let out = serde_json::json!({
                    "status": "success",
                    "summary": summary,
                    "policy_reminder": "dormant≠broken. Dormant tools worked before and are still valuable. Only delete/archive BROKEN tools.",
                    "tools": tool_reports
                });
                Ok(serde_json::to_vec_pretty(&out).unwrap())
            }

            "archive" => {
                let name = tool_name
                    .ok_or_else(|| ToolError::ExecutionFailed("Missing 'tool_name' for archive action".into()))?;
                self.registry.archive_tool(name)
                    .map_err(|e| ToolError::ExecutionFailed(e))?;
                let out = serde_json::json!({
                    "status": "success",
                    "message": format!("Tool '{}' archived. It won't appear in discover_tools but is preserved on disk.", name)
                });
                Ok(serde_json::to_vec_pretty(&out).unwrap())
            }

            "unarchive" => {
                let name = tool_name
                    .ok_or_else(|| ToolError::ExecutionFailed("Missing 'tool_name' for unarchive action".into()))?;
                // Unarchive: set health_status back to "active"
                // We need to do this via a note + status update
                // Since we don't have a direct unarchive method, let's record a success to reset it
                self.registry.record_tool_usage(name, true);
                let out = serde_json::json!({
                    "status": "success",
                    "message": format!("Tool '{}' restored to active status. It will now appear in discover_tools.", name)
                });
                Ok(serde_json::to_vec_pretty(&out).unwrap())
            }

            "delete" => {
                let name = tool_name
                    .ok_or_else(|| ToolError::ExecutionFailed("Missing 'tool_name' for delete action".into()))?;
                
                // Safety check: verify the tool exists and check if it's safe to delete
                if let Some(schema) = self.registry.get_schema(name) {
                    if schema.health_status == "active" && schema.success_count > 0 {
                        return Err(ToolError::ExecutionFailed(format!(
                            "⚠️ SAFETY: Tool '{}' is ACTIVE with {} successes. Deleting an active tool is dangerous. Archive it instead, or set it to broken first if you're sure.",
                            name, schema.success_count
                        )));
                    }
                }

                self.registry.delete_tool(name)
                    .map_err(|e| ToolError::ExecutionFailed(e))?;
                let out = serde_json::json!({
                    "status": "success",
                    "message": format!("Tool '{}' permanently deleted from memory and disk.", name)
                });
                Ok(serde_json::to_vec_pretty(&out).unwrap())
            }

            _ => Err(ToolError::ExecutionFailed(format!("Unknown action: '{}'. Use 'health', 'archive', 'unarchive', or 'delete'.", action))),
        }
    }
}

// 23. Rhai Tool Studio - Unified Integrated Development Environment for Tools
#[derive(Clone)]
#[cfg(feature = "full")]
pub struct RhaiToolStudio {
    registry: std::sync::Arc<dyn ToolRegistry>,
}

#[cfg(feature = "full")]
impl RhaiToolStudio {
    pub fn new(registry: std::sync::Arc<dyn ToolRegistry>) -> Self {
        Self { registry }
    }

    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "rhai_tool_studio".into(),
            description: "Unified Integrated Development Environment for dynamic Rhai tools. \
            ACTIONS:\n\
            - 'read': Inspect a tool's source code and schema. Requires 'tool_name'.\n\
            - 'test_run': Execute a Rhai script in the sandbox without saving it. Requires 'rhai_code' and 'test_params'.\n\
            - 'overwrite': Save/Update a tool permanently to disk. Requires 'tool_name', 'description', 'parameters_schema', and 'rhai_code'.\n\
            - 'delete': Permanently delete a tool. Requires 'tool_name'.\n\
            \n\
            RHAI SYNTAX REFERENCE:\n\
            ▸ STRINGS: Double quotes `\"hello\"`. Backtick interpolation: `` `hello ${name}` ``\n\
            ▸ MAP ACCESS: MUST use bracket: `map[\"key\"]`. To check if key exists: `\"key\" in map` or `map.contains(\"key\")`. DO NOT USE `map.keys().contains()`.\n\
            ▸ CONTROL FLOW: You MUST use `{}` braces for ALL `if`, `for`, `while` statements. Example: `if x == 1 { return x; }`. DO NOT USE PYTHON SYNTAX!\n\
            ▸ HTTP: `http_get_with_fallback([\"url1\"])` -> returns string. `try_parse_json(text)` -> parses JSON safely.\n\
            ▸ RETURN: Last expression without semicolon is returned.\n\
            ".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["read", "test_run", "overwrite", "delete"], "description": "The action to perform" },
                        "tool_name": { "type": "string", "description": "Name of the tool (for read, overwrite, delete)" },
                        "description": { "type": "string", "description": "Description of the tool (for overwrite)" },
                        "parameters_schema": { "type": "string", "description": "JSON string of the tool's input schema (for overwrite)" },
                        "rhai_code": { "type": "string", "description": "The script code (for test_run, overwrite)" },
                        "test_params": { "type": "string", "description": "JSON string of params to test the script with (for test_run)" }
                    },
                    "required": ["action"]
                }),
            },
            risk_level: RiskLevel::HighRisk,
            ..Default::default()
        }
    }
}

#[cfg(feature = "full")]
#[async_trait]
impl ToolExecutor for RhaiToolStudio {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let action = params.get("action").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'action'".into()))?;
        
        match action {
            "read" => {
                let name = params.get("tool_name").and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::ExecutionFailed("Missing 'tool_name' for read action".into()))?;
                    
                if let Some(schema) = self.registry.get_schema(name) {
                    let source_code = if let Some(exec) = self.registry.get_executor(name) {
                        exec.source_code().unwrap_or_else(|| "(source code not available)".to_string())
                    } else {
                        "(executor not found/native tool)".to_string()
                    };
                    
                    let out = serde_json::json!({
                        "status": "success",
                        "tool_name": name,
                        "description": schema.description,
                        "parameters_schema": schema.parameters_schema.raw_schema,
                        "health": schema.health_status,
                        "rhai_code": source_code,
                    });
                    Ok(serde_json::to_vec_pretty(&out).unwrap())
                } else {
                    let out = serde_json::json!({ "status": "error", "message": format!("Tool '{}' not found", name) });
                    Ok(serde_json::to_vec_pretty(&out).unwrap())
                }
            }
            "test_run" => {
                let rhai_code = params.get("rhai_code").and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::ExecutionFailed("Missing 'rhai_code' for test_run".into()))?.to_string();
                let test_params_str = params.get("test_params").and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::ExecutionFailed("Missing 'test_params' for test_run".into()))?;
                let test_params: Value = serde_json::from_str(test_params_str)
                    .map_err(|e| ToolError::ExecutionFailed(format!("Invalid test params JSON: {}", e)))?;
                
                let sandbox = std::sync::Arc::new(crate::script_sandbox::ScriptSandbox::new());
                match sandbox.execute(&rhai_code, test_params) {
                    Ok(result) => {
                        let out = serde_json::json!({
                            "status": "success",
                            "test_output": result
                        });
                        Ok(serde_json::to_vec_pretty(&out).unwrap())
                    }
                    Err(e) => {
                        let mut error_msg = format!("{}", e);
                        let mut error_line = None;
                        if let Some(line_idx) = error_msg.find("line ") {
                            let after_line = &error_msg[line_idx+5..];
                            if let Some(comma_idx) = after_line.find(',') {
                                if let Ok(line_num) = after_line[..comma_idx].trim().parse::<usize>() {
                                    error_line = Some(line_num);
                                }
                            }
                        }
                        
                        let out = serde_json::json!({
                            "status": "test_failed",
                            "error": error_msg,
                            "error_line": error_line,
                            "available_functions": {
                                "http_get": "http_get(url_string) -> String. Single URL fetch. Example: http_get(\"https://example.com/api\")",
                                "http_get_with_fallback": "http_get_with_fallback(json_array_string) -> String. Tries URLs sequentially. The argument MUST be a JSON STRING, NOT a Rhai array. Example: http_get_with_fallback(\"[\\\"https://url1.com\\\", \\\"https://url2.com\\\"]\")",
                                "parse_json": "parse_json(string) -> Map. Parses JSON string into Rhai object. Throws on invalid JSON.",
                                "try_parse_json": "try_parse_json(string) -> Dynamic. Safe JSON parse: returns parsed object on success, original string on failure.",
                                "to_json": "to_json(value) -> String. Converts Rhai value back to JSON string."
                            },
                            "hint": if error_msg.contains("Syntax error") || error_msg.contains("Expecting '{'") {
                                "CRITICAL SYNTAX ERROR: Rhai STRICTLY REQUIRES `{}` braces for all `if`, `for`, and `while` blocks. E.g., `if condition { return true; }`. DO NOT use Python/Ruby style syntax."
                            } else {
                                "IMPORTANT: http_get_with_fallback takes a JSON-encoded string array, NOT a Rhai array. Use: http_get_with_fallback(\"[\\\"url\\\"]\")"
                            }
                        });
                        Ok(serde_json::to_vec_pretty(&out).unwrap())
                    }
                }
            }
            "overwrite" => {
                let name = params.get("tool_name").and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::ExecutionFailed("Missing 'tool_name' for overwrite".into()))?.to_string();
                
                if let Err(reason) = validate_tool_name(&name) {
                    let out = serde_json::json!({ "status": "error", "message": format!("Tool name '{}' rejected: {}", name, reason) });
                    return Ok(serde_json::to_vec_pretty(&out).unwrap());
                }
                
                let description = params.get("description").and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::ExecutionFailed("Missing 'description'".into()))?.to_string();
                let schema_str = params.get("parameters_schema").and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::ExecutionFailed("Missing 'parameters_schema'".into()))?;
                let rhai_code = params.get("rhai_code").and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::ExecutionFailed("Missing 'rhai_code'".into()))?.to_string();
                
                let mut raw_schema: Value = serde_json::from_str(schema_str)
                    .map_err(|e| ToolError::ExecutionFailed(format!("Invalid parameter schema JSON: {}", e)))?;
                if let Value::String(s) = &raw_schema {
                    raw_schema = serde_json::from_str(s).unwrap_or(raw_schema);
                }
                
                let existing = self.registry.get_schema(&name);
                let (version, iteration) = if let Some(ref old) = existing {
                    (bump_patch_version(&old.version), old.iteration + 1)
                } else {
                    ("1.0.0".to_string(), 0)
                };
                
                let new_schema = ToolSchema {
                    name: name.clone(),
                    description,
                    parameters_schema: JsonSchema { raw_schema },
                    risk_level: RiskLevel::Normal,
                    version: version.clone(),
                    iteration,
                    parent_tool: existing.as_ref().map(|_| name.clone()),
                    change_reason: Some("Updated via ToolStudio".to_string()),
                    experience_notes: existing.as_ref().map(|e| e.experience_notes.clone()).unwrap_or_default(),
                    success_count: existing.as_ref().map(|e| e.success_count).unwrap_or(0),
                    failure_count: existing.as_ref().map(|e| e.failure_count).unwrap_or(0),
                    last_success_at: existing.as_ref().map(|e| e.last_success_at).unwrap_or(0),
                    last_failure_at: existing.as_ref().map(|e| e.last_failure_at).unwrap_or(0),
                    health_status: existing.as_ref().map(|e| e.health_status.clone()).unwrap_or_else(|| "active".to_string()),
                };
                
                let sandbox = std::sync::Arc::new(crate::script_sandbox::ScriptSandbox::new());
                let new_executor = std::sync::Arc::new(crate::script_sandbox::ScriptExecutor::new(rhai_code, sandbox));
                
                if let Err(e) = self.registry.register_dynamic_tool(new_schema, new_executor) {
                    crate::fire_tool_creation_hook(&name, false, existing.is_some());
                    return Err(ToolError::ExecutionFailed(format!("Failed to register tool: {}", e)));
                }
                crate::fire_tool_creation_hook(&name, true, existing.is_some());
                
                let out = serde_json::json!({
                    "status": "success",
                    "message": format!("Tool '{}' successfully saved and registered.", name),
                    "version": version
                });
                Ok(serde_json::to_vec_pretty(&out).unwrap())
            }
            "delete" => {
                let name = params.get("tool_name").and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::ExecutionFailed("Missing 'tool_name' for delete".into()))?;
                
                if self.registry.get_schema(name).is_some() {
                    self.registry.delete_tool(name).map_err(|e| ToolError::ExecutionFailed(e))?;
                    let out = serde_json::json!({
                        "status": "success",
                        "message": format!("Tool '{}' permanently deleted.", name)
                    });
                    Ok(serde_json::to_vec_pretty(&out).unwrap())
                } else {
                    let out = serde_json::json!({ "status": "error", "message": format!("Tool '{}' not found", name) });
                    Ok(serde_json::to_vec_pretty(&out).unwrap())
                }
            }
            _ => Err(ToolError::ExecutionFailed(format!("Unknown action: {}", action)))
        }
    }
}
