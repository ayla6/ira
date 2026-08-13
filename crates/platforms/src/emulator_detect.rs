use crate::emulator_systems;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct DetectedEmulator {
    pub display_name: String,
    pub launch_command: String,
}

pub fn detect_emulator_choices(
    native_names: &[&str],
    flatpak_apps: &[(&str, &str)],
    display_name: &str,
) -> Vec<DetectedEmulator> {
    let mut choices = Vec::new();
    choices.extend(detect_native_all(native_names, display_name));
    for (app_id, app_name) in flatpak_apps {
        if let Some(flatpak) = detect_flatpak(app_id, app_name) {
            choices.push(flatpak);
        }
    }
    choices
}

#[derive(Clone)]
pub struct RaCore {
    pub display_name: String,
    pub path: String,
}

pub fn is_flatpak_installed(flatpak_id: &str) -> bool {
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

pub fn detect_native(names: &[&str], display_name: &str) -> Option<DetectedEmulator> {
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

fn detect_native_all(names: &[&str], display_name: &str) -> Vec<DetectedEmulator> {
    let mut choices = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for name in names {
        if let Some(path) = which(name) {
            if seen.insert(path.clone()) {
                choices.push(DetectedEmulator {
                    display_name: format!("{} (native)", display_name),
                    launch_command: path,
                });
            }
        }
    }
    choices
}

pub fn detect_emulators(console: &str) -> Vec<DetectedEmulator> {
    let def = ira_models::find_console(console);
    let mut choices = Vec::new();
    if let Some(d) = def {
        let mut native_names = d.binary_names.to_vec();
        native_names.extend_from_slice(emulator_systems::native_names(console));
        let mut flatpak_apps = emulator_systems::flatpak_apps(console).to_vec();
        if !d.flatpak_id.is_empty() {
            flatpak_apps.push((d.flatpak_id, d.emu_display_name));
        }
        choices.extend(detect_emulator_choices(
            &native_names,
            &flatpak_apps,
            d.emu_display_name,
        ));
    }
    if emulator_systems::has_retroarch_cores(console) {
        choices.extend(detect_emulator_choices(
            &["retroarch"],
            &[("org.libretro.RetroArch", "RetroArch")],
            "RetroArch",
        ));
    }
    choices
}

pub fn detect_ra_cores_for_console(console: &str) -> Vec<RaCore> {
    if !emulator_systems::has_retroarch_cores(console) {
        return Vec::new();
    }

    let names = emulator_systems::core_names(console);
    if names.is_empty() {
        return detect_ra_cores();
    }

    detect_ra_cores()
        .into_iter()
        .filter(|core| {
            let id = core_id(&core.path);
            names
                .iter()
                .any(|name| id == *name || id.starts_with(&format!("{name}_")))
        })
        .collect()
}

pub fn supports_retroarch_cores(console: &str) -> bool {
    emulator_systems::has_retroarch_cores(console)
}

fn core_id(path: &str) -> &str {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| {
            name.strip_suffix("_libretro.so")
                .or_else(|| name.strip_suffix(".so"))
        })
        .unwrap_or(path)
}

pub fn is_retroarch(launch_command: &str) -> bool {
    launch_command.contains("retroarch") || launch_command == "flatpak:org.libretro.RetroArch"
}

fn ra_core_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(
            PathBuf::from(&home).join(".var/app/org.libretro.RetroArch/config/retroarch/cores"),
        );
        dirs.push(PathBuf::from(&home).join(".config/retroarch/cores"));
        dirs.push(PathBuf::from(&home).join(".local/share/libretro/cores"));
    }
    dirs.push(PathBuf::from("/usr/lib/libretro"));
    dirs.push(PathBuf::from("/usr/lib64/libretro"));
    dirs
}

