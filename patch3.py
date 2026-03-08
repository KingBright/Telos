import sys

with open('crates/telos_daemon/src/main.rs', 'r') as f:
    content = f.read()

target = """    let mut tool_registry = telos_tooling::retrieval::VectorToolRegistry::new_keyword_only();
"""
replacement = """    let mut tool_registry = telos_tooling::retrieval::VectorToolRegistry::new_keyword_only();
    tool_registry.register_tool(
        telos_tooling::native::FsListDirTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::FsListDirTool)),
    );
    tool_registry.register_tool(
        telos_tooling::native::CodeSearchTool::schema(),
        Some(std::sync::Arc::new(telos_tooling::native::CodeSearchTool)),
    );
"""

new_content = content.replace(target, replacement, 1)

with open('crates/telos_daemon/src/main.rs', 'w') as f:
    f.write(new_content)
