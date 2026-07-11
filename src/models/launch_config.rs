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
            virtual_desktop: false,
            virtual_desktop_res: String::new(),
            dpi_enabled: false,
            dpi: 96,
            gamemode: false,
            mangohud: false,
            gamescope: false,
            gamescope_flags: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LaunchConfig {
    pub game: GameLaunchConfig,
    pub wine: Option<WineConfig>,
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
    }

    #[test]
    fn test_wine_config_serde_roundtrip() {
        let cfg = WineConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: WineConfig = serde_json::from_str(&json).unwrap();
        assert!(deserialized.enabled);
        assert_eq!(deserialized.version, "system");
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
