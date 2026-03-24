# MemoryOS 系统性升级计划 (Inspired by Supermemory)

> 本计划基于对 [supermemory](https://github.com/supermemoryai/supermemory) 项目（LongMemEval/LoCoMo/ConvoMem 三大 Benchmark #1）的深度学习，系统性对标 Telos 的 `telos_memory` 模块，识别出 **6 个核心升级维度** 和具体实施步骤。

---

## 对比概览：当前 Telos vs Supermemory

| 维度 | 当前 Telos | Supermemory | 差距等级 |
|------|-----------|-------------|----------|
| **记忆版本控制** | 无。新旧事实并存，仅靠 `confidence` 降权 | `version` + `isLatest` + `parentMemoryId` + `rootMemoryId` 完整版本链 | 🔴 严重 |
| **记忆关系图** | 无。所有记忆是扁平 KV 存储 | 3 种关系类型：`updates` / `extends` / `derives` | 🔴 严重 |
| **时间感知遗忘** | Ebbinghaus 衰减曲线（强度衰减） | `forgetAfter` 精确到期时间 + `isForgotten` 标记 + `forgetReason` | 🟡 部分 |
| **用户画像** | 单一 `UserProfile` 类型，全量注入 prompt | `static` (长期事实) + `dynamic` (近期上下文) 双层结构 | 🔴 严重 |
| **混合搜索** | 仅 Vector Search (cosine > 0.5) | Hybrid Search (RAG + Memory 联合查询) + `rerank` + `rewriteQuery` | 🟡 部分 |
| **检索过滤** | 无。`retrieve_all` 全表扫描 | `containerTag` 隔离 + `isLatest` 过滤 + `isForgotten` 过滤 + 元数据 filter | 🔴 严重 |
| **冲突解决** | 有基础实现 (`conflict.rs`)，但只调整 confidence，不标记版本关系 | 通过 `updates` 关系自动创建新版本，旧记忆 `isLatest=false` | 🟡 部分 |
| **推理记忆** | 有 `reconsolidation.rs` 从 Episodic → Semantic | `derives` 关系：从模式中推导新事实，链接到源记忆 | 🟡 部分 |

---

## ✅ 升级 1: 记忆版本控制系统 (Memory Versioning) — COMPLETED (P0 阶段完成)

### 问题
当前 `MemoryEntry` 是扁平结构，当用户说"我换了特斯拉"时，系统只能降低旧记忆 "我开比亚迪" 的 confidence 到 0.2，但两条记忆仍然独立存在并可能被检索到。

### 目标
实现 Supermemory 式的版本链：新记忆通过 `parentMemoryId` 链接到被它替代的旧记忆，形成一条可追溯的更新链。检索时默认只返回 `is_latest = true` 的记忆。

### 实施

#### [MODIFY] `telos_memory/src/types.rs`
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub memory_type: MemoryType,
    pub content: String,
    pub base_strength: f32,
    pub current_strength: f32,
    pub created_at: u64,
    pub last_accessed: u64,
    pub embedding: Option<Vec<f32>>,
    pub access_count: u32,
    pub confidence: f32,
    pub similarity_score: Option<f32>,
    
    // === NEW: Version Control ===
    /// Version number (starts at 1, increments on update)
    #[serde(default = "default_version")]
    pub version: u32,
    /// Whether this is the latest version in its chain
    #[serde(default = "default_true")]
    pub is_latest: bool,
    /// ID of the memory this entry supersedes (forms version chain)
    #[serde(default)]
    pub parent_memory_id: Option<String>,
    /// ID of the root memory in the version chain
    #[serde(default)]
    pub root_memory_id: Option<String>,
}
```

#### [MODIFY] `telos_memory/src/engine.rs`
- 在 `retrieve()` 方法中，按默认过滤 `is_latest == false` 的记忆
- 添加 `retrieve_with_history()` 方法，可选返回整条版本链

#### [MODIFY] `telos_memory/src/conflict.rs`
- 当 LLM 判定新记忆 **supersedes** 旧记忆时（`old_confidence < 0.3`），执行：
  1. 将旧记忆标记为 `is_latest = false`
  2. 新记忆设置 `parent_memory_id = 旧记忆ID`，`root_memory_id = 旧记忆的root或自身`，`version = 旧version + 1`

---

## ✅ 升级 2: 记忆关系图 (Memory Relations) — COMPLETED

### 问题
当前记忆之间没有任何关联。用户说"我在 Stripe 当 PM"和"我负责支付基础设施"这两条事实完全独立，检索时无法关联上下文。

### 目标
引入 Supermemory 的 3 种关系类型：
- **`Updates`**: 新事实替代旧事实（"换了特斯拉" updates "开比亚迪"）
- **`Extends`**: 新事实补充旧事实（"负责支付基础设施" extends "在 Stripe 当 PM"）
- **`Derives`**: 系统从模式中推导出新事实（"可能是支付产品核心团队" derives "讨论支付API和反欺诈"）

### 实施

#### [MODIFY] `telos_memory/src/types.rs`
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MemoryRelation {
    Updates,
    Extends,
    Derives,
}

// 在 MemoryEntry 中添加:
/// Relationship map: related_memory_id -> relation_type
#[serde(default)]
pub memory_relations: HashMap<String, MemoryRelation>,
```

