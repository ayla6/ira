use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GameLaunchConfig {
    pub exe: String,
    pub args: String,
    pub working_dir: String,
    pub env_vars: Vec<(String, String)>,
    pub ld_preload: String,
    pub ld_library_path: String,
    #[serde(default)]
    pub pre_launch: String,
    #[serde(default)]
    pub overlay_enabled: Option<bool>,
    // System-level settings (moved from WineConfig — these apply to ALL games, not just Wine)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gamemode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mangohud: Option<bool>,
    #[serde(default)]
    pub gamescope: Option<bool>,
    #[serde(default)]
    pub gamescope_flags: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gamescope_w: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gamescope_h: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gamescope_fps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gamescope_upscaling: Option<String>,
    #[serde(default)]
    pub gpu: String,
    #[serde(default)]
    pub overlay_encoder: Option<u32>,
    #[serde(default)]
    pub overlay_recording_quality: Option<u32>,
    #[serde(default)]
    pub overlay_font_family: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WineConfig {
    pub enabled: bool,
    pub prefix: String,
    pub version: String,
    pub custom_wine_path: String,
    pub arch: String,
    pub esync: bool,
    pub fsync: bool,
    pub dxvk: bool,
    pub vkd3d: bool,
    pub d3d_extras: bool,
    pub dxvk_nvapi: bool,
    pub fsr: bool,
    pub battleye: bool,
    pub eac: bool,
    pub show_debug: String,
    pub dll_overrides: Vec<(String, String)>,
    pub audio: String,
    pub graphics: String,
    pub desktop_integration: bool,
    pub show_crash_dialogs: bool,
    pub mouse_warp_override: String,
    // Virtual desktop removed — barely works on newer Wine versions.
    // Kept for backward-compat deserialization of old DB rows.
    #[serde(default)]
    pub _virtual_desktop: bool,
    #[serde(default)]
    pub _virtual_desktop_res: String,
    pub dpi_enabled: bool,
    pub dpi: i32,
    // gamemode/mangohud/gamescope moved to GameLaunchConfig.
    // Kept here with #[serde(default)] for backward-compat deserialization of old DB rows.
    #[serde(default)]
    pub _gamemode: bool,
    #[serde(default)]
    pub _mangohud: bool,
    #[serde(default)]
    pub _gamescope: bool,
    #[serde(default)]
    pub _gamescope_flags: String,
    #[serde(default)]
    pub dxvk_frame_rate: i32,
    #[serde(default)]
    pub dxvk_hud: bool,
    #[serde(default = "default_true")]
    pub proton_wow64: bool,
    #[serde(default = "default_true")]
    pub proton_ntsync: bool,
    #[serde(default = "default_true")]
    pub proton_disable_lsteamclient: bool,
    #[serde(default)]
    pub umu_enabled: bool,
    #[serde(default)]
    pub overridden_fields: Vec<String>,
    // GPU moved to GameLaunchConfig. Kept for backward-compat deserialization.
    #[serde(default)]
    pub _gpu: String,
}

fn default_true() -> bool {
    true
}

impl Default for WineConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            prefix: String::new(),
            version: "system".to_string(),
            custom_wine_path: String::new(),
            arch: "auto".to_string(),
            esync: true,
            fsync: true,
            dxvk: true,
            vkd3d: true,
            d3d_extras: true,
            dxvk_nvapi: true,
            fsr: true,
            battleye: true,
            eac: true,
            show_debug: "-all".to_string(),
            dll_overrides: Vec::new(),
            audio: "auto".to_string(),
            graphics: "auto".to_string(),
            desktop_integration: false,
            show_crash_dialogs: false,
            mouse_warp_override: "enable".to_string(),
            _virtual_desktop: false,
            _virtual_desktop_res: String::new(),
            dpi_enabled: false,
            dpi: 96,
            _gamemode: false,
            _mangohud: false,
            _gamescope: false,
            _gamescope_flags: String::new(),
            dxvk_frame_rate: 0,
            dxvk_hud: false,
            proton_wow64: true,
            proton_ntsync: true,
            proton_disable_lsteamclient: true,
            umu_enabled: true,
            overridden_fields: Vec::new(),
            _gpu: String::new(),
        }
    }
}

