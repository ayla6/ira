use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct DetectedEmulator {
    pub display_name: String,
    pub launch_command: String,
}

#[derive(Clone)]
pub struct RaCore {
    pub display_name: String,
    pub path: String,
}

fn is_flatpak_installed(flatpak_id: &str) -> bool {
    Command::new("flatpak")
        .args(["info", flatpak_id])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn which(name: &str) -> Option<String> {
    which::which(name)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

fn detect_flatpak(flatpak_id: &str, display_name: &str) -> Option<DetectedEmulator> {
    if is_flatpak_installed(flatpak_id) {
        Some(DetectedEmulator {
            display_name: format!("{} (Flatpak)", display_name),
            launch_command: format!("flatpak:{}", flatpak_id),
        })
    } else {
        None
    }
}

fn detect_native(names: &[&str], display_name: &str) -> Option<DetectedEmulator> {
    for name in names {
        if let Some(path) = which(name) {
            return Some(DetectedEmulator {
                display_name: format!("{} (native)", display_name),
                launch_command: path,
            });
        }
    }
    None
}

pub fn detect_emulators(console: &str) -> Vec<DetectedEmulator> {
    let mut result = Vec::new();
    if let Some(def) = ira_models::find_console(console) {
        if let Some(e) = detect_native(def.binary_names, def.emu_display_name) {
            result.push(e);
        }
        if let Some(e) = detect_flatpak(def.flatpak_id, def.emu_display_name) {
            result.push(e);
        }
    }
    if let Some(e) = detect_native(&["retroarch"], "RetroArch") {
        result.push(e);
    }
    if let Some(e) = detect_flatpak("org.libretro.RetroArch", "RetroArch") {
        result.push(e);
    }
    result
}

pub fn is_retroarch(launch_command: &str) -> bool {
    launch_command.contains("retroarch") || launch_command == "flatpak:org.libretro.RetroArch"
}

fn ra_core_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(PathBuf::from(&home).join(".var/app/org.libretro.RetroArch/config/retroarch/cores"));
        dirs.push(PathBuf::from(&home).join(".config/retroarch/cores"));
        dirs.push(PathBuf::from(&home).join(".local/share/libretro/cores"));
    }
    dirs.push(PathBuf::from("/usr/lib/libretro"));
    dirs.push(PathBuf::from("/usr/lib64/libretro"));
    dirs
}

fn core_display_name(filename: &str) -> String {
    let stem = filename.strip_suffix("_libretro.so").or_else(|| filename.strip_suffix(".so")).unwrap_or(filename);
    stem.replace('_', " ")
}

pub fn detect_ra_cores() -> Vec<RaCore> {
    let mut cores = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for dir in ra_core_dirs() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("so") {
                    let path_str = path.to_string_lossy().into_owned();
                    if seen.insert(path_str.clone()) {
                        let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                        cores.push(RaCore {
                            display_name: core_display_name(filename),
                            path: path_str,
                        });
                    }
                }
            }
        }
    }
    cores.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    cores
}

pub fn build_launch_command(
    exe: &str,
    rom_path: &str,
    ra_core: &str,
    fullscreen: bool,
    fullscreen_flag: &str,
) -> Vec<String> {
    let mut cmd = if let Some(flatpak_id) = exe.strip_prefix("flatpak:") {
        if is_retroarch(exe) && !ra_core.is_empty() {
            vec![
                "flatpak".to_string(),
                "run".to_string(),
                flatpak_id.to_string(),
                "-L".to_string(),
                ra_core.to_string(),
            ]
        } else {
            vec![
                "flatpak".to_string(),
                "run".to_string(),
                flatpak_id.to_string(),
            ]
        }
    } else if is_retroarch(exe) && !ra_core.is_empty() {
        vec![exe.to_string(), "-L".to_string(), ra_core.to_string()]
    } else {
        vec![exe.to_string()]
    };
    if fullscreen {
        cmd.push(fullscreen_flag.to_string());
    }
    cmd.push(rom_path.to_string());
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_retroarch_native() {
        assert!(is_retroarch("/usr/bin/retroarch"));
    }

    #[test]
    fn test_is_retroarch_flatpak() {
        assert!(is_retroarch("flatpak:org.libretro.RetroArch"));
    }

    #[test]
    fn test_is_retroarch_not() {
        assert!(!is_retroarch("/usr/bin/duckstation-qt"));
        assert!(!is_retroarch("flatpak:org.duckstation.DuckStation"));
    }

    #[test]
    fn test_build_launch_command_native() {
        let cmd = build_launch_command("/usr/bin/duckstation-qt", "/games/rom.bin", "", false, "--fullscreen");
        assert_eq!(cmd, vec!["/usr/bin/duckstation-qt", "/games/rom.bin"]);
    }

    #[test]
    fn test_build_launch_command_flatpak() {
        let cmd = build_launch_command("flatpak:org.duckstation.DuckStation", "/games/rom.bin", "", false, "--fullscreen");
        assert_eq!(cmd, vec!["flatpak", "run", "org.duckstation.DuckStation", "/games/rom.bin"]);
    }

    #[test]
    fn test_build_launch_command_retroarch_native_with_core() {
        let cmd = build_launch_command("/usr/bin/retroarch", "/games/rom.bin", "/usr/lib/libretro/mednafen_psx_hw_libretro.so", false, "--fullscreen");
        assert_eq!(cmd, vec!["/usr/bin/retroarch", "-L", "/usr/lib/libretro/mednafen_psx_hw_libretro.so", "/games/rom.bin"]);
    }

    #[test]
    fn test_build_launch_command_fullscreen() {
        let cmd = build_launch_command("/usr/bin/duckstation-qt", "/games/rom.bin", "", true, "--fullscreen");
        assert_eq!(cmd, vec!["/usr/bin/duckstation-qt", "--fullscreen", "/games/rom.bin"]);
    }

    #[test]
    fn test_build_launch_command_pcsx2_fullscreen() {
        let cmd = build_launch_command("/usr/bin/pcsx2-qt", "/games/rom.iso", "", true, "-fullscreen");
        assert_eq!(cmd, vec!["/usr/bin/pcsx2-qt", "-fullscreen", "/games/rom.iso"]);
    }

    #[test]
    fn test_build_launch_command_retroarch_flatpak_with_core() {
        let cmd = build_launch_command("flatpak:org.libretro.RetroArch", "/games/rom.bin", "/path/to/core.so", false, "--fullscreen");
        assert_eq!(cmd, vec!["flatpak", "run", "org.libretro.RetroArch", "-L", "/path/to/core.so", "/games/rom.bin"]);
    }

    #[test]
    fn test_build_launch_command_retroarch_no_core() {
        let cmd = build_launch_command("/usr/bin/retroarch", "/games/rom.bin", "", false, "--fullscreen");
        assert_eq!(cmd, vec!["/usr/bin/retroarch", "/games/rom.bin"]);
    }

    #[test]
    fn test_core_display_name() {
        assert_eq!(core_display_name("mednafen_psx_hw_libretro.so"), "mednafen psx hw");
        assert_eq!(core_display_name("pcsx2_libretro.so"), "pcsx2");
        assert_eq!(core_display_name("ppsspp_libretro.so"), "ppsspp");
    }
}
