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
ITER = 27
TRACES_DIR = "test_traces"
os.makedirs(TRACES_DIR, exist_ok=True)

# ─── Test Cases ───────────────────────────────────────────────────────
test_cases = [
    # Category: Identity & Self-Awareness
    {
        "id": 1,
        "category": "Identity",
        "query": "你和 ChatGPT 有什么本质区别？你有哪些独特的能力是它没有的？",
        "description": "自我认知差异化 — 测试 SOUL persona 边界意识",
    },
    # Category: Math & Logic (应用题)
    {
        "id": 2,
        "category": "Math",
        "query": "某工厂有A、B两条生产线。A线每小时生产120件产品，B线每小时生产80件。客户订单需要4800件产品，但B线在运行3小时后发生故障停机维修2小时后恢复。请问从开工到完成全部订单，最少需要几小时？",
        "description": "生产线应用题 — 需要分段计算",
    },
    # Category: Knowledge (物理原理)
    {
        "id": 3,
        "category": "Knowledge",
        "query": "为什么飞机在万米高空飞行时，机舱外温度可以达到零下50度，而机舱内却很温暖？请从工程和物理两个角度解释。",
        "description": "物理常识+原理解释",
    },
    # Category: Real-time Search (金融)
    {
        "id": 4,
        "category": "Search",
        "query": "比特币现在多少钱？以太坊呢？最近24小时涨了还是跌了？",
        "description": "实时加密货币查询 — 双币种对比",
    },
    # Category: Deep Research (新话题)
    {
        "id": 5,
        "category": "DeepResearch",
        "query": "帮我深度调研一下2026年全球AI芯片市场的竞争格局，包括主要玩家（英伟达、AMD、Intel、华为、Google TPU等）和各自的技术路线",
        "description": "AI芯片深度调研 — 新话题、多厂商",
    },
    # Category: Time Awareness (复合计算)
    {
        "id": 6,
        "category": "TimeAware",
        "query": "今天是星期几？本月还剩多少个工作日（不算周末）？",
        "description": "复合时间计算 — 需要星期+日历推理",
    },
    # Category: Coding (Rust并发)
    {
        "id": 7,
        "category": "Coding",
        "query": "用Rust写一个泛型的线程安全LRU Cache结构体，支持get、put和len方法，容量在初始化时指定，并写单元测试",
        "description": "Rust泛型+并发+LRU算法",
    },
    # Category: Reasoning (技术+商业)
    {
        "id": 8,
        "category": "Reasoning",
        "query": "请从技术生态和商业竞争两个角度分析，为什么 Rust 目前还没有完全取代 C++ 在系统编程领域的地位？未来5年你认为会改变吗？",
        "description": "技术+商业混合推理 — 需要多维度分析",
    },
    # Category: Edge Case (符号输入)
    {
        "id": 9,
        "category": "EdgeCase",
        "query": "🤔",
        "description": "极端表情输入 — 仅一个emoji",
    },
    # Category: Planning (旅行规划)
    {
        "id": 10,
        "category": "Planning",
        "query": "我和3个朋友下周末想去成都吃火锅+看大熊猫，两天一夜，人均预算800元（不含交通），帮我规划行程，要考虑我的花生过敏。",
        "description": "短途旅行规划 — 需结合用户偏好",
    },
    # Category: Coding (Go设计模式)
    {
        "id": 11,
        "category": "Coding",
        "query": "用Go语言实现一个并发安全的发布-订阅(pub/sub)模式，支持按topic过滤消息，并写一个使用示例",
        "description": "Go语言设计模式 — 区别于JS/Python/Rust",
    },
    # Category: Reasoning (跨学科假设推理)
    {
        "id": 12,
        "category": "Reasoning",
        "query": "如果地球突然停止自转（但公转不变），会发生什么？请从物理学、气象学和生物学三个角度分析。",
        "description": "跨学科假设推理 — 需要多领域知识整合",
    },
    # Category: Memory - User Preference Storage & Recall
    {
        "id": 13,
        "category": "Memory",
        "query": "帮我记一下：我有乳糖不耐受，喝牛奶会拉肚子。还有我特别怕辣，微辣都受不了",
        "description": "用户偏好记忆存储 — 新的健康信息",
    },
    # Category: Memory - Cross-session Recall
    {
        "id": 14,
        "category": "Memory",
        "query": "考虑到我的饮食禁忌，下午想去喝杯奶茶合适吗？",
        "description": "跨会话记忆回忆 — 测试乳糖不耐受的应用",
    },
    # Category: Memory - Conflict/Update
    {
        "id": 15,
        "category": "MemoryConflict",
        "query": "不好意思我之前说错了，我不是乳糖不耐受，我其实是对牛奶蛋白过敏，豆奶、燕麦奶这些植物奶我是可以喝的！",
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
        "query": "我刚才是不是修改了我的饮食禁忌信息？最后确认的是什么情况？",
        "description": "近期历史回忆 — 窗口内，应直接从对话历史回答",
    },
    # Case 18: In-window contextual back-reference
    {
        "id": 18,
        "category": "HistoryRecall",
        "query": "回头看看，我们前面写的那段Rust代码用了什么同步原语？如果改成无锁设计会有什么权衡？",
        "description": "上下文指代回忆 + 延伸推理",
    },
    # Case 19: Out-of-window recall
    {
        "id": 19,
        "category": "DeepMemoryRecall",
        "query": "回到最开始的生产线问题，如果B线故障停机时间从2小时变成4小时，最少需要几小时完成订单？",
        "description": "深度记忆回忆 — 窗口外，测试对生产线问题参数的回忆",
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
        "query": "帮我创建一个名为 `get_weather` 的工具，用于获取指定城市的当前天气信息。请使用 open-meteo.com 的免费API（不需要 API Key，示例：api.open-meteo.com/v1/forecast?latitude=31.30&longitude=120.62&current=temperature_2m,weather_code,wind_speed_10m,relative_humidity_2m&timezone=Asia/Shanghai）。创建成功后请立刻用这个工具查询苏州的天气并告诉我。",
        "description": "动态工具自造 — 使用 open-meteo.com API（中国可访问，稳定可靠）",
    },
    # Case 24: Procedural Memory Setup (Learning a Workflow)
    {
        "id": 24,
        "category": "ProceduralSetup",
        "query": "帮我审查这段代码的安全隐患：`os.system(f'ping -c 4 {user_input}')`。请详细分析漏洞类型并给出修复方案。之后请把你的命令注入审查流程提炼成一个名为 'Command_Injection_Audit' 的经验模板存入你的程序记忆中。",
        "description": "命令注入审查 — 测试新漏洞类型的流程蒸馏",
    },
    # Case 25: Procedural Memory Application (Reusing Workflow)
    {
        "id": 25,
        "category": "ProceduralApply",
        "query": "又发现一段危险代码：`subprocess.call(f'convert {filename} output.pdf', shell=True)`，其中filename来自用户上传。请严格按照前一步总结的 'Command_Injection_Audit' 流程来审查并修复它。",
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
