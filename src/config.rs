use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

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

fn get_secret(key: &str) -> String {
    let out = Command::new("secret-tool")
        .args(["lookup", "app", "achievement-viewer", "key", key])
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => String::new(),
    }
}

fn set_secret(key: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        let _ = Command::new("secret-tool")
            .args(["clear", "app", "achievement-viewer", "key", key])
            .output();
        return Ok(());
    }
    let mut cmd = Command::new("secret-tool");
    cmd.args([
        "store",
        "--label=Achievement Viewer Key",
        "app",
        "achievement-viewer",
        "key",
        key,
    ]);
    cmd.stdin(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    use std::io::Write;
    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(value.as_bytes());
    }
    child.wait().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn load_config() -> Config {
    let mut c = Config::default();
    if let Ok(data) = std::fs::read(config_path()) {
        if let Ok(loaded) = serde_json::from_slice::<Config>(&data) {
            c = loaded;
        }
    }
    let steam_key = get_secret("steam");
    if !steam_key.is_empty() {
        c.steam_api_key = steam_key;
    }
    let sgdb_key = get_secret("steamgriddb");
    if !sgdb_key.is_empty() {
        c.steam_griddb_api_key = sgdb_key;
    }
    c
}

impl Config {
    pub fn save(&self) -> Result<(), String> {
        let steam_err = set_secret("steam", &self.steam_api_key);
        let sgdb_err = set_secret("steamgriddb", &self.steam_griddb_api_key);

        let mut plaintext = Config {
            steam_api_key: String::new(),
            steam_griddb_api_key: String::new(),
            notifications_enabled: self.notifications_enabled,
            close_to_background: self.close_to_background,
            show_hidden_games: self.show_hidden_games,
            grid_cover_width: self.grid_cover_width,
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
