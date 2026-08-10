use std::path::PathBuf;

use crate::config_struct::{Config, ConsoleConfig};
use crate::secrets;

pub(crate) fn config_path() -> PathBuf {
    xdg::BaseDirectories::new()
        .get_config_home()
        .map(|p| p.join("ira").join("config.json"))
        .unwrap_or_else(|| {
            PathBuf::from(".")
                .join(".config")
                .join("ira")
                .join("config.json")
        })
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
        let raw: serde_json::Value =
            serde_json::from_slice(&data).unwrap_or(serde_json::Value::Null);
        if raw.get("consoles").is_none() {
            for def in ira_models::all_consoles() {
                let legacy_id = if def.id == "virtualboy" { "vb" } else { def.id };
                let enabled = raw
                    .get(format!("{}_enabled", legacy_id))
                    .and_then(|v| v.as_bool())
                    .unwrap_or_else(|| def.uses_rom_folder());
                let folder = raw
                    .get(format!("ra_{}_folder", legacy_id))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let executable = raw
                    .get(format!("ra_{}_executable", legacy_id))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let ra_core = raw
                    .get(format!("ra_{}_ra_core", legacy_id))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let fullscreen = raw
                    .get(format!("ra_{}_fullscreen", legacy_id))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                c.consoles.insert(
                    def.id.to_string(),
                    ConsoleConfig {
                        enabled,
                        folder,
                        executable,
                        ra_core,
                        fullscreen,
                    },
                );
            }
            if raw
                .get("ra_enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                for def in ira_models::CONSOLES {
                    c.consoles
                        .entry(def.id.to_string())
                        .and_modify(|c| c.enabled = true);
                }
            }
            needs_migration = true;
        }
        for def in ira_models::all_consoles() {
            c.consoles
                .entry(def.id.to_string())
                .or_insert_with(|| ConsoleConfig {
                    enabled: def.uses_rom_folder(),
                    ..Default::default()
                });
        }
        // PRE-RELEASE: initialize all ROM platforms once. Older configs stored
        // false defaults for platforms that were never exposed by the ROM
        // library. After this marker is saved, later user toggles are kept.
        if !c.rom_platforms_initialized {
            for def in ira_models::all_consoles().filter(|def| def.uses_rom_folder()) {
                c.console_mut(def.id).enabled = true;
            }
            c.rom_platforms_initialized = true;
            needs_migration = true;
        }
        // PRE-RELEASE: remove this compatibility migration after the existing
        // database/config has been migrated to the virtualboy console ID.
        if let Some(legacy) = c.consoles.remove("vb") {
            c.consoles.entry("virtualboy".to_string()).or_insert(legacy);
            needs_migration = true;
        }
        if let Some(legacy) = c.overlay.source_overrides.remove("vb") {
            c.overlay
                .source_overrides
                .entry("virtualboy".to_string())
                .or_insert(legacy);
            needs_migration = true;
        }
        if let Some(legacy) = c.overlay.source_gamescope.remove("vb") {
            c.overlay
                .source_gamescope
                .entry("virtualboy".to_string())
                .or_insert(legacy);
            needs_migration = true;
        }
    }
    let (steam_key, sgdb_key, ra_token, ra_password) = std::thread::scope(|s| {
        let steam_key = s.spawn(|| secrets::get_secret("steam"));
        let sgdb_key = s.spawn(|| secrets::get_secret("steamgriddb"));
        let ra_token = s.spawn(|| secrets::get_secret("ra_token"));
        let ra_password = s.spawn(|| secrets::get_secret("ra_password"));
        (
            steam_key.join().unwrap_or_default(),
            sgdb_key.join().unwrap_or_default(),
            ra_token.join().unwrap_or_default(),
            ra_password.join().unwrap_or_default(),
        )
    });
    if !steam_key.is_empty() {
        c.steam_api_key = steam_key;
    }
    if !sgdb_key.is_empty() {
        c.steam_griddb_api_key = sgdb_key;
    }
    if !ra_token.is_empty() {
        c.ra_token = ra_token;
    }
    if !ra_password.is_empty() {
        c.ra_password = ra_password;
    }
    if needs_migration {
        let _ = c.save();
    }
    c
}
