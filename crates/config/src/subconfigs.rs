use ira_models::ControllerInputMode;
use serde::{Deserialize, Serialize};

use super::defaults::{default_console_enabled, default_true};

/// A SteamGridDB author whose art is filtered. `name` drives the filtering
/// (the API matches authors by name); `steam64` links to their profile page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SgdbFilteredUser {
    pub name: String,
    #[serde(default)]
    pub steam64: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleConfig {
    #[serde(default = "default_console_enabled")]
    pub enabled: bool,
    pub executable: String,
    pub ra_core: String,
    /// Console-wide fullscreen override (`None` = inherit the global
    /// `default_fullscreen`). Big picture mode forces fullscreen regardless
    /// of this setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fullscreen: Option<bool>,
    /// Optional console-wide input-remapping gate before controller defaults
    /// (`None` = inherit). The virtual controller type comes from the
    /// selected layout, not from this setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller_mode: Option<ControllerInputMode>,
    /// Shared layout used for this console before a game-specific layout.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub controller_profile: String,
    /// Pause controller input and gyro when another window is focused
    /// (`None` = inherit the per-game default, on).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_pause_unfocused: Option<bool>,
    /// Pause the game while Ira's overlay is visible (`None` = inherit the
    /// global `pause_game_on_overlay`). Cooperative via the Goldberg fork
    /// for emulated games, process-group freeze in external-host mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_game_on_overlay: Option<bool>,
}

impl Default for ConsoleConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            executable: String::new(),
            ra_core: String::new(),
            fullscreen: None,
            controller_mode: None,
            controller_profile: String::new(),
            input_pause_unfocused: None,
            pause_game_on_overlay: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Expose gamescope's Vulkan WSI layer by default (Steam/ChimeraOS
    /// sessions export it too); opt out per game or globally.
    #[serde(default = "default_true")]
    pub gamescope_wsi: bool,
    /// Pass `--expose-wayland` to gamescope. Off by default, matching
    /// gamescope itself (Wayland clients need it explicitly).
    #[serde(default)]
    pub gamescope_wayland: bool,
    #[serde(default)]
    pub gpu: String,
    #[serde(default)]
    pub env_vars: Vec<(String, String)>,
    #[serde(default)]
    pub ld_preload: String,
    #[serde(default)]
    pub ld_library_path: String,
}

impl Default for SystemDefaults {
    fn default() -> Self {
        Self {
            gamemode: false,
            mangohud: false,
            gamescope: false,
            gamescope_flags: String::new(),
            gamescope_w: 0,
            gamescope_h: 0,
            gamescope_fps: 0,
            gamescope_upscaling: String::new(),
            gamescope_wsi: true,
            gamescope_wayland: false,
            gpu: String::new(),
            env_vars: Vec::new(),
            ld_preload: String::new(),
            ld_library_path: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_console_config_default() {
        let cc = ConsoleConfig::default();
        assert!(cc.enabled);
        assert_eq!(cc.executable, "");
        assert_eq!(cc.ra_core, "");
        assert_eq!(cc.fullscreen, None);
        assert_eq!(cc.controller_mode, None);
        assert!(cc.controller_profile.is_empty());
    }

    #[test]
    fn test_console_config_fullscreen_json_stays_backward_compatible() {
        // Configs written before fullscreen became inheritable carry an
        // explicit bool; it must load as a console-level override.
        let console: ConsoleConfig =
            serde_json::from_str(r#"{"executable":"","ra_core":"","fullscreen":true}"#).unwrap();
        assert_eq!(console.fullscreen, Some(true));
        // New configs omit the key entirely, meaning inherit.
        let console: ConsoleConfig =
            serde_json::from_str(r#"{"executable":"","ra_core":""}"#).unwrap();
        assert_eq!(console.fullscreen, None);
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
}
