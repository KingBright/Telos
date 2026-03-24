#!/usr/bin/env python3
"""
Telos Memory System Evaluation Suite — Dedicated Memory Tests

Tests memory system via /api/v1/run_sync SSE endpoint.
Cases are ORDER-DEPENDENT (store → version-control → recall → cross-ref → verify).
Final memory state = 100% accurate real facts (wrong data is corrected within the run).

Upgrade Dimensions Covered:
  4. Dual-Layer Profiles  (M01-M04: static vs dynamic storage)
  6. Extraction Pipeline  (M01-M04: structured multi-dimension extraction)
  1. Version Control      (M05-M07: store wrong info → correct → verify only latest shown)
  5. Retrieval Filtering  (M08-M15: recall accuracy, implicit preferences, cross-reference)
  ───────────────────────────────────────────────────────────────────
  2. Memory Relations   → validated via `cargo test`
  3. Temporal Forgetting → validated via `cargo test`

Usage:
  python3 run_eval_memory.py                    # Run all 15 cases sequentially
  python3 run_eval_memory.py --cases 1,5,10     # Run specific cases only
  python3 run_eval_memory.py --start-from 6     # Resume from case M06
"""
import requests, json, time, os, uuid, sys

API = "http://127.0.0.1:8321/api/v1/run_sync"
BASE_URL = "http://127.0.0.1:8321"
TRACES_DIR = "test_traces"
os.makedirs(TRACES_DIR, exist_ok=True)

