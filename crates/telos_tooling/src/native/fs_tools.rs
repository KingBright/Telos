use crate::{JsonSchema, ToolError, ToolExecutor, ToolSchema};
use tracing::{info, debug, warn, error};
use async_trait::async_trait;
use serde_json::Value;
use telos_core::RiskLevel;
use std::fs;
use tokio::process::Command;

// 1. File Reader Tool
#[derive(Clone)]
pub struct FsReadTool;

#[async_trait]
impl ToolExecutor for FsReadTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'path' parameter".into()))?;

        match fs::read_to_string(path) {
            Ok(content) => Ok(content.into_bytes()),
            Err(e) => Err(ToolError::ExecutionFailed(format!(
                "Failed to read file: {}",
                e
            ))),
        }
    }
}

impl FsReadTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "fs_read".into(),
            description: "Reads the content of a file from the disk. Requires a 'path' parameter."
                .into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    },
                    "required": ["path"]
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}

// 2. File Writer Tool
#[derive(Clone)]
pub struct FsWriteTool;

#[async_trait]
impl ToolExecutor for FsWriteTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'path' parameter".into()))?;

        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'content' parameter".into()))?;

        match fs::write(path, content) {
            Ok(_) => Ok(b"{\"status\":\"success\"}".to_vec()),
            Err(e) => Err(ToolError::ExecutionFailed(format!(
                "Failed to write file: {}",
                e
            ))),
        }
    }
}

impl FsWriteTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "fs_write".into(),
            description:
                "Writes content to a file on the disk. Requires 'path' and 'content' parameters."
                    .into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"]
                }),
            },
            risk_level: RiskLevel::HighRisk,
            ..Default::default()
        }
    }
}


// 8. File Edit Tool — Enhanced with 5-strategy fuzzy matching
#[derive(Clone)]
pub struct FileEditTool;

#[async_trait]
impl ToolExecutor for FileEditTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'path'".into()))?;
        let search = params
            .get("search")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'search'".into()))?;
        let replace = params
            .get("replace")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'replace'".into()))?;

        // If file doesn't exist, treat it as empty.
        let content = std::fs::read_to_string(path).unwrap_or_else(|_| String::new());

        let (modified_content, strategy_used) = if search.is_empty() {
            // Overwrite if search string is empty
            (replace.to_string(), "overwrite")
        } else {
            fuzzy_match::find_and_replace(&content, search, replace)?
        };

        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }

        std::fs::write(path, &modified_content).map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to write {}: {}", path, e))
        })?;

        let response = serde_json::json!({
            "status": "success",
            "message": format!("File updated successfully (match strategy: {})", strategy_used)
        });
        Ok(serde_json::to_vec(&response).unwrap_or_else(|_|
            b"{\"status\": \"success\", \"message\": \"File updated successfully\"}".to_vec()
        ))
    }
}

impl FileEditTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "file_edit".into(),
            description: "Edits a file by replacing a search string with a replacement string. Uses fuzzy matching: tries exact match first, then trimmed, indentation-flexible, whitespace-normalized, and Levenshtein similarity."
                .into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Absolute path to the file" },
                        "search": { "type": "string", "description": "Text to search for (supports fuzzy matching)" },
                        "replace": { "type": "string", "description": "Replacement text" }
                    },
                    "required": ["path", "search", "replace"]
                }),
            },
            risk_level: RiskLevel::HighRisk,
            ..Default::default()
        }
    }
}

/// Fuzzy matching module for FileEditTool.
/// Implements 5 cascade strategies to handle LLM-generated search strings that may
/// have minor whitespace/indentation differences from the actual file content.
mod fuzzy_match {
    use super::ToolError;

    /// Attempt to find `search` in `content` using cascading strategies.
    /// Returns (modified_content, strategy_name) on success.
    pub fn find_and_replace(content: &str, search: &str, replace: &str) -> Result<(String, &'static str), ToolError> {
        // Strategy 1: Exact match
        if content.contains(search) {
            return Ok((content.replacen(search, replace, 1), "exact"));
        }

        // Strategy 2: Trimmed whitespace on both ends
        let trimmed_search = search.trim();
        if !trimmed_search.is_empty() {
            if let Some(pos) = content.find(trimmed_search) {
                let mut result = String::with_capacity(content.len());
                result.push_str(&content[..pos]);
                result.push_str(replace);
                result.push_str(&content[pos + trimmed_search.len()..]);
                return Ok((result, "trimmed"));
            }
        }

        // Strategy 3: Indentation-flexible line-by-line match
        if let Some(result) = try_indentation_flexible(content, search, replace) {
            return Ok((result, "indent_flexible"));
        }

        // Strategy 4: Whitespace-normalized line match (collapse internal whitespace)
        if let Some(result) = try_whitespace_normalized(content, search, replace) {
            return Ok((result, "ws_normalized"));
        }

        // Strategy 5: Levenshtein similarity sliding window (threshold = 0.85)
        if let Some(result) = try_levenshtein_match(content, search, replace, 0.85) {
            return Ok((result, "levenshtein"));
        }

        // All strategies failed — provide a helpful error
        let preview: String = search.chars().take(120).collect();
        Err(ToolError::ExecutionFailed(format!(
            "Search string not found after trying 5 matching strategies (exact, trimmed, indent-flexible, ws-normalized, levenshtein). Search preview: '{}'",
            preview
        )))
    }

