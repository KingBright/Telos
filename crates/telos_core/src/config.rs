use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelosConfig {
    pub openai_api_key: String,
    pub openai_base_url: String,
    pub openai_model: String,
    pub openai_embedding_model: String,
    pub db_path: String,

    #[serde(default = "default_tools_dir")]
    pub tools_dir: String,

    // Optional chatbot integrations
    pub telegram_bot_token: Option<String>,
    #[serde(default)]
    pub bot_send_state_changes: bool,

    pub active_project_id: Option<String>,

    /// Maximum concurrent requests (default: 20)
    /// Set to 0 to disable internal rate limiting
    #[serde(default = "default_max_concurrent_requests")]
    pub max_concurrent_requests: usize,

    /// Log level for feedback verbosity (quiet, normal, verbose, debug)
    #[serde(default = "default_log_level")]
    pub log_level: String,

    /// Global prompt context for LLM (e.g., "user is in China, prefer domestic websites")
    /// This context will be injected into planning prompts to help LLM make better decisions
    #[serde(default)]
    pub global_prompt: Option<String>,

    /// Optional HTTP proxy URL for accessing foreign websites
    /// When set, LLM can decide to use this proxy for tools like web_search
    /// Example: "http://127.0.0.1:7890" or "socks5://127.0.0.1:1080"
    #[serde(default)]
    pub proxy: Option<String>,

    /// Router Persona Name
    #[serde(default = "default_persona_name")]
    pub router_persona_name: String,

    /// Router Persona Trait
    #[serde(default = "default_persona_trait")]
    pub router_persona_trait: String,
}

fn default_persona_name() -> String {
    "小特".to_string()
}

fn default_persona_trait() -> String {
    "聪明、活泼且不失风趣".to_string()
}

fn default_max_concurrent_requests() -> usize {
    20
}

fn default_log_level() -> String {
    "normal".to_string()
}


fn default_tools_dir() -> String {
    let mut path = TelosConfig::telos_home();
    path.push("tools");
    path.to_string_lossy().into_owned()
}

impl TelosConfig {
    pub fn telos_home() -> PathBuf {
        let mut path = dirs::home_dir().expect("Could not find home directory");
        path.push(".telos");
        path
    }
    pub fn cleanup_orphaned_memory_files() -> std::io::Result<()> {
        // Logic to cleanup all legacy files/directories
        let home_dir = dirs::home_dir().expect("Could not find home directory");
        
        let legacy_logs = home_dir.join(".telos_logs");
        if legacy_logs.exists() {
            let _ = fs::remove_dir_all(legacy_logs);
        }

        let home = Self::telos_home();
        if !home.exists() {
            return Ok(());
        }

        // The current standardized path is ~/.telos/memory.redb
        // We want to clean up any ~/.telos/memory.redb.* or ~/.telos_memory.redb.*
        let home_dir = dirs::home_dir().expect("Could not find home directory");
        
        let patterns = vec![
            (home.clone(), "memory.redb."),
            (home_dir, ".telos_memory.redb."),
        ];

        for (dir, prefix) in patterns {
            if !dir.exists() { continue; }
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let file_name = entry.file_name();
                let name_str = file_name.to_string_lossy();
                if name_str.starts_with(prefix) {
                    if let Ok(metadata) = entry.metadata() {
                        if metadata.is_file() {
                            let _ = fs::remove_file(entry.path());
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn logs_dir() -> PathBuf {
        let mut path = Self::telos_home();
        path.push("logs");
        path
    }

    pub fn memory_db_path() -> PathBuf {
        let mut path = Self::telos_home();
        path.push("memory.redb");
        path
    }

    pub fn config_file_path() -> PathBuf {
        let mut path = Self::telos_home();
        path.push("config.toml");
        path
    }

    pub fn old_config_file_path() -> PathBuf {
        let mut path = dirs::home_dir().expect("Could not find home directory");
        path.push(".telos_config.toml");
        path
    }

    pub fn load() -> Result<Self, String> {
        let path = Self::config_file_path();
        let old_path = Self::old_config_file_path();

        // Migration logic
        if !path.exists() && old_path.exists() {
            let home = Self::telos_home();
            if !home.exists() {
                fs::create_dir_all(&home).map_err(|e| e.to_string())?;
            }
            fs::copy(&old_path, &path).map_err(|e| format!("Failed to migrate config: {}", e))?;
            // Delete old config after successful migration
            let _ = fs::remove_file(&old_path);
        }

        if !path.exists() {
            return Err("Config file not found".into());
        }

        let contents = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut config: TelosConfig = toml::from_str(&contents).map_err(|e| e.to_string())?;

        // Standardize paths if they are still legacy
        let home_str = dirs::home_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let old_db_default = format!("{}/.telos_memory.redb", home_str);
        if config.db_path == old_db_default || config.db_path.is_empty() {
            config.db_path = Self::memory_db_path().to_string_lossy().into_owned();
        }

        let old_tools_default = format!("{}/.telos/tools", home_str);
        if config.tools_dir == old_tools_default {
             config.tools_dir = default_tools_dir();
        }

        Ok(config)
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_file_path();
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        let contents = toml::to_string(self).map_err(|e| e.to_string())?;
        fs::write(path, contents).map_err(|e| e.to_string())?;
        Ok(())
    }
}
