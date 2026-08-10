use ira_models::WineConfig;
use ira_overlay_ipc::OverlaySettings;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::load::{config_path, xdg_dir};
use crate::secrets;

const DEFAULT_GRID_COVER_WIDTH: i32 = 200;
fn default_grid_cover_width() -> i32 {
    DEFAULT_GRID_COVER_WIDTH
}

fn default_true() -> bool {
    true
}

fn default_language_preferences() -> Vec<String> {
    vec!["english".to_string()]
}

fn default_save_dir() -> String {
    xdg_dir(xdg::BaseDirectories::new().get_data_home())
        .join("ira")
        .to_string_lossy()
        .to_string()
}

fn default_console_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleConfig {
    #[serde(default = "default_console_enabled")]
    pub enabled: bool,
    pub executable: String,
    pub ra_core: String,
    pub fullscreen: bool,
}

impl Default for ConsoleConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            executable: String::new(),
            ra_core: String::new(),
            fullscreen: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SystemDefaults {
    #[serde(default)]
    pub gamemode: bool,
    #[serde(default)]
    pub mangohud: bool,
    #[serde(default)]
    pub gamescope: bool,
    #[serde(default)]
    pub gamescope_flags: String,
    #[serde(default)]
    pub gamescope_w: u32,
    #[serde(default)]
    pub gamescope_h: u32,
    #[serde(default)]
    pub gamescope_fps: u32,
    #[serde(default)]
    pub gamescope_upscaling: String,
    #[serde(default)]
    pub gpu: String,
    #[serde(default)]
    pub env_vars: Vec<(String, String)>,
    #[serde(default)]
    pub ld_preload: String,
    #[serde(default)]
    pub ld_library_path: String,
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
    pub vita3k_enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub vita3k_executable: String,
    #[serde(default)]
    pub cemu_enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cemu_executable: String,
    #[serde(default)]
    pub steam_enabled: bool,

    #[serde(default = "default_save_dir")]
    pub save_dir: String,
    #[serde(default)]
    pub default_wine_config: WineConfig,
    #[serde(default)]
    pub default_system: SystemDefaults,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prefix_base_dir: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default_game_folder: String,
    /// Shared ROM root. Each console uses <roms_folder>/<console id>.
    /// Empty means ROM discovery is disabled until a root is selected.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub roms_folder: String,
    #[serde(default)]
    pub default_native_env_vars: Vec<(String, String)>,
    #[serde(default)]
    pub default_api_emu_version: String,
    /// Remembered auto-add emulator choice. `None` = ask every time,
    /// `Some(true)` = always install without prompting, `Some(false)` = never install.
    #[serde(default)]
    pub auto_emu_install: Option<bool>,
    /// When enabled, game save data is centralized to <save_dir>/saves/<app_id>/
    /// at launch time via symlinks. Per-game migrate button works regardless.
    #[serde(default = "default_true")]
    pub centralize_game_saves: bool,
    /// Preferred languages for game emulator configs, in priority order.
    /// When a game is added, the first matching supported language is used.
    #[serde(default = "default_language_preferences")]
    pub language_preferences: Vec<String>,
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
    #[serde(default)]
    pub overlay: OverlaySettings,
}

impl Default for Config {
    fn default() -> Self {
        let mut consoles = HashMap::new();
        for def in ira_models::all_consoles() {
            consoles.insert(
                def.id.to_string(),
                ConsoleConfig {
                    enabled: def.uses_rom_folder(),
                    ..Default::default()
                },
            );
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
            vita3k_enabled: false,
            vita3k_executable: String::new(),
            cemu_enabled: false,
            cemu_executable: String::new(),
            steam_enabled: false,

            save_dir: default_save_dir(),
            default_wine_config: WineConfig::default(),
            default_system: SystemDefaults::default(),
            prefix_base_dir: String::new(),
            default_game_folder: String::new(),
            roms_folder: String::new(),
            default_native_env_vars: Vec::new(),
            default_api_emu_version: String::new(),
            auto_emu_install: None,
            centralize_game_saves: true,
            language_preferences: default_language_preferences(),
            sort_mode: ira_models::SortMode::default(),
            sort_descending: false,
            ra_enabled: false,
            ra_username: String::new(),
            ra_token: String::new(),
            ra_password: String::new(),
            consoles,
            overlay: OverlaySettings::default(),
        }
    }
}

impl Config {
    pub fn console(&self, platform_id: &str) -> &ConsoleConfig {
        self.consoles.get(platform_id).unwrap_or(&EMPTY_CONSOLE)
    }

    pub fn console_mut(&mut self, platform_id: &str) -> &mut ConsoleConfig {
        self.consoles.entry(platform_id.to_string()).or_default()
    }

