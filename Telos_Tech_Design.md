# **Telos：面向长程复杂任务的纯Rust自主智能体全链路架构设计方案**

## **0\. 架构哲学与系统定位**

**“Telos”（泰勒斯）**源自古希腊语，意为“终极目的”或“内在目的”。这寓意着系统通过高度复杂的有向无环图（DAG）和自我重规划能力，在充满不确定性的真实环境中，始终能从原点出发，穿越无数可能的分支路径，最终收敛并抵达任务的终点。

**核心架构原则与选型基调：**

1. **单体守护进程 (Headless Daemon) 与 Actor模型**：系统核心作为无状态守护进程运行。基于 tokio 异步运行时，各个核心模块本质上是一个个通过 mpsc channel 通信的 Actor。
2. **纯Rust原生极致性能栈**：彻底摒弃 Redis、Postgres 等独立进程数据库。状态存储与向量检索必须嵌入到同一个进程内存空间，实现“零网络开销”的微秒级数据交换。
3. **DAG驱动的控制流与数据流分离**：LLM 仅作为纯粹的“计算内核”和“决策节点”，绝不掌控全局控制流。全局控制流由严密的 Rust DAG 图算法管理。
4. **防御性编程与零信任沙盒**：假设 LLM 是一个“聪明但不可信的外包员工”。所有生成的可执行代码必须在 WebAssembly (Wasm) 或微型容器沙盒中运行，权限按需动态租用。

## **1\. 触发、反馈与交互系统 (HCI & Event Bus)**

**[设计理念]**
采用单向数据流（Unidirectional Data Flow）。内核完全不知道外部是 CLI、Tauri 桌面端还是 Web 界面。外部系统只是“事件生成器”和“状态观察者”。

**[核心算法与实现机制]**
- **背压（Backpressure）控制**：在 mpsc channel 中设置合理的缓冲区大小（如 1024）。当用户疯狂点击或外部事件激增时，丢弃非核心事件，保障系统内核不 OOM。
- **事件幂等性**：为每个触发的 AgentEvent 生成全局唯一的 UUID (Trace ID)，防止网络抖动导致的重复执行。

**[核心评测与达标指标]**
- **事件分发延迟**：从接收到用户输入到 DAG 引擎唤醒的内部路由耗时 < 1ms。
- **UI状态同步准确率**：DAG 状态机的每次转移必须 100% 同步到视图层，不可出现僵死状态。

```rust
/// 系统全局统一事件总线数据结构
pub enum AgentEvent {
    UserInput { session_id: String, payload: String },
    AutoTrigger { source: String, payload: Vec<u8> },
    UserApproval { task_id: String, approved: bool, supplement_info: Option<String> },
    ReplanRequested { node_id: String, reason: String, partial_result: NodeResult },
}

pub enum AgentFeedback {
    StateChanged { task_id: String, current_node: String, status: NodeStatus },
    RequireHumanIntervention { task_id: String, prompt: String, risk_level: RiskLevel },
    Output { session_id: String, content: String, is_final: bool },
}

pub trait EventBroker: Send + Sync {
    fn publish_event(&self, event: AgentEvent);
    fn subscribe_feedback(&self) -> tokio::sync::broadcast::Receiver<AgentFeedback>;
}
```

## **2\. 任务规划、执行与跟踪模块 (DAG Engine)**

**[设计理念]**
延迟绑定（Late Binding）与动态拓扑。长程任务不可能一开始就规划好 10 小时的完美路径，只能采用走一步看一步的动态路由策略。

**[核心算法与实现机制]**
- **基于 Kahn 算法的拓扑调度**：引擎维护一个就绪队列（入度为 0 的节点）。并行执行所有就绪节点，完成后解除其子节点的入度依赖。
- **防抖动重规划（Debounced Replanning）**：当节点触发 ReplanRequested 时，引擎不是立即全盘推翻 DAG，而是通过图剪枝（Graph Pruning）算法，仅撤销依赖该失败节点的下游子图，并由 LLM 重新生成补救子图接入原图。
- **微秒级快照 (Checkpointing)**：利用 redb。使用写时复制（COW）特性，序列化 TaskGraph 时不阻塞执行线程。

**[核心评测与达标指标]**
- **调度开销**：DAG 引擎的纯图谱计算和节点调度耗时在整体任务中占比 < 5%。
- **灾难恢复时间 (MTTR)**：从进程意外 Killed 到重启并恢复至最后一个 Checkpoint 的时间 < 10ms。

