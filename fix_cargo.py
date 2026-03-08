import sys

with open('crates/telos_tooling/Cargo.toml', 'r') as f:
    lines = f.readlines()

new_lines = []
for line in lines:
    if "tempfile" not in line:
        new_lines.append(line)

new_lines.append("tempfile = \"3.10\"\n")

with open('crates/telos_tooling/Cargo.toml', 'w') as f:
    f.writelines(new_lines)