    pub fn any_console_enabled(&self) -> bool {
        ira_models::all_consoles()
            .filter(|def| def.uses_rom_folder())
            .any(|def| self.console(def.id).enabled)
    }

    pub fn rom_folder(&self, platform_id: &str) -> std::path::PathBuf {
        if self.roms_folder.trim().is_empty() {
            std::path::PathBuf::new()
        } else {
            std::path::Path::new(self.roms_folder.trim()).join(platform_id)
        }
    }

    pub fn ensure_rom_folders(&self) -> Result<(), String> {
        if self.roms_folder.trim().is_empty() {
            return Ok(());
        }
        let root = std::path::Path::new(self.roms_folder.trim());
        std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
        for console in ira_models::all_consoles().filter(|def| def.uses_rom_folder()) {
            std::fs::create_dir_all(root.join(console.id)).map_err(|e| e.to_string())?;
        }
        Ok(())
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
            vita3k_enabled: self.vita3k_enabled,
            vita3k_executable: self.vita3k_executable.clone(),
            cemu_enabled: self.cemu_enabled,
            cemu_executable: self.cemu_executable.clone(),
            steam_enabled: self.steam_enabled,
            save_dir: self.save_dir.clone(),
            default_wine_config: self.default_wine_config.clone(),
            default_system: self.default_system.clone(),
            prefix_base_dir: self.prefix_base_dir.clone(),
            default_game_folder: self.default_game_folder.clone(),
            roms_folder: self.roms_folder.clone(),
            default_native_env_vars: self.default_native_env_vars.clone(),
            default_api_emu_version: self.default_api_emu_version.clone(),
            auto_emu_install: self.auto_emu_install,
            centralize_game_saves: self.centralize_game_saves,
            language_preferences: self.language_preferences.clone(),
            sort_mode: self.sort_mode,
            sort_descending: self.sort_descending,
            ra_enabled: self.ra_enabled,
            ra_username: self.ra_username.clone(),
            consoles: self.consoles.clone(),
            overlay: self.overlay.clone(),
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
    executable: String::new(),
    ra_core: String::new(),
    fullscreen: false,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::load_config;

    #[test]
    fn test_config_default_has_consoles() {
        let cfg = Config::default();
        for def in ira_models::all_consoles() {
            let cc = cfg.console(def.id);
            assert_eq!(cc.enabled, def.uses_rom_folder());
            assert_eq!(cc.executable, "");
            assert_eq!(cc.ra_core, "");
            assert!(!cc.fullscreen);
        }
    }

    #[test]
    fn test_config_save_and_load_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let prev = std::env::var("XDG_CONFIG_HOME").ok();
        std::env::set_var("XDG_CONFIG_HOME", tmp.path());

        let cfg = Config {
            notifications_enabled: false,
            show_hidden_games: true,
            grid_cover_width: 300,
            sort_descending: true,
            save_dir: "/tmp/test_save_dir".to_string(),
            roms_folder: "/tmp/roms".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string_pretty(&cfg).unwrap();
        let path = crate::load::config_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, json).unwrap();

        let loaded = load_config();
        assert_eq!(loaded.notifications_enabled, cfg.notifications_enabled);
        assert_eq!(loaded.show_hidden_games, cfg.show_hidden_games);
        assert_eq!(loaded.grid_cover_width, cfg.grid_cover_width);
        assert_eq!(loaded.sort_descending, cfg.sort_descending);
        assert_eq!(loaded.save_dir, cfg.save_dir);
        assert_eq!(loaded.roms_folder, cfg.roms_folder);
        assert_eq!(loaded.console("n64").enabled, cfg.console("n64").enabled);

        match prev {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }

    #[test]
    fn test_console_config_default() {
        let cc = ConsoleConfig::default();
        assert!(cc.enabled);
        assert_eq!(cc.executable, "");
        assert_eq!(cc.ra_core, "");
        assert!(!cc.fullscreen);
    }

    #[test]
    fn test_rom_folder_uses_shared_root() {
        let cfg = Config {
            roms_folder: "/games/roms".to_string(),
            ..Default::default()
        };
        assert_eq!(
            cfg.rom_folder("gba"),
            std::path::PathBuf::from("/games/roms/gba")
        );
    }

    #[test]
    fn test_rom_folder_uses_virtualboy_console_id() {
        let cfg = Config {
            roms_folder: "/games/roms".to_string(),
            ..Default::default()
        };
        assert_eq!(
            cfg.rom_folder("virtualboy"),
            std::path::PathBuf::from("/games/roms/virtualboy")
        );
    }

    #[test]
    fn test_rom_folder_is_empty_without_shared_root() {
        assert!(Config::default().rom_folder("gba").as_os_str().is_empty());
    }
}
