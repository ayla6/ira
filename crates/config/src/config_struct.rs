use ira_models::{ControllerInputMode, WineConfig};
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControllerInputConfig {
    /// Controller bridge to use when no per-game override is set.
    pub mode: ControllerInputMode,
    /// Managed profile path to use when no per-game profile is selected.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub profile: String,
}

impl Default for ControllerInputConfig {
    fn default() -> Self {
        Self {
            mode: ControllerInputMode::Disabled,
            profile: String::new(),
        }
    }
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
    /// Optional console-wide input-remapping gate before controller defaults
    /// (`None` = inherit). The virtual controller type comes from the
    /// selected layout, not from this setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller_mode: Option<ControllerInputMode>,
    /// Shared layout used for this console before a game-specific layout.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub controller_profile: String,
}

impl Default for ConsoleConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            executable: String::new(),
            ra_core: String::new(),
            fullscreen: false,
            controller_mode: None,
            controller_profile: String::new(),
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
    #[serde(default = "default_true")]
    pub big_picture_square_capsules: bool,
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
    pub azahar_enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub azahar_executable: String,
    #[serde(default)]
    pub steam_enabled: bool,
    #[serde(default = "default_true")]
    pub auto_reload_steam: bool,
    #[serde(default)]
    pub auto_reload_roms: bool,
    #[serde(default = "default_true")]
    pub auto_reload_shadps4: bool,
    #[serde(default = "default_true")]
    pub auto_reload_rpcs3: bool,
    #[serde(default = "default_true")]
    pub auto_reload_vita3k: bool,
    #[serde(default = "default_true")]
    pub auto_reload_cemu: bool,
    #[serde(default = "default_true")]
    pub auto_reload_azahar: bool,
    /// Installed-title scanning spans every detected yuzu-like and
    /// Ryujinx-like at once, so it is gated per family of emulators, not
    /// per executable.
    #[serde(default = "default_true")]
    pub auto_reload_switch: bool,
    /// Stream .zip/.7z/.zst DS ROMs in memory to read icons and hashes.
    /// Off by default: unpacking is slower than plain-file reads.
    #[serde(default)]
    pub unpack_roms: bool,

    #[serde(default = "default_save_dir")]
    pub save_dir: String,
    #[serde(default)]
    pub default_wine_config: WineConfig,
    #[serde(default)]
    pub default_system: SystemDefaults,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prefix_base_dir: String,
    /// Primary PC games folder: install destination and detection root.
    /// Additional roots live in `extra_game_folders`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default_game_folder: String,
    /// Additional PC games folders scanned alongside `default_game_folder`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_game_folders: Vec<String>,
    /// Primary ROM root. Each console uses <roms_folder>/<console id>.
    /// Empty means ROM discovery is disabled until a root is selected.
    /// Additional roots live in `extra_roms_folders`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub roms_folder: String,
    /// Additional ROM roots, each with per-console subfolders.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_roms_folders: Vec<String>,
    #[serde(default)]
    pub default_native_env_vars: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub linux_controller_profile: String,
    /// Input-remapping gate for native Linux games (`None` = inherit).
    /// The virtual controller type comes from the selected layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linux_controller_mode: Option<ControllerInputMode>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub wine_controller_profile: String,
    /// Input-remapping gate for Wine games (`None` = inherit).
    /// The virtual controller type comes from the selected layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wine_controller_mode: Option<ControllerInputMode>,
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
    pub ra_web_api_key: String,
    #[serde(default)]
    pub consoles: HashMap<String, ConsoleConfig>,
    #[serde(default)]
    pub overlay: OverlaySettings,
    /// Controller defaults keyed by USB vendor/product, e.g. `2dc8:6012`.
    #[serde(default)]
    pub controller_defaults: HashMap<String, ControllerInputConfig>,
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
            big_picture_square_capsules: true,
            grid_cover_width: DEFAULT_GRID_COVER_WIDTH,
            shadps4_enabled: false,
            shadps4_executable: String::new(),
            rpcs3_enabled: false,
            rpcs3_executable: String::new(),
            vita3k_enabled: false,
            vita3k_executable: String::new(),
            cemu_enabled: false,
            cemu_executable: String::new(),
            azahar_enabled: false,
            azahar_executable: String::new(),
            steam_enabled: false,
            auto_reload_steam: true,
            auto_reload_roms: false,
            auto_reload_shadps4: true,
            auto_reload_rpcs3: true,
            auto_reload_vita3k: true,
            auto_reload_cemu: true,
            auto_reload_azahar: true,
            auto_reload_switch: true,
            unpack_roms: true,

            save_dir: default_save_dir(),
            default_wine_config: WineConfig::default(),
            default_system: SystemDefaults::default(),
            prefix_base_dir: String::new(),
            default_game_folder: String::new(),
            extra_game_folders: Vec::new(),
            roms_folder: String::new(),
            extra_roms_folders: Vec::new(),
            default_native_env_vars: Vec::new(),
            linux_controller_profile: String::new(),
            linux_controller_mode: None,
            wine_controller_profile: String::new(),
            wine_controller_mode: None,
            default_api_emu_version: String::new(),
            auto_emu_install: None,
            centralize_game_saves: true,
            language_preferences: default_language_preferences(),
            sort_mode: ira_models::SortMode::default(),
            sort_descending: false,
            ra_enabled: false,
            ra_username: String::new(),
            ra_web_api_key: String::new(),
            consoles,
            overlay: OverlaySettings::default(),
            controller_defaults: HashMap::new(),
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

    pub fn controller_key(vendor: u16, product: u16) -> String {
        format!("{vendor:04x}:{product:04x}")
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

    /// All PC games folders in priority order: the primary folder first,
    /// then extras. Empty and duplicate entries are dropped.
    pub fn all_game_folders(&self) -> Vec<std::path::PathBuf> {
        let mut entries = vec![self.default_game_folder.clone()];
        entries.extend(self.extra_game_folders.iter().cloned());
        trimmed_unique_paths(entries)
    }

    /// All ROM roots in priority order: the primary root first, then extras.
    pub fn all_rom_roots(&self) -> Vec<std::path::PathBuf> {
        let mut entries = vec![self.roms_folder.clone()];
        entries.extend(self.extra_roms_folders.iter().cloned());
        trimmed_unique_paths(entries)
    }

    /// Resolve a stored ROM path to an absolute path. Absolute paths pass
    /// through; relative paths live under <root>/<platform>/ and are looked
    /// up in every configured root before falling back to the primary one.
    pub fn resolve_rom_path(&self, platform_id: &str, path: &str) -> Option<std::path::PathBuf> {
        let path = std::path::Path::new(path);
        if path.is_absolute() {
            return Some(path.to_path_buf());
        }
        let roots = self.all_rom_roots();
        if roots.is_empty() {
            return None;
        }
        for root in &roots {
            let candidate = root.join(platform_id).join(path);
            if candidate.exists() {
                return Some(candidate);
            }
        }
        Some(roots[0].join(platform_id).join(path))
    }

    pub fn ensure_rom_folders(&self) -> Result<(), String> {
        for root in self.all_rom_roots() {
            std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
            for console in ira_models::all_consoles().filter(|def| def.uses_rom_folder()) {
                std::fs::create_dir_all(root.join(console.id)).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    pub fn save(&self) -> Result<(), String> {
        let steam_err = secrets::set_secret("steam", &self.steam_api_key);
        let sgdb_err = secrets::set_secret("steamgriddb", &self.steam_griddb_api_key);
        let ra_web_err = secrets::set_secret("ra_web_api_key", &self.ra_web_api_key);
        let mut plaintext = self.clone();
        plaintext.steam_api_key = String::new();
        plaintext.steam_griddb_api_key = String::new();
        plaintext.ra_web_api_key = String::new();
        if steam_err.is_err() {
            plaintext.steam_api_key = self.steam_api_key.clone();
        }
        if sgdb_err.is_err() {
            plaintext.steam_griddb_api_key = self.steam_griddb_api_key.clone();
        }
        if ra_web_err.is_err() {
            plaintext.ra_web_api_key = self.ra_web_api_key.clone();
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
    controller_mode: None,
    controller_profile: String::new(),
};

/// Trim whitespace, drop empties, dedupe, and convert to paths in order.
fn trimmed_unique_paths(entries: Vec<String>) -> Vec<std::path::PathBuf> {
    let mut out: Vec<std::path::PathBuf> = Vec::new();
    for e in entries {
        let trimmed = e.trim().to_string();
        if trimmed.is_empty() || out.contains(&std::path::PathBuf::from(&trimmed)) {
            continue;
        }
        out.push(std::path::PathBuf::from(trimmed));
    }
    out
}

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
    fn test_config_default_auto_reload_sources() {
        let cfg = Config::default();

        assert!(cfg.auto_reload_steam);
        assert!(!cfg.auto_reload_roms);
        assert!(cfg.auto_reload_shadps4);
        assert!(cfg.auto_reload_rpcs3);
        assert!(cfg.auto_reload_vita3k);
        assert!(cfg.auto_reload_cemu);
        assert!(cfg.auto_reload_azahar);
        assert!(cfg.auto_reload_switch);
        assert!(cfg.unpack_roms);
    }

    #[test]
    fn test_controller_key_uses_stable_usb_identity() {
        assert_eq!(Config::controller_key(0x2dc8, 0x6012), "2dc8:6012");
    }

    #[test]
    fn test_controller_input_config_json_roundtrip() {
        let cfg = ControllerInputConfig {
            mode: ControllerInputMode::Enabled,
            profile: "x".to_string(),
        };
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(json["mode"], "enabled");
        assert_eq!(json["profile"], "x");

        let deserialized: ControllerInputConfig = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.mode, ControllerInputMode::Enabled);
        assert_eq!(deserialized.profile, "x");
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
            extra_roms_folders: vec!["/mnt/hdd/roms".to_string()],
            default_game_folder: "/games/pc".to_string(),
            extra_game_folders: vec!["/mnt/hdd/games".to_string()],
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
        assert_eq!(loaded.extra_roms_folders, cfg.extra_roms_folders);
        assert_eq!(loaded.default_game_folder, cfg.default_game_folder);
        assert_eq!(loaded.extra_game_folders, cfg.extra_game_folders);
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
        assert_eq!(cc.controller_mode, None);
        assert!(cc.controller_profile.is_empty());
    }

    #[test]
    fn test_config_default_pc_controller_profiles_are_empty() {
        let config = Config::default();
        assert!(config.linux_controller_profile.is_empty());
        assert!(config.wine_controller_profile.is_empty());
    }

    #[test]
    fn test_console_config_accepts_controller_profile() {
        let console: ConsoleConfig =
            serde_json::from_str(
                r#"{"enabled":true,"executable":"","ra_core":"","fullscreen":false,"controller_mode":"disabled","controller_profile":"/layouts/ps1.json"}"#,
            )
            .unwrap();
        assert_eq!(console.controller_mode, Some(ControllerInputMode::Disabled));
        assert_eq!(console.controller_profile, "/layouts/ps1.json");
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
    fn test_config_level_controller_modes_json_roundtrip() {
        let cfg = Config {
            linux_controller_mode: Some(ControllerInputMode::Enabled),
            wine_controller_mode: Some(ControllerInputMode::Disabled),
            ..Default::default()
        };
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(json["linux_controller_mode"], "enabled");
        assert_eq!(json["wine_controller_mode"], "disabled");

        let deserialized: Config = serde_json::from_value(json).unwrap();
        assert_eq!(
            deserialized.linux_controller_mode,
            Some(ControllerInputMode::Enabled)
        );
        assert_eq!(
            deserialized.wine_controller_mode,
            Some(ControllerInputMode::Disabled)
        );
    }

    #[test]
    fn test_rom_folder_is_empty_without_shared_root() {
        assert!(Config::default().rom_folder("gba").as_os_str().is_empty());
    }

    #[test]
    fn test_all_game_folders_primary_first_then_extras() {
        let cfg = Config {
            default_game_folder: "/games/pc".to_string(),
            extra_game_folders: vec![
                "/mnt/hdd/games".to_string(),
                "  ".to_string(),
                "/games/pc".to_string(),
            ],
            ..Default::default()
        };
        assert_eq!(
            cfg.all_game_folders(),
            vec![
                std::path::PathBuf::from("/games/pc"),
                std::path::PathBuf::from("/mnt/hdd/games"),
            ]
        );
    }

    #[test]
    fn test_all_rom_roots_empty_without_any_root() {
        assert!(Config::default().all_rom_roots().is_empty());
    }

    #[test]
    fn test_resolve_rom_path_prefers_existing_root() {
        let tmp = tempfile::TempDir::new().unwrap();
        let primary = tmp.path().join("roms1");
        let extra = tmp.path().join("roms2");
        std::fs::create_dir_all(primary.join("gba")).unwrap();
        std::fs::create_dir_all(extra.join("gba")).unwrap();
        std::fs::write(extra.join("gba").join("mario.gba"), b"x").unwrap();

        let cfg = Config {
            roms_folder: primary.to_string_lossy().into_owned(),
            extra_roms_folders: vec![extra.to_string_lossy().into_owned()],
            ..Default::default()
        };
        let resolved = cfg.resolve_rom_path("gba", "mario.gba").unwrap();
        assert_eq!(resolved, extra.join("gba").join("mario.gba"));
        // Missing files fall back to the primary root.
        let missing = cfg.resolve_rom_path("gba", "nope.gba").unwrap();
        assert_eq!(missing, primary.join("gba").join("nope.gba"));
    }

    #[test]
    fn test_resolve_rom_path_absolute_passes_through() {
        let cfg = Config {
            roms_folder: "/games/roms".to_string(),
            ..Default::default()
        };
        assert_eq!(
            cfg.resolve_rom_path("gba", "/other/x.gba"),
            Some(std::path::PathBuf::from("/other/x.gba"))
        );
    }

    #[test]
    fn test_resolve_rom_path_none_without_roots() {
        assert!(Config::default().resolve_rom_path("gba", "x.gba").is_none());
    }
}
