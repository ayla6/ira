use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub steam_api_key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub steam_griddb_api_key: String,
    #[serde(default = "default_true")]
    pub notifications_enabled: bool,
    #[serde(default)]
    pub close_to_background: bool,
    #[serde(default)]
    pub show_hidden_games: bool,
    #[serde(default = "default_grid_cover_width")]
    pub grid_cover_width: i32,
    #[serde(default)]
    pub shadps4_enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shadps4_executable: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            steam_api_key: String::new(),
            steam_griddb_api_key: String::new(),
            notifications_enabled: true,
            close_to_background: false,
            show_hidden_games: false,
            grid_cover_width: DEFAULT_GRID_COVER_WIDTH,
            shadps4_enabled: false,
            shadps4_executable: String::new(),
        }
    }
}

const DEFAULT_GRID_COVER_WIDTH: i32 = 200;
fn default_grid_cover_width() -> i32 {
    DEFAULT_GRID_COVER_WIDTH
}

fn default_true() -> bool {
    true
}

fn config_path() -> PathBuf {
    let dir = dirs();
    dir.join("achievement-viewer").join("config.json")
}

fn dirs() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config")
    } else {
        PathBuf::from(".")
    }
}

pub fn load_config() -> Config {
    let mut c = Config::default();
    if let Ok(data) = std::fs::read(config_path()) {
        if let Ok(loaded) = serde_json::from_slice::<Config>(&data) {
            c = loaded;
        }
    }
    c
}

impl Config {
    pub fn save(&self) -> Result<(), String> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let data = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, data).map_err(|e| e.to_string())?;
        Ok(())
    }
}
