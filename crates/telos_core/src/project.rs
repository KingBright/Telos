use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub description: Option<String>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ProjectConfig {
    pub custom_instructions: Option<String>,
    pub ignored_paths: Vec<String>,
}

impl Project {
    pub fn new(name: String, path: PathBuf, description: Option<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            path,
            description,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        }
    }
}

impl ProjectConfig {
    pub fn load(project_path: &std::path::Path) -> Self {
        let config_path = project_path.join(".telos_project.toml");
        if !config_path.exists() {
            return Self::default();
        }
        std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|contents| toml::from_str(&contents).ok())
            .unwrap_or_else(Self::default)
    }
}