#### [MODIFY] `telos_memory/src/engine.rs`
- 在 `store()` 中，当检测到冲突时，自动填充 `memory_relations`
- 在 `retrieve()` 结果中，可选展开关联记忆（类似 Supermemory V4 API 的 `include.relatedMemories`）

#### [MODIFY] `telos_memory/src/reconsolidation.rs`
- 当 Episodic → Semantic 提升时，使用 `Derives` 关系链接源 Episodic 记忆

### 实施记录
- `Updates` 关系：在 `engine.rs` `store()` 冲突解决中已实现（old_conf < 0.4 时创建版本链 + Updates 关系）
- `Extends` 关系：在 `engine.rs` `store()` 中新增 else 分支（old_conf >= 0.4 && new_conf >= 0.4 时创建双向 Extends 关系）
- `Derives` 关系：在 `reconsolidation.rs` Episodic → Semantic 提升时已实现
- 关系感知检索：新增 `expand_relations()` 方法到 `MemoryOS` trait，`RedbGraphStore` 实现按 ID 查找关联记忆
- 新增 3 个测试：`test_extends_relation_created`、`test_reconsolidation_creates_derives_relation`、`test_expand_relations`

---

## ✅ 升级 3: 时间感知遗忘 (Temporal Forgetting) — COMPLETED (P0 阶段完成)

### 问题
当前使用 Ebbinghaus 曲线进行强度衰减，适合学术记忆但不适合临时事实。"明天有考试" 这类信息应在特定日期后被精确遗忘，而不是缓慢衰减。

### 目标
补充 Supermemory 式的精确时间遗忘机制，与现有 Ebbinghaus 衰减共存。

### 实施

#### [MODIFY] `telos_memory/src/types.rs`
```rust
// 在 MemoryEntry 中添加:
/// If set, this memory should be forgotten after this timestamp
#[serde(default)]
pub forget_after: Option<u64>,
/// Whether this memory has been explicitly forgotten
#[serde(default)]
pub is_forgotten: bool,
/// Reason for forgetting (temporal, contradicted, user-requested)
#[serde(default)]
pub forget_reason: Option<String>,
/// Whether this is a stable fact (never decays)
#[serde(default)]
pub is_static: bool,
```

#### [MODIFY] `telos_memory/src/decay.rs`
- 在 `apply_decay` 中增加 `forget_after` 检查：如果当前时间 > `forget_after`，直接标记为 `is_forgotten = true`
- 如果 `is_static == true`，跳过所有衰减计算

#### [MODIFY] `telos_memory/src/engine.rs`
- 在 `retrieve()` 中默认过滤 `is_forgotten == true` 的记忆

#### [MODIFY] 记忆提取 Prompt (daemon 侧)
- 在 `extract_and_store_user_profile` 的 prompt 中，要求 LLM 判断事实是否有时效性
- 如果有（如"明天有会议"），提取出 `forget_after` 时间戳

---

## ✅ 升级 4: 双层用户画像 (Static + Dynamic Profiles) — COMPLETED

### 问题
当前 `UserProfile` 类型的所有记忆被全量拼接注入 System Prompt。随着时间积累，UserProfile 列表会包含大量临时状态（"正在调试 auth 服务"）和过期信息，污染上下文。

### 目标
将用户画像拆分为 Supermemory 式的双层结构：
- **Static Profile**: 长期稳定的事实（职业、偏好、技能）→ 始终注入 System Prompt
- **Dynamic Profile**: 近期上下文（当前项目、最近讨论）→ 按需注入

### 实施

