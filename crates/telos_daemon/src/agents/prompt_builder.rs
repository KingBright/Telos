use telos_core::SystemRegistry;
use tracing::{info, warn};

/// SILENT_REPLY_TOKEN: 当 Agent 无需向用户展示中间结果时返回此标记
/// CLI/TUI 端遇到此 token 应忽略，不向用户显示
pub const SILENT_REPLY_TOKEN: &str = "<<SILENT>>";

/// 全局缓存的 SOUL 内容（daemon 启动时加载一次）
static SOUL_CONTENT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// 初始化 SOUL 内容（应在 daemon 启动时调用一次）
/// 查找顺序: ~/.telos/SOUL.md → 项目根目录/SOUL.md → 默认人格
pub fn init_soul(project_root: &str) {
    let candidates = vec![
        // 1. User config dir
        dirs::home_dir().map(|h| h.join(".telos").join("SOUL.md")).unwrap_or_default(),
        // 2. Explicit project root
        std::path::PathBuf::from(project_root).join("SOUL.md"),
        // 3. Cargo workspace (for dev builds)
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../SOUL.md"),
    ];

    let mut loaded = false;
    for soul_path in &candidates {
        if soul_path.exists() {
            if let Ok(c) = std::fs::read_to_string(soul_path) {
                info!("[PromptBuilder] ✅ Loaded SOUL from: {:?}", soul_path);
                let _ = SOUL_CONTENT.set(c);
                loaded = true;
                break;
            }
        }
    }
    if !loaded {
        info!("[PromptBuilder] 📍 No SOUL.md found, using default personality");
        let _ = SOUL_CONTENT.set(default_soul());
    }
}

fn default_soul() -> String {
    "You are a smart and capable personal AI assistant. You can provide all kinds of help including information search, coding, task planning, and daily assistance. You are professional, warm, proactive, and honest.".to_string()
}

/// 获取 SOUL 内容
pub fn get_soul() -> &'static str {
    SOUL_CONTENT.get().map(|s| s.as_str()).unwrap_or("You are a helpful AI assistant.")
}

/// PromptBuilder: 模块化动态 Prompt 组装器
/// 
/// 组装顺序: Identity (SOUL) → Environment → Memory → Tools → Role-specific instructions
pub struct PromptBuilder {
    sections: Vec<(String, String)>, // (section_name, content)
}

impl PromptBuilder {
    pub fn new() -> Self {
        Self { sections: Vec::new() }
    }

    /// 注入 SOUL 身份（从全局缓存读取）
    pub fn with_identity(mut self) -> Self {
        let soul = get_soul();
        self.sections.push(("IDENTITY".into(), format!("[IDENTITY]\n{}\n", soul)));
        self
    }

    /// 注入环境上下文（时间、地点）
    pub fn with_environment(mut self, registry: &dyn SystemRegistry) -> Self {
        if let Some(ctx) = registry.get_system_context() {
            self.sections.push(("ENVIRONMENT".into(), 
                format!("[ENVIRONMENT CONTEXT]\nLocal Time: {}\nPhysical Location: {}\n", 
                    ctx.current_time, ctx.location)));
        }
        self
    }

    /// 注入记忆上下文
    pub fn with_memory(mut self, memory_context: &Option<String>) -> Self {
        if let Some(mem) = memory_context {
            if !mem.is_empty() {
                self.sections.push(("MEMORY".into(), mem.clone()));
            }
        }
        self
    }

    /// 注入工具列表 — 完整模式（包含参数 schema）
    pub fn with_tools_full(mut self, tools: &[telos_tooling::ToolSchema]) -> Self {
        if tools.is_empty() {
            self.sections.push(("TOOLS".into(), 
                "Available Tools:\nNo specialized tools found. Use general reasoning.\n".into()));
        } else {
            let tools_str = tools.iter()
                .map(|t| format!("- {}: {} (Params: {})", t.name, t.description, t.parameters_schema.raw_schema))
                .collect::<Vec<_>>()
                .join("\n");
            self.sections.push(("TOOLS".into(), format!("Available Tools:\n{}\n", tools_str)));
        }
        self
    }

    /// 注入工具列表 — 懒加载模式（仅名称+简介，节省 token）
    /// Planner 选择工具后，完整 schema 才在执行节点中注入
    pub fn with_tools_lazy(mut self, tools: &[telos_tooling::ToolSchema]) -> Self {
        if tools.is_empty() {
            self.sections.push(("TOOLS".into(), 
                "Available Tools:\nNo specialized tools found.\n".into()));
        } else {
            let tools_str = tools.iter()
                .map(|t| format!("- {}: {}", t.name, t.description))
                .collect::<Vec<_>>()
                .join("\n");
            self.sections.push(("TOOLS".into(), format!("Available Tools (summary — full schema provided at execution):\n{}\n", tools_str)));
        }
        self
    }

    /// 注入角色指令（自定义 raw string）
    pub fn with_role_instructions(mut self, instructions: &str) -> Self {
        self.sections.push(("ROLE".into(), instructions.to_string()));
        self
    }

    /// 注入用户画像上下文
    pub fn with_user_profile(mut self, profile_context: &str) -> Self {
        if !profile_context.is_empty() {
            self.sections.push(("USER_PROFILE".into(), profile_context.to_string()));
        }
        self
    }

    /// 构建最终 system prompt
    pub fn build(self) -> String {
        self.sections.into_iter()
            .map(|(_, content)| content)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// 检查 LLM 输出是否为静默回复
pub fn is_silent_reply(content: &str) -> bool {
    content.trim() == SILENT_REPLY_TOKEN || content.trim().starts_with(SILENT_REPLY_TOKEN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prompt_builder_basic() {
        let prompt = PromptBuilder::new()
            .with_role_instructions("You are a test agent.")
            .build();
        assert!(prompt.contains("You are a test agent."));
    }

    #[test]
    fn test_silent_reply_detection() {
        assert!(is_silent_reply("<<SILENT>>"));
        assert!(is_silent_reply("  <<SILENT>>  "));
        assert!(is_silent_reply("<<SILENT>> some internal note"));
        assert!(!is_silent_reply("Hello, how can I help?"));
    }

    #[test]
    fn test_prompt_builder_tools_lazy() {
        let tools = vec![
            telos_tooling::ToolSchema {
                name: "web_search".into(),
                description: "Searches the web".into(),
                parameters_schema: telos_tooling::JsonSchema { raw_schema: serde_json::json!({}) },
                ..Default::default()
            },
        ];
        let prompt = PromptBuilder::new()
            .with_tools_lazy(&tools)
            .build();
        assert!(prompt.contains("web_search: Searches the web"));
        assert!(!prompt.contains("Params:"));
    }

    #[test]
    fn test_prompt_builder_tools_full() {
        let tools = vec![
            telos_tooling::ToolSchema {
                name: "web_search".into(),
                description: "Searches the web".into(),
                parameters_schema: telos_tooling::JsonSchema { raw_schema: serde_json::json!({"type": "object"}) },
                ..Default::default()
            },
        ];
        let prompt = PromptBuilder::new()
            .with_tools_full(&tools)
            .build();
        assert!(prompt.contains("Params:"));
    }
}
