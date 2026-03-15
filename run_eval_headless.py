#!/usr/bin/env python3
"""Telos 评估数据采集 — 调用 /api/v1/run_sync SSE 收集输出"""
import requests, json, time, os, uuid

API = "http://127.0.0.1:3000/api/v1/run_sync"
os.makedirs("test_traces", exist_ok=True)

queries = [
    "你好，你叫什么名字？",
    "计算 25 的平方根加上 150 的 15%",
    "北京和上海哪个城市面积更大？大多少？",
    "今天苏州天气怎么样？",
    "总结2026年3月AI领域的最新进展",
    "现在几点了？今天是几月几号？",
    "帮我写一个Python函数，输入一个列表，返回其中所有偶数的平方和",
    "解释一下什么是Actor-Critic模式",
]

for i, q in enumerate(queries):
    n = i + 1
    print(f"\n== Case {n}: {q}")
    start = time.time()
    final_output, heartbeats, summary = "", [], {}
    try:
        r = requests.post(API, json={"payload": q, "trace_id": str(uuid.uuid4())},
                          headers={"Accept": "text/event-stream"}, stream=True, timeout=240)
        # Parse SSE: accumulate multiline data blocks
        event_type, data_lines = "", []
        for raw_line in r.iter_lines():
            line = raw_line.decode("utf-8") if isinstance(raw_line, bytes) else raw_line
            if line.startswith("event:"):
                event_type = line[6:].strip()
            elif line.startswith("data:"):
                data_lines.append(line[5:].strip())
            elif line == "":
                # Event boundary — flush
                data = "\n".join(data_lines)
                if event_type == "output":
                    final_output = data
                elif event_type == "heartbeat":
                    heartbeats.append(data)
                elif event_type == "completed":
                    try: summary = json.loads(data)
                    except: summary = {"raw": data}
                event_type, data_lines = "", []
    except Exception as e:
        final_output = f"ERROR: {e}"

    elapsed = time.time() - start
    # Combine: if final_output is short, heartbeats may have the real content
    full_output = final_output if len(final_output) > 100 else "\n".join(heartbeats + [final_output])

    print(f"   {elapsed:.1f}s | final={len(final_output)}c | heartbeats={len(heartbeats)} | total={len(full_output)}c")
    print(f"   {full_output[:200]}")

    with open(f"test_traces/iter14_case_{n}.json", "w", encoding="utf-8") as f:
        json.dump({
            "query": q, "elapsed": round(elapsed,1),
            "final_output": final_output,
            "heartbeats": heartbeats,
            "full_output": full_output,
            "summary": summary,
        }, f, ensure_ascii=False, indent=2)

print(f"\n✅ Done. Traces in test_traces/iter14_case_*.json")