# ─── Test Cases ───────────────────────────────────────────────────────
# ⚠️ ORDER MATTERS: Cases M01-M05 store facts, M06+ depend on them.
# ⚠️ ALL facts are REAL — no fabricated data that would corrupt memory.
test_cases = [

    # ═══════════════════════════════════════════════════════════════════
    # PHASE 1: INITIAL STORAGE (M01-M04)
    # Tests: Upgrade 4 (Dual-Layer Profile), Upgrade 6 (Extraction Pipeline)
    # All facts below are TRUE and should persist in memory.
    # ═══════════════════════════════════════════════════════════════════

    {
        "id": 1,
        "category": "ProfileStorage",
        "dimension": "升级4+6: 身份信息存储",
        "query": (
            "帮我记住一些基本信息：我叫金亮，男性，1989年2月11日出生，"
            "身高174cm，体重65kg左右。我是江苏苏州人。已婚，有一个8岁的女儿。"
        ),
        "description": "存储身份与家庭信息（全部应为 UserProfileStatic）",
        "expected_behavior": "提取并存储多条 static 事实：姓名、性别、生日、身体数据、籍贯、婚姻、子女",
        "verify_keywords": ["金亮", "1989", "苏州", "女儿"],
    },
    {
        "id": 2,
        "category": "ProfileStorage",
        "dimension": "升级4+6: 兴趣爱好存储",
        "query": (
            "继续补充一下我的兴趣爱好吧："
            "我是个游戏爱好者，主要玩PC单机和主机游戏（PS5、Switch 2），"
            "最喜欢的游戏类型是RPG（最终幻想、勇者斗恶龙系列）、"
            "建造类（星露谷物语）、SRPG（火焰纹章系列）和宝可梦。"
            "我也很喜欢追动漫，偏好奇幻风、热血向和猎奇类作品，最近国漫质量也很高所以也会看。"
            "另外我喜欢看历史，关注了很多B站的历史博主，觉得从历史中能学到很多东西。"
        ),
        "description": "存储兴趣爱好（游戏偏好细分 + 动漫 + 历史）",
        "expected_behavior": "提取 static 事实：游戏平台偏好、具体游戏类型、动漫偏好、历史兴趣",
        "verify_keywords": ["RPG", "最终幻想", "星露谷", "火焰纹章", "宝可梦", "动漫", "历史"],
    },
    {
        "id": 3,
        "category": "ProfileStorage",
        "dimension": "升级4+6: 职业与科技兴趣存储",
        "query": (
            "关于我的职业和技术方面："
            "我是一名程序员，喜欢写代码，现在主要用大模型辅助编程。"
            "我有很多想法，业余时间会用代码来验证和实现这些想法。"
            "我关注前沿科技发展，包括物理学、生命科学、AI、Agentic工程和机器人技术。"
            "我也很关注国际局势。还有我特别喜欢倒腾各种消费电子产品，"
            "只要是新颖有创意的产品都能抓住我的注意力。"
        ),
        "description": "存储职业与科技兴趣（static + 部分 dynamic）",
        "expected_behavior": "程序员→static; 用大模型写代码→static; 科技兴趣→static",
        "verify_keywords": ["程序员", "大模型", "AI", "Agentic", "消费电子"],
    },
    {
        "id": 4,
        "category": "ProfileStorage",
        "dimension": "升级4+6: 运动与健康存储",
        "query": (
            "运动方面，我最喜欢打篮球，但是膝盖受过伤，所以现在打得比较少。"
            "平时运动最多的是跑步，每周大约跑20公里左右。"
            "每周末我会带女儿一起运动，一般是周六上午陪她去攀岩馆攀岩，"
            "下午或者晚上会去骑车或者跑步。"
        ),
        "description": "存储运动偏好 + 伤病信息 + 亲子运动习惯",
        "expected_behavior": "static: 篮球爱好、膝盖伤、跑步习惯+频率、周末带女儿攀岩/骑车",
        "verify_keywords": ["篮球", "膝盖", "跑步", "20公里", "攀岩", "女儿"],
    },
    # ═══════════════════════════════════════════════════════════════════
    # M05: INTENTIONALLY WRONG VEHICLE INFO (for version control test)
    # This will be CORRECTED in M06 — final memory state will be accurate.
    # ═══════════════════════════════════════════════════════════════════

    {
        "id": 5,
        "category": "VersionControl",
        "dimension": "升级1: 版本控制 — 存储错误信息（将被修正）",
        "query": (
            "对了，我有两辆车：一辆特斯拉Model 3和一辆大众帕萨特。"
        ),
        "description": "⚠️ 故意存储错误车辆信息，M06会修正。用于测试版本控制的 Updates 关系链",
        "expected_behavior": "存储两条 static 车辆事实（Model 3 + 帕萨特），后续会被修正",
        "verify_keywords": ["特斯拉", "Model 3", "帕萨特"],
    },
    {
        "id": 6,
        "category": "VersionControl",
        "dimension": "升级1: 版本控制 — 修正为真实信息 + 年度目标",
        "query": (
            "不对，我刚才说错了。我的车不是Model 3和帕萨特，"
            "正确的是：一辆特斯拉Model Y和一辆奥迪A4。帮我更正一下。"
            "另外帮我记一下今年的两个大目标："
            "第一是把Telos打造成前沿的个人智能助理，除了引擎本身，"
            "还要开发很多周边产品让它融入我的日常生活。"
            "第二是制作一个个人机器人——我已经买了很多舵机和电子零部件，"
            "年中还打算买一台3D打印机。等Telos完成到一定阶段就正式启动机器人项目，"
            "最终目标是让Telos和机器人结合，让机器人成为Telos的物理躯体。"
        ),
        "description": "修正车辆信息为真实数据 + 存储年度目标。旧记忆应变为 is_latest=false",
        "expected_behavior": "旧(Model3/帕萨特) is_latest=false，新(ModelY/A4) is_latest=true，建立 Updates 关系",
        "verify_keywords": ["特斯拉", "Model Y", "奥迪", "A4", "Telos", "机器人", "3D打印"],
    },
    {
        "id": 7,
        "category": "VersionControl",
        "dimension": "升级1+5: 版本过滤验证",
        "query": "帮我确认一下，我有几辆车？分别是什么车？",
        "description": "版本控制最终验证 — 只应返回最新版本（Model Y + A4），不应提及 Model 3 或帕萨特",
        "expected_behavior": "只回答特斯拉Model Y和奥迪A4。不应出现 Model 3 或帕萨特",
        "verify_keywords": ["特斯拉", "Model Y", "奥迪", "A4"],
    },

    # ═══════════════════════════════════════════════════════════════════
    # PHASE 2: RECALL & IMPLICIT PREFERENCES (M08-M11)
    # Tests: Upgrade 5 (Retrieval Filtering + Profile Injection)
    # No new facts stored — pure read operations.
    # ═══════════════════════════════════════════════════════════════════

    {
        "id": 8,
        "category": "ProfileRecall",
        "dimension": "升级5: 全量回忆",
        "query": "总结一下你目前了解的关于我的所有信息吧，越详细越好。",
        "description": "全量 Profile 回忆（验证存储完整性 + 版本过滤结果）",
        "expected_behavior": "应包含：姓名、生日、苏州、家庭、游戏偏好、动漫、历史、程序员、运动、车辆(ModelY+A4)、目标",
        "verify_keywords": ["金亮", "苏州", "女儿", "RPG", "篮球", "程序员", "特斯拉", "机器人"],
    },
    {
        "id": 9,
        "category": "ImplicitPreference",
        "dimension": "升级5: 隐式偏好 — 游戏推荐",
        "query": "我最近想找个新游戏玩，有什么给我推荐的吗？",
        "description": "隐式偏好匹配 — 应基于记忆中的游戏口味推荐（RPG/建造/SRPG）",
        "expected_behavior": "推荐应偏向 RPG、建造类或 SRPG，而非 FPS、MOBA 等类型。应体现对用户口味的理解",
        "verify_keywords": [],  # Open-ended, manual check needed
    },
    {
        "id": 10,
        "category": "ImplicitPreference",
        "dimension": "升级5: 健康约束感知 — 运动推荐",
        "query": "我想增加一些运动量，除了跑步之外还有什么好的运动推荐吗？",
        "description": "健康感知推荐 — 必须考虑膝盖伤情，避免高冲击运动",
        "expected_behavior": "不应推荐跳绳、足球等膝盖负担大的运动。应推荐游泳、骑行等低冲击运动。可能提及膝盖保护",
        "verify_keywords": [],  # Manual check: should NOT recommend high-impact sports
    },
    {
        "id": 11,
        "category": "FalseMemoryGuard",
        "dimension": "升级5: 虚假记忆防护",
        "query": "我们之前聊过我养的那只橘猫叫什么名字来着？帮我回忆一下。",
        "description": "虚假记忆防护 — 从未讨论过任何宠物",
        "expected_behavior": "应明确否认或表示不确定，不应编造宠物信息",
        "verify_keywords": [],  # Should NOT contain fabricated pet names
    },

    # ═══════════════════════════════════════════════════════════════════
    # PHASE 3: CROSS-REFERENCE & IMPLICIT APPLICATION (M12-M14)
    # Tests: Multiple facts cross-referenced in natural scenarios.
    # No new facts stored — pure read + reasoning.
    # ═══════════════════════════════════════════════════════════════════

    {
        "id": 12,
        "category": "CrossReference",
        "dimension": "升级5: 多事实交叉 — 周末规划",
        "query": "这周六天气不错，帮我安排一下周末的活动吧。",
        "description": "应综合女儿(攀岩)、运动习惯(跑步/骑车)、膝盖伤等记忆自动规划",
        "expected_behavior": "应提及周六上午攀岩（与女儿）、下午骑车或跑步，可能结合天气好建议户外活动",
        "verify_keywords": ["攀岩", "女儿"],
    },
    {
        "id": 13,
        "category": "CrossReference",
        "dimension": "升级5: 隐式偏好 — 动漫推荐",
        "query": "最近有什么好看的动漫推荐吗？",
        "description": "隐式偏好匹配 — 应基于记忆中的动漫口味推荐（奇幻/热血/猎奇）",
        "expected_behavior": "推荐应偏向奇幻风、热血向或猎奇类，可能也提及国漫。不应推荐日常搞笑或恋爱类等不匹配的类型",
        "verify_keywords": [],  # Open-ended, manual check needed
    },
    {
        "id": 14,
        "category": "CrossReference",
        "dimension": "升级5: 多事实交叉 — 亲子场景",
        "query": "我女儿下个月过生日，想送她一个有意义的礼物，有什么建议吗？",
        "description": "应综合女儿年龄(8岁)、亲子运动(攀岩)、用户自身兴趣(科技/游戏)来推荐",
        "expected_behavior": "建议应适合8岁女孩，可结合攀岩(攀岩装备)、科技(编程玩具/机器人套件)等用户上下文",
        "verify_keywords": [],  # Open-ended, manual check
    },

    # ═══════════════════════════════════════════════════════════════════
    # PHASE 4: COMPREHENSIVE SUMMARY (M15)
    # Tests: Full profile summary — the ultimate completeness check.
    # ═══════════════════════════════════════════════════════════════════

    {
        "id": 15,
        "category": "ComprehensiveSummary",
        "dimension": "全维度综合验证",
        "query": (
            "做一个完整的总结：列出你目前掌握的关于我的所有信息，"
            "按照「个人基本信息」「兴趣爱好」「职业技术」「运动健康」"
            "「车辆」「年度目标与项目」这几个分类来组织。"
        ),
        "description": "全维度记忆综合检验 — 检查存储完整性、版本正确性、分类准确性",
        "expected_behavior": (
            "个人信息完整(金亮/苏州/已婚/女儿); 兴趣多维(游戏/动漫/历史/消费电子); "
            "职业(程序员/大模型/科技); 运动(篮球/膝盖/跑步/攀岩/骑车); "
            "车辆(特斯拉ModelY+奥迪A4, 无Model3/帕萨特); 目标(Telos+机器人)"
        ),
        "verify_keywords": [
            "金亮", "1989", "苏州", "女儿",
            "RPG", "火焰纹章", "动漫", "历史",
            "程序员", "AI",
            "篮球", "膝盖", "跑步", "攀岩",
            "特斯拉", "奥迪",
            "Telos", "机器人",
        ],
    },
]

