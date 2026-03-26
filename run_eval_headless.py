#!/usr/bin/env python3
"""
Telos Agent Evaluation Suite — Iteration 33 (Post Memory System Upgrade)
Tests all agent categories via /api/v1/run_sync SSE endpoint.

FULLY REFRESHED TEST CASES for post-memory-upgrade validation.
Personal info grounded in REAL user facts from memory eval suite:
  - 金亮 / 苏州 / 程序员 / 1989年生 / 已婚有8岁女儿
  - 特斯拉Model Y + 奥迪A4 / 膝盖受伤 / 每周跑步20km
  - 游戏(RPG/SRPG/建造/宝可梦) / 动漫(奇幻/热血/猎奇) / 历史
  - 年度目标: Telos智能助理 + 个人机器人(3D打印)

Categories: Identity, Math, Common Knowledge, Real-time Search,
            Deep Research, Time Awareness, Coding, Knowledge Reasoning,
            Ambiguous/Edge, Multi-step Planning, Memory, Persona,
            Tool Creation, Procedural Memory, Scheduled Missions
"""
import requests, json, time, os, uuid, sys, re

API = "http://127.0.0.1:8321/api/v1/run_sync"
BASE_URL = "http://127.0.0.1:8321"
ITER = 35
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
        "query": "请介绍一下你自己。你叫什么名字？你的记忆系统是怎么工作的？如果我告诉你一些我的个人信息，下次再来找你的时候你还会记得吗？请解释这背后的技术原理。",
        "description": "自我认知+记忆机制 — 测试 Telos 对自身身份和记忆系统的认知",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Math & Logic ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 2,
        "category": "Math",
        "query": "一个农夫需要把狐狸、鸡和一袋玉米运过河，但船每次只能载他和其中一样东西。如果他不在场，狐狸会吃鸡，鸡会吃玉米。请给出完整的过河步骤方案，并证明不存在更少步骤的解法。",
        "description": "经典过河问题 — 需要约束推理+最优性论证",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Knowledge (天文物理) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 3,
        "category": "Knowledge",
        "query": "黑洞的'事件视界'和'光子球'是什么关系？有人说掉进黑洞的人会经历'意面化效应'——这是什么原理？理论上超大质量黑洞和恒星级黑洞哪个更容易让人安然穿过视界？",
        "description": "天体物理知识 — 黑洞结构与极端物理效应",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Real-time Search (科技新闻) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 4,
        "category": "Search",
        "query": "2026年最近一个月内，AI 领域有哪些重磅新闻或论文发布？特别关注大模型推理能力和 Agent 框架方面的进展。至少列出3条。",
        "description": "实时AI新闻 — 2026年最新AI领域动态",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Deep Research (嵌入式数据库对比) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 5,
        "category": "DeepResearch",
        "query": "帮我深度调研 2026 年 Rust 生态中可嵌入式数据库的最新格局。重点对比 redb、sled、rocksdb(rust-rocksdb)、surrealdb embedded、和 LanceDB 这五个在以下维度的差异：ACID 事务支持、向量搜索能力、并发模型（多线程读写）、嵌入二进制大小、以及各自最适合的应用场景。最后给出一份面向'纯Rust无外部进程的AI Agent记忆系统'的选型推荐。",
        "description": "嵌入式DB深度调研 — 贴合Telos存储架构设计需要",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Time Awareness ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 6,
        "category": "TimeAware",
        "query": "今天是几号？星期几？距离2026年中秋节（农历八月十五）还有多少天？帮我算一下如果每天跑步5公里，从今天到中秋总共能跑多少公里。",
        "description": "时间感知+日历推理+计算 — 需要知道当前日期和农历转换",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Coding (Rust) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 7,
        "category": "Coding",
        "query": "请帮我用 Rust 写一个极简的 Actor 模型框架。要求：不使用外部框架，纯靠 tokio::sync::mpsc 进行异步消息通信。需要包含一个基础的 Actor trait，以及一个简单的例子（比如扮演计算器的 Actor，能接收 Add 和 Get 消息）。",
        "description": "Rust Actor模型代码生成 — 贴合Telos架构设计（纯async/channel通信）",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Reasoning (认知科学) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 8,
        "category": "Reasoning",
        "query": "人类大脑的记忆系统分为工作记忆、情景记忆和语义记忆。当代AI的记忆系统（如RAG、向量数据库、记忆版本控制）和人脑的这三类记忆有什么对应关系？各有哪些根本性的能力差距？你认为未来最有希望弥合这些差距的技术路线是什么？",
        "description": "认知科学+AI交叉推理 — 人脑vs机器记忆的深度比较",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Edge Case ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 9,
        "category": "EdgeCase",
        "query": "请完成这个任务：把数字 1 到 10 用中文大写（壹贰叁肆伍陆柒捌玖拾）写出来，然后把每个中文大写数字拆解成偏旁部首。最后统计哪个偏旁出现次数最多。",
        "description": "中文字形拆解+统计 — 测试跨模态字符级操作能力",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Planning (科技项目 — 机器人) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 10,
        "category": "Planning",
        "query": "我计划在今年年中启动一个个人机器人项目，已经买了舵机和电子零件，还打算买一台3D打印机。目标是做一个小型人形桌面机器人，高度30cm左右，能做简单的手势动作和语音交互。预算5000元以内（不含3D打印机）。请帮我：1) 列出完整的硬件BOM清单和预估成本 2) 推荐适合的3D打印机型号（2000元以内） 3) 规划一个3个月的开发里程碑 4) 推荐软件栈（最好有Rust支持）。",
        "description": "个人机器人项目规划 — 贴合用户年度目标（真实计划）",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Coding (Python — 数据可视化) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 11,
        "category": "Coding",
        "query": "用 Python 写一个跑步数据分析脚本。输入是一个包含每日跑步记录的JSON文件（格式：[{\"date\": \"2026-03-01\", \"distance_km\": 5.2, \"duration_min\": 28, \"heart_rate_avg\": 155}]）。要求：1) 计算周均/月均跑量 2) 找出配速最快的前3次 3) 检测是否有连续3天以上未跑步的'断训'期 4) 生成一段文字摘要报告。只用标准库+json模块，不要第三方依赖。",
        "description": "Python跑步分析 — 贴合用户跑步爱好（每周20km）",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Reasoning (军事战略+历史) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 12,
        "category": "Reasoning",
        "query": "赤壁之战是三国时期最关键的转折点之一。请从以下角度综合分析这场战役：1) 曹操为什么在兵力优势下仍然大败？列出至少3个关键失误 2) 如果曹操听取了贾诩'暂缓南下'的建议，历史走向会怎样？ 3) 火攻在古代战争中的应用还有哪些经典案例？请做类比分析。",
        "description": "赤壁之战多角度推理 — 贴合用户历史兴趣（反事实+类比推理）",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Memory — Store Personal Facts (真实信息) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 13,
        "category": "Memory",
        "query": "帮我记住这些信息：我叫金亮，1989年2月生，住在苏州，已婚，有一个8岁的女儿。我是程序员，平时用大模型辅助编程。两辆车：特斯拉Model Y和奥迪A4。我膝盖受过伤，但仍然坚持每周跑步20公里，周末带女儿攀岩。",
        "description": "记忆存储 — 真实个人信息（从memory eval验证过的事实）",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Memory — Contextual Health Recall ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 14,
        "category": "Memory",
        "query": "朋友约我这周末去打羽毛球然后跑个10公里，晚上再一起踢场野球。我全都参加没问题吧？帮我分析一下。",
        "description": "记忆应用 — 需要回忆膝盖伤情并给出健康建议（应警告足球对膝盖风险）",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Memory — Hobby Storage + Update ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 15,
        "category": "MemoryConflict",
        "query": "补充一些信息：我最近迷上了《火焰纹章 风花雪月》，已经玩了200多小时了。另外我之前说的两辆车要更新一下，奥迪A4已经给我爸了，现在车库里只剩特斯拉Model Y了。帮我更新记忆。",
        "description": "记忆更新 — 新增游戏偏好 + 删除车辆（A4→给爸爸，只留Model Y）",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Persona ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 16,
        "category": "Persona",
        "query": "如果你必须用三个词来形容自己的核心价值观，你会选哪三个词？另外，面对用户提出的一个你完全不确定答案的问题，你的处理策略是什么？会坦诚说不知道，还是会猜测一个答案？",
        "description": "人格价值观+不确定性处理 — 测试 SOUL persona 深度一致性",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: History Recall — Vehicle Update Confirmation ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 17,
        "category": "HistoryRecall",
        "query": "等一下，帮我确认一下现在我名下到底还有几辆车？分别什么品牌和型号？之前有过变动对吧？",
        "description": "车辆记忆验证 — 测试版本控制后的最新状态回忆",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Cross-turn Context — Code Review ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 18,
        "category": "HistoryRecall",
        "query": "前面你帮我写的那个 Rust Actor 模型框架，你觉得有什么设计缺陷吗？如果要加入一个 supervisor 机制来自动重启崩溃的 Actor，应该怎么设计？",
        "description": "上下文指代 + 架构扩展（引用Case 7的Actor代码）",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Deep Memory Recall — Problem Mutation ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 19,
        "category": "DeepMemoryRecall",
        "query": "我们之前讨论过的那个过河问题（狐狸、鸡、玉米），如果再加一条蛇（蛇会吃狐狸，但不吃其他东西），变成四样东西过河，最少需要几步？",
        "description": "深度记忆回忆 — 回到早期问题并增加约束",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Multi-fact Summary ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 20,
        "category": "HistoryRecall",
        "query": "做一个完整的自我画像：把你目前知道的关于我的所有信息，按「个人基本」「家庭」「职业技术」「运动健康」「兴趣爱好」「车辆财产」几个维度分类整理出来。",
        "description": "多维度信息汇总 — 需要整合并结构化所有记忆中的用户事实",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Implicit Preference Application (运动+女儿) ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 21,
        "category": "PreferenceApplication",
        "query": "女儿学校下周有亲子运动会，需要我和她组队参加三个项目。帮我选择最适合我们俩的项目组合，考虑到她的年龄和我的身体状况。另外运动会当天午餐带什么合适？",
        "description": "隐式偏好应用 — 需综合女儿8岁+膝盖伤+运动偏好自动选择",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: False Memory Guard ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 22,
        "category": "FalseMemoryGuard",
        "query": "上次你帮我调那个 Kubernetes 集群的内存泄漏问题最后怎么解决的来着？是哪个 Pod 一直 OOM？",
        "description": "虚假记忆防护 — 从未讨论过Kubernetes/OOM",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Tool Creation ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 23,
        "category": "ToolCreation",
        "query": "帮我创建一个名为 `bmi_calculator` 的工具，输入身高（cm）和体重（kg），输出BMI值和健康评级（偏瘦/正常/偏胖/肥胖）。创建好后，请用这个工具计算身高174cm、体重65kg的BMI值。",
        "description": "动态工具自造 — 纯计算型（使用真实用户身体数据验证）",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Procedural Memory — Store ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 24,
        "category": "ProceduralSetup",
        "query": "帮我审查这段Rust代码的内存安全问题：\n```rust\nlet mut data = vec![1, 2, 3, 4, 5];\nlet first = &data[0];\ndata.push(6);\nprintln!(\"first: {}\", first);\n```\n请详细分析为什么这段代码有问题，Rust编译器会如何阻止它，以及正确的写法是什么。之后请把你的'Rust借用检查器常见陷阱审查流程'提炼成一个名为 'Rust_Borrow_Checker_Audit' 的经验模板存入程序记忆。",
        "description": "Rust借用检查审查 — 贴合用户主力语言 + 程序记忆蒸馏",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Procedural Memory — Apply ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 25,
        "category": "ProceduralApply",
        "query": "又有一段可疑的Rust代码：\n```rust\nlet mut map = HashMap::new();\nmap.insert(\"key\".to_string(), vec![1, 2, 3]);\nlet values = map.get(\"key\").unwrap();\nmap.insert(\"key2\".to_string(), vec![4, 5, 6]);\nprintln!(\"{:?}\", values);\n```\n请严格按照前一步总结的 'Rust_Borrow_Checker_Audit' 流程来审查并修复它。",
        "description": "程序记忆重用 — 测试 Rust_Borrow_Checker_Audit 模板检索和应用",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Relevance Filter ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 26,
        "category": "RelevanceFilter",
        "query": "帮我优化这段 SQL 查询的性能：`SELECT * FROM orders WHERE customer_id IN (SELECT id FROM customers WHERE city = '苏州') ORDER BY created_at DESC LIMIT 100;` 需要加什么索引？有没有更高效的写法？",
        "description": "相关性过滤 — SQL优化不应触发借用检查器审查模板",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Progressive Discovery ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 27,
        "category": "ProgressiveDiscovery",
        "query": "帮我查一下现在苏州到上海虹桥的高铁最早一班是几点的？大约多长时间？票价多少？",
        "description": "渐进式暴露 — 需要发现并使用工具获取实时交通数据",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Tool Mutation ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 28,
        "category": "ToolMutation",
        "query": "帮我创建一个工具 `random_quote_v2` 来获取随机名言，使用 api.quotable.io/random 这个API。但请故意把API的域名拼错（比如拼成 api.quotabl.io）。执行失败后，请利用 mutate_tool 修复域名拼写错误，然后成功获取一条名言给我看看。",
        "description": "工具基因突变 — 故意DNS错误→修复→验证闭环",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Scheduled Mission ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 29,
        "category": "ScheduledMission",
        "query": "帮我设置一个定时任务：每天早上7:30提醒我今天的跑步目标（工作日5公里，周末8公里），并在提醒中附上当前苏州的天气状况。请用 schedule_mission 工具创建，cron表达式要正确。",
        "description": "定时任务创建 — 结合跑步习惯+苏州地理",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Mission Management ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 30,
        "category": "ScheduledMission",
        "query": "列出我当前所有的定时任务，然后把刚才创建的跑步提醒任务暂停掉（不是删除）。操作完后确认一下任务状态。",
        "description": "定时任务管理 — list + pause/cancel + verify",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Multi-language ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 31,
        "category": "MultiLang",
        "query": "请帮我把'程序员的浪漫，是用代码表达诗意'这句话翻译成日文、韩文和英文。然后用英文版写一首4行的小诗，每行押韵。最后解释翻译过程中遇到的文化适配难点。",
        "description": "多语言翻译+创作 — 中→日/韩/英+诗歌创作+文化解析",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Memory — Question Guard ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 32,
        "category": "MemoryQuestionGuard",
        "query": "我之前有没有告诉过你我的血型是什么？还有我是不是养了一只猫？你回忆一下。",
        "description": "提问防护 — 不应从疑问句中提取'血型'或'养猫'作为事实存储",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Memory — Gaming Preference Application ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 33,
        "category": "PreferenceApplication",
        "query": "Switch 2 据说要出了，帮我列一个'Day 1必买'的游戏清单。只推荐符合我口味的类型，不要凑数推荐我不会喜欢的游戏类型。",
        "description": "游戏偏好精准匹配 — 测试是否基于记忆中RPG/SRPG/建造/宝可梦偏好过滤推荐",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Knowledge — 历史地理 ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 34,
        "category": "Knowledge",
        "query": "苏州古称'姑苏'，这个名字的由来是什么？苏州在中国历史上最繁荣的朝代是哪个？当时苏州的经济地位在全国排第几？另外'上有天堂下有苏杭'这句话最早出自哪里？",
        "description": "苏州历史文化知识 — 贴合用户苏州+历史双重兴趣",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Real-time Search — 消费电子 ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 35,
        "category": "Search",
        "query": "2026年最值得关注的消费电子新品有哪些？特别是可穿戴设备和AR/VR领域的。有没有什么特别有创意的新产品让你印象深刻的？",
        "description": "实时消费电子搜索 — 贴合用户的消费电子爱好",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: UTF-8 Stress Test 1 ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 36,
        "category": "UTF8_Stress",
        "query": "请帮我记住下面这一大段没有任何意义的超长中文古文，主要是为了测试记忆系统的截断情况：天地玄黄宇宙洪荒日月盈昃辰宿列张寒来暑往秋收冬藏闰余成岁律吕调阳云腾致雨露结为霜金生丽水玉出昆冈剑号巨阙珠称夜光果珍李柰菜重芥姜海咸河淡鳞潜羽翔龙师火帝鸟官人皇始制文字乃服衣裳推位让国有虞陶唐吊民伐罪周发殷汤坐朝问道垂拱平章爱育黎首臣伏戎羌遐迩一体率宾归王鸣凤在竹白驹食场化被草木赖及万方。之后再请用一句话告诉我刚才我让你记住了什么？",
        "description": "UTF-8安全截断测试1 — 纯长中文，触发记忆存储时的80/200字符截断边界",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: UTF-8 Stress Test 2 ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 37,
        "category": "UTF8_Stress",
        "query": "你记得刚刚那段关于天地玄黄的超长文本吗？请把它提取为你的系统内部处理经验，起个名字叫 'CJK_String_Bounds_Test_Template'，在这个经验里，详细说明在 Rust 中切片（Slicing）多字节字符（如中日韩文、Emoji）时可能发生的 Panic 原因。",
        "description": "经验提取测试2 — 强制生成超长模板描述触发 spawner.rs 中的 60/80 截断",
    },

    # ══════════════════════════════════════════════════════════════════
    # ── Category: Tool Mutation & Debugging ──
    # ══════════════════════════════════════════════════════════════════
    {
        "id": 38,
        "category": "ToolMutation",
        "query": "我刚才让你创建的 `bmi_calculator` 工具，它的健康评级标准还是有问题的：对于亚洲人来说，BMI >= 24 就应该算是偏胖了，原来的脚本可能是用的世卫组织的 25。请帮我使用工具编辑器直接读取并修改这个现有的代码，加上亚洲人的标准，然后用你修改后的工具重新测一下身高174cm、体重65kg。",
        "description": "动态工具调试 — 验证读取现有工具、Sandbox 测试和修改覆盖的能力",
    },
]

# ─── SSE Request Helper ───────────────────────────────────────────────
def run_query(query: str) -> dict:
    """Send query to /api/v1/run_sync, parse SSE events, return result dict."""
    start = time.time()
    final_output, heartbeats, summary = "", [], {}
    error = None
    last_activity = [time.time()]  # mutable ref for watchdog thread
    stall_timeout = 600  # 10 minutes idle timeout (matches server-side)

    try:
        trace_id = str(uuid.uuid4())
        r = requests.post(
            API,
            json={"payload": query, "trace_id": trace_id},
            headers={"Accept": "text/event-stream"},
            stream=True,
            timeout=None, # Removed hard timeout, rely on watchdog stall_timeout
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
