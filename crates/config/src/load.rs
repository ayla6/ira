use std::path::PathBuf;

use crate::config_struct::{Config, ConsoleConfig};
use crate::secrets;

pub(crate) fn config_path() -> PathBuf {
    xdg::BaseDirectories::new()
        .get_config_home()
        .map(|p| p.join("ira").join("config.json"))
        .unwrap_or_else(|| PathBuf::from(".").join(".config").join("ira").join("config.json"))
}

pub(crate) fn xdg_dir(xdg_home: Option<PathBuf>) -> PathBuf {
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
