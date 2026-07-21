mod secrets;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use ira_models::WineConfig;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConsoleConfig {
    pub enabled: bool,
    pub folder: String,
    pub executable: String,
    pub ra_core: String,
    pub fullscreen: bool,
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
    pub rpcs3_enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rpcs3_executable: String,
    #[serde(default)]
    pub steam_enabled: bool,

    #[serde(default = "default_save_dir")]
    pub save_dir: String,
    #[serde(default)]
    pub default_wine_config: WineConfig,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prefix_base_dir: String,
    #[serde(default)]
    pub default_native_env_vars: Vec<(String, String)>,
    #[serde(default)]
    pub default_api_emu_version: String,
    #[serde(default)]
    pub sort_mode: ira_models::SortMode,
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
    pub consoles: HashMap<String, ConsoleConfig>,
}

impl Default for Config {
    fn default() -> Self {
        let mut consoles = HashMap::new();
        for def in ira_models::CONSOLES {
            consoles.insert(def.id.to_string(), ConsoleConfig::default());
        }
        Self {
            steam_api_key: String::new(),
            steam_griddb_api_key: String::new(),
            notifications_enabled: true,
            close_to_background: false,
            show_hidden_games: false,
            grid_cover_width: DEFAULT_GRID_COVER_WIDTH,
            shadps4_enabled: false,
            shadps4_executable: String::new(),
            rpcs3_enabled: false,
            rpcs3_executable: String::new(),
            steam_enabled: false,

            save_dir: default_save_dir(),
            default_wine_config: WineConfig::default(),
            prefix_base_dir: String::new(),
            default_native_env_vars: Vec::new(),
            default_api_emu_version: String::new(),
            sort_mode: ira_models::SortMode::default(),
            sort_descending: false,
            ra_enabled: false,
            ra_username: String::new(),
            ra_token: String::new(),
            ra_password: String::new(),
            consoles,
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
        }
        let raw: serde_json::Value = serde_json::from_slice(&data).unwrap_or(serde_json::Value::Null);
        if raw.get("consoles").is_none() {
            for def in ira_models::CONSOLES {
                let enabled = raw.get(format!("{}_enabled", def.id))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let folder = raw.get(format!("ra_{}_folder", def.id))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let executable = raw.get(format!("ra_{}_executable", def.id))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let ra_core = raw.get(format!("ra_{}_ra_core", def.id))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let fullscreen = raw.get(format!("ra_{}_fullscreen", def.id))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                c.consoles.insert(def.id.to_string(), ConsoleConfig {
                    enabled, folder, executable, ra_core, fullscreen,
                });
            }
            if raw.get("ra_enabled").and_then(|v| v.as_bool()).unwrap_or(false) {
                for def in ira_models::CONSOLES {
                    c.consoles.entry(def.id.to_string()).and_modify(|c| c.enabled = true);
                }
            }
            needs_migration = true;
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
    pub fn console(&self, platform_id: &str) -> &ConsoleConfig {
        self.consoles.get(platform_id).unwrap_or(&EMPTY_CONSOLE)
    }

    pub fn console_mut(&mut self, platform_id: &str) -> &mut ConsoleConfig {
        self.consoles.entry(platform_id.to_string()).or_default()
    }

    pub fn any_console_enabled(&self) -> bool {
        ira_models::CONSOLES
            .iter()
            .any(|def| self.console(def.id).enabled)
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
            rpcs3_enabled: self.rpcs3_enabled,
            rpcs3_executable: self.rpcs3_executable.clone(),
            steam_enabled: self.steam_enabled,
            save_dir: self.save_dir.clone(),
            default_wine_config: self.default_wine_config.clone(),
            prefix_base_dir: self.prefix_base_dir.clone(),
            default_native_env_vars: self.default_native_env_vars.clone(),
            default_api_emu_version: self.default_api_emu_version.clone(),
            sort_mode: self.sort_mode,
            sort_descending: self.sort_descending,
            ra_enabled: self.ra_enabled,
            ra_username: self.ra_username.clone(),
            consoles: self.consoles.clone(),
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

static EMPTY_CONSOLE: ConsoleConfig = ConsoleConfig {
    enabled: false,
    folder: String::new(),
    executable: String::new(),
    ra_core: String::new(),
    fullscreen: false,
};
