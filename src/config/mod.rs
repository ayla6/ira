mod secrets;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::models::WineConfig;

#[derive(Debug, Clone, Default)]
pub struct RaConfig {
    pub enabled: bool,
    pub username: String,
    pub token: String,
    pub psx_folder: String,
    pub psx_executable: String,
    pub ps2_folder: String,
    pub ps2_executable: String,
    pub psp_folder: String,
    pub psp_executable: String,
}

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
    #[serde(default)]
    pub steam_enabled: bool,
    #[serde(default = "default_true")]
    pub lutris_enabled: bool,
    #[serde(default = "default_save_dir")]
    pub save_dir: String,
    #[serde(default)]
    pub default_wine_config: WineConfig,
    #[serde(default)]
    pub default_native_env_vars: Vec<(String, String)>,
    #[serde(default)]
    pub default_api_emu_version: String,
    #[serde(default = "default_sort_mode")]
    pub sort_mode: String,
    #[serde(default)]
    pub sort_descending: bool,
    #[serde(default)]
    pub ra_enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ra_username: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ra_token: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ra_psx_folder: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ra_psx_executable: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ra_ps2_folder: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ra_ps2_executable: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ra_psp_folder: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ra_psp_executable: String,
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
            steam_enabled: false,
            lutris_enabled: true,
            save_dir: default_save_dir(),
            default_wine_config: WineConfig::default(),
            default_native_env_vars: Vec::new(),
            default_api_emu_version: String::new(),
            sort_mode: default_sort_mode(),
            sort_descending: false,
            ra_enabled: false,
            ra_username: String::new(),
            ra_token: String::new(),
            ra_psx_folder: String::new(),
            ra_psx_executable: String::new(),
            ra_ps2_folder: String::new(),
            ra_ps2_executable: String::new(),
            ra_psp_folder: String::new(),
            ra_psp_executable: String::new(),
        }
    }
}

const DEFAULT_GRID_COVER_WIDTH: i32 = 200;
fn default_grid_cover_width() -> i32 {
    DEFAULT_GRID_COVER_WIDTH
}

fn default_sort_mode() -> String {
    "alphabetical".to_string()
}

fn default_true() -> bool {
    true
}

fn default_save_dir() -> String {
    xdg_dir(xdg::BaseDirectories::new().get_data_home())
        .join("ira")
        .to_string_lossy()
        .to_string()
}

fn config_path() -> PathBuf {
    xdg::BaseDirectories::new()
        .get_config_home()
        .map(|p| p.join("ira").join("config.json"))
        .unwrap_or_else(|| PathBuf::from(".").join(".config").join("ira").join("config.json"))
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
    let ra_token = secrets::get_secret("ra_token");
    if !ra_token.is_empty() {
        c.ra_token = ra_token;
    }
    c
}

impl Config {
    pub fn ra_config(&self) -> RaConfig {
        RaConfig {
            enabled: self.ra_enabled,
            username: self.ra_username.clone(),
            token: self.ra_token.clone(),
            psx_folder: self.ra_psx_folder.clone(),
            psx_executable: self.ra_psx_executable.clone(),
            ps2_folder: self.ra_ps2_folder.clone(),
            ps2_executable: self.ra_ps2_executable.clone(),
            psp_folder: self.ra_psp_folder.clone(),
            psp_executable: self.ra_psp_executable.clone(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let steam_err = secrets::set_secret("steam", &self.steam_api_key);
        let sgdb_err = secrets::set_secret("steamgriddb", &self.steam_griddb_api_key);
        let ra_err = secrets::set_secret("ra_token", &self.ra_token);

        let mut plaintext = Config {
            steam_api_key: String::new(),
            steam_griddb_api_key: String::new(),
            ra_token: String::new(),
            notifications_enabled: self.notifications_enabled,
            close_to_background: self.close_to_background,
            show_hidden_games: self.show_hidden_games,
            grid_cover_width: self.grid_cover_width,
            shadps4_enabled: self.shadps4_enabled,
            shadps4_executable: self.shadps4_executable.clone(),
            steam_enabled: self.steam_enabled,
            lutris_enabled: self.lutris_enabled,
            save_dir: self.save_dir.clone(),
            default_wine_config: self.default_wine_config.clone(),
            default_native_env_vars: self.default_native_env_vars.clone(),
            default_api_emu_version: self.default_api_emu_version.clone(),
            sort_mode: self.sort_mode.clone(),
            sort_descending: self.sort_descending,
            ra_enabled: self.ra_enabled,
            ra_username: self.ra_username.clone(),
            ra_psx_folder: self.ra_psx_folder.clone(),
            ra_psx_executable: self.ra_psx_executable.clone(),
            ra_ps2_folder: self.ra_ps2_folder.clone(),
            ra_ps2_executable: self.ra_ps2_executable.clone(),
            ra_psp_folder: self.ra_psp_folder.clone(),
            ra_psp_executable: self.ra_psp_executable.clone(),
        };
        if steam_err.is_err() {
            plaintext.steam_api_key = self.steam_api_key.clone();
        }
        if sgdb_err.is_err() {
            plaintext.steam_griddb_api_key = self.steam_griddb_api_key.clone();
        }
        if ra_err.is_err() {
            plaintext.ra_token = self.ra_token.clone();
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
