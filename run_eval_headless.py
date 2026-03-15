import subprocess
import json
import time
import os

os.makedirs("test_traces", exist_ok=True)

queries = [
    # Baseline
    "今天天气怎么样？",
    "计算 25 的平方根加上 150 的 15%", # Case 2: previously caused infinite loop
    "帮我计划一个为期3天的北京旅游行程",
    "2026年3月14日火星发生的爆炸新闻是什么？",
    # Phase 4/5
    "你是谁？你的名字是什么？",
    "执行一个长文本研究任务：总结AI在2026年的前沿进展",
    "系统现在正在跑什么编程或是搜索任务吗？",
    "回忆一下在刚才的测试中，我问过的第一个问题是什么？"
]

print("Starting Deterministic Headless Execution via CLI...")
print("NOTE: No timeout or parallelism. All cases run sequentially.")
for i, q in enumerate(queries):
    case_num = i + 1
    print(f"========================================")
    print(f"Running Case {case_num}: {q}")
    
    start_time = time.time()
    # No timeout — the CLI's idle-based timeout (120s) handles deadlock detection.
    # All cases run sequentially to avoid concurrent API pressure.
    result = subprocess.run(
        [os.path.expanduser("~/.cargo/bin/telos"), "run", q],
        capture_output=True,
        text=True,
    )
    elapsed = time.time() - start_time
    print(f"Finished Case {case_num} in {elapsed:.2f}s")
    print(result.stdout)
    if result.stderr:
        print("Errors:", result.stderr[:500])  # Truncate verbose stderr
        
    with open(f"test_traces/iter12_case_{case_num}_cli_output.log", "w", encoding="utf-8") as f:
        f.write(result.stdout + "\n" + result.stderr)

print("========================================")
print("All tasks finished successfully.")