    /// Strategy 3: Match lines ignoring leading indentation differences.
    /// Strips the minimum common indentation from both search and content windows,
    /// then compares line-by-line.
    fn try_indentation_flexible(content: &str, search: &str, replace: &str) -> Option<String> {
        let search_lines: Vec<&str> = search.lines().collect();
        if search_lines.is_empty() {
            return None;
        }

        let content_lines: Vec<&str> = content.lines().collect();
        if content_lines.len() < search_lines.len() {
            return None;
        }

        // Strip minimum indentation from search lines
        let search_stripped = strip_common_indent(&search_lines);

        // Slide a window of search_lines.len() over content_lines
        for start in 0..=(content_lines.len() - search_lines.len()) {
            let window = &content_lines[start..start + search_lines.len()];
            let window_stripped = strip_common_indent(window);

            if search_stripped.len() == window_stripped.len()
                && search_stripped.iter().zip(window_stripped.iter()).all(|(s, w)| s == w)
            {
                // Found match — reconstruct content with the window replaced
                let mut result = String::new();
                // Lines before the match
                for line in &content_lines[..start] {
                    result.push_str(line);
                    result.push('\n');
                }
                // Replacement
                result.push_str(replace);
                if !replace.ends_with('\n') && start + search_lines.len() < content_lines.len() {
                    result.push('\n');
                }
                // Lines after the match
                for (i, line) in content_lines[start + search_lines.len()..].iter().enumerate() {
                    result.push_str(line);
                    if start + search_lines.len() + i < content_lines.len() - 1 {
                        result.push('\n');
                    }
                }
                // Preserve trailing newline if original had one
                if content.ends_with('\n') && !result.ends_with('\n') {
                    result.push('\n');
                }
                return Some(result);
            }
        }
        None
    }

    /// Strategy 4: Normalize each line by collapsing whitespace, then line-match.
    fn try_whitespace_normalized(content: &str, search: &str, replace: &str) -> Option<String> {
        let search_lines: Vec<&str> = search.lines().collect();
        if search_lines.is_empty() {
            return None;
        }

        let content_lines: Vec<&str> = content.lines().collect();
        if content_lines.len() < search_lines.len() {
            return None;
        }

        let search_normalized: Vec<String> = search_lines.iter()
            .map(|l| normalize_whitespace(l))
            .collect();

        // Filter out empty lines from search for matching purposes
        let search_non_empty: Vec<&str> = search_normalized.iter()
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .collect();

        for start in 0..=(content_lines.len() - search_lines.len()) {
            let window = &content_lines[start..start + search_lines.len()];
            let window_normalized: Vec<String> = window.iter()
                .map(|l| normalize_whitespace(l))
                .collect();
            let window_non_empty: Vec<&str> = window_normalized.iter()
                .map(|s| s.as_str())
                .filter(|s| !s.is_empty())
                .collect();

            if search_non_empty == window_non_empty {
                let mut result = String::new();
                for line in &content_lines[..start] {
                    result.push_str(line);
                    result.push('\n');
                }
                result.push_str(replace);
                if !replace.ends_with('\n') && start + search_lines.len() < content_lines.len() {
                    result.push('\n');
                }
                for (i, line) in content_lines[start + search_lines.len()..].iter().enumerate() {
                    result.push_str(line);
                    if start + search_lines.len() + i < content_lines.len() - 1 {
                        result.push('\n');
                    }
                }
                if content.ends_with('\n') && !result.ends_with('\n') {
                    result.push('\n');
                }
                return Some(result);
            }
        }
        None
    }

