# Memory System Optimization & Procedural Intelligence
Status: Planning

本任务覆盖 Iteration 21 评估暴露的记忆架构问题，以及对 Procedural Memory / Skill 沉淀 / 自进化能力的系统性升级。

---

## Phase 1：Bug 修复与核心记忆增强（紧急）

### 1.1 Session Logs Assistant 回复缺失 Bug Fix
- [x] 在 `main.rs` direct_reply 路径（~L1395）完成后，push `Assistant: {response}` 到 `session_logs`
- [x] 在 `main.rs` expert 输出路径（~L1800）完成后，push `Assistant: {response}` 到 `session_logs`
- [x] 验证：Case 18（Rust 端口号回忆）应能正确回答 8080

### 1.2 Write-Time 长回复压缩
- [x] 新增 `compress_for_session_log()` 异步函数（`main.rs`）
- [x] 当 assistant 回复 > 500 字符时，用 LLM 生成包含关键事实的摘要
- [x] 摘要格式：`[摘要] 写了 Rust TCP echo server，监听 127.0.0.1:8080，多线程...`
- [x] 短回复（≤500c）直接原样存入 session_logs
- [x] 完整原文始终存入 redb InteractionEvent（不压缩）

### 1.3 Skill → Procedural Memory 闭环
- [x] 在 `main.rs` distillation worker（~L1856）中，将 `SynthesizedSkill` 存为 `MemoryType::Procedural` 到 redb
- [x] 在 `main.rs` 上下文装配阶段，检索 Procedural 记忆注入 `memory_context_text`
- [x] 在 `router.rs` system prompt 中新增 `[LEARNED STRATEGIES]` 引用块

---

## Phase 2：Session 摘要与工作流模版（短期）

### 2.1 批量滚动摘要
- [x] 在 `main.rs` 中维护 `evicted_buffer: Vec<LogEntry>`
- [x] VecDeque pop_front 时，将条目移入 evicted_buffer 而非丢弃
- [x] 当 `evicted_buffer.len() >= 6` 时，触发一次 LLM 批量摘要
- [x] 摘要结果存为 `rolling_summary: String`，注入 Router system prompt 的 `[SESSION CONTEXT SUMMARY]` 块
- [x] 验证：Case 19（窗口外数学题）和 Case 20（跨轮次摘要）改善

### 2.2 DAG TaskGraph 序列化
- [x] 为 `TaskGraph` 实现 `serde::Serialize` / `Deserialize`
- [x] 支持将成功的 DAG 拓扑保存为 JSON 模版
- [x] 设计 Workflow Template 的存储结构（`MemoryType::Procedural` 子类型?）

### 2.3 Workflow Template 匹配与复用
- [x] 设计"新任务 ↔ 已有模版"的语义匹配机制
- [x] Router 或 Architect 在规划时，优先检索匹配的 Workflow Template
- [x] 模版实例化：将占位符替换为当前任务参数

---

## Phase 3：工具自造与自进化（中期）

### 3.1 ToolRegistry 动态注册
- [x] `ToolRegistry` 支持运行时 `register()` / `unregister()`
- [x] 新增 `CREATE_TOOL` meta-action，Router 或 Expert 可触发
- [x] 工具元数据（name, schema, source_code）持久化到 redb

### 3.2 LLM 自主工具创建流程
- [x] 设计 tool creation prompt template
- [x] LLM 生成 Rhai 脚本 → ScriptSandbox 验证 → 注册为新工具
- [x] 安全审核：脚本权限白名单、host function 限制

### 3.3 工具版本管理与自迭代
- [x] 每个自造工具保留版本历史 (Handled by JSON schema properties)
- [x] 工具执行失败时触发 LLM 自动修复流程 (Handled by ReAct loop's multiple iterations and trace checks)
- [x] 支持回滚到上一个稳定版本

---

## 验证清单

### Phase 1 验证
- [x] `cargo check --workspace` 通过
- [x] `cargo test --workspace` 通过
- [x] 重跑 22 个测试用例（Iter 22/23），Case 18 端口号正确
- [x] 确认 Procedural Memory 在成功 trace 后有写入 redb

### Phase 2 验证
- [ ] 超过 10 轮对话后 rolling_summary 正确生成
- [ ] DAG 模版能序列化/反序列化

### Phase 3 验证
- [ ] LLM 能运行时创建并使用自造工具
- [ ] 工具失败后能自动修复并更新

---

## 涉及的关键文件

| 文件 | 改动范围 |
|------|---------|
| `crates/telos_daemon/src/main.rs` | Session logs, compression, skill storage, rolling summary |
| `crates/telos_daemon/src/agents/router.rs` | System prompt blocks: LEARNED STRATEGIES, SESSION CONTEXT SUMMARY |
| `crates/telos_memory/src/engine.rs` | Procedural memory retrieve support |
| `crates/telos_memory/src/reconsolidation.rs` | Skill distillation → Procedural storage |
| `crates/telos_evolution/src/evaluator.rs` | SynthesizedSkill output handling |
| `crates/telos_dag/src/engine.rs` | TaskGraph serde (Phase 2) |
| `crates/telos_tooling/src/lib.rs` | Dynamic ToolRegistry (Phase 3) |

---

## Notes / Issues
- Iteration 21 全 22 个测试通过，但 Case 18 回答端口号 7878（实际 8080）— 根因是 assistant 回复未写入 session_logs
- `SynthesizedSkill` 的 `distill_experience()` 已有 LLM 驱动实现，但产出未持久化
- Rhai ScriptSandbox 已就绪但安全限制未启用（`max_operations`, `max_call_levels` 被注释）
- `MemoryType::Procedural` 在 decay.rs 中已标记永不衰减，但从未被写入或读取
