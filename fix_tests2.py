import sys

with open('crates/telos_tooling/src/tests.rs', 'r') as f:
    lines = f.readlines()

# find the last "}" that closes the module
closing_bracket_index = -1
for i in range(len(lines) - 1, -1, -1):
    if lines[i].strip() == "}":
        closing_bracket_index = i
        break

if closing_bracket_index != -1:
    lines[closing_bracket_index] = "\n"
    lines.append("}\n")

with open('crates/telos_tooling/src/tests.rs', 'w') as f:
    f.writelines(lines)
