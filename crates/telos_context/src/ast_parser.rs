use crate::clustering::Edu;

/// Supported programming languages for AST-like parsing.
#[derive(Debug, Clone, Copy)]
pub enum CodeLanguage {
    Rust,
    Python,
    JavaScript,
    Unknown,
}

/// Detect the programming language from content heuristics.
pub fn detect_language(content: &str) -> CodeLanguage {
    let lines: Vec<&str> = content.lines().take(30).collect();
    let sample = lines.join("\n");

    if sample.contains("fn ") && (sample.contains("-> ") || sample.contains("pub ") || sample.contains("impl ")) {
        CodeLanguage::Rust
    } else if sample.contains("def ") && sample.contains(":") && !sample.contains("{") {
        CodeLanguage::Python
    } else if sample.contains("function ") || sample.contains("const ") || sample.contains("=>") {
        CodeLanguage::JavaScript
    } else {
        CodeLanguage::Unknown
    }
}

/// Checks whether content looks like code (as opposed to natural language prose).
pub fn is_code_content(content: &str) -> bool {
    let lines: Vec<&str> = content.lines().take(50).collect();
    if lines.is_empty() {
        return false;
    }

    let code_indicators = [
        "fn ", "pub ", "impl ", "struct ", "enum ", "mod ", "use ",  // Rust
        "def ", "class ", "import ", "from ", "if __name__",         // Python
        "function ", "const ", "let ", "var ", "=>", "export ",      // JavaScript
        "#include", "int main",                                       // C/C++
    ];

    let indicator_count = lines.iter()
        .filter(|line| code_indicators.iter().any(|kw| line.contains(kw)))
        .count();

    // If more than 15% of lines look like code, treat as code
    (indicator_count as f32 / lines.len() as f32) > 0.15
}

/// Parse code content into structural EDUs based on function, class, and impl boundaries.
/// This is a lightweight alternative to tree-sitter that uses structural pattern matching
/// to split code at semantically meaningful boundaries (function definitions, class/impl
/// blocks, etc.) rather than at sentence-ending punctuation.
pub fn parse_code_into_edus(content: &str, base_id: &str) -> Vec<Edu> {
    let lang = detect_language(content);
    match lang {
        CodeLanguage::Rust => parse_rust_edus(content, base_id),
        CodeLanguage::Python => parse_python_edus(content, base_id),
        CodeLanguage::JavaScript => parse_js_edus(content, base_id),
        CodeLanguage::Unknown => parse_generic_code_edus(content, base_id),
    }
}

/// Parse Rust code into EDUs at function/impl/struct/enum boundaries.
fn parse_rust_edus(content: &str, base_id: &str) -> Vec<Edu> {
    let mut edus = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut current_block = String::new();
    let mut brace_depth: i32 = 0;
    let mut block_start = false;
    let mut counter = 0;

    for line in &lines {
        let trimmed = line.trim();

        // Detect block start patterns
        let is_block_start = brace_depth == 0 && (
            trimmed.starts_with("pub fn ") || trimmed.starts_with("fn ") ||
            trimmed.starts_with("pub async fn ") || trimmed.starts_with("async fn ") ||
            trimmed.starts_with("impl ") || trimmed.starts_with("pub struct ") ||
            trimmed.starts_with("struct ") || trimmed.starts_with("pub enum ") ||
            trimmed.starts_with("enum ") || trimmed.starts_with("pub mod ") ||
            trimmed.starts_with("mod ") || trimmed.starts_with("#[test]") ||
            trimmed.starts_with("pub trait ") || trimmed.starts_with("trait ")
        );

        if is_block_start && !current_block.trim().is_empty() {
            // Flush previous block
            edus.push(Edu {
                id: format!("{}_rust_{}", base_id, counter),
                text: current_block.trim().to_string(),
                embedding: None,
            });
            counter += 1;
            current_block.clear();
        }

        if is_block_start {
            block_start = true;
        }

        // Track brace depth
        for ch in trimmed.chars() {
            match ch {
                '{' => brace_depth += 1,
                '}' => brace_depth -= 1,
                _ => {}
            }
        }

        current_block.push_str(line);
        current_block.push('\n');

        // End of a top-level block
        if block_start && brace_depth == 0 && trimmed.contains('}') {
            edus.push(Edu {
                id: format!("{}_rust_{}", base_id, counter),
                text: current_block.trim().to_string(),
                embedding: None,
            });
            counter += 1;
            current_block.clear();
            block_start = false;
        }
    }

    // Flush remaining
    if !current_block.trim().is_empty() {
        edus.push(Edu {
            id: format!("{}_rust_{}", base_id, counter),
            text: current_block.trim().to_string(),
            embedding: None,
        });
    }

    // If we only got 1 EDU (e.g., no clear structure), fall back to line-based chunking
    if edus.len() <= 1 && lines.len() > 20 {
        return chunk_by_lines(content, base_id, 30);
    }

    edus
}