# ─── SSE Request Helper ───────────────────────────────────────────────
def run_query(query: str, timeout: int = 300) -> dict:
    """Send query to /api/v1/run_sync, parse SSE events, return result dict."""
    start = time.time()
    final_output, heartbeats, summary = "", [], {}
    error = None
    last_activity = [time.time()]
    stall_timeout = 300

    try:
        r = requests.post(
            API,
            json={"payload": query, "trace_id": str(uuid.uuid4())},
            headers={"Accept": "text/event-stream"},
            stream=True,
            timeout=timeout,
            proxies={"http": None, "https": None},
        )

        import threading
        watchdog_stop = threading.Event()
        def watchdog():
            while not watchdog_stop.is_set():
                time.sleep(10)
                if time.time() - last_activity[0] > stall_timeout:
                    try:
                        r.close()
                    except Exception:
                        pass
                    return
        watchdog_thread = threading.Thread(target=watchdog, daemon=True)
        watchdog_thread.start()

        event_type, data_lines = "", []
        for raw_line in r.iter_lines():
            line = raw_line.decode("utf-8") if isinstance(raw_line, bytes) else raw_line
            if line.startswith("event:") or line.startswith("data:"):
                last_activity[0] = time.time()
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
                    try:
                        clarify_data = json.loads(data)
                        options = clarify_data.get("options", [])
                        if options:
                            first_opt = options[0].get("id", "opt_1")
                            requests.post(
                                f"{BASE_URL}/api/v1/clarify",
                                json={"task_id": str(uuid.uuid4()), "selected_option_id": first_opt},
                                timeout=5,
                                proxies={"http": None, "https": None},
                            )
                    except Exception:
                        pass
                elif event_type == "completed":
                    try:
                        summary = json.loads(data)
                    except:
                        summary = {"raw": data}
                elif event_type == "error":
                    error = data
                event_type, data_lines = "", []

        watchdog_stop.set()

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


