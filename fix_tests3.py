import sys

with open('crates/telos_tooling/src/tests.rs', 'r') as f:
    lines = f.readlines()

new_lines = []
for line in lines:
    if "use crate::native::FsListDirTool;" in line:
        new_lines.append(line)
        new_lines.append("        use serde_json::json;\n")
        new_lines.append("        use crate::ToolExecutor;\n")
    elif "use crate::native::CodeSearchTool;" in line:
        new_lines.append(line)
        new_lines.append("        use serde_json::json;\n")
        new_lines.append("        use crate::ToolExecutor;\n")
    else:
        new_lines.append(line)

with open('crates/telos_tooling/src/tests.rs', 'w') as f:
    f.writelines(new_lines)
