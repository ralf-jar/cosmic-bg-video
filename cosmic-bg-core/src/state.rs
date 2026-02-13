use serde::{Serialize, Deserialize};
use std::{collections::HashMap, fs, path::PathBuf};

#[derive(Serialize, Deserialize, Default)]
pub struct AppState {
    pub monitors: HashMap<String, String>, // Monitor -> Ruta del Video
}

impl AppState {
    pub fn config_path() -> PathBuf {
        dirs::config_dir().expect("No se encontró carpeta config")
            .join("cosmic-bg/state.json")
    }

    pub fn load() -> Self {
        fs::read_to_string(Self::config_path())
            .and_then(|c| Ok(serde_json::from_str(&c).unwrap_or_default()))
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = Self::config_path();
        let _ = fs::create_dir_all(path.parent().unwrap());
        let _ = fs::write(path, serde_json::to_string_pretty(self).unwrap());
    }

    pub fn get_pid_path(monitor: &str) -> PathBuf {
        std::env::temp_dir().join(format!("cosmic-bg-{}.pid", monitor))
    }
}