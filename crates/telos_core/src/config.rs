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

    // Optional chatbot integrations
    pub telegram_bot_token: Option<String>,
    #[serde(default)]
    pub bot_send_state_changes: bool,
}

impl TelosConfig {
    pub fn config_file_path() -> PathBuf {
        let mut path = dirs::home_dir().expect("Could not find home directory");
        path.push(".telos_config.toml");
        path
    }

    pub fn load() -> Result<Self, String> {
        let path = Self::config_file_path();
        if !path.exists() {
            return Err("Config file not found".into());
        }

        let contents = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let config: TelosConfig = toml::from_str(&contents).map_err(|e| e.to_string())?;
        Ok(config)
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_file_path();
        let contents = toml::to_string(self).map_err(|e| e.to_string())?;
        fs::write(path, contents).map_err(|e| e.to_string())?;
        Ok(())
    }
}
