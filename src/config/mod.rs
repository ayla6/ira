mod secrets;

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
    #[serde(default = "default_save_dir")]
    pub save_dir: String,
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
            save_dir: default_save_dir(),
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

fn default_save_dir() -> String {
    xdg_dir(xdg::BaseDirectories::new().ok().map(|b| b.get_data_home()))
        .join("achievement-viewer")
        .to_string_lossy()
        .to_string()
}

fn config_path() -> PathBuf {
    xdg::BaseDirectories::new()
        .map(|b| b.get_config_home().join("achievement-viewer").join("config.json"))
        .unwrap_or_else(|_| PathBuf::from(".").join(".config").join("achievement-viewer").join("config.json"))
}

fn xdg_dir(xdg_home: Option<PathBuf>) -> PathBuf {
    xdg_home.unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".local").join("share")
    })
}

pub fn load_config() -> Config {
    let mut c = Config::default();
    if let Ok(data) = std::fs::read(config_path()) {
        if let Ok(loaded) = serde_json::from_slice::<Config>(&data) {
            c = loaded;
        }
    }
    let steam_key = secrets::get_secret("steam");
    if !steam_key.is_empty() {
        c.steam_api_key = steam_key;
    }
    let sgdb_key = secrets::get_secret("steamgriddb");
    if !sgdb_key.is_empty() {
        c.steam_griddb_api_key = sgdb_key;
    }
    c
}

impl Config {
    pub fn save(&self) -> Result<(), String> {
        let steam_err = secrets::set_secret("steam", &self.steam_api_key);
        let sgdb_err = secrets::set_secret("steamgriddb", &self.steam_griddb_api_key);

        let mut plaintext = Config {
            steam_api_key: String::new(),
            steam_griddb_api_key: String::new(),
            notifications_enabled: self.notifications_enabled,
            close_to_background: self.close_to_background,
            show_hidden_games: self.show_hidden_games,
            grid_cover_width: self.grid_cover_width,
            shadps4_enabled: self.shadps4_enabled,
            shadps4_executable: self.shadps4_executable.clone(),
            save_dir: self.save_dir.clone(),
        };
        if steam_err.is_err() {
            plaintext.steam_api_key = self.steam_api_key.clone();
        }
        if sgdb_err.is_err() {
            plaintext.steam_griddb_api_key = self.steam_griddb_api_key.clone();
        }

        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let data = serde_json::to_string_pretty(&plaintext).map_err(|e| e.to_string())?;
        std::fs::write(&path, data).map_err(|e| e.to_string())?;
        Ok(())
    }
}