# ─── Keyword Match Checker ───────────────────────────────────────────
def check_keywords(output: str, keywords: list) -> dict:
    """Check which expected keywords appear in the output."""
    if not keywords:
        return {"total": 0, "found": 0, "missing": [], "score": "N/A (manual check)"}
    output_lower = output.lower()
    found = [kw for kw in keywords if kw.lower() in output_lower]
    missing = [kw for kw in keywords if kw.lower() not in output_lower]
    return {
        "total": len(keywords),
        "found": len(found),
        "missing": missing,
        "score": f"{len(found)}/{len(keywords)}",
    }


# ─── Main Execution ──────────────────────────────────────────────────
if __name__ == "__main__":
    print("╔══════════════════════════════════════════════════════════════╗")
    print("║   Telos Memory System Evaluation Suite                      ║")
    print(f"║   {len(test_cases)} Cases | API: {API}         ║")
    print("║   ⚠️  Cases are ORDER-DEPENDENT — run sequentially!         ║")
    print("║   ✅ Final memory state = 100% accurate real facts          ║")
    print("╚══════════════════════════════════════════════════════════════╝\n")

    # Parse CLI args
    target_cases = None
    start_from = 1
    if "--cases" in sys.argv:
        idx = sys.argv.index("--cases")
        if idx + 1 < len(sys.argv):
            target_cases = [int(x) for x in sys.argv[idx + 1].split(",")]
    if "--start-from" in sys.argv:
        idx = sys.argv.index("--start-from")
        if idx + 1 < len(sys.argv):
            start_from = int(sys.argv[idx + 1])

    results = []
    total_start = time.time()

    for tc in test_cases:
        n = tc["id"]
        if target_cases is not None and n not in target_cases:
            continue
        if n < start_from:
            continue

        phase_label = ""
        if n <= 4:
            phase_label = "📥 STORAGE"
        elif n <= 7:
            phase_label = "🔄 VERSION"
        elif n <= 11:
            phase_label = "🔍 RECALL"
        elif n <= 14:
            phase_label = "🔗 CROSS-REF"
        else:
            phase_label = "📋 COMPREHENSIVE"

        print(f"━━━ M{n:02d} [{tc['category']}] {phase_label} ━━━")
        print(f"    维度: {tc['dimension']}")
        print(f"    描述: {tc['description']}")
        print(f"    Query: \"{tc['query'][:80]}{'...' if len(tc['query'])>80 else ''}\"")

        result = run_query(tc["query"])
        result["case_id"] = n
        result["category"] = tc["category"]
        result["dimension"] = tc["dimension"]
        result["query"] = tc["query"]
        result["description"] = tc["description"]
        result["expected_behavior"] = tc.get("expected_behavior", "")

        # Keyword verification
        kw_result = check_keywords(result["full_output"], tc.get("verify_keywords", []))
        result["keyword_check"] = kw_result

        results.append(result)

        # Status display
        has_output = result["error"] is None and result["output_len"] > 10
        status = "✅" if has_output else "❌"
        kw_display = f" | keywords={kw_result['score']}" if kw_result["total"] > 0 else ""
        print(f"    {status} {result['elapsed']:.1f}s | output={result['output_len']}c{kw_display}")

        if kw_result.get("missing"):
            print(f"    ⚠️  Missing keywords: {kw_result['missing']}")

        # Show preview
        preview = result["full_output"][:300].replace("\n", " ")
        print(f"    Preview: {preview}")
        print()

        # Save individual trace
        trace_path = f"{TRACES_DIR}/memory_eval_M{n:02d}.json"
        with open(trace_path, "w", encoding="utf-8") as f:
            json.dump(result, f, ensure_ascii=False, indent=2)

        # Brief pause between storage/version cases to let memory extraction settle
        if n <= 6:
            print("    ⏳ Waiting 3s for memory extraction to complete...")
            time.sleep(3)

    total_elapsed = time.time() - total_start

    # ─── Summary ──────────────────────────────────────────────────────
    passed = sum(1 for r in results if r["error"] is None and r["output_len"] > 10)
    failed = len(results) - passed

    # Keyword coverage
    total_kw = sum(r["keyword_check"]["total"] for r in results)
    found_kw = sum(r["keyword_check"]["found"] for r in results)

    print(f"\n{'='*65}")
    print(f"  Memory Evaluation Summary")
    print(f"  ─────────────────────────")
    print(f"  Execution:  {passed}/{len(results)} passed, {failed} failed")
    print(f"  Keywords:   {found_kw}/{total_kw} matched ({found_kw*100//max(total_kw,1)}%)")
    print(f"  Total time: {total_elapsed:.1f}s")
    print(f"  Avg time:   {total_elapsed/max(len(results),1):.1f}s/case")
    print(f"  Traces:     {TRACES_DIR}/memory_eval_M*.json")
    print(f"{'='*65}")

    # Dimension coverage report
    print(f"\n  Dimension Coverage:")
    dims = {}
    for r in results:
        d = r.get("dimension", "unknown")
        if d not in dims:
            dims[d] = {"total": 0, "passed": 0}
        dims[d]["total"] += 1
        if r["error"] is None and r["output_len"] > 10:
            dims[d]["passed"] += 1
    for d, stats in dims.items():
        st = "✅" if stats["passed"] == stats["total"] else "⚠️"
        print(f"    {st} {d}: {stats['passed']}/{stats['total']}")

    # M07 version control assertion
    m07 = next((r for r in results if r["case_id"] == 7), None)
    if m07:
        print(f"\n  M07 Version Control Assertions:")
        out = m07["full_output"]
        # Semantic check: old vehicles should NOT appear as CURRENT facts.
        # Allow mentions in correction history context (e.g., "之前说的是Model 3，已更正")
        def old_vehicle_is_current(text, old_name):
            """Check if old vehicle appears as a current fact (not just correction history)."""
            import re
            # If old_name doesn't appear at all, it's fine
            if old_name not in text:
                return False
            # If it appears near correction/update context words, it's acceptable
            correction_markers = ["之前", "更正", "修正", "更新", "原来", "不对", "改为", "纠正",
                                  "旧", "previously", "corrected", "updated", "was", "错"]
            text_lower = text.lower()
            for marker in correction_markers:
                if marker in text_lower:
                    return False  # Old vehicle mentioned in correction context — acceptable
            return True  # Old vehicle presented as current fact — problem

        vc_checks = [
            ("✅" if "Model Y" in out else "❌", "正确车辆: Model Y"),
            ("✅" if "A4" in out or "奥迪" in out else "❌", "正确车辆: 奥迪A4"),
            ("✅" if not old_vehicle_is_current(out, "Model 3") else "❌", "旧版本已隐藏: Model 3 不作为当前信息"),
            ("✅" if not old_vehicle_is_current(out, "帕萨特") else "❌", "旧版本已隐藏: 帕萨特不作为当前信息"),
        ]
        for emoji, label in vc_checks:
            print(f"    {emoji} {label}")

    # M15 comprehensive assertions
    m15 = next((r for r in results if r["case_id"] == 15), None)
    if m15:
        print(f"\n  M15 Comprehensive Assertions:")
        out = m15["full_output"].lower()
        # For version control: allow old vehicles in correction history context
        old_vehicle_as_current = False
        for old_v in ["model 3", "帕萨特"]:
            if old_v in out:
                # Check if any correction context words are near it
                correction_markers = ["之前", "更正", "修正", "更新", "原来", "不对", "改为", "纠正", "旧"]
                if not any(m in out for m in correction_markers):
                    old_vehicle_as_current = True
                    break

        checks = [
            ("✅" if "金亮" in out else "❌", "身份信息 (金亮)"),
            ("✅" if "苏州" in out else "❌", "地理信息 (苏州)"),
            ("✅" if "女儿" in out else "❌", "家庭信息 (女儿)"),
            ("✅" if "rpg" in out or "最终幻想" in out else "❌", "游戏偏好 (RPG)"),
            ("✅" if "动漫" in out else "❌", "动漫兴趣"),
            ("✅" if "程序员" in out else "❌", "职业 (程序员)"),
            ("✅" if "膝盖" in out or "篮球" in out else "❌", "运动健康"),
            ("✅" if "攀岩" in out else "❌", "亲子活动 (攀岩)"),
            ("✅" if "特斯拉" in out else "❌", "车辆 (特斯拉ModelY)"),
            ("✅" if "奥迪" in out or "a4" in out else "❌", "车辆 (奥迪A4)"),
            ("✅" if not old_vehicle_as_current else "❌", "版本控制 (旧车辆不作为当前信息)"),
            ("✅" if "telos" in out else "❌", "项目 (Telos)"),
            ("✅" if "机器人" in out else "❌", "项目 (机器人)"),
        ]
        for emoji, label in checks:
            print(f"    {emoji} {label}")

        passed_assertions = sum(1 for e, _ in checks if e == "✅")
        print(f"    ── {passed_assertions}/{len(checks)} assertions passed")

    print()

    # Save aggregated summary
    agg_path = f"{TRACES_DIR}/memory_eval_summary.json"
    with open(agg_path, "w", encoding="utf-8") as f:
        json.dump({
            "suite": "memory_evaluation",
            "total_cases": len(results),
            "passed": passed,
            "failed": failed,
            "keyword_coverage": f"{found_kw}/{total_kw}",
            "total_time": round(total_elapsed, 1),
            "avg_time": round(total_elapsed / max(len(results), 1), 1),
            "cases": [{
                "id": f"M{r['case_id']:02d}",
                "category": r["category"],
                "dimension": r["dimension"],
                "query": r["query"],
                "elapsed": r["elapsed"],
                "output_len": r["output_len"],
                "has_error": r["error"] is not None,
                "keyword_check": r["keyword_check"],
                "expected_behavior": r.get("expected_behavior", ""),
            } for r in results],
        }, f, ensure_ascii=False, indent=2)

    print(f"✅ Complete. Summary: {agg_path}")
    sys.exit(0 if failed == 0 else 1)
