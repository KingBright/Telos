import sys

with open("crates/telos_daemon/src/main.rs", "r") as f:
    lines = f.readlines()

new_lines = []
skip = False
i = 0
while i < len(lines):
    if "if let Some(reply) = &dag_plan.reply {" in lines[i]:
        # just look manually for the next block
        next_i = i + 1
        found_dup = False
        while next_i < len(lines) and next_i < i + 20:
            if "if let Some(reply) = &dag_plan.reply {" in lines[next_i]:
                found_dup = True
                break
            next_i += 1

        if found_dup:
            print("Found duplicate at lines", i, next_i)
            # Add the first block
            new_lines.extend(lines[i:next_i])
            # Skip the second block
            # we know the block is 8 lines
            i = next_i + 8
            continue

    new_lines.append(lines[i])
    i += 1

with open("crates/telos_daemon/src/main.rs", "w") as f:
    f.writelines(new_lines)
