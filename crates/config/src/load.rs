use std::path::PathBuf;

use crate::config_struct::Config;
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
    let mut c = match std::fs::read(config_path()) {
        Ok(data) => serde_json::from_slice::<Config>(&data).unwrap_or_else(|e| {
            eprintln!("Failed to parse config: {e}");
            Config::default()
        }),
        Err(_) => Config::default(),
    };
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
    c
}