#### [MODIFY] `telos_memory/src/types.rs`
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MemoryType {
    Episodic,
    Semantic,
    Procedural,
    UserProfileStatic,   // 重命名：长期事实
    UserProfileDynamic,  // 新增：近期上下文
    InteractionEvent,
}
```
> 向后兼容：现有 `UserProfile` 类型在反序列化时自动映射为 `UserProfileStatic`

#### [NEW] `telos_memory/src/profile.rs`
- 实现 `build_user_profile()` 函数：
  1. 检索所有 `UserProfileStatic`（`is_latest == true`, `is_forgotten == false`）→ 组成 `static` 列表
  2. 检索最近 N 条 `UserProfileDynamic` 和 `InteractionEvent` → 组成 `dynamic` 列表
  3. 返回 `UserProfile { static_facts: Vec<String>, dynamic_context: Vec<String> }`

#### [MODIFY] daemon `prompt_builder.rs`
- 将 `[USER PROFILE]` 注入改为调用 `build_user_profile()` 的结果
- 格式化为：
  ```
  [USER BACKGROUND]
  - Senior engineer specializing in Rust
  - Prefers CLI tools over GUIs
  
  [CURRENT CONTEXT]
  - Working on Telos memory system upgrade
  - Recently debugging authentication issues
  ```

> **实施完成**: `telos_memory/src/profile.rs` 已创建，包含 `build_user_profile()` + `format_profile_for_prompt()` + `build_and_format_profile()`。daemon 侧 `event_loop.rs`、`router.rs` 的 3 个注入点已重构为统一调用。3 个单元测试通过。

---

## ✅ 升级 5: 检索增强 (Retrieval Enhancements) — COMPLETED (P0 阶段完成)

### 问题
当前 `retrieve()` 方法对全表做线性扫描。随着记忆量增长（1000+），性能和精度都会下降。此外，检索结果不会过滤已遗忘或非最新的记忆。

### 目标
在保持 redb 嵌入式零网络开销的前提下，增加检索时的智能过滤和排序。

### 实施

#### [MODIFY] `telos_memory/src/engine.rs` - `retrieve()`
1. **默认过滤**：
   - `is_forgotten == true` → 排除
   - `is_latest == false` → 排除（除非查询明确要求历史）
   - `confidence < 0.3` → 排除（低置信度记忆不进入结果）
2. **时间加权排序**：
   - 检索结果的排序不仅依赖 cosine similarity，还需加入时间权重：`final_score = similarity * 0.7 + recency_score * 0.3`
   - `recency_score = 1.0 / (1.0 + (now - created_at) / ONE_DAY_SECS)`

#### [MODIFY] `telos_memory/src/types.rs` - `MemoryQuery`
```rust
pub enum MemoryQuery {
    VectorSearch { query: Vec<f32>, top_k: usize },
    SemanticSearch { query: String, top_k: usize },
    EntityLookup { entity: String },
    TimeRange { start: u64, end: u64 },
    // NEW: Include history in results
    VectorSearchWithHistory { query: Vec<f32>, top_k: usize },
}
```

---

## ✅ 升级 6: 记忆提取增强 (Extraction Pipeline) — COMPLETED

### 问题
当前的 `extract_and_store_user_profile` 仅提取单一事实字符串列表，缺乏对事实类型、时效性和关系的判断。

### 目标
升级提取 Prompt，让 LLM 在提取事实时一并输出结构化元信息。

### 实施

#### [MODIFY] daemon `event_loop.rs` 中的提取 prompt
从：
```
Extract key facts about the user from this conversation...
Return a JSON array of strings.
```
改为：
```
Extract key facts about the user. For each fact, determine:
1. content: The fact itself
2. fact_type: "static" (long-term) or "dynamic" (temporary/project-specific)
3. forget_after: null if permanent, or ISO timestamp if temporary
4. relation: If this updates or extends a known fact, specify { type: "updates"|"extends", target_content: "..." }

Return JSON:
[{
  "content": "User is a senior Rust engineer",
  "fact_type": "static",
  "forget_after": null,
  "relation": null
}]
```

> **实施完成**: `user_profile.rs` 提取 prompt 已升级为结构化 JSON 输出（含 `fact_type` + `forget_after`）。`ExtractionResponse` struct 解析后按 `fact_type` 路由存储为 `UserProfileStatic` 或 `UserProfileDynamic`。

---

## 实施优先级

| 优先级 | 升级项 | 预估工作量 | 影响 |
|--------|--------|-----------|------|
| **P0** | 升级 1: 版本控制 + 升级 5: 检索过滤 | 2-3 小时 | 直接解决幻觉/矛盾事实问题 |
| **P1** | 升级 4: 双层画像 + 升级 6: 提取增强 | 2-3 小时 | 大幅提升个性化质量和 Token 效率 |
| **P2** | 升级 3: 时间遗忘 | 1-2 小时 | 解决临时信息污染问题 |
| **P3** | 升级 2: 关系图 | 3-4 小时 | 提升上下文关联能力 |

---

## 验证计划

### 单元测试
- 在 `telos_memory/src/tests.rs` 补充：
  - `test_version_chain_creation`: 存入冲突记忆后验证 `is_latest` 状态
  - `test_temporal_forgetting`: 设置 `forget_after` 后触发衰减，验证记忆被标记为遗忘
  - `test_retrieve_filters_forgotten`: 验证 `retrieve()` 默认不返回 `is_forgotten` 记忆
  - `test_static_dynamic_profile`: 验证 `build_user_profile()` 正确分离静态和动态事实

### 集成测试
- 运行 `cargo test --workspace` 确保无回归
- 通过 `run_eval_headless.py` 重新跑评估用例，对比记忆相关 Case 的得分变化

### 手动测试
- 启动 daemon，进行多轮对话：
  1. "我的车是比亚迪" → Agent 记住
  2. "我换了特斯拉" → Agent 建立版本链，旧记忆标记为 `is_latest=false`
  3. "我开什么车？" → Agent 只回答特斯拉
  4. "明天有个会议" → Agent 记住并设置 `forget_after`
  5. (次日) "有什么安排？" → 会议记忆已过期不再注入

## Notes/Issues
- Supermemory 的核心引擎代码是闭源的（运行在 Cloudflare Workers 上），我们只能从其 API Schema、文档和 MCP 客户端代码中逆向推断其内部实现逻辑。以上设计是基于公开信息的最佳实践适配。
- 在保持 Telos "Pure Rust / Zero Network Overhead" 哲学的前提下，所有升级都在 `redb` 嵌入式存储内完成，不引入外部数据库。