impl WineConfig {
    pub fn merge_with_default(&self, default: &WineConfig) -> WineConfig {
        let o = &self.overridden_fields;
        let has = |f: &str| o.iter().any(|x| x == f);
        WineConfig {
            enabled: self.enabled,
            prefix: self.prefix.clone(),
            version: self.version.clone(),
            custom_wine_path: self.custom_wine_path.clone(),
            arch: self.arch.clone(),
            esync: if has("esync") {
                self.esync
            } else {
                default.esync
            },
            fsync: if has("fsync") {
                self.fsync
            } else {
                default.fsync
            },
            dxvk: if has("dxvk") { self.dxvk } else { default.dxvk },
            vkd3d: if has("vkd3d") {
                self.vkd3d
            } else {
                default.vkd3d
            },
            d3d_extras: if has("d3d_extras") {
                self.d3d_extras
            } else {
                default.d3d_extras
            },
            dxvk_nvapi: if has("dxvk_nvapi") {
                self.dxvk_nvapi
            } else {
                default.dxvk_nvapi
            },
            fsr: if has("fsr") { self.fsr } else { default.fsr },
            battleye: if has("battleye") {
                self.battleye
            } else {
                default.battleye
            },
            eac: if has("eac") { self.eac } else { default.eac },
            show_debug: if has("show_debug") {
                self.show_debug.clone()
            } else {
                default.show_debug.clone()
            },
            dll_overrides: if has("dll_overrides") {
                self.dll_overrides.clone()
            } else {
                default.dll_overrides.clone()
            },
            audio: if has("audio") {
                self.audio.clone()
            } else {
                default.audio.clone()
            },
            graphics: if has("graphics") {
                self.graphics.clone()
            } else {
                default.graphics.clone()
            },
            desktop_integration: if has("desktop_integration") {
                self.desktop_integration
            } else {
                default.desktop_integration
            },
            show_crash_dialogs: if has("show_crash_dialogs") {
                self.show_crash_dialogs
            } else {
                default.show_crash_dialogs
            },
            mouse_warp_override: if has("mouse_warp_override") {
                self.mouse_warp_override.clone()
            } else {
                default.mouse_warp_override.clone()
            },
            _virtual_desktop: false,
            _virtual_desktop_res: String::new(),
            dpi_enabled: if has("dpi_enabled") {
                self.dpi_enabled
            } else {
                default.dpi_enabled
            },
            dpi: if has("dpi") { self.dpi } else { default.dpi },
            _gamemode: false,
            _mangohud: false,
            _gamescope: false,
            _gamescope_flags: String::new(),
            dxvk_frame_rate: if has("dxvk_frame_rate") {
                self.dxvk_frame_rate
            } else {
                default.dxvk_frame_rate
            },
            dxvk_hud: if has("dxvk_hud") {
                self.dxvk_hud
            } else {
                default.dxvk_hud
            },
            proton_wow64: if has("proton_wow64") {
                self.proton_wow64
            } else {
                default.proton_wow64
            },
            proton_ntsync: if has("proton_ntsync") {
                self.proton_ntsync
            } else {
                default.proton_ntsync
            },
            proton_disable_lsteamclient: if has("proton_disable_lsteamclient") {
                self.proton_disable_lsteamclient
            } else {
                default.proton_disable_lsteamclient
            },
            umu_enabled: self.umu_enabled,
            overridden_fields: self.overridden_fields.clone(),
            _gpu: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WineProfile {
    pub id: i64,
    pub name: String,
    pub wine_version: String,
    pub custom_wine_path: String,
    pub prefix: String,
    pub arch: String,
    #[serde(default)]
    pub umu_enabled: bool,
}

impl Default for WineProfile {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            wine_version: "system".to_string(),
            custom_wine_path: String::new(),
            prefix: String::new(),
            arch: "auto".to_string(),
            umu_enabled: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_game_launch_config_default() {
        let cfg = GameLaunchConfig::default();
        assert!(cfg.exe.is_empty());
        assert!(cfg.env_vars.is_empty());
        assert!(cfg.gamemode.is_none());
        assert!(cfg.mangohud.is_none());
    }

    #[test]
    fn test_wine_config_default() {
        let cfg = WineConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.version, "system");
        assert_eq!(cfg.arch, "auto");
        assert!(cfg.esync);
        assert!(cfg.dxvk);
        assert!(cfg.dll_overrides.is_empty());
        assert!(cfg.proton_ntsync);
        assert!(cfg.proton_wow64);
        assert!(cfg.proton_disable_lsteamclient);
        assert!(cfg.overridden_fields.is_empty());
    }

    #[test]
    fn test_wine_config_serde_roundtrip() {
        let cfg = WineConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: WineConfig = serde_json::from_str(&json).unwrap();
        assert!(deserialized.enabled);
        assert_eq!(deserialized.version, "system");
        assert!(deserialized.proton_ntsync);
    }

    #[test]
    fn test_wine_config_serde_missing_new_fields() {
        let json = r#"{"enabled":true,"prefix":"","version":"system","custom_wine_path":"","arch":"auto","esync":true,"fsync":true,"dxvk":true,"vkd3d":true,"d3d_extras":true,"dxvk_nvapi":true,"fsr":true,"battleye":true,"eac":true,"show_debug":"-all","dll_overrides":[],"audio":"auto","graphics":"auto","desktop_integration":false,"show_crash_dialogs":false,"mouse_warp_override":"enable","virtual_desktop":false,"virtual_desktop_res":"","dpi_enabled":false,"dpi":96,"gamemode":false,"mangohud":false,"gamescope":false,"gamescope_flags":""}"#;
        let deserialized: WineConfig = serde_json::from_str(json).unwrap();
        assert!(deserialized.proton_ntsync);
        assert!(deserialized.proton_wow64);
        assert!(deserialized.proton_disable_lsteamclient);
        assert!(deserialized.overridden_fields.is_empty());
    }

    #[test]
    fn test_merge_with_default() {
        let per_game = WineConfig {
            esync: false,
            overridden_fields: vec!["esync".to_string()],
            ..Default::default()
        };
        let default = WineConfig::default();
        let merged = per_game.merge_with_default(&default);
        assert!(!merged.esync);
        assert!(merged.fsync);
        assert!(merged.proton_ntsync);
    }

    #[test]
    fn test_game_launch_config_serde_roundtrip() {
        let cfg = GameLaunchConfig {
            exe: "/home/user/game.exe".to_string(),
            args: "-foo bar".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: GameLaunchConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.exe, "/home/user/game.exe");
        assert_eq!(deserialized.args, "-foo bar");
    }
}
