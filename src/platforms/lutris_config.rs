use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use std::path::PathBuf;

use super::lutris::lutris_db_path;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LutrisGameConfig {
    #[serde(default)]
    pub game: LutrisGameSection,
    #[serde(default)]
    pub wine: LutrisWineSection,
    #[serde(default)]
    pub system: LutrisSystemSection,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LutrisGameSection {
    #[serde(default)]
    pub exe: String,
    #[serde(default)]
    pub args: String,
    #[serde(default, rename = "working_dir")]
    pub working_dir: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub arch: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LutrisWineSection {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub dxvk: bool,
    #[serde(default)]
    pub esync: bool,
    #[serde(default)]
    pub fsync: bool,
    #[serde(default)]
    pub fsr: bool,
    #[serde(default)]
    pub vkd3d: bool,
    #[serde(default, rename = "d3d_extras")]
    pub d3d_extras: bool,
    #[serde(default, rename = "dxvk_nvapi")]
    pub dxvk_nvapi: bool,
    #[serde(default)]
    pub battleye: bool,
    #[serde(default)]
    pub eac: bool,
    #[serde(default)]
    pub overrides: std::collections::HashMap<String, String>,
    #[serde(default, rename = "show_debug")]
    pub show_debug: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LutrisSystemSection {
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub gamescope: bool,
    #[serde(default, rename = "gamescope_flags")]
    pub gamescope_flags: String,
    #[serde(default)]
    pub mangohud: bool,
    #[serde(default)]
    pub gamemode: bool,
}

fn lutris_config_dir() -> PathBuf {
    xdg::BaseDirectories::new()
        .get_config_home()
        .map(|p| p.join("lutris").join("games"))
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home)
                .join(".config")
                .join("lutris")
                .join("games")
        })
}

fn lutris_data_games_dir() -> PathBuf {
    xdg::BaseDirectories::new()
        .get_data_home()
        .map(|p| p.join("lutris").join("games"))
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("lutris")
                .join("games")
        })
}

pub fn read_lutris_game_config(
    lutris_id: i64,
) -> Result<(String, String, LutrisGameConfig), String> {
    let db_path = lutris_db_path();
    let conn = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("open {}: {}", db_path.display(), e))?;

    let (configpath, runner, directory) = conn
        .query_row(
            "SELECT configpath, runner, directory FROM games WHERE id = ?1",
            rusqlite::params![lutris_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                ))
            },
        )
        .map_err(|e| format!("query game {}: {}", lutris_id, e))?;

    if configpath.is_empty() {
        return Err(format!("no configpath for Lutris game {}", lutris_id));
    }

    let filename = format!("{}.yml", configpath);
    let config_path = lutris_config_dir().join(&filename);
    let yaml_content = if config_path.exists() {
        std::fs::read_to_string(&config_path)
            .map_err(|e| format!("read {}: {}", config_path.display(), e))?
    } else {
        let fallback = lutris_data_games_dir().join(&filename);
        std::fs::read_to_string(&fallback)
            .map_err(|e| format!("read {}: {}", fallback.display(), e))?
    };

    let config: LutrisGameConfig =
        serde_yaml::from_str(&yaml_content).map_err(|e| format!("parse YAML: {}", e))?;

    Ok((runner, directory, config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lutris_game_config_default() {
        let cfg = LutrisGameConfig::default();
        assert!(cfg.game.exe.is_empty());
        assert!(cfg.game.args.is_empty());
        assert!(cfg.game.working_dir.is_empty());
        assert!(cfg.game.prefix.is_empty());
        assert!(cfg.game.arch.is_empty());
        assert!(cfg.wine.version.is_empty());
        assert!(!cfg.wine.dxvk);
        assert!(!cfg.wine.esync);
        assert!(!cfg.wine.fsync);
        assert!(!cfg.wine.fsr);
        assert!(!cfg.wine.vkd3d);
        assert!(!cfg.wine.d3d_extras);
        assert!(!cfg.wine.dxvk_nvapi);
        assert!(!cfg.wine.battleye);
        assert!(!cfg.wine.eac);
        assert!(cfg.wine.overrides.is_empty());
        assert!(cfg.wine.show_debug.is_empty());
        assert!(cfg.system.env.is_empty());
        assert!(!cfg.system.gamescope);
        assert!(cfg.system.gamescope_flags.is_empty());
        assert!(!cfg.system.mangohud);
        assert!(!cfg.system.gamemode);
    }

    #[test]
    fn test_lutris_game_config_parse_full_yaml() {
        let yaml = r#"
game:
  exe: /path/to/game.exe
  args: -windowed
  working_dir: /path/to/
  prefix: /home/user/.wine-prefixes/game
  arch: win64
wine:
  version: lutris-GE-Proton8-26
  dxvk: true
  esync: true
  fsync: true
  fsr: true
  vkd3d: true
  d3d_extras: true
  dxvk_nvapi: true
  battleye: true
  eac: true
  overrides:
    d3d11: native,builtin
  show_debug: "-all"
system:
  env:
    MY_VAR: value
  gamescope: false
  gamescope_flags: "-W 1920 -H 1080"
  mangohud: false
  gamemode: false
"#;
        let cfg: LutrisGameConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.game.exe, "/path/to/game.exe");
        assert_eq!(cfg.game.args, "-windowed");
        assert_eq!(cfg.game.working_dir, "/path/to/");
        assert_eq!(cfg.game.prefix, "/home/user/.wine-prefixes/game");
        assert_eq!(cfg.game.arch, "win64");
        assert_eq!(cfg.wine.version, "lutris-GE-Proton8-26");
        assert!(cfg.wine.dxvk);
        assert!(cfg.wine.esync);
        assert!(cfg.wine.fsync);
        assert!(cfg.wine.fsr);
        assert!(cfg.wine.vkd3d);
        assert!(cfg.wine.d3d_extras);
        assert!(cfg.wine.dxvk_nvapi);
        assert!(cfg.wine.battleye);
        assert!(cfg.wine.eac);
        assert_eq!(
            cfg.wine.overrides.get("d3d11"),
            Some(&"native,builtin".to_string())
        );
        assert_eq!(cfg.wine.show_debug, "-all");
        assert_eq!(cfg.system.env.get("MY_VAR"), Some(&"value".to_string()));
        assert!(!cfg.system.gamescope);
        assert_eq!(cfg.system.gamescope_flags, "-W 1920 -H 1080");
        assert!(!cfg.system.mangohud);
        assert!(!cfg.system.gamemode);
    }

    #[test]
    fn test_lutris_game_config_partial_yaml() {
        let yaml = "game:\n  exe: /foo\n";
        let cfg: LutrisGameConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.game.exe, "/foo");
        assert!(cfg.game.args.is_empty());
        assert!(!cfg.wine.dxvk);
        assert!(cfg.wine.overrides.is_empty());
        assert!(cfg.system.env.is_empty());
    }
}
