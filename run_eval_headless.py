#!/usr/bin/env python3
"""
Telos Agent Evaluation Suite — Iteration 32 (Memory System Refinement)
Tests all agent categories via /api/v1/run_sync SSE endpoint.

UPDATED TEST CASES — personalized scenarios based on actual user profile.
Test cases now cover personal info grounding (Rust developer in Suzhou,
game/anime/history enthusiast, Tesla → BYD vehicle update), memory CRUD
with conflict resolution, and all agent categories.

Categories: Identity, Math, Common Knowledge, Real-time Search,
            Deep Research, Time Awareness, Coding, Knowledge Reasoning,
            Ambiguous/Edge, Multi-step Planning, Memory, Persona,
            Tool Creation, Procedural Memory, Scheduled Missions
"""
import requests, json, time, os, uuid, sys, re

API = "http://127.0.0.1:8321/api/v1/run_sync"
BASE_URL = "http://127.0.0.1:8321"
ITER = 32
TRACES_DIR = "test_traces"
os.makedirs(TRACES_DIR, exist_ok=True)

# ─── Test Cases ───────────────────────────────────────────────────────
test_cases = [
    # ══════════════════════════════════════════════════════════════════
    # ── Category: Identity & Self-Awareness ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 1,
        "category": "Identity",
        "query": "如果有人说你只是一个'API套壳'的产品，你会怎么反驳？请从你的架构独特性出发，说明你和市面上其他AI助手的根本差异。",
        "description": "自我架构认知 — 测试 Telos 对自身 DAG/Memory/Tool 架构的理解",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Math & Logic ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 2,
        "category": "Math",
        "query": "小明有一个8升桶和一个5升桶，水龙头无限量供水。他需要精确量出6升水。请给出最少步骤的方案，并说明每步后两个桶里分别有多少水。",
        "description": "经典量水问题 — 需要搜索/回溯推理",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Knowledge (生物学) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 3,
        "category": "Knowledge",
        "query": "章鱼有三颗心脏，它们分别负责什么功能？如果切断其中一颗会怎样？另外，章鱼的血液为什么是蓝色的？",
        "description": "生物学冷知识 — 章鱼循环系统",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Real-time Search (体育) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 4,
        "category": "Search",
        "query": "2026年F1赛季目前车手积分榜前三名是谁？最近一站比赛是在哪里举行的，冠军是谁？",
        "description": "实时体育赛事 — 2026 F1赛季最新数据",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Deep Research (贴近用户 Rust 兴趣) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 5,
        "category": "DeepResearch",
        "query": "帮我深度调研一下 2026 年 Rust 异步运行时的最新生态格局。重点对比 tokio、async-std、smol、glommio 这四个运行时在以下维度的差异：性能基准测试数据、io_uring 支持情况、生态库兼容性、以及各自适用的场景。最后给出一份选型建议矩阵。",
        "description": "Rust异步运行时深度调研 — 贴合用户技术栈",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Time Awareness (贴近苏州) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 6,
        "category": "TimeAware",
        "query": "距离2026年国庆节还有多少天？如果我从今天开始每天存100元，到国庆能存多少钱？",
        "description": "时间推理+简单计算 — 需要知道今天日期",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Coding (Rust — 贴合用户主力语言) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 7,
        "category": "Coding",
        "query": "用 Rust 实现一个无锁的 MPSC 环形缓冲区（ring buffer），要求：1) 使用 AtomicUsize 做 head/tail 索引 2) 支持泛型 T: Send 3) 提供 try_push 和 try_pop 方法，满时/空时返回 None 4) 附带完整的 #[cfg(test)] 模块，至少包含并发读写正确性测试",
        "description": "Rust无锁数据结构 — 贴合用户专业领域",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Reasoning (科学哲学) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 8,
        "category": "Reasoning",
        "query": "费米悖论问的是：宇宙这么大这么老，为什么我们没有找到外星文明？请列出至少三种主流的解释假说，并说说你认为哪个最有说服力，为什么。",
        "description": "科学哲学推理 — 费米悖论开放性分析",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Edge Case ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 9,
        "category": "EdgeCase",
        "query": "请把这段话反过来念：'苏州园林甲天下'，然后用每个字造一个四字成语。",
        "description": "字符串反转+创意造句 — 中文字符级操作+输出足够长",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Planning (贴近个人兴趣 — 游戏/历史) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 10,
        "category": "Planning",
        "query": "我打算利用周末两天时间在苏州做一次'三国文化'主题的自驾游。请帮我规划路线，包括常熟的虞山（与虞仲传说相关）、苏州的盘门（伍子胥故事）等三国前后的历史遗迹，还要穿插推荐当地特色美食（但注意我对芒果严重过敏），住宿最好在平江路附近的精品民宿。预算控制在2000元以内。",
        "description": "个性化旅行计划 — 结合苏州本地+历史爱好+过敏信息",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Coding (Python) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 11,
        "category": "Coding",
        "query": "用Python写一个命令行工具，能够读取CSV文件并自动生成数据分析摘要。要求：1) 自动检测每列的数据类型（数值、日期、文本） 2) 对数值列输出均值/中位数/标准差 3) 对文本列输出唯一值数量和最常见的前3个值 4) 结果打印为格式化的表格。请使用标准库+csv模块，不要依赖pandas。",
        "description": "Python CLI工具 — 纯标准库数据分析",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Reasoning (历史 — 贴合用户兴趣) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 12,
        "category": "Reasoning",
        "query": "三国时期诸葛亮的'隆中对'战略提出了'跨有荆益，联吴抗曹'的大计。请从现代地缘政治和博弈论的角度分析：这个战略在执行中有哪些结构性矛盾？为什么最终'联吴'走向破裂？如果你是诸葛亮的战略顾问，你会如何修正这个方案？",
        "description": "历史+博弈论推理 — 贴合用户的三国/历史爱好",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Memory — Store New Facts (个人真实信息) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 13,
        "category": "Memory",
        "query": "帮我记住以下信息：我叫金亮，是一个 Rust 开发者，现在住在苏州。我的车是一辆白色特斯拉 Model Y，车牌号苏E·88888。我对芒果严重过敏，另外我是三国历史和《原神》的忠实粉丝。",
        "description": "记忆存储 — 真实个人信息（姓名+职业+城市+车辆+过敏+兴趣）",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Memory — Contextual Recall (过敏应用) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 14,
        "category": "Memory",
        "query": "同事从泰国出差回来送了我一箱热带水果，里面有芒果、山竹、榴莲和火龙果。哪些我可以放心吃？",
        "description": "记忆应用 — 需要回忆芒果过敏并自动警告",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Memory — Update/Conflict Resolution ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 15,
        "category": "MemoryConflict",
        "query": "更正一下，我最近换车了。旧的白色特斯拉 Model Y 已经卖掉了，现在开的是一辆黑色比亚迪汉EV，新车牌是苏E·66666。请帮我更新记忆。",
        "description": "记忆更新 — 测试旧信息覆盖/冲突处理（Tesla→BYD）",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Persona ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 16,
        "category": "Persona",
        "query": "用一段话描述你的性格特点和处事风格。你觉得自己更像哪个知名人物或虚构角色？为什么？",
        "description": "人格自我描述 — 测试 SOUL persona 一致性",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: History Recall — Vehicle Update Confirmation ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 17,
        "category": "HistoryRecall",
        "query": "对了，我更新完的车辆信息是什么来着？牌号和颜色帮我确认一下。",
        "description": "近期历史回忆 — 确认车辆更新（应回答比亚迪汉EV/苏E·66666）",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Cross-turn Context — Code Reference ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 18,
        "category": "HistoryRecall",
        "query": "回头看一下我们前面写的那个Rust环形缓冲区代码，它用了哪些Rust高级特性？如果要让它支持动态扩容应该怎么改？",
        "description": "上下文指代 + 技术延伸（引用Case 7的Rust代码）",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Deep Memory Recall — Problem Mutation ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 19,
        "category": "DeepMemoryRecall",
        "query": "我们之前讨论的量水问题，如果把条件改成需要量出7升水（桶还是8升和5升），最少需要几步？",
        "description": "深度记忆回忆 — 回到早期问题并修改条件",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Multi-fact Summary ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 20,
        "category": "HistoryRecall",
        "query": "总结一下到目前为止你了解的关于我的所有个人信息（姓名、职业、城市、车辆、过敏、兴趣爱好等）。",
        "description": "跨轮次信息汇总 — 需要整合多轮记忆（应包含金亮/Rust/苏州/比亚迪/芒果过敏/三国/原神）",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Implicit Preference Application (过敏+兴趣) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 21,
        "category": "PreferenceApplication",
        "query": "下周末想请朋友们来家里聚餐，帮我设计一个6人份的菜单。甜品也要有，最好能和我喜欢的游戏《原神》来个主题联动。",
        "description": "隐式偏好应用 — 测试是否自动避开芒果+融合原神兴趣",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: False Memory Guard ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 22,
        "category": "FalseMemoryGuard",
        "query": "我记得我们之前好像一起调试过一个Go语言的gRPC服务端的问题，你还记得当时报的是什么错吗？",
        "description": "虚假记忆防护 — 从未讨论过Go/gRPC",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Tool Creation ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 23,
        "category": "ToolCreation",
        "query": "帮我创建一个名为 `convert_units` 的工具，用于单位换算。支持：长度（米↔英尺）、重量（千克↔磅）、温度（摄氏↔华氏）。创建成功后，请用这个工具帮我把 180cm 换算成英尺，以及 72°F 换算成摄氏度。",
        "description": "动态工具自造 — 纯逻辑计算型（不依赖外部API）",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Procedural Memory — Store ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 24,
        "category": "ProceduralSetup",
        "query": "帮我审查这段Rust代码的安全隐患：`let query = format!(\"SELECT * FROM users WHERE name = '{}'\", user_input); conn.execute(&query, [])?;` 请详细分析漏洞类型并给出修复方案（使用参数化查询）。之后请把你的SQL注入审查流程提炼成一个名为 'SQL_Injection_Audit' 的经验模板存入程序记忆。",
        "description": "SQL注入审查（Rust版本）— 贴合用户主力语言 + 流程蒸馏",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Procedural Memory — Apply ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 25,
        "category": "ProceduralApply",
        "query": "又发现一段可疑的Rust代码：`conn.execute(&format!(\"DELETE FROM orders WHERE id = {}\", order_id), [])?;` 请严格按照前一步总结的 'SQL_Injection_Audit' 流程来审查并修复它。",
        "description": "流程经验重用 — 测试 SQL_Injection_Audit 模板检索和应用",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Relevance Filter ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 26,
        "category": "RelevanceFilter",
        "query": "帮我看看这段CSS动画代码的性能问题：`@keyframes slide { from { left: 0; } to { left: 100%; } }` 用了 `left` 属性做动画。",
        "description": "相关性过滤 — 前端CSS不应触发 SQL_Injection_Audit 模板",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Progressive Discovery ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 27,
        "category": "ProgressiveDiscovery",
        "query": "帮我查一下现在苏州的空气质量指数(AQI)怎么样？PM2.5是多少？",
        "description": "渐进式暴露 — 需要发现并使用工具获取实时数据",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Tool Mutation ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 28,
        "category": "ToolMutation",
        "query": "帮我创建一个工具 `get_exchange_rate_v2` 来获取汇率信息，使用 open.er-api.com/v6/latest/USD 这个免费API。但请故意在代码里写一个小错误（比如把URL路径拼错）。执行失败后，请利用 mutate_tool 修复它，然后告诉我1美元等于多少人民币。",
        "description": "工具基因突变 — 故意出错→修复→验证闭环",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Scheduled Mission ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 29,
        "category": "ScheduledMission",
        "query": "帮我设置一个定时任务：每周一早上9点检查苏州的天气预报并推送给我，如果有雨就提醒我带伞。请用 schedule_mission 工具创建，cron表达式要正确。",
        "description": "定时任务创建 — 结合用户所在城市苏州",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Mission Management ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 30,
        "category": "ScheduledMission",
        "query": "列出所有正在运行的定时任务，然后把刚才创建的天气提醒任务取消掉。取消后再确认一下是否成功删除了。",
        "description": "定时任务管理 — list + cancel + verify",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Multi-language ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 31,
        "category": "MultiLang",
        "query": "请用英文写一首关于'人工智能'的五行俳句(Haiku: 5-7-5音节结构)，然后翻译成中文，并解释你是如何控制音节数的。",
        "description": "多语言创作 — 英文俳句创作+翻译+过程解释",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── NEW: Memory — Question Guard (P1 fix verification) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 32,
        "category": "MemoryQuestionGuard",
        "query": "你还记得我喜欢什么颜色吗？我之前有没有跟你说过？",
        "description": "提问防护 — 测试P1修复：不应从疑问句中提取虚假事实",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── NEW: Memory — Hobby Contextual Application ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 33,
        "category": "PreferenceApplication",
        "query": "推荐5款和《原神》风格类似的开放世界游戏，考虑到我是用 Rust 做后端开发的，如果其中有使用 Rust 编写的游戏引擎也请特别标注。",
        "description": "兴趣关联推荐 — 测试是否能结合用户的原神+Rust双重兴趣",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── NEW: Knowledge — Suzhou-specific local knowledge ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 34,
        "category": "Knowledge",
        "query": "苏州的拙政园、留园、网师园和沧浪亭并称'苏州四大名园'，请分别用一句话概括它们各自最突出的造园特色。另外哪个最适合雨天去逛？",
        "description": "苏州本地知识 — 园林文化常识题",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── NEW: Real-time Search — Tech News (Rust-specific) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 35,
        "category": "Search",
        "query": "Rust 语言最新的 stable 版本号是多少？这个版本有什么重要的新特性？另外 Rust 2024 edition 的关键变化有哪些？",
        "description": "实时技术搜索 — Rust最新版本+edition信息",
    },
]

