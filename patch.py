import sys

with open('crates/telos_daemon/src/main.rs', 'r') as f:
    lines = f.readlines()

new_lines = []
inserted = False
for line in lines:
    if not inserted and 'tool_registry.register_tool(' in line:
        new_lines.append('    tool_registry.register_tool(\n')
        new_lines.append('        telos_tooling::native::FsListDirTool::schema(),\n')
        new_lines.append('        Some(std::sync::Arc::new(telos_tooling::native::FsListDirTool)),\n')
        new_lines.append('    );\n')
        new_lines.append('    tool_registry.register_tool(\n')
        new_lines.append('        telos_tooling::native::CodeSearchTool::schema(),\n')
        new_lines.append('        Some(std::sync::Arc::new(telos_tooling::native::CodeSearchTool)),\n')
        new_lines.append('    );\n')
        inserted = True
    new_lines.append(line)

with open('crates/telos_daemon/src/main.rs', 'w') as f:
    f.writelines(new_lines)