    /// Strategy 5: Levenshtein similarity with sliding window.
    /// For each window of `search.len() ± 20%` characters in content,
    /// compute normalized Levenshtein similarity. Accept if >= threshold.
    fn try_levenshtein_match(content: &str, search: &str, replace: &str, threshold: f64) -> Option<String> {
        let search_len = search.len();
        if search_len == 0 || content.len() < search_len / 2 {
            return None;
        }

        // Only attempt for reasonably-sized search strings to avoid O(n*m) blowup
        if search_len > 2000 {
            return None;
        }

        let min_window = (search_len as f64 * 0.8) as usize;
        let max_window = std::cmp::min((search_len as f64 * 1.2) as usize, content.len());

        let mut best_score = 0.0_f64;
        let mut best_start = 0_usize;
        let mut best_end = 0_usize;

        // Slide windows of varying sizes
        for window_size in [search_len, min_window, max_window].iter().copied() {
            if window_size > content.len() || window_size == 0 {
                continue;
            }

            // Use char boundaries for safe slicing
            let content_chars: Vec<(usize, char)> = content.char_indices().collect();
            if content_chars.is_empty() {
                continue;
            }

            // Step through content in line-aligned chunks for efficiency
            let line_starts: Vec<usize> = std::iter::once(0)
                .chain(content.match_indices('\n').map(|(i, _)| i + 1))
                .filter(|&i| i < content.len())
                .collect();

            for &start_byte in &line_starts {
                // Find end approximately window_size bytes ahead
                let target_end = start_byte + window_size;
                if target_end > content.len() {
                    break;
                }

                // Snap to next newline for cleaner boundaries
                let end_byte = content[target_end..].find('\n')
                    .map(|offset| target_end + offset)
                    .unwrap_or(content.len());

                let window = &content[start_byte..end_byte];
                let score = normalized_levenshtein(search, window);

                if score > best_score {
                    best_score = score;
                    best_start = start_byte;
                    best_end = end_byte;
                }

                // Early exit if we found a near-perfect match
                if best_score >= 0.98 {
                    break;
                }
            }

            if best_score >= 0.98 {
                break;
            }
        }

        if best_score >= threshold {
            let mut result = String::with_capacity(content.len());
            result.push_str(&content[..best_start]);
            result.push_str(replace);
            result.push_str(&content[best_end..]);
            return Some(result);
        }

        None
    }

    // ==== Helper functions ====

    /// Strip the minimum common leading whitespace from a set of lines.
    fn strip_common_indent(lines: &[&str]) -> Vec<String> {
        let min_indent = lines.iter()
            .filter(|l| !l.trim().is_empty()) // skip blank lines
            .map(|l| l.len() - l.trim_start().len())
            .min()
            .unwrap_or(0);

        lines.iter()
            .map(|l| {
                if l.len() >= min_indent {
                    l[min_indent..].to_string()
                } else {
                    l.trim().to_string()
                }
            })
            .collect()
    }

    /// Collapse all whitespace sequences to a single space, then trim.
    fn normalize_whitespace(s: &str) -> String {
        s.split_whitespace().collect::<Vec<&str>>().join(" ")
    }

    /// Compute normalized Levenshtein similarity (0.0 to 1.0).
    /// Uses optimized two-row algorithm for O(min(m,n)) space.
    fn normalized_levenshtein(a: &str, b: &str) -> f64 {
        let max_len = std::cmp::max(a.len(), b.len());
        if max_len == 0 {
            return 1.0;
        }
        let distance = levenshtein_distance(a, b);
        1.0 - (distance as f64 / max_len as f64)
    }

