import sys

with open("crates/telos_daemon/src/main.rs", "r") as f:
    lines = f.readlines()

new_lines = []
skip = False
i = 0
while i < len(lines):
    if "if let Some(reply) = &dag_plan.reply {" in lines[i]:
        # Count consecutive identical blocks and skip the second one
        block = "".join(lines[i:i+8])
        next_block = "".join(lines[i+9:i+17]) # Taking into account the empty newline between

        if block == next_block:
            new_lines.extend(lines[i:i+8])
            i += 17 # Skip the duplicate and the newline
            continue

    new_lines.append(lines[i])
    i += 1

with open("crates/telos_daemon/src/main.rs", "w") as f:
    f.writelines(new_lines)
