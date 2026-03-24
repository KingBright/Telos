#!/usr/bin/env python3
"""Targeted test for Cases 23 (ToolCreation) and 28 (ToolMutation).
Run with: nohup python3 run_case_23_28.py > test_traces/case_23_28.log 2>&1 &
"""
import requests, json, time, uuid, sys, os

API = "http://127.0.0.1:8321/api/v1/run_sync"
TRACES_DIR = "test_traces"
os.makedirs(TRACES_DIR, exist_ok=True)

cases = [
    {
        "id": 23,
        "category": "ToolCreation",
        "query": "帮我创建一个名为 `convert_units` 的工具，用于单位换算。支持：长度（米↔英尺）、重量（千克↔磅）、温度（摄氏↔华氏）。创建成功后，请用这个工具帮我把 180cm 换算成英尺，以及 72°F 换算成摄氏度。",
    },
    {
        "id": 28,
        "category": "ToolMutation",
        "query": "帮我创建一个工具 `get_exchange_rate_v2` 来获取汇率信息，使用 open.er-api.com/v6/latest/USD 这个免费API。但请故意在代码里写一个小错误（比如把URL路径拼错）。执行失败后，请利用 mutate_tool 修复它，然后告诉我1美元等于多少人民币。",
    },
]

def run_query(query, timeout=300):
    start = time.time()
    final_output, heartbeats, error, summary = "", [], None, {}
    try:
        r = requests.post(
            API,
            json={"payload": query, "trace_id": str(uuid.uuid4())},
            headers={"Accept": "text/event-stream"},
            stream=True,
            timeout=timeout,
            proxies={"http": None, "https": None},
        )
        event_type, data_lines = "", []
        for raw_line in r.iter_lines():
            line = raw_line.decode("utf-8") if isinstance(raw_line, bytes) else raw_line
            if line.startswith("event:"):
                event_type = line[6:].strip()
            elif line.startswith("data:"):
                data_lines.append(line[5:].strip())
            elif line == "":
                data = "\n".join(data_lines)
                ts = time.strftime('%H:%M:%S')
                if event_type == "output":
                    final_output = data
                    print(f"  [{ts}] OUTPUT ({len(data)}c)", flush=True)
                elif event_type == "heartbeat":
                    heartbeats.append(data[:200])
                    print(f"  [{ts}] HB: {data[:100]}", flush=True)
                elif event_type == "completed":
                    try: summary = json.loads(data)
                    except: summary = {"raw": data}
                    print(f"  [{ts}] COMPLETED", flush=True)
                elif event_type == "error":
                    error = data
                    print(f"  [{ts}] ERROR: {data[:200]}", flush=True)
                elif event_type == "started":
                    print(f"  [{ts}] STARTED", flush=True)
                else:
                    print(f"  [{ts}] {event_type}", flush=True)
                event_type, data_lines = "", []
        if r.status_code != 200:
            error = f"HTTP {r.status_code}"
    except Exception as e:
        error = str(e)
        final_output = f"ERROR: {e}"
    elapsed = time.time() - start
    return {
        "elapsed": round(elapsed, 1),
        "final_output": final_output,
        "heartbeats": heartbeats,
        "error": error,
        "summary": summary,
        "output_len": len(final_output),
    }

if __name__ == "__main__":
    print(f"=== Tool Creation/Mutation Test ===", flush=True)
    print(f"    Start: {time.strftime('%Y-%m-%d %H:%M:%S')}", flush=True)
    print(f"    API: {API}\n", flush=True)

    for tc in cases:
        cid = tc["id"]
        print(f"━━━ Case {cid} [{tc['category']}] ━━━", flush=True)
        print(f"  Query: {tc['query'][:80]}...", flush=True)

        result = run_query(tc["query"])

        status = "✅" if result["error"] is None and result["output_len"] > 10 else "❌"
        print(f"\n  {status} {result['elapsed']:.1f}s | output={result['output_len']}c | hb={len(result['heartbeats'])}", flush=True)

        # Show output preview
        out = result["final_output"]
        print(f"\n  --- Output (first 1500c) ---", flush=True)
        print(out[:1500], flush=True)
        if len(out) > 1500:
            print(f"\n  --- Output (last 500c) ---", flush=True)
            print(out[-500:], flush=True)

        # Save trace
        trace_path = f"{TRACES_DIR}/case{cid}_retest.json"
        with open(trace_path, "w", encoding="utf-8") as f:
            json.dump(result, f, ensure_ascii=False, indent=2)
        print(f"  Trace: {trace_path}\n", flush=True)

    print(f"\n=== Done: {time.strftime('%Y-%m-%d %H:%M:%S')} ===", flush=True)