fn core_display_name(filename: &str) -> String {
    let stem = filename
        .strip_suffix("_libretro.so")
        .or_else(|| filename.strip_suffix(".so"))
        .unwrap_or(filename);
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

pub fn resolve_ra_core_for_console(console: &str, configured: &str) -> Option<String> {
    if !configured.is_empty() && std::path::Path::new(configured).is_file() {
        return Some(configured.to_string());
    }
    detect_ra_cores_for_console(console)
        .first()
        .map(|core| core.path.clone())
}

pub fn build_launch_command(
    exe: &str,
    rom_path: &str,
    ra_core: &str,
    fullscreen: bool,
    fullscreen_flag: &str,
) -> Vec<String> {
    build_launch_command_with_filesystem(exe, rom_path, ra_core, fullscreen, fullscreen_flag, None)
}

pub fn build_launch_command_with_filesystem(
    exe: &str,
    rom_path: &str,
    ra_core: &str,
    fullscreen: bool,
    fullscreen_flag: &str,
    filesystem_root: Option<&std::path::Path>,
) -> Vec<String> {
    let mut cmd = if let Some(flatpak_id) = exe.strip_prefix("flatpak:") {
        let mut command = flatpak_command_prefix(flatpak_id, filesystem_root);
        if is_retroarch(exe) && !ra_core.is_empty() {
            command.push("-L".to_string());
            command.push(ra_core.to_string());
        }
        command
    } else if is_retroarch(exe) && !ra_core.is_empty() {
        vec![exe.to_string(), "-L".to_string(), ra_core.to_string()]
    } else {
        vec![exe.to_string()]
    };
    if fullscreen && !fullscreen_flag.is_empty() {
        cmd.push(fullscreen_flag.to_string());
    }
    cmd.push(rom_path.to_string());
    cmd
}

pub fn build_command_with_filesystem(
    exe: &str,
    args: &[String],
    filesystem_root: Option<&std::path::Path>,
) -> Vec<String> {
    if let Some(app_id) = exe.strip_prefix("flatpak:") {
        let mut cmd = flatpak_command_prefix(app_id, filesystem_root);
        cmd.extend(args.iter().cloned());
        cmd
    } else {
        let mut cmd = vec![exe.to_string()];
        cmd.extend(args.iter().cloned());
        cmd
    }
}

fn flatpak_command_prefix(app_id: &str, filesystem_root: Option<&std::path::Path>) -> Vec<String> {
    let mut cmd = if std::env::var_os("FLATPAK_ID").is_some() {
        vec![
            "flatpak-spawn".to_string(),
            "--host".to_string(),
            "flatpak".to_string(),
            "run".to_string(),
        ]
    } else {
        vec!["flatpak".to_string(), "run".to_string()]
    };
    if let Some(root) = filesystem_root.filter(|p| !p.as_os_str().is_empty()) {
        cmd.push(format!("--filesystem={}:ro", root.display()));
    }
    cmd.push(app_id.to_string());
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
        let cmd = build_launch_command(
            "/usr/bin/duckstation-qt",
            "/games/rom.bin",
            "",
            false,
            "-fullscreen",
        );
        assert_eq!(cmd, vec!["/usr/bin/duckstation-qt", "/games/rom.bin"]);
    }

    #[test]
    fn test_build_launch_command_flatpak() {
        let cmd = build_launch_command(
            "flatpak:org.duckstation.DuckStation",
            "/games/rom.bin",
            "",
            false,
            "-fullscreen",
        );
        assert_eq!(
            cmd,
            vec![
                "flatpak",
                "run",
                "org.duckstation.DuckStation",
                "/games/rom.bin"
            ]
        );
    }

    #[test]
    fn test_build_launch_command_flatpak_with_filesystem() {
        let cmd = build_launch_command_with_filesystem(
            "flatpak:org.ppsspp.PPSSPP",
            "/games/rom.bin",
            "",
            false,
            "--fullscreen",
            Some(std::path::Path::new("/games")),
        );
        assert_eq!(
            cmd,
            vec![
                "flatpak",
                "run",
                "--filesystem=/games:ro",
                "org.ppsspp.PPSSPP",
                "/games/rom.bin"
            ]
        );
    }

    #[test]
    fn test_build_launch_command_retroarch_native_with_core() {
        let cmd = build_launch_command(
            "/usr/bin/retroarch",
            "/games/rom.bin",
            "/usr/lib/libretro/mednafen_psx_hw_libretro.so",
            false,
            "--fullscreen",
        );
        assert_eq!(
            cmd,
            vec![
                "/usr/bin/retroarch",
                "-L",
                "/usr/lib/libretro/mednafen_psx_hw_libretro.so",
                "/games/rom.bin"
            ]
        );
    }

    #[test]
    fn test_build_launch_command_fullscreen() {
        let cmd = build_launch_command(
            "/usr/bin/duckstation-qt",
            "/games/rom.bin",
            "",
            true,
            "-fullscreen",
        );
        assert_eq!(
            cmd,
            vec!["/usr/bin/duckstation-qt", "-fullscreen", "/games/rom.bin"]
        );
    }

    #[test]
    fn test_build_launch_command_pcsx2_fullscreen() {
        let cmd = build_launch_command(
            "/usr/bin/pcsx2-qt",
            "/games/rom.iso",
            "",
            true,
            "-fullscreen",
        );
        assert_eq!(
            cmd,
            vec!["/usr/bin/pcsx2-qt", "-fullscreen", "/games/rom.iso"]
        );
    }

    #[test]
    fn test_build_launch_command_retroarch_flatpak_with_core() {
        let cmd = build_launch_command(
            "flatpak:org.libretro.RetroArch",
            "/games/rom.bin",
            "/path/to/core.so",
            false,
            "--fullscreen",
        );
        assert_eq!(
            cmd,
            vec![
                "flatpak",
                "run",
                "org.libretro.RetroArch",
                "-L",
                "/path/to/core.so",
                "/games/rom.bin"
            ]
        );
    }

    #[test]
    fn test_build_launch_command_retroarch_no_core() {
        let cmd = build_launch_command(
            "/usr/bin/retroarch",
            "/games/rom.bin",
            "",
            false,
            "--fullscreen",
        );
        assert_eq!(cmd, vec!["/usr/bin/retroarch", "/games/rom.bin"]);
    }

    #[test]
    fn test_core_display_name() {
        assert_eq!(
            core_display_name("mednafen_psx_hw_libretro.so"),
            "mednafen psx hw"
        );
        assert_eq!(core_display_name("pcsx2_libretro.so"), "pcsx2");
        assert_eq!(core_display_name("ppsspp_libretro.so"), "ppsspp");
    }

    #[test]
    fn test_core_id_and_system_mapping() {
        assert_eq!(
            core_id("/usr/lib/libretro/mednafen_psx_hw_libretro.so"),
            "mednafen_psx_hw"
        );
        assert!(emulator_systems::core_names("gba").contains(&"mgba"));
        assert!(!emulator_systems::has_retroarch_cores("switch"));
    }

    #[test]
    fn test_resolve_ra_core_preserves_existing_custom_core() {
        let core = tempfile::NamedTempFile::new().unwrap();
        let path = core.path().to_string_lossy();
        assert_eq!(
            resolve_ra_core_for_console("switch", &path),
            Some(path.into_owned())
        );
    }

    #[test]
    fn test_resolve_ra_core_returns_none_without_supported_cores() {
        assert_eq!(resolve_ra_core_for_console("switch", ""), None);
    }
}