```rust
#[async_trait::async_trait]
pub trait ExecutableNode: Send + Sync {
    async fn execute(&self, ctx: &ScopedContext, registry: &dyn SystemRegistry) -> Result<NodeResult, NodeError>;
}

pub struct TaskGraph {
    pub graph_id: String,
    pub nodes: HashMap<String, Box<dyn ExecutableNode>>,
    pub edges: Vec<DirectedEdge>, // 基于 petgraph 的边定义
    pub current_state: GraphState,
}

pub trait ExecutionEngine {
    async fn run_graph(&mut self, graph: TaskGraph, broker: &dyn EventBroker);
    fn checkpoint(&self, graph_id: &str) -> Result<(), StorageError>;
}
```

## **3\. 上下文管理模块 (Context Compression)**

**[设计理念]**
对抗“中间迷失”与算力浪费。将海量数据通过本地算法结构化，按需披露给大模型。

**[核心算法与实现机制]**
- **EDU (基本篇章单元) 分解树**：使用 tree-sitter 将代码解析为 AST 树，使用 NLP 算法将自然语言按照逻辑意群拆分，确保拆分不破坏代码或句子的完整性。
- **RAPTOR 软聚类算法**：将切分的底层块计算文本 Embeddings。使用高斯混合模型（GMM）进行软聚类（Soft Clustering），允许一个段落属于多个语义簇。调用快速本地模型对每个簇进行摘要，生成父节点，层层递进形成摘要树。DAG 节点查询时，计算 Cosine Similarity，自顶向下遍历，只召回得分高的树枝。

**[核心评测与达标指标]**
- **压缩比（Compression Ratio）**：在不丢失核心事实的前提下，将 100k Token 的长文本压缩至 < 4k Token（> 95% 压缩率）。
- **召回准确率 (IRR)**：在“大海捞针 (NIAH)”测试中，经过 RAPTOR 压缩后的上下文，依然能保证核心指令 > 95% 的召回率。

```rust
pub struct RawContext {
    pub history_logs: Vec<LogEntry>,
    pub retrieved_docs: Vec<Document>,
}

pub struct ScopedContext {
    pub budget_tokens: usize,
    pub summary_tree: Vec<SummaryNode>,
    pub precise_facts: Vec<Fact>,
}

pub trait ContextManager: Send + Sync {
    fn compress_for_node(&self, raw: &RawContext, node_req: &NodeRequirement) -> ScopedContext;
    fn ingest_new_info(&mut self, info: NodeResult);
}
```

## **4\. 记忆系统模块 (Hierarchical Memory OS)**

**[设计理念]**
神经符号图记忆与生物学遗忘。单纯堆砌向量会导致逻辑断裂，必须将记忆升级为带因果关系的网络。

**[核心算法与实现机制]**
- **记忆重固化 (Reconsolidation)**：当写入新事实时，通过 GraphRAG 检测与旧图谱中实体的逻辑冲突。如发现冲突，触发 LLM 仲裁节点重新分配置信度权重（Confidence Score）。
- **FadeMem 艾宾浩斯遗忘模型**：后台定时运行衰减公式：$R = e^{-t/S}$。其中 $R$ 是保留概率，$t$ 是时间，$S$ 是记忆强度（由访问频次、语义显著度计算）。当 $R$ 低于阈值时，自动归档或抹除该节点。

**[核心评测与达标指标]**
- **长程一致性**：在 LoCoMo 和 LongMemEval 记忆基准测试中，跨越多周模拟周期的事实回忆准确率 > 85%。
- **存储稳态**：即便运行 30 天，由于遗忘机制的介入，向量库与图谱的总容量应维持在一个动态平衡的区间，不会发生无限制的线性膨胀。

```rust
pub enum MemoryType {
    Episodic(EventTrace),
    Semantic(GraphNode),
    Procedural(SkillTemplate),
}

pub trait MemoryOS: Send + Sync {
    async fn store(&self, mem_type: MemoryType);
    async fn retrieve(&self, query: &MemoryQuery, limit: usize) -> Vec<MemoryType>;
    fn trigger_fade_consolidation(&self);
}
```

## **5\. 工具与能力模块 (Tooling & MCP)**

**[设计理念]**
统一 MCP 协议与零信任安全沙盒执行。将原生工具链标准化，并将所有动态代码约束在底层沙盒中。

**[核心算法与实现机制]**
- **动态工具检索 (Tool Search)**：当系统中存在几百个 MCP 注册工具时，利用向量检索对工具的 description 进行匹配，仅向 LLM 上下文中注入 Top-5 最相关的工具 Schema。
- **Wasm 物理限额**：使用 wasmtime 引擎。在拉起实例前，强制配置 Config::consume_fuel 限制 CPU 指令执行条数（防止 while(1) 死循环），并硬性分配内存上限（如 50MB，防止内存泄漏攻击）。

