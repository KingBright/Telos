import sys

with open('crates/telos_tooling/src/tests.rs', 'r') as f:
    lines = f.readlines()

new_lines = []
for line in lines:
    if "EOF" in line or "    #[tokio::test]" in line:
        pass
    new_lines.append(line)

with open('crates/telos_tooling/src/tests.rs', 'w') as f:
    f.writelines(new_lines)
