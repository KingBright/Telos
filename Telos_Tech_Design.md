# **Telos：面向长程复杂任务的纯Rust自主智能体全链路架构设计方案**

## **0\. 架构哲学与系统定位**

\*\*“Telos”（泰勒斯）\*\*源自古希腊语，意为“终极目的”或“内在目的”。这寓意着系统通过高度复杂的有向无环图（DAG）和自我重规划能力，在充满不确定性的真实环境中，始终能从原点出发，穿越无数可能的分支路径，最终收敛并抵达任务的终点。

**核心架构原则：**

1. **单体守护进程 (Headless Daemon)**：核心引擎是纯Rust编写的无头单体进程。CLI、API、Web、Yororen UI（桌面端）等全部作为纯粹的视图层，通过事件总线（Event Bus）与内核进行数据驱动交互。  
2. **纯Rust原生高性能栈**：彻底摒弃外部重型数据库（如Redis、Postgres），采用嵌入式纯Rust存储库（redb 作为极速KV状态快照，lance 作为列式向量检索），在单体架构下实现微秒级延迟。  
3. **DAG驱动的原子能力编排**：抛弃将复杂流程硬编码在提示词中的做法。长程任务被动态拆解为细粒度的原子节点，每个节点仅分配所需的“最小上下文（Scoped Context）”。  
4. **防御性编程与零信任**：LLM的输出不可信。所有执行下放至短暂沙盒，并配合严格的权限凭证动态注入与硬性预算熔断。

## **1\. 触发、反馈与交互系统 (HCI & Event Bus)**

**职责**：系统的通讯中枢。实现内核与多UI形态的绝对解耦，支持用户介入（HITL）的渐进式交互。

/// 系统全局统一事件总线数据结构  
pub enum AgentEvent {  
    /// 触发：来自用户的自然语言或结构化指令  
    UserInput { session\_id: String, payload: String },  
    /// 触发：环境回调或定时器自动唤醒  
    AutoTrigger { source: String, payload: Vec\<u8\> },  
    /// 交互：用户对高危操作的审批结果或补充信息  
    UserApproval { task\_id: String, approved: bool, supplement\_info: Option\<String\> },  
    /// 内部协同：DAG节点要求重规划  
    ReplanRequested { node\_id: String, reason: String, partial\_result: NodeResult },  
}

/// 系统给UI/外部的反馈数据结构  
pub enum AgentFeedback {  
    /// 状态更新：用于UI绘制进度条或DAG拓扑图  
    StateChanged { task\_id: String, current\_node: String, status: NodeStatus },  
    /// 交互请求：需要用户介入（如输入密码、确认高危代码提交）  
    RequireHumanIntervention { task\_id: String, prompt: String, risk\_level: RiskLevel },  
    /// 结果输出：阶段性或最终结果  
    Output { session\_id: String, content: String, is\_final: bool },  
}

/// 统一事件代理  
pub trait EventBroker: Send \+ Sync {  
    fn publish\_event(\&self, event: AgentEvent);  
    fn subscribe\_feedback(\&self) \-\> tokio::sync::broadcast::Receiver\<AgentFeedback\>;  
}

## **2\. 任务规划、执行与跟踪模块 (DAG Engine)**

**职责**：维护全局任务的拓扑结构，控制状态机流转，支持动态重规划（Replanning）与崩溃后的防抖回放（Replay）。

/// 原子节点定义  
pub trait ExecutableNode: Send \+ Sync {  
    /// 节点执行函数，只接收经过压缩的局部上下文  
    async fn execute(\&self, ctx: \&ScopedContext, registry: \&dyn SystemRegistry) \-\> Result\<NodeResult, NodeError\>;  
}

/// DAG图定义  
pub struct TaskGraph {  
    pub graph\_id: String,  
    pub nodes: HashMap\<String, Box\<dyn ExecutableNode\>\>,  
    pub edges: Vec\<DirectedEdge\>,  
    pub current\_state: GraphState,  
}

pub struct NodeResult {  
    pub output\_data: Vec\<u8\>,  
    pub extracted\_knowledge: Option\<Knowledge\>,  
    pub next\_routing\_hint: Option\<String\>, // 指导重规划引擎更新DAG图  
}

pub trait ExecutionEngine {  
    /// 启动或恢复DAG执行  
    async fn run\_graph(\&mut self, graph: TaskGraph, broker: \&dyn EventBroker);  
    /// 状态快照持久化（写入 redb 嵌入式存储）  
    fn checkpoint(\&self, graph\_id: \&str) \-\> Result\<(), StorageError\>;  
}

## **3\. 上下文管理模块 (Context Compression)**

**职责**：实施渐进式披露，解决长程任务中的“中间迷失”与Token暴涨。通过EDU（基本篇章单元）树状分解实现无损压缩。

/// 原始庞大的日志与检索数据  
pub struct RawContext {  
    pub history\_logs: Vec\<LogEntry\>,  
    pub retrieved\_docs: Vec\<Document\>,  
}

