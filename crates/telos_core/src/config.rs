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

    /// Optional OpenAI-compatible Audio API base URL (for STT and TTS)
    #[serde(default)]
    pub openai_audio_base_url: Option<String>,

    /// Optional OpenAI-compatible Audio API Key
    #[serde(default)]
    pub openai_audio_api_key: Option<String>,

    /// Voice ID for TTS (e.g., "alloy", "echo", "fable", "onyx", "nova", "shimmer")
    #[serde(default = "default_tts_voice_id")]
    pub tts_voice_id: String,

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

    /// Optional default physical location (e.g., "Suzhou, Jiangsu, China")
    /// Defaults to None. If set, ignores IP-based geolocation.
    #[serde(default)]
    pub default_location: Option<String>,
}

fn default_persona_name() -> String {
    "小特".to_string()
}

fn default_persona_trait() -> String {
    "聪明、活泼且不失风趣".to_string()
}

fn default_tts_voice_id() -> String {
    "alloy".to_string()
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
        let home_dir = dirs::home_dir().expect("Could not find home directory");
        
        let legacy_logs = home_dir.join(".telos_logs");
        if legacy_logs.exists() {
            let _ = fs::remove_dir_all(legacy_logs);
        }

        let home = Self::telos_home();
        if !home.exists() {
            return Ok(());
        }

        let patterns = vec![
            (home.clone(), "memory.redb."),
            (home_dir, ".telos_memory.redb."),
        ];

        for (dir, prefix) in patterns {
            if !dir.exists() { continue; }
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let file_name = entry.file_name();
                    let name_str = file_name.to_string_lossy();
                    // Identify chunk/compaction leftover files, but NOT the main .redb file. 
                    // e.g. .telos_memory.redb.tmp
                    if name_str.starts_with(prefix) && name_str != prefix.trim_end_matches('.') {
                        if let Ok(metadata) = entry.metadata() {
                            if metadata.is_file() {
                                let _ = fs::remove_file(entry.path());
                            }
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

        let home = Self::telos_home();
        if !home.exists() {
            fs::create_dir_all(&home).map_err(|e| e.to_string())?;
        }

        // Migration logic for config file
        if !path.exists() && old_path.exists() {
            fs::copy(&old_path, &path).map_err(|e| format!("Failed to migrate config: {}", e))?;
            let _ = fs::remove_file(&old_path);
        }

        if !path.exists() {
            return Err("Config file not found".into());
        }

        let contents = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let mut config: TelosConfig = toml::from_str(&contents).map_err(|e| e.to_string())?;

        let mut needs_save = false;
        let home_str = dirs::home_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        
        // Standardize DB path and migrate old database file if needed
        let new_db_path = Self::memory_db_path();
        let old_db_path = dirs::home_dir().expect("no home").join(".telos_memory.redb");
        
        let old_db_default = format!("{}/.telos_memory.redb", home_str);
        
        if config.db_path == old_db_default || config.db_path.is_empty() {
            config.db_path = new_db_path.to_string_lossy().into_owned();
            needs_save = true;
        }

        // Physically move the legacy database file if it exists and the new one doesn't
        if old_db_path.exists() && !new_db_path.exists() {
            if fs::rename(&old_db_path, &new_db_path).is_ok() {
                // Ignore cleanup errors for old chunk files
                let _ = Self::cleanup_orphaned_memory_files();
            }
        }

        let old_tools_default = format!("{}/.telos/tools", home_str);
        if config.tools_dir == old_tools_default || config.tools_dir.is_empty() {
             config.tools_dir = default_tools_dir();
             needs_save = true;
        }

        if needs_save {
            let _ = config.save();
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
