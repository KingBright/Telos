# Iteration 18: Workflow Template 智能进化

Status: Phase 1 ✅ | Phase 2 ✅ | Phase 3 Pending

本迭代系统性解决 Workflow Template 的**可靠复用**与**自适应进化**问题。核心目标：让模板在复用时"认得准"、成功时"变得强"、失败时"学得到"。

---

## Phase 1：三道门 — 相关性过滤（紧急） ✅

当前问题：检索到的 Workflow 模板可能与任务风马牛不相及（"程序员架构师设计房子"），但 Architect 被 `MUST reuse` 强制套用。

### 1.1 向量检索增加 Similarity Score 返回
- [x] `MemoryEntry` 新增 `#[serde(default)] pub similarity_score: Option<f32>` 字段
- [x] `engine.rs` 中 `VectorSearch` 处理逻辑修改：将 cosine similarity 写入返回的 MemoryEntry
- [x] 更新所有构造 MemoryEntry 的代码（reconsolidation.rs, conflict.rs, tests.rs）

### 1.2 检索层硬门槛过滤
- [x] `integration.rs` 的 `retrieve_procedural_memories()` 增加相似度阈值（`0.65`）
- [x] 仅返回 `similarity_score >= threshold` 的条目
- [x] 如果所有候选都低于阈值，返回空 Vec（不注入无关模板）

### 1.3 Architect Prompt 从"强制"改为"建议"
- [x] `architect.rs` 的模板注入 prompt 从 `MUST reuse` 改为建议性措辞
- [x] 新措辞要求 Architect 先判断模板与当前任务的相关性，不相关则忽略
- [x] Architect 输出 JSON 新增 `"adopted_templates": [desc1, desc2]` 字段
- [x] 模板中的 `[FailureNote]` 行以警告格式注入 Architect prompt

### 1.4 Event Loop 采用标记修正
- [x] `event_loop.rs` 中的复用检测逻辑从读取 `reused_workflow_count` 改为读取 `adopted_templates`
- [x] 只有 `adopted_templates` 非空时才填充 `ExecutionTrace.reused_workflow_ids`

---

## Phase 2：失败反思与物种分化（核心） ✅

当前问题：模板复用失败后什么都不做（仅发 metric 事件），同样的失败会反复发生。

### 2.1 Failure Note 附加机制
- [x] `MemoryIntegration` trait 新增 `attach_failure_note()` → `Result<u32, String>`（返回新的失败计数）
- [x] 实现：对匹配的模板内容追加 `[FailureNote]` 行 + 递增 `[FailureCount]`
- [x] 最多保留最近 3 条 failure notes（避免膨胀）

### 2.2 Strength 温和惩罚
- [x] `MemoryIntegration` trait 新增 `penalize_workflow_template()`
- [x] 实现：匹配模板后 `base_strength -= 0.3`（下限 1.0）
- [x] 当 strength 已经是 1.0 时跳过（避免无效写入）

### 2.3 Spawner 失败路径集成
- [x] `spawner.rs` 失败路径：从 `trace.errors_encountered` 或失败步骤构建 failure_note
- [x] 对每个 `reused_workflow_id` 调用 `attach_failure_note()` 和 `penalize_workflow_template()`

### 2.4 物种分化（累积失败触发）
- [x] 当 `attach_failure_note()` 返回 failure_count ≥ 2 时触发物种分化
- [x] 分化流程：LLM 分析原模板 + failure notes → 生成适应性变体 JSON
- [x] 变体作为新模板独立存储（`[Variant]` 前缀，独立 ID），原模板保留
- [x] 使用 `strong_reasoning: true` 以获得更好的变体质量

---

## Phase 3：观测与仪表盘 ✅

### 3.1 Dashboard 工作流生命周期视图
- [x] `WorkflowMetrics` 新增 `version`, `is_variant`, `last_failure_note` 字段
- [x] `by_workflow()` 聚合逻辑：版本计数、variant 检测
- [x] `/api/v1/workflows/summary` 接口增加 `version`, `type`, `failure_count` 字段
- [x] Dashboard UI 工作流卡片展示：VARIANT/ORIGINAL 类型徽章、版本号、色彩分级失败计数

---

## 验证清单

### Phase 1 验证
- [x] `cargo check --workspace` 通过
- [x] `cargo test --workspace` 通过（除 pre-existing fastembed 环境问题）

### Phase 2 验证
- [x] `cargo check --workspace` 通过
- [x] `cargo test --workspace` 通过（除 pre-existing fastembed 环境问题）
- [ ] 手动验证：累积 2 次失败后触发 LLM 变体生成（日志可观测 `[Daemon] 🧬`）

### Phase 3 验证
- [x] `cargo check --workspace` 通过
- [x] `cargo test --workspace` 通过（除 pre-existing fastembed 环境问题）
- [ ] Dashboard 工作流 Tab 正确展示新字段 (需部署后人工验证)

---

## 涉及的关键文件

| 文件 | 改动范围 |
|------|----------|
| `crates/telos_memory/src/types.rs` | `MemoryEntry` 新增 `similarity_score` 字段 |
| `crates/telos_memory/src/engine.rs` | VectorSearch 返回 cosine similarity score |
| `crates/telos_memory/src/integration.rs` | 相似度阈值过滤、`attach_failure_note`、`penalize_workflow_template` |
| `crates/telos_memory/src/reconsolidation.rs` | 更新 MemoryEntry 构造 |
| `crates/telos_memory/src/conflict.rs` | 更新 MemoryEntry 构造 |
| `crates/telos_memory/src/tests.rs` | 更新 MemoryEntry 构造 |
| `crates/telos_daemon/src/agents/architect.rs` | Prompt 建议化、`adopted_templates` 输出、Failure Note 注入 |
| `crates/telos_daemon/src/workers/event_loop.rs` | 复用检测改为读取 `adopted_templates` |
| `crates/telos_daemon/src/workers/spawner.rs` | 失败路径 failure note + strength 惩罚 + 物种分化触发 |

---

## Notes / Issues
- `similarity_score` 使用 `Option<f32>` 以保持向后兼容（现有序列化数据缺少该字段时默认 `None`）
- 相似度阈值 0.65 是初始值，随系统运行可通过 eval 数据调优
- 物种分化使用 `strong_reasoning: true` 以获得更高质量的变体
- Failure notes 最多保留 3 条以避免模板内容膨胀
- Strength 惩罚下限 1.0（不至于一次失败就废掉，Ebbinghaus 衰减中 Procedural 类型不衰减）
- `from_value` 需要 `.clone()` 因为它会消费 serde_json::Value