# ─── SSE Request Helper ───────────────────────────────────────────────
def run_query(query: str, timeout: int = 300) -> dict:
    """Send query to /api/v1/run_sync, parse SSE events, return result dict."""
    start = time.time()
    final_output, heartbeats, summary = "", [], {}
    error = None
    last_activity = [time.time()]  # mutable ref for watchdog thread
    stall_timeout = 300  # idle timeout in seconds (matches server-side)

    try:
        trace_id = str(uuid.uuid4())
        r = requests.post(
            API,
            json={"payload": query, "trace_id": trace_id},
            headers={"Accept": "text/event-stream"},
            stream=True,
            timeout=timeout,
            proxies={"http": None, "https": None},
        )

        # Idle watchdog: closes connection if no activity for stall_timeout seconds
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
                last_activity[0] = time.time()  # reset idle timer on valid SSE content
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
                elif event_type == "error":
                    error = data
                event_type, data_lines = "", []
        
        watchdog_stop.set()  # stop watchdog after stream ends

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

    target_cases = None
    if "--cases" in sys.argv:
        idx = sys.argv.index("--cases")
        if idx + 1 < len(sys.argv):
            target_cases = [int(x) for x in sys.argv[idx + 1].split(",")]

    for tc in test_cases:
        n = tc["id"]
        if target_cases is not None and n not in target_cases:
            continue
        print(f"━━━ Case {n:02d} [{tc['category']}] {tc['description']} ━━━")
        print(f"    Query: \"{tc['query'][:80]}{'...' if len(tc['query'])>80 else ''}\"")

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