/// Parse Python code into EDUs at def/class boundaries using indentation.
fn parse_python_edus(content: &str, base_id: &str) -> Vec<Edu> {
    let mut edus = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut current_block = String::new();
    let mut counter = 0;
    let mut in_block = false;

    for line in &lines {
        let trimmed = line.trim();

        // Detect top-level block start (no leading whitespace)
        let is_top_level_def = !line.is_empty() &&
            !line.starts_with(' ') && !line.starts_with('\t') && (
            trimmed.starts_with("def ") || trimmed.starts_with("class ") ||
            trimmed.starts_with("async def ") || trimmed.starts_with("@")
        );

        if is_top_level_def && in_block && !current_block.trim().is_empty() {
            edus.push(Edu {
                id: format!("{}_py_{}", base_id, counter),
                text: current_block.trim().to_string(),
                embedding: None,
            });
            counter += 1;
            current_block.clear();
        }

        if is_top_level_def {
            in_block = true;
        }

        current_block.push_str(line);
        current_block.push('\n');
    }

    if !current_block.trim().is_empty() {
        edus.push(Edu {
            id: format!("{}_py_{}", base_id, counter),
            text: current_block.trim().to_string(),
            embedding: None,
        });
    }

    if edus.len() <= 1 && lines.len() > 20 {
        return chunk_by_lines(content, base_id, 30);
    }

    edus
}

/// Parse JavaScript/TypeScript code into EDUs at function/class boundaries.
fn parse_js_edus(content: &str, base_id: &str) -> Vec<Edu> {
    let mut edus = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut current_block = String::new();
    let mut brace_depth: i32 = 0;
    let mut block_start = false;
    let mut counter = 0;

    for line in &lines {
        let trimmed = line.trim();

        let is_block_start = brace_depth == 0 && (
            trimmed.starts_with("function ") || trimmed.starts_with("async function ") ||
            trimmed.starts_with("class ") || trimmed.starts_with("export ") ||
            trimmed.starts_with("const ") && trimmed.contains("=>")
        );

        if is_block_start && !current_block.trim().is_empty() {
            edus.push(Edu {
                id: format!("{}_js_{}", base_id, counter),
                text: current_block.trim().to_string(),
                embedding: None,
            });
            counter += 1;
            current_block.clear();
        }

        if is_block_start {
            block_start = true;
        }

        for ch in trimmed.chars() {
            match ch {
                '{' => brace_depth += 1,
                '}' => brace_depth -= 1,
                _ => {}
            }
        }

        current_block.push_str(line);
        current_block.push('\n');

        if block_start && brace_depth == 0 && trimmed.contains('}') {
            edus.push(Edu {
                id: format!("{}_js_{}", base_id, counter),
                text: current_block.trim().to_string(),
                embedding: None,
            });
            counter += 1;
            current_block.clear();
            block_start = false;
        }
    }

    if !current_block.trim().is_empty() {
        edus.push(Edu {
            id: format!("{}_js_{}", base_id, counter),
            text: current_block.trim().to_string(),
            embedding: None,
        });
    }

    if edus.len() <= 1 && lines.len() > 20 {
        return chunk_by_lines(content, base_id, 30);
    }

    edus
}

/// Fallback: split unknown code by blank-line-separated chunks.
fn parse_generic_code_edus(content: &str, base_id: &str) -> Vec<Edu> {
    chunk_by_lines(content, base_id, 30)
}

/// Chunk content by a fixed number of lines (fallback for unrecognized patterns).
fn chunk_by_lines(content: &str, base_id: &str, chunk_size: usize) -> Vec<Edu> {
    let lines: Vec<&str> = content.lines().collect();
    let mut edus = Vec::new();

    for (i, chunk) in lines.chunks(chunk_size).enumerate() {
        let text = chunk.join("\n");
        if !text.trim().is_empty() {
            edus.push(Edu {
                id: format!("{}_chunk_{}", base_id, i),
                text: text.trim().to_string(),
                embedding: None,
            });
        }
    }

    edus
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_rust() {
        let code = "pub fn main() -> Result<(), Box<dyn Error>> {\n    println!(\"hello\");\n}\n";
        assert!(matches!(detect_language(code), CodeLanguage::Rust));
    }

    #[test]
    fn test_detect_python() {
        let code = "def hello():\n    print('hello')\n\nclass Foo:\n    pass\n";
        assert!(matches!(detect_language(code), CodeLanguage::Python));
    }

    #[test]
    fn test_parse_rust_edus() {
        let code = r#"use std::io;

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn subtract(a: i32, b: i32) -> i32 {
    a - b
}

struct Point {
    x: f32,
    y: f32,
}
"#;
        let edus = parse_code_into_edus(code, "test");
        assert!(edus.len() >= 3, "Expected at least 3 EDUs, got {}", edus.len());
    }

    #[test]
    fn test_parse_python_edus() {
        let code = "import os\n\ndef hello():\n    print('hello')\n\ndef world():\n    print('world')\n\nclass Foo:\n    def bar(self):\n        pass\n";
        let edus = parse_code_into_edus(code, "test");
        assert!(edus.len() >= 3, "Expected at least 3 EDUs, got {}", edus.len());
    }

    #[test]
    fn test_is_code_content() {
        assert!(is_code_content("fn main() {\n    let x = 5;\n    println!(\"{}\", x);\n}\n"));
        assert!(!is_code_content("The weather today is sunny. It will be warm tomorrow."));
    }
}
