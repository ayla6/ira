use std::path::PathBuf;

use crate::config_struct::Config;
use crate::secrets;

/// Serializes every config read-modify-write cycle: concurrent
/// `std::fs::write`s on the same JSON can interleave and leave a file no
/// parser accepts, which used to reset the whole config to defaults.
pub(crate) fn config_io_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, MutexGuard};
    static CONFIG_IO_LOCK: Mutex<()> = Mutex::new(());
    // A panicked writer must not take config saving down with it.
    let guard: MutexGuard<'static, ()> = CONFIG_IO_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    guard
}

/// Copy of the last config that parsed and saved cleanly, written on every
/// save and consulted when the live file no longer parses.
pub(crate) fn backup_path() -> PathBuf {
    config_path().with_extension("json.bak")
}

fn parse_config_file(path: &PathBuf) -> Option<Config> {
    let data = std::fs::read(path).ok()?;
    match serde_json::from_slice::<Config>(&data) {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("Failed to parse {}: {e}", path.display());
            None
        }
    }
}

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
    let _guard = config_io_lock();
    let path = config_path();
    let mut c = match std::fs::read(&path) {
        Ok(data) => match serde_json::from_slice::<Config>(&data) {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("Failed to parse config: {e}; trying the backup copy");
                parse_config_file(&backup_path())
            }
        },
        // No config yet (fresh install); the backup is not consulted so a
        // deleted config stays deleted.
        Err(_) => None,
    }
    .unwrap_or_default();
    let (steam_key, sgdb_key, ra_web_api_key) = std::thread::scope(|s| {
        let steam_key = s.spawn(|| secrets::get_secret("steam"));
        let sgdb_key = s.spawn(|| secrets::get_secret("steamgriddb"));
        let ra_web_api_key = s.spawn(|| secrets::get_secret("ra_web_api_key"));
        (
            steam_key.join().unwrap_or_default(),
            sgdb_key.join().unwrap_or_default(),
            ra_web_api_key.join().unwrap_or_default(),
        )
    });
    if !steam_key.is_empty() {
        c.steam_api_key = steam_key;
    }
    if !sgdb_key.is_empty() {
        c.steam_griddb_api_key = sgdb_key;
    }
    if !ra_web_api_key.is_empty() {
        c.ra_web_api_key = ra_web_api_key;
    }
    c
}
