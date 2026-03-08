import sys

with open('crates/telos_tooling/src/native.rs', 'r') as f:
    lines = f.readlines()

new_lines = []
skip = False
for line in lines:
    if "for entry in dir {" in line:
        new_lines.append("        for entry in dir.flatten() {\n")
        skip = True
    elif skip and "if let Ok(entry) = entry {" in line:
        pass
    elif skip and "            }" in line and len(line.strip()) == 1:
        skip = False
    else:
        new_lines.append(line)

with open('crates/telos_tooling/src/native.rs', 'w') as f:
    f.writelines(new_lines)