/// 压缩后的精准局部上下文（仅供当前DAG节点使用）  
pub struct ScopedContext {  
    pub budget\_tokens: usize,  
    pub summary\_tree: Vec\<SummaryNode\>, // 基于RAPTOR的树状摘要  
    pub precise\_facts: Vec\<Fact\>,       // EDU拆解后的原子事实  
}

pub trait ContextManager: Send \+ Sync {  
    /// 动态渐进式披露：根据DAG当前节点的依赖要求，将 Raw 压缩为 Scoped  
    fn compress\_for\_node(\&self, raw: \&RawContext, node\_req: \&NodeRequirement) \-\> ScopedContext;  
    /// 融合新执行结果，更新本地树状结构  
    fn ingest\_new\_info(\&mut self, info: NodeResult);  
}

## **4\. 记忆系统模块 (Hierarchical Memory OS)**

**职责**：提供跨越会话的状态一致性。通过工作、情景、语义和程序四维记忆结构，以及生物学指数遗忘机制，保持检索信噪比。

pub enum MemoryType {  
    Episodic(EventTrace),      // 场景记忆：历史交互轨迹与时间线  
    Semantic(GraphNode),       // 语义记忆：客观事实、领域规则与用户偏好  
    Procedural(SkillTemplate), // 程序记忆：沉淀的代码/经验工作流（Task Recipes）  
}

pub struct MemoryQuery {  
    pub semantic\_vector: Vec\<f32\>, // 用于 Lance 库向量检索  
    pub time\_range: TimeRange,  
    pub tags: Vec\<String\>,  
}

pub trait MemoryOS: Send \+ Sync {  
    /// 异步写入记忆（后台合并知识图谱节点）  
    async fn store(\&self, mem\_type: MemoryType);  
    /// 多路召回检索（结合向量相似度与图谱遍历）  
    async fn retrieve(\&self, query: \&MemoryQuery, limit: usize) \-\> Vec\<MemoryType\>;  
    /// 生物学选择性遗忘机制（后台定时触发艾宾浩斯衰减）  
    fn trigger\_fade\_consolidation(\&self);  
}

## **5\. 工具与能力模块 (Tooling & MCP)**

**职责**：管理内置工具、CLI管道、浏览器自动化以及MCP协议接入点。所有能力被抽象为统一接口。

/// 标准化工具Schema (兼容MCP规范)  
pub struct ToolSchema {  
    pub name: String,  
    pub description: String,  
    pub parameters\_schema: JsonSchema,  
    pub risk\_level: RiskLevel, // Normal, HighRisk  
}

pub trait ToolExecutor: Send \+ Sync {  
    /// 执行工具（通常在物理隔离或进程隔离的沙盒中）  
    async fn call(\&self, params: serde\_json::Value) \-\> Result\<Vec\<u8\>, ToolError\>;  
}

pub trait ToolRegistry: Send \+ Sync {  
    /// 动态检索最匹配当前任务意图的工具列表（按需加载，防止Prompt超载）  
    fn discover\_tools(\&self, intent: \&str, limit: usize) \-\> Vec\<ToolSchema\>;  
    /// 获取执行器  
    fn get\_executor(\&self, tool\_name: \&str) \-\> Option\<Box\<dyn ToolExecutor\>\>;  
}

## **6\. 评测与自我进化模块 (Evolution)**

**职责**：部署独立于执行者的“监督智能体”。防范无限死循环，并从失败轨迹中蒸馏新技能以固化到记忆中。

pub struct ExecutionTrace {  
    pub task\_id: String,  
    pub node\_path: Vec\<String\>,  
    pub errors\_encountered: Vec\<NodeError\>,  
}

/// 提炼出的新技能（程序记忆）  
pub struct SynthesizedSkill {  
    pub trigger\_condition: String,  
    pub executable\_code: String, // 动态合成的Rust/Python控制脚本  
    pub success\_rate: f32,  
}

pub trait Evaluator: Send \+ Sync {  
    /// 基于高维向量余弦相似度检测语义死循环或目标漂移  
    fn detect\_drift(\&self, trace: \&ExecutionTrace) \-\> Result\<(), DriftWarning\>;  
    /// 从混合执行轨迹中蒸馏先验经验（Cross-Instance Reuse）  
    fn distill\_experience(\&self, trace: \&ExecutionTrace) \-\> Option\<SynthesizedSkill\>;  
}

## **7\. 模型路由与网关模块 (Model Gateway & Resource Governor)**

**职责**：统一的 LLM 抽象层。隔离不同大模型供应商，负责请求重试、模型降级（Failover）、并发控制以及 Token 预算消耗的硬熔断。

/// 统一的模型请求参数  
pub struct LlmRequest {  
    pub messages: Vec\<Message\>,  
    pub required\_capabilities: Capability, // 例如：需要视觉能力，或强逻辑推理能力  
    pub budget\_limit: usize,  
}

pub trait ModelGateway: Send \+ Sync {  
    /// 自动路由到最合适的模型（如从高智力云端模型降级到本地小模型）  
    async fn generate(\&self, req: LlmRequest) \-\> Result\<LlmResponse, GatewayError\>;  
      
    /// 资源监控：当十小时任务消耗成本超过阈值时，触发系统级中断  
    fn check\_budget(\&self, session\_id: \&str) \-\> Result\<(), QuotaExceededError\>;  
}

