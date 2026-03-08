import sys

with open("crates/telos_daemon/src/main.rs", "r") as f:
    lines = f.readlines()

new_lines = []
skip = False
i = 0
while i < len(lines):
    if "if let Some(reply) = &dag_plan.reply {" in lines[i] and i + 17 < len(lines):
        block = "".join(lines[i:i+8])
        next_block = "".join(lines[i+9:i+17])

        if block == next_block:
            new_lines.extend(lines[i:i+8])
            i += 17
            continue

    new_lines.append(lines[i])
    i += 1

with open("crates/telos_daemon/src/main.rs", "w") as f:
    f.writelines(new_lines)