**[核心评测与达标指标]**
- **冷启动延迟**：拉起一个包含基本上下文的 Wasm 沙盒耗时 < 10ms，保障密集工具调用的吞吐量。
- **隔离有效性**：恶意代码无法通过任何提权漏洞逃逸至宿主 Rust 进程。

```rust
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters_schema: JsonSchema,
    pub risk_level: RiskLevel,
}

#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn call(&self, params: serde_json::Value) -> Result<Vec<u8>, ToolError>;
}
```

## **6\. 评测与自我进化模块 (Evolution)**

**[设计理念]**
Actor-Critic 双组件反思。分离干活的 Agent（Actor）和评估的 Agent（Critic）。从海量试错中提炼“程序记忆”。

**[核心算法与实现机制]**
- **语义死循环检测 (Semantic Loop Detection)**：在本地使用 ndarray 维护一个滑动窗口（如最近 5 步的 Prompt 嵌入向量）。计算 $\cos(\vec{v}_t, \vec{v}_{t-1})$。如果连续发生高度重复（如余弦相似度 > 0.95），则判定陷入局部极小值，发出物理中断信号。
- **经验蒸馏 (Experience Distillation)**：监督模型（Critic）读取失败到成功的完整 Track。提取“触发条件”，并生成确定的 Rust/Python 脚本作为 SkillTemplate 存入记忆。

**[核心评测与达标指标]**
- **循环拦截率**：死循环拦截准确率应接近 100%，且误杀率（False Positive Rate） < 1%。
- **技能转化率**：由自我反思合成的代码技能，在经过单元测试沙盒验证后的成功保留率 > 60%。

```rust
pub struct SynthesizedSkill {
    pub trigger_condition: String,
    pub executable_code: String,
    pub success_rate: f32,
}

pub trait Evaluator: Send + Sync {
    fn detect_drift(&self, trace: &ExecutionTrace) -> Result<(), DriftWarning>;
    fn distill_experience(&self, trace: &ExecutionTrace) -> Option<SynthesizedSkill>;
}
```

## **7\. 模型路由与网关模块 (Model Gateway & Resource Governor)**

**[设计理念]**
高可用、成本防波堤与智力降级。

**[核心算法与实现机制]**
- **退避重试 (Exponential Backoff)**：针对 429 和 503 错误，算法为 $T_{wait} = 2^c \times base\_delay + jitter$。
- **漏桶限流 (Leaky Bucket)**：对 Session 级别的 Token 消耗进行硬性统计，到达阈值直接熔断并通知用户。

**[核心评测与达标指标]**
- **网关吞吐与拦截损耗**：网关中间件带来的额外请求延迟 < 2ms。

```rust
pub struct LlmRequest {
    pub messages: Vec<Message>,
    pub required_capabilities: Capability,
    pub budget_limit: usize,
}

#[async_trait::async_trait]
pub trait ModelGateway: Send + Sync {
    async fn generate(&self, req: LlmRequest) -> Result<LlmResponse, GatewayError>;
    fn check_budget(&self, session_id: &str) -> Result<(), QuotaExceededError>;
}
```

## **8\. 零信任权限与凭证管理模块 (Zero-Trust Security & Vault)**

**[设计理念]**
基于属性的访问控制（ABAC）与动态凭证注入。Agent 只有逻辑控制权，没有底层密钥所有权。

**[核心算法与实现机制]**
- **按需派生 Token**：当执行 MCP 工具时，Vault 在调用瞬间为沙盒注入具有 TTL（如 30 秒过期）的 JWT Token，沙盒销毁即失效。
- **Casbin 策略校验**：通过配置文件动态评估 sub (Agent ID), obj (Resource), act (Read/Write) 规则。

```rust
pub trait SecurityVault: Send + Sync {
    fn validate_tool_call(&self, tool_name: &str, params: &serde_json::Value) -> Result<(), SecurityError>;
    fn lease_temporary_credential(&self, tool_name: &str) -> Option<SecureString>;
}
```

## **9\. 可观测性与遥测系统 (Observability & Telemetry)**

**[设计理念]**
分布式长程追踪（Distributed Tracing）。十小时的任务不能是黑盒。

**[核心算法与实现机制]**
- **层级 Span 上下文**：使用 Rust tracing 宏，跨越不同 Actor 线程边界传递 Trace ID。将每一次 LLM 调用、工具执行和 DAG 节点封装在嵌套的 Span 中。
- **结构化导出**：异步将 Span 信息组装为 OTLP (OpenTelemetry Protocol) 格式，无阻塞写入本地文件或外部监控后台。

```rust
pub trait TelemetryProvider: Send + Sync {
    fn record_metric(&self, metric_name: &str, value: f64, tags: HashMap<String, String>);
    fn export_trace_log(&self, trace_id: &str) -> Result<ExecutionTrace, ExportError>;
}
```
