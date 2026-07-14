mod secrets;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::models::WineConfig;

#[derive(Debug, Clone, Default)]
pub struct RaConfig {
    pub ra_enabled: bool,
    pub username: String,
    pub token: String,
    pub password: String,
    pub psx_enabled: bool,
    pub psx_folder: String,
    pub psx_executable: String,
    pub psx_ra_core: String,
    pub ps2_enabled: bool,
    pub ps2_folder: String,
    pub ps2_executable: String,
    pub ps2_ra_core: String,
    pub psp_enabled: bool,
    pub psp_folder: String,
    pub psp_executable: String,
    pub psp_ra_core: String,
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
    pub ra_password: String,
    #[serde(default)]
    pub psx_enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ra_psx_folder: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ra_psx_executable: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ra_psx_ra_core: String,
    #[serde(default)]
    pub ps2_enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ra_ps2_folder: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ra_ps2_executable: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ra_ps2_ra_core: String,
    #[serde(default)]
    pub psp_enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ra_psp_folder: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ra_psp_executable: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ra_psp_ra_core: String,
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
            ra_password: String::new(),
            psx_enabled: false,
            ra_psx_folder: String::new(),
            ra_psx_executable: String::new(),
            ra_psx_ra_core: String::new(),
            ps2_enabled: false,
            ra_ps2_folder: String::new(),
            ra_ps2_executable: String::new(),
            ra_ps2_ra_core: String::new(),
            psp_enabled: false,
            ra_psp_folder: String::new(),
            ra_psp_executable: String::new(),
            ra_psp_ra_core: String::new(),
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
    let mut needs_migration = false;
    if let Ok(data) = std::fs::read(config_path()) {
        if let Ok(loaded) = serde_json::from_slice::<Config>(&data) {
            c = loaded;
            // One-off migration: if ra_enabled was set but per-console enables don't exist yet,
            // copy ra_enabled to all three console toggles.
            let raw: serde_json::Value = serde_json::from_slice(&data).unwrap_or(serde_json::Value::Null);
            if raw.get("psx_enabled").is_none() && c.ra_enabled {
                c.psx_enabled = true;
                c.ps2_enabled = true;
                c.psp_enabled = true;
                needs_migration = true;
            }
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
    let ra_password = secrets::get_secret("ra_password");
    if !ra_password.is_empty() {
        c.ra_password = ra_password;
    }
    if needs_migration {
        let _ = c.save();
    }
    c
}

impl Config {
    pub fn ra_config(&self) -> RaConfig {
        RaConfig {
            ra_enabled: self.ra_enabled,
            username: self.ra_username.clone(),
            token: self.ra_token.clone(),
            password: self.ra_password.clone(),
            psx_enabled: self.psx_enabled,
            psx_folder: self.ra_psx_folder.clone(),
            psx_executable: self.ra_psx_executable.clone(),
            psx_ra_core: self.ra_psx_ra_core.clone(),
            ps2_enabled: self.ps2_enabled,
            ps2_folder: self.ra_ps2_folder.clone(),
            ps2_executable: self.ra_ps2_executable.clone(),
            ps2_ra_core: self.ra_ps2_ra_core.clone(),
            psp_enabled: self.psp_enabled,
            psp_folder: self.ra_psp_folder.clone(),
            psp_executable: self.ra_psp_executable.clone(),
            psp_ra_core: self.ra_psp_ra_core.clone(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let steam_err = secrets::set_secret("steam", &self.steam_api_key);
        let sgdb_err = secrets::set_secret("steamgriddb", &self.steam_griddb_api_key);
        let ra_err = secrets::set_secret("ra_token", &self.ra_token);
        let ra_pw_err = secrets::set_secret("ra_password", &self.ra_password);

        let mut plaintext = Config {
            steam_api_key: String::new(),
            steam_griddb_api_key: String::new(),
            ra_token: String::new(),
            ra_password: String::new(),
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
            psx_enabled: self.psx_enabled,
            ra_psx_folder: self.ra_psx_folder.clone(),
            ra_psx_executable: self.ra_psx_executable.clone(),
            ra_psx_ra_core: self.ra_psx_ra_core.clone(),
            ps2_enabled: self.ps2_enabled,
            ra_ps2_folder: self.ra_ps2_folder.clone(),
            ra_ps2_executable: self.ra_ps2_executable.clone(),
            ra_ps2_ra_core: self.ra_ps2_ra_core.clone(),
            psp_enabled: self.psp_enabled,
            ra_psp_folder: self.ra_psp_folder.clone(),
            ra_psp_executable: self.ra_psp_executable.clone(),
            ra_psp_ra_core: self.ra_psp_ra_core.clone(),
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
        if ra_pw_err.is_err() {
            plaintext.ra_password = self.ra_password.clone();
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
