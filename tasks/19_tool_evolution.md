# Iteration 30: Tool OS & Progressive Exposure (自主工具链扩增与渐进式暴露)

Status: Phase 1 Pending | Phase 2 Pending | Phase 3 Pending

结合 Workflow 进化框架，本迭代旨在解决工具数激增时的**Context 灾难（渐进式暴露）**以及工具的**经验沉淀与突变（自动进化）**。工作流将能主动预装依赖工具，Agent 能基于少量核心工具自助发现海量插件。

---

## Phase 1: 渐进式暴露基建 (Progressive Exposure)

当前问题：所有插件无脑打入 Prompt，导致 Context 溢出。
解决方案：将工具分为“元工具(Core/Native)”与“扩展层(Plugins)”。Agent 开局只带少量核心工具，通过 Intent 动态检索额外工具。

- [ ] `crates/telos_tooling/src/registry.rs`: 扩展注册表，支持 `discover_tools(query, top_k)` 的向量检索（Embedding基于Tool Definition）。
- [ ] `crates/telos_tooling/src/native/dev_tools.rs`: 新增原生元工具 `discover_tools`，供 LLM 在迷茫时手动拉取。
- [ ] `crates/telos_daemon/src/agents/prompt_builder.rs`: 设定 Core Tools 清单（如搜索、发信、发现工具），默认不加载所有 Plugins。
- [ ] `crates/telos_memory/src/types.rs`: `ProceduralMemory` 增加 `required_tools: Vec<String>`，当工作流被采纳时，自动把这些要求的工具暴露进当前 Prompt。

---

## Phase 2: 工具经验记忆 (Tool Procedural Memory)

当前问题：工具被错误使用后缺乏反思记忆机制。
解决方案：将工具的 `experience_notes` 保存到 JSON 的 Meta 字段，并在被 LLM 检索（暴露）到时一并渲染注入。

- [ ] `crates/telos_tooling/src/types.rs`: `ToolDefinition` 新增 `experience_notes: Vec<String>` 及反序列化支持。
- [ ] `crates/telos_tooling/src/native/dev_tools.rs`: 新增原生工具 `attach_tool_note` 供模型标记坑点。
- [ ] 后端拦截器或路由逻辑中：当大模型 QA 失败或遭遇使用异常时，自动向该工具写入一条 Note 存档。

---

## Phase 3: 工具代码突变 (Tool Species Divergence)

当前问题：哪怕找对了工具看到了 Note，依然无法改写死板的底层代码以支持明天的预测需求。
解决方案：提供原生的 `mutate_tool`，拉起专职的 Software Coder 彻底重写。

- [ ] `crates/telos_tooling/src/native/dev_tools.rs`: 扩充 `mutate_tool(tool_name, mutation_instruction)` 原生能力。
- [ ] `crates/telos_daemon/src/agents/evolutor.rs`: 突变引擎核心——读取原 `rhai`、原 `json` Schema 和过去的错误 `experience_notes`，重新撰写。
- [ ] 备份原脚本/配制（`.bak`），以原名 `_v1` 或同名覆写更新存储并在内存热重载。清空原错误记忆缓存。

---

## 验证与发布

- [ ] `run_eval_headless.py`: Case A (渐进暴露)，开局不塞天气插件，逼 Agent 根据任务主动发现 `get_weather`。
- [ ] `run_eval_headless.py`: Case B (突变覆盖)，逼 Agent 用旧版工具报错后，引发 Tool Mutation 成功获取明天的天气。
- [ ] 确保 Dashboard 中的 Tools 和 Workflows 面板展现突变次数（版本），保持全链路监控。
