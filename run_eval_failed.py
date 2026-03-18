#!/usr/bin/env python3
"""
Telos Agent Evaluation Suite — Iteration 16 (Post Memory OS Upgrade)
Tests all agent categories via /api/v1/run_sync SSE endpoint.

Categories: Identity, Math, Common Knowledge, Real-time Search,
            Deep Research, Time Awareness, Coding, Knowledge Reasoning,
            Ambiguous/Edge, Multi-step Planning, Memory, Persona
"""
import requests, json, time, os, uuid, sys, re

API = "http://127.0.0.1:8321/api/v1/run_sync"
BASE_URL = "http://127.0.0.1:8321"
ITER = 25
TRACES_DIR = "test_traces"
os.makedirs(TRACES_DIR, exist_ok=True)

# ─── Test Cases ───────────────────────────────────────────────────────
test_cases = [
    # Category: Identity & Persona
    {
        "id": 1,
        "category": "Identity",
        "query": "你是基于什么底层架构运行的？你的创造使命是什么？",
        "description": "身份识别 + 开发者信息",
    },
    # Category: Math & Logic
    {
        "id": 2,
        "category": "Math",
        "query": "假设我在银行存了20000元，年利率是3.5%，复利计算，3年后我总共有多少钱？保留两位小数。",
        "description": "多步数学运算",
    },
    # Category: Common Knowledge
    {
        "id": 3,
        "category": "Knowledge",
        "query": "木星的体积大约是地球的多少倍？它们的主要大气成分分别是什么？",
        "description": "常识比较题",
    },
    # Category: Real-time Search
    {
        "id": 4,
        "category": "Search",
        "query": "请帮我查一下昨天谷歌母公司Alphabet的股票收盘价是多少？",
        "description": "实时财经查询 — 需联网搜索",
    },
    # Category: Deep Research
    {
        "id": 5,
        "category": "DeepResearch",
        "query": "帮我深度调研一下目前固态电池技术的商业化落地现状以及主要厂商",
        "description": "深度研究 — 多源汇总",
    },
    # Category: Time Awareness
    {
        "id": 6,
        "category": "TimeAware",
        "query": "距离今年的圣诞节还有多少天？",
        "description": "时间感知 — 需系统时间上下文",
    },
    # Category: Coding (Simple)
    {
        "id": 7,
        "category": "Coding",
        "query": "用JavaScript写一个能够实现防抖(debounce)功能的函数，并给出一个简单用法示例",
        "description": "简单编码任务",
    },
    # Category: Knowledge Reasoning
    {
        "id": 8,
        "category": "Reasoning",
        "query": "请对比分析一下微服务架构中 Choreography（协同）和 Orchestration（编排）这两种模式的优缺点及适用场景",
        "description": "概念对比推理",
    },
    # Category: Ambiguous / Edge Case
    {
        "id": 9,
        "category": "EdgeCase",
        "query": "怎么",
        "description": "极短模糊指令 — 测试容错",
    },
    # Category: Multi-step Planning
    {
        "id": 10,
        "category": "Planning",
        "query": "我计划周末给朋友办一场大约10人的户外烧烤派对。预算2000元以内，请帮我列一个详尽的采购清单，包括食材、工具和娱乐项目。",
        "description": "多步规划任务 — 需搜索+结构化输出",
    },
    # Category: Code + Explanation
    {
        "id": 11,
        "category": "Coding",
        "query": "用Python实现一个简单的LRU Cache类，要求包含get和put方法，且时间复杂度均为O(1)，并加上关键注释",
        "description": "带解释的编码任务 — 测试代码质量",
    },
    # Category: Translation + Reasoning
    {
        "id": 12,
        "category": "Reasoning",
        "query": "将 '纸上得来终觉浅，绝知此事要躬行' 翻译为英文，并结合现代职场环境阐述它的哲学指导意义",
        "description": "翻译 + 文化推理",
    },
    # Category: Memory - User Preference Storage & Recall
    {
        "id": 13,
        "category": "Memory",
        "query": "请帮我记录一下：我对海鲜严重过敏，而且我不吃香菜",
        "description": "用户偏好记忆存储 — 测试 memory_write 工具",
    },
    # Category: Memory - Cross-session Recall
    {
        "id": 14,
        "category": "Memory",
        "query": "考虑到我的饮食禁忌，今晚去吃日料合适吗？",
        "description": "跨会话记忆回忆 — 测试 memory_read 工具",
    },
    # Category: Memory - Conflict/Update
    {
        "id": 15,
        "category": "MemoryConflict",
        "query": "不好意思我昨天记错了，其实我是对花生过敏，海鲜我是可以吃的！",
        "description": "记忆冲突更新 — 测试 conflict detection",
    },
    # Category: Persona
    {
        "id": 16,
        "category": "Persona",
        "query": "如果遇到你无法解决或者是系统错误的情况，你通常会表现出什么样的态度？",
        "description": "人格独立性 — 测试 SOUL persona",
    },
    # Case 17: In-window recall
    {
        "id": 17,
        "category": "HistoryRecall",
        "query": "我刚才是不是又修改了我的过敏原信息？最后确认的是什么过敏？",
        "description": "近期历史回忆 — 窗口内，应直接从对话历史回答",
    },
    # Case 18: In-window contextual back-reference
    {
        "id": 18,
        "category": "HistoryRecall",
        "query": "我们前面写的那个LRU Cache类，如果容量设为完全一样，它和普通的内置字典在删除元素倾向上有什么区别？",
        "description": "上下文指代回忆 — 窗口内，测试'之前'代词消歧",
    },
    # Case 19: Out-of-window recall
    {
        "id": 19,
        "category": "DeepMemoryRecall",
        "query": "回到我们刚开始算的那个存款利息的数学题，如果是单利计算，结果会差多少钱？",
        "description": "深度记忆回忆 — 窗口外，应触发 memory_read 工具检索",
    },
    # Case 20: Multi-fact cross-turn recall
    {
        "id": 20,
        "category": "HistoryRecall",
        "query": "回顾一下我们聊到现在，你都掌握了关于我个人的哪些健康和偏好信息？",
        "description": "跨轮次摘要 — 需整合多轮对话",
    },
    # Case 21: Implicit preference application from memory
    {
        "id": 21,
        "category": "PreferenceApplication",
        "query": "帮我推荐三道适合做今晚晚餐的简单家常菜",
        "description": "隐式偏好应用 — 测试agent是否主动避开花生/香菜等忌口",
    },
    # Case 22: False memory test
    {
        "id": 22,
        "category": "FalseMemoryGuard",
        "query": "我们之前是不是讨论过怎么修理漏水的马桶？",
        "description": "虚假记忆防护 — 测试agent不会捏造不存在的对话",
    },
    # Case 23: Tool Creation & Dynamic Registration
    {
        "id": 23,
        "category": "ToolCreation",
        "query": "帮我创建一个名为 `get_crypto_price` 的工具，用于获取比特币(BTC)当前的美元价格，你可以用任何无需API Key的公共API。创建成功后请立刻调用一次并告诉我当前价格。",
        "description": "动态工具自造与立即调用 — 测试 ScriptSandbox 和注册流",
    },
    # Case 24: Procedural Memory Setup (Learning a Workflow)
    {
        "id": 24,
        "category": "ProceduralSetup",
        "query": "我这有一段存在安全隐患的SQL查询语句：`SELECT * FROM users WHERE username = '\" + userInput + \"' AND password = '\" + passInput + \"'`。请帮我指出它的漏洞并给出修复建议。在这之后，请把你的安全审查步骤提炼成一个名为 'SQL_Injection_Audit' 的经验模板存入你的程序记忆中。",
        "description": "流程经验蒸馏 — 测试工作流模版提取",
    },
    # Case 25: Procedural Memory Application (Reusing Workflow)
    {
        "id": 25,
        "category": "ProceduralApply",
        "query": "我又发现了一段糟糕的代码：`cursor.execute(f\"UPDATE accounts SET balance = balance - {withdraw_amount} WHERE id = {user_id}\")`。请严格按照我们前一步总结的 'SQL_Injection_Audit' 流程来审查并修复它。",
        "description": "流程经验重用 — 测试从 Procedural Memory 检索并实例化模版",
    }
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
                                proxies={"http": None, "https": None},
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
        
        if r.status_code != 200:
            error = f"HTTP {r.status_code}: {r.text}"
            final_output = f"ERROR: HTTP {r.status_code}"
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

    test_cases_filtered = [tc for tc in test_cases if tc["id"] in [19, 23, 25]]
    for tc in test_cases_filtered:
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
