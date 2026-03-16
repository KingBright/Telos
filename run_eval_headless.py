#!/usr/bin/env python3
"""
Telos Agent Evaluation Suite — Iteration 16 (Post Memory OS Upgrade)
Tests all agent categories via /api/v1/run_sync SSE endpoint.

Categories: Identity, Math, Common Knowledge, Real-time Search,
            Deep Research, Time Awareness, Coding, Knowledge Reasoning,
            Ambiguous/Edge, Multi-step Planning, Memory, Persona
"""
import requests, json, time, os, uuid, sys, re

API = "http://127.0.0.1:3000/api/v1/run_sync"
BASE_URL = "http://127.0.0.1:3000"
ITER = 17
TRACES_DIR = "test_traces"
os.makedirs(TRACES_DIR, exist_ok=True)

# ─── Test Cases ───────────────────────────────────────────────────────
test_cases = [
    # Category: Identity & Persona
    {
        "id": 1,
        "category": "Identity",
        "query": "你好，你叫什么名字？你是由谁开发的？",
        "description": "身份识别 + 开发者信息",
    },
    # Category: Math & Logic
    {
        "id": 2,
        "category": "Math",
        "query": "计算 25 的平方根加上 150 的 15%",
        "description": "多步数学运算",
    },
    # Category: Common Knowledge
    {
        "id": 3,
        "category": "Knowledge",
        "query": "北京和上海哪个城市面积更大？大多少？",
        "description": "常识比较题",
    },
    # Category: Real-time Search (Weather)
    {
        "id": 4,
        "category": "Search",
        "query": "今天苏州天气怎么样？",
        "description": "实时天气查询 — 需联网搜索",
    },
    # Category: Deep Research
    {
        "id": 5,
        "category": "DeepResearch",
        "query": "总结2026年3月AI领域的最新进展",
        "description": "深度研究 — 多源汇总",
    },
    # Category: Time Awareness
    {
        "id": 6,
        "category": "TimeAware",
        "query": "现在几点了？今天是几月几号？",
        "description": "时间感知 — 需系统时间上下文",
    },
    # Category: Coding (Simple)
    {
        "id": 7,
        "category": "Coding",
        "query": "帮我写一个Python函数，输入一个列表，返回其中所有偶数的平方和",
        "description": "简单编码任务",
    },
    # Category: Knowledge Reasoning
    {
        "id": 8,
        "category": "Reasoning",
        "query": "解释一下什么是Actor-Critic模式，以及它在强化学习中和Q-Learning的区别",
        "description": "概念对比推理",
    },
    # Category: Ambiguous / Edge Case
    {
        "id": 9,
        "category": "EdgeCase",
        "query": "帮我",
        "description": "极短模糊指令 — 测试容错",
    },
    # Category: Multi-step Planning
    {
        "id": 10,
        "category": "Planning",
        "query": "帮我制定一个3天的苏州旅行计划，要包含景点、美食和交通建议",
        "description": "多步规划任务 — 需搜索+结构化输出",
    },
    # Category: Code + Explanation
    {
        "id": 11,
        "category": "Coding",
        "query": "用Rust写一个简单的TCP echo server，并解释每一行代码的作用",
        "description": "带解释的编码任务 — 测试代码质量",
    },
    # Category: Translation + Reasoning
    {
        "id": 12,
        "category": "Reasoning",
        "query": "请将以下句子翻译成英文，并解释其中的文化含义：'塞翁失马，焉知非福'",
        "description": "翻译 + 文化推理",
    },
    # ─── NEW: Memory & Persona Test Cases (Iteration 16) ──────────────
    # Category: Memory - User Preference Storage & Recall
    {
        "id": 13,
        "category": "Memory",
        "query": "请记住：我最喜欢的颜色是蓝色，我喜欢早起",
        "description": "用户偏好记忆存储 — 测试 memory_write 工具",
    },
    # Category: Memory - Cross-session Recall
    {
        "id": 14,
        "category": "Memory",
        "query": "你还记得我喜欢什么颜色吗？",
        "description": "跨会话记忆回忆 — 测试 memory_read 工具",
    },
    # Category: Memory - Conflict/Update
    {
        "id": 15,
        "category": "MemoryConflict",
        "query": "更正一下，我最喜欢的颜色其实是绿色而不是蓝色",
        "description": "记忆冲突更新 — 测试 conflict detection",
    },
    # Category: Persona
    {
        "id": 16,
        "category": "Persona",
        "query": "你有什么独特的个性和特点吗？你和其他AI有什么不同？",
        "description": "人格独立性 — 测试 SOUL persona",
    },
]