## **8\. 零信任权限与凭证管理模块 (Zero-Trust Security & Vault)**

**职责**：集成基于属性的访问控制（ABAC）。防止“间接提示词注入（Prompt Injection）”引发的提权攻击，采用短期动态凭证注入。

pub struct SecurityPolicy {  
    pub allowed\_paths: Vec\<String\>, // 文件系统沙盒读写边界  
    pub allowed\_network\_hosts: Vec\<String\>, // 网络外发白名单  
}

pub trait SecurityVault: Send \+ Sync {  
    /// 拦截器：在工具被执行前，校验 Agent 生成的参数是否违规  
    fn validate\_tool\_call(\&self, tool\_name: \&str, params: \&serde\_json::Value) \-\> Result\<(), SecurityError\>;  
      
    /// 动态获取短期凭证（不向 LLM 暴露明文，仅在沙盒生命周期内有效）  
    fn lease\_temporary\_credential(\&self, tool\_name: \&str) \-\> Option\<SecureString\>;  
}

## **9\. 可观测性与遥测系统 (Observability & Telemetry)**

**职责**：为长程任务提供“黑匣子”追踪。基于 tracing 生态记录结构化日志，为诊断状态漂移与自我进化提供溯源数据。

pub trait TelemetryProvider: Send \+ Sync {  
    /// 记录结构化的状态漂移、延迟及Token消耗等指标  
    fn record\_metric(\&self, metric\_name: \&str, value: f64, tags: HashMap\<String, String\>);  
      
    /// 将包含 Span ID 和 Trace ID 的完整执行流快照持久化，供自我进化模块（Evaluator）事后学习  
    fn export\_trace\_log(\&self, trace\_id: \&str) \-\> Result\<ExecutionTrace, ExportError\>;  
}

## **10\. 全局协同：数据流转与生命周期**

以下通过一个高复杂度长程任务场景：**“用户通过桌面端 UI 要求修复本地某个庞大项目的未知编译错误，并推送到远程仓库”**，来展示 9 大模块如何严密协同。

### **阶段一：感知、记忆与规划 (Trigger & Plan)**

1. **\[交互总线\]** 桌面端 Yororen UI 封装 AgentEvent::UserInput 并发送至 EventBroker。  
2. **\[遥测模块\]** Telemetry 监听事件，生成全局 Trace ID，开始记录生命周期。  
3. **\[模型网关 & 记忆\]** 规划智能体通过 ModelGateway 请求高智力推理模型。同时向 MemoryOS 发起 retrieve，调取用户的“开发习惯与项目历史结构”语义记忆。  
4. **\[DAG引擎\]** 生成初始 TaskGraph（包含节点：1.分析错误 \-\> 2.沙盒调试 \-\> 3.生成补丁 \-\> 4.提交Git）。

### **阶段二：安全执行与上下文极简压缩 (Execution & Security)**

5. **\[DAG引擎\]** 激活“分析错误”节点，调取CLI文件读取工具。  
6. **\[上下文管理\]** 面对读取回来的几十万行项目日志，ContextManager 使用 RAPTOR 树状压缩算法，将其提炼为精准的 ScopedContext（不超过 4k Token）。  
7. **\[权限管理 & 工具\]** DAG 准备执行“沙盒调试”工具。调用工具前，SecurityVault 拦截校验，确认编译命令未超出安全策略，并动态签发一个仅存活 3 分钟的容器执行 Token。  
8. **\[模型网关\]** 对于简单的日志匹配节点，ModelGateway 智能降级路由至本地高速小模型，节约算力和成本。

### **阶段三：动态重规划与人机协同 (Replanning & HITL)**

9. **\[DAG引擎\]** 沙盒调试失败（返回未预见的依赖冲突 NodeError）。  
10. **\[进化与防错\]** Evaluator 后台检查发现该失败已被重试了3次，触发 DriftWarning 阻止死循环。  
11. **\[DAG引擎\]** 抛出 ReplanRequested 事件，图引擎动态将原 DAG 图的“沙盒调试”节点替换为新的“更新依赖库”分支。  
12. **\[交互总线\]** 执行至最后“提交Git”高危节点。挂起状态机，EventBroker 广播 RequireHumanIntervention。  
13. **\[交互总线\]** UI 呈现代码 Diff，用户确认并补充提交信息，发送 UserApproval 恢复 DAG。

### **阶段四：经验蒸馏与持久化固化 (Evolution & Checkpoint)**

14. **\[DAG引擎\]** 任务完成。每执行完一个阶段，底层调用 redb 完成纳秒级 checkpoint() 快照。  
15. **\[可观测性\]** Telemetry 导出完整的 ExecutionTrace。  
16. **\[进化与记忆\]** Evaluator 提取整条调试路径的逻辑，蒸馏出一个 SynthesizedSkill（应对该类依赖冲突的通用修复脚本）。  
17. **\[记忆系统\]** 脚本作为 Procedural Memory 写入 lance 向量库与 redb 图谱。未来即便遇到不同项目，Telos 也能凭借记忆库瞬间调用此技能，实现能力的自主演进。