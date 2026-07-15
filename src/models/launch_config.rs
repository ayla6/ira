use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameLaunchConfig {
    pub exe: String,
    pub args: String,
    pub working_dir: String,
    pub env_vars: Vec<(String, String)>,
    pub ld_preload: String,
    pub ld_library_path: String,
}

impl Default for GameLaunchConfig {
    fn default() -> Self {
        Self {
            exe: String::new(),
            args: String::new(),
            working_dir: String::new(),
            env_vars: Vec::new(),
            ld_preload: String::new(),
            ld_library_path: String::new(),
        }
    }
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
    pub virtual_desktop: bool,
    pub virtual_desktop_res: String,
    pub dpi_enabled: bool,
    pub dpi: i32,
    pub gamemode: bool,
    pub mangohud: bool,
    pub gamescope: bool,
    pub gamescope_flags: String,
    #[serde(default)]
    pub dxvk_frame_rate: i32,
    #[serde(default = "default_true")]
    pub proton_wow64: bool,
    #[serde(default = "default_true")]
    pub proton_ntsync: bool,
    #[serde(default)]
    pub wine_env_vars: Vec<(String, String)>,
    #[serde(default)]
    pub overridden_fields: Vec<String>,
}

fn default_true() -> bool { true }

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
            virtual_desktop: false,
            virtual_desktop_res: String::new(),
            dpi_enabled: false,
            dpi: 96,
            gamemode: false,
            mangohud: false,
            gamescope: false,
            gamescope_flags: String::new(),
            dxvk_frame_rate: 0,
            proton_wow64: true,
            proton_ntsync: true,
            wine_env_vars: Vec::new(),
            overridden_fields: Vec::new(),
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
            esync: if has("esync") { self.esync } else { default.esync },
            fsync: if has("fsync") { self.fsync } else { default.fsync },
            dxvk: if has("dxvk") { self.dxvk } else { default.dxvk },
            vkd3d: if has("vkd3d") { self.vkd3d } else { default.vkd3d },
            d3d_extras: if has("d3d_extras") { self.d3d_extras } else { default.d3d_extras },
            dxvk_nvapi: if has("dxvk_nvapi") { self.dxvk_nvapi } else { default.dxvk_nvapi },
            fsr: if has("fsr") { self.fsr } else { default.fsr },
            battleye: if has("battleye") { self.battleye } else { default.battleye },
            eac: if has("eac") { self.eac } else { default.eac },
            show_debug: if has("show_debug") { self.show_debug.clone() } else { default.show_debug.clone() },
            dll_overrides: if has("dll_overrides") { self.dll_overrides.clone() } else { default.dll_overrides.clone() },
            audio: if has("audio") { self.audio.clone() } else { default.audio.clone() },
            graphics: if has("graphics") { self.graphics.clone() } else { default.graphics.clone() },
            desktop_integration: if has("desktop_integration") { self.desktop_integration } else { default.desktop_integration },
            show_crash_dialogs: if has("show_crash_dialogs") { self.show_crash_dialogs } else { default.show_crash_dialogs },
            mouse_warp_override: if has("mouse_warp_override") { self.mouse_warp_override.clone() } else { default.mouse_warp_override.clone() },
            virtual_desktop: if has("virtual_desktop") { self.virtual_desktop } else { default.virtual_desktop },
            virtual_desktop_res: if has("virtual_desktop_res") { self.virtual_desktop_res.clone() } else { default.virtual_desktop_res.clone() },
            dpi_enabled: if has("dpi_enabled") { self.dpi_enabled } else { default.dpi_enabled },
            dpi: if has("dpi") { self.dpi } else { default.dpi },
            gamemode: if has("gamemode") { self.gamemode } else { default.gamemode },
            mangohud: if has("mangohud") { self.mangohud } else { default.mangohud },
            gamescope: if has("gamescope") { self.gamescope } else { default.gamescope },
            gamescope_flags: if has("gamescope_flags") { self.gamescope_flags.clone() } else { default.gamescope_flags.clone() },
            dxvk_frame_rate: if has("dxvk_frame_rate") { self.dxvk_frame_rate } else { default.dxvk_frame_rate },
            proton_wow64: if has("proton_wow64") { self.proton_wow64 } else { default.proton_wow64 },
            proton_ntsync: if has("proton_ntsync") { self.proton_ntsync } else { default.proton_ntsync },
            wine_env_vars: if has("wine_env_vars") { self.wine_env_vars.clone() } else { default.wine_env_vars.clone() },
            overridden_fields: self.overridden_fields.clone(),
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
        assert!(deserialized.overridden_fields.is_empty());
    }

    #[test]
    fn test_merge_with_default() {
        let mut per_game = WineConfig::default();
        per_game.esync = false;
        per_game.overridden_fields = vec!["esync".to_string()];
        let default = WineConfig::default();
        let merged = per_game.merge_with_default(&default);
        assert!(!merged.esync);
        assert!(merged.fsync);
        assert!(merged.proton_ntsync);
    }

    #[test]
    fn test_game_launch_config_serde_roundtrip() {
        let mut cfg = GameLaunchConfig::default();
        cfg.exe = "/home/user/game.exe".to_string();
        cfg.args = "-foo bar".to_string();
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: GameLaunchConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.exe, "/home/user/game.exe");
        assert_eq!(deserialized.args, "-foo bar");
    }
}