    /// Levenshtein edit distance with O(min(m,n)) space complexity.
    fn levenshtein_distance(a: &str, b: &str) -> usize {
        let a_chars: Vec<char> = a.chars().collect();
        let b_chars: Vec<char> = b.chars().collect();
        let (short, long) = if a_chars.len() <= b_chars.len() {
            (&a_chars, &b_chars)
        } else {
            (&b_chars, &a_chars)
        };

        let short_len = short.len();
        let long_len = long.len();

        // Early termination: if strings are too different in length
        if long_len - short_len > long_len / 3 {
            return long_len; // fast reject
        }

        let mut prev: Vec<usize> = (0..=short_len).collect();
        let mut curr = vec![0; short_len + 1];

        for i in 1..=long_len {
            curr[0] = i;
            for j in 1..=short_len {
                let cost = if long[i - 1] == short[j - 1] { 0 } else { 1 };
                curr[j] = std::cmp::min(
                    std::cmp::min(curr[j - 1] + 1, prev[j] + 1),
                    prev[j - 1] + cost,
                );
            }
            std::mem::swap(&mut prev, &mut curr);
        }
        prev[short_len]
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_strategy_1_exact_match() {
            let content = "fn main() {\n    println!(\"hello\");\n}\n";
            let search = "    println!(\"hello\");";
            let replace = "    println!(\"world\");";
            let (result, strategy) = find_and_replace(content, search, replace).unwrap();
            assert_eq!(strategy, "exact");
            assert!(result.contains("println!(\"world\")"));
            assert!(!result.contains("println!(\"hello\")"));
        }

        #[test]
        fn test_strategy_2_trimmed_whitespace() {
            let content = "fn main() {\n    let x = 1;\n}\n";
            // LLM adds leading/trailing whitespace
            let search = "  \n    let x = 1;\n  ";
            let replace = "    let x = 2;";
            let (result, strategy) = find_and_replace(content, search, replace).unwrap();
            assert_eq!(strategy, "trimmed");
            assert!(result.contains("let x = 2"));
        }

        #[test]
        fn test_strategy_3_indentation_flexible() {
            let content = "        fn inner() {\n            let x = 1;\n            let y = 2;\n        }\n";
            // LLM outputs with different (less) indentation
            let search = "fn inner() {\n    let x = 1;\n    let y = 2;\n}";
            let replace = "fn inner() {\n    let x = 10;\n    let y = 20;\n}";
            let (result, strategy) = find_and_replace(content, search, replace).unwrap();
            assert_eq!(strategy, "indent_flexible");
            assert!(result.contains("let x = 10"));
        }

        #[test]
        fn test_strategy_4_whitespace_normalized() {
            let content = "fn  main()  {\n    let   x  =  1;\n}\n";
            // LLM normalizes internal whitespace differently
            let search = "fn main() {\n let x = 1;\n}";
            let replace = "fn main() {\n    let x = 2;\n}";
            let (result, strategy) = find_and_replace(content, search, replace).unwrap();
            assert_eq!(strategy, "ws_normalized");
            assert!(result.contains("let x = 2"));
        }

        #[test]
        fn test_strategy_5_levenshtein() {
            let content = "fn calculate(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
            // LLM gets the function almost right but has a typo
            let search = "fn calcualte(a: i32, b: i32) -> i32 {\n    a + b\n}";
            let replace = "fn calculate(a: i32, b: i32) -> i32 {\n    a * b\n}";
            let (result, strategy) = find_and_replace(content, search, replace).unwrap();
            assert_eq!(strategy, "levenshtein");
            assert!(result.contains("a * b"));
        }

        #[test]
        fn test_all_strategies_fail() {
            let content = "fn main() {\n    println!(\"hello\");\n}\n";
            let search = "completely_different_content_that_doesnt_exist";
            let replace = "replacement";
            let result = find_and_replace(content, search, replace);
            assert!(result.is_err());
            let err_msg = format!("{:?}", result.unwrap_err());
            assert!(err_msg.contains("5 matching strategies"));
        }

        #[test]
        fn test_levenshtein_distance_basic() {
            assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
            assert_eq!(levenshtein_distance("", "abc"), 3);
            assert_eq!(levenshtein_distance("abc", "abc"), 0);
        }
    }
}

// 9. Glob Tool
#[derive(Clone)]
pub struct GlobTool;

#[async_trait]
impl ToolExecutor for GlobTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let pattern = params
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'pattern'".into()))?;

        let output = Command::new("find")
            .arg(".")
            .arg("-name")
            .arg(pattern)
            .output()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Command failed: {}", e)))?;

        Ok(output.stdout)
    }
}

impl GlobTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "glob".into(),
            description:
                "Finds files matching a pattern using 'find . -name <pattern>'. Example: '*.rs'"
                    .into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string" }
                    },
                    "required": ["pattern"]
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}

// 10. Grep Tool
#[derive(Clone)]
pub struct GrepTool;

#[async_trait]
impl ToolExecutor for GrepTool {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        let pattern = params
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("Missing 'pattern'".into()))?;
        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let output = Command::new("grep")
            .arg("-rn")
            .arg(pattern)
            .arg(path)
            .output()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Command failed: {}", e)))?;

        Ok(output.stdout)
    }
}

impl GrepTool {
    pub fn schema() -> ToolSchema {
        ToolSchema {
            name: "grep".into(),
            description: "Searches for a pattern in files using 'grep -rn <pattern> <path>'. Returns matches with line numbers.".into(),
            parameters_schema: JsonSchema {
                raw_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string" },
                        "path": { "type": "string", "description": "Defaults to '.'" }
                    },
                    "required": ["pattern"]
                }),
            },
            risk_level: RiskLevel::Normal,
            ..Default::default()
        }
    }
}

