use std::fs;
use std::path::{PathBuf};
use serde::{Deserialize, Serialize};
use telos_core::project::{Project, ProjectConfig};
use telos_core::config::TelosConfig;

#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectRegistryState {
    pub projects: Vec<Project>,
}

pub struct ProjectRegistry {
    registry_path: PathBuf,
}

impl Default for ProjectRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectRegistry {
    pub fn new() -> Self {
        let mut path = dirs::home_dir().expect("Could not find home directory");
        path.push(".telos");
        if !path.exists() {
            let _ = fs::create_dir_all(&path);
        }
        path.push("projects.json");
        Self { registry_path: path }
    }

    pub fn load_state(&self) -> Result<ProjectRegistryState, String> {
        if !self.registry_path.exists() {
            return Ok(ProjectRegistryState { projects: Vec::new() });
        }
        let contents = fs::read_to_string(&self.registry_path).map_err(|e| e.to_string())?;
        serde_json::from_str(&contents).map_err(|e| e.to_string())
    }

    pub fn save_state(&self, state: &ProjectRegistryState) -> Result<(), String> {
        let contents = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
        fs::write(&self.registry_path, contents).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn create_project(&self, name: String, path: Option<String>, description: Option<String>) -> Result<Project, String> {
        let mut state = self.load_state()?;

        let project_path = match path {
            Some(p) => PathBuf::from(p),
            None => {
                let current_dir = std::env::current_dir().map_err(|e| e.to_string())?;
                current_dir.join(&name)
            }
        };

        if !project_path.exists() {
            fs::create_dir_all(&project_path).map_err(|e| format!("Failed to create project directory: {}", e))?;
        }

        // Create default project config
        let project_config_path = project_path.join(".telos_project.toml");
        if !project_config_path.exists() {
            let config = ProjectConfig::default();
            let config_contents = toml::to_string(&config).map_err(|e| e.to_string())?;
            let _ = fs::write(project_config_path, config_contents);
        }

        let new_project = Project::new(name, project_path, description);
        state.projects.push(new_project.clone());
        self.save_state(&state)?;

        Ok(new_project)
    }

    pub fn list_projects(&self) -> Result<Vec<Project>, String> {
        let state = self.load_state()?;
        Ok(state.projects)
    }

    pub fn get_project(&self, id_or_name: &str) -> Result<Option<Project>, String> {
        let state = self.load_state()?;
        Ok(state.projects.into_iter().find(|p| p.id == id_or_name || p.name == id_or_name))
    }

    pub fn set_active_project(&self, id_or_name: &str) -> Result<Project, String> {
        let project = self.get_project(id_or_name)?
            .ok_or_else(|| format!("Project '{}' not found", id_or_name))?;

        let mut config = TelosConfig::load()?;
        config.active_project_id = Some(project.id.clone());
        config.save()?;

        Ok(project)
    }
}