# ─── SSE Request Helper ───────────────────────────────────────────────
def run_query(query: str, timeout: int = 300) -> dict:
    """Send query to /api/v1/run_sync, parse SSE events, return result dict."""
    start = time.time()
    final_output, heartbeats, summary = "", [], {}
    error = None

    try:
        r = requests.post(
            API,
            json={"payload": query, "trace_id": str(uuid.uuid4())},
            headers={"Accept": "text/event-stream"},
            stream=True,
            timeout=timeout,
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
                if event_type == "output":
                    final_output = data
                elif event_type == "heartbeat":
                    heartbeats.append(data)
                elif event_type == "clarification":
                    # Auto-select first option for headless eval
                    try:
                        clarify_data = json.loads(data)
                        options = clarify_data.get("options", [])
                        if options:
                            first_opt = options[0].get("id", "opt_1")
                            requests.post(
                                f"{BASE_URL}/api/v1/clarify",
                                json={"task_id": trace_id, "selected_option_id": first_opt},
                                timeout=5,
                            )
                            heartbeats.append(f"[Clarification] Auto-selected: {options[0].get('label', first_opt)}")
                    except Exception:
                        pass
                elif event_type == "completed":
                    try:
                        summary = json.loads(data)
                    except:
                        summary = {"raw": data}
                event_type, data_lines = "", []
    except Exception as e:
        error = str(e)
        final_output = f"ERROR: {e}"

    elapsed = time.time() - start
    full_output = final_output if len(final_output) > 100 else "\n".join(heartbeats + [final_output])

    return {
        "elapsed": round(elapsed, 1),
        "final_output": final_output,
        "heartbeats": heartbeats,
        "full_output": full_output,
        "summary": summary,
        "error": error,
        "output_len": len(full_output),
    }


# ─── Main Execution ──────────────────────────────────────────────────
if __name__ == "__main__":
    print(f"╔══════════════════════════════════════════════════════════╗")
    print(f"║   Telos Agent Evaluation Suite — Iteration {ITER}          ║")
    print(f"║   {len(test_cases)} test cases | API: {API}    ║")
    print(f"╚══════════════════════════════════════════════════════════╝\n")

    results = []
    total_start = time.time()

    for tc in test_cases:
        n = tc["id"]
        print(f"━━━ Case {n:02d} [{tc['category']}] {tc['description']} ━━━")
        print(f"    Query: \"{tc['query'][:60]}{'...' if len(tc['query'])>60 else ''}\"")

        result = run_query(tc["query"])
        result["case_id"] = n
        result["category"] = tc["category"]
        result["query"] = tc["query"]
        result["description"] = tc["description"]
        results.append(result)

        status = "✅" if result["error"] is None and result["output_len"] > 10 else "❌"
        print(f"    {status} {result['elapsed']:.1f}s | output={result['output_len']}c | heartbeats={len(result['heartbeats'])}")
        # Show first 200 chars of output
        preview = result["full_output"][:200].replace("\n", " ")
        print(f"    {preview}\n")

        # Save individual trace
        trace_path = f"{TRACES_DIR}/iter{ITER}_case_{n:02d}.json"
        with open(trace_path, "w", encoding="utf-8") as f:
            json.dump(result, f, ensure_ascii=False, indent=2)

    total_elapsed = time.time() - total_start

    # ─── Summary ──────────────────────────────────────────────────────
    passed = sum(1 for r in results if r["error"] is None and r["output_len"] > 10)
    failed = len(results) - passed

    print(f"\n{'='*60}")
    print(f"  Summary: {passed}/{len(results)} passed, {failed} failed")
    print(f"  Total time: {total_elapsed:.1f}s")
    print(f"  Avg time:   {total_elapsed/len(results):.1f}s")
    print(f"  Traces:     {TRACES_DIR}/iter{ITER}_case_*.json")
    print(f"{'='*60}")

    # Save aggregated results
    agg_path = f"{TRACES_DIR}/iter{ITER}_summary.json"
    with open(agg_path, "w", encoding="utf-8") as f:
        json.dump({
            "iteration": ITER,
            "total_cases": len(results),
            "passed": passed,
            "failed": failed,
            "total_time": round(total_elapsed, 1),
            "avg_time": round(total_elapsed / len(results), 1),
            "cases": [{
                "id": r["case_id"],
                "category": r["category"],
                "query": r["query"],
                "elapsed": r["elapsed"],
                "output_len": r["output_len"],
                "has_error": r["error"] is not None,
                "heartbeat_count": len(r["heartbeats"]),
            } for r in results],
        }, f, ensure_ascii=False, indent=2)

    print(f"\n✅ Complete. Summary: {agg_path}")
    sys.exit(0 if failed == 0 else 1)
