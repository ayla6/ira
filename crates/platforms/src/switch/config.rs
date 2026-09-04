//! Eden (Switch emulator) directories: where its game-list icon cache and
//! configuration live.

use std::path::PathBuf;

use crate::emu_dirs;

/// Directories holding Eden's `game_list` cache — per-title `<id>.jpeg`
/// icons and `<id>.appname.txt` names — best source first. Directories may
/// be absent; readers skip those.
///
/// Several Switch emulators can be installed side by side, each with its
/// own NAND, so every detected install contributes its portable cache;
/// the shared XDG caches follow.
pub fn game_list_cache_dirs_for(executable: &str) -> Vec<PathBuf> {
    game_list_cache_dirs_in(executable, &crate::switch_detect::detected_launch_commands())
}

pub(crate) fn game_list_cache_dirs_in(executable: &str, detected: &[String]) -> Vec<PathBuf> {
    let mut exes: Vec<&str> = vec![executable];
    exes.extend(detected.iter().map(String::as_str));

    let mut roots: Vec<PathBuf> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push_root = |root: PathBuf, out: &mut Vec<PathBuf>| {
        if seen.insert(root.clone()) {
            out.push(root);
        }
    };

    // Portable layouts: each install keeps config, data and cache inside
    // its own `user` directory, matching Eden's resolution order.
    for exe in &exes {
        if exe.is_empty() || exe.starts_with("flatpak:") {
            continue;
        }
        let path = std::path::Path::new(exe);
        for root in [path.parent(), Some(path)].into_iter().flatten() {
            push_root(root.join("user").join("cache"), &mut roots);
        }
    }
    // Sandboxed installs redirect the cache under the app id.
    for exe in &exes {
        if let Some(app) = emu_dirs::flatpak_app_dir(exe) {
            push_root(app.join("cache").join("eden"), &mut roots);
        }
    }
    // Eden also reads the directories of the emulators it forked from;
    // their caches hold the same per-title files, newest fork first.
    // AppImage managers that override XDG_CACHE_HOME scatter installs
    // under ~/.cache/AppImage-Cache/<name>.
    let names = ["eden", "suyu", "citron", "sudachi", "yuzu"];
    let cache_home = emu_dirs::cache_home();
    for name in names {
        push_root(cache_home.join(name), &mut roots);
    }
    for name in names {
        push_root(cache_home.join("AppImage-Cache").join(name), &mut roots);
    }
    roots
        .into_iter()
        .map(|root| root.join("game_list"))
        .collect()
}

/// The yuzu-family NAND content directories: each holds a
/// `Registered/` folder of installed NCAs, one entry per installed
/// title. Same install layouts as the cache: portable `user` trees of
/// every detected install, then the shared XDG data dirs.
pub fn nand_registered_dirs_for(executable: &str) -> Vec<PathBuf> {
    nand_registered_dirs_in(executable, &crate::switch_detect::detected_launch_commands())
}

pub(crate) fn nand_registered_dirs_in(executable: &str, detected: &[String]) -> Vec<PathBuf> {
    let mut exes: Vec<&str> = vec![executable];
    exes.extend(detected.iter().map(String::as_str));

    let mut roots: Vec<PathBuf> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push_root = |root: PathBuf, out: &mut Vec<PathBuf>| {
        if seen.insert(root.clone()) {
            out.push(root);
        }
    };

    for exe in &exes {
        if exe.is_empty() || exe.starts_with("flatpak:") {
            continue;
        }
        let path = std::path::Path::new(exe);
        for root in [path.parent(), Some(path)].into_iter().flatten() {
            push_root(root.join("user/data/nand/System/Contents"), &mut roots);
        }
    }
    for exe in &exes {
        if let Some(app) = emu_dirs::flatpak_app_dir(exe) {
            push_root(app.join("data/eden/nand/System/Contents"), &mut roots);
        }
    }
    for name in ["eden", "suyu", "citron", "sudachi", "yuzu"] {
        push_root(
            emu_dirs::data_home().join(name).join("nand/System/Contents"),
            &mut roots,
        );
    }
    roots
        .into_iter()
        .map(|root| root.join("Registered"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_cache(root: &std::path::Path) {
        std::fs::create_dir_all(root.join("game_list")).unwrap();
    }

    #[test]
    fn test_cache_dirs_probe_portable_before_xdg() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("eden.AppImage");
        std::fs::write(&exe, b"").unwrap();
        let portable_cache = tmp.path().join("user/cache");
        write_cache(&portable_cache);

        let dirs = game_list_cache_dirs_in(&exe.to_string_lossy(), &[]);
        assert_eq!(dirs.first(), Some(&portable_cache.join("game_list")));
        // The shared XDG caches of the forks still follow the portable one.
        assert!(dirs.contains(&emu_dirs::cache_home().join("eden/game_list")));
    }

    #[test]
    fn test_cache_dirs_include_every_detected_install() {
        let tmp = tempfile::tempdir().unwrap();
        let other = tmp.path().join("other");
        std::fs::write(&other, b"").unwrap();

        // A second emulator's portable cache is probed even though another
        // executable is configured.
        let dirs = game_list_cache_dirs_in(
            "/configured/eden.AppImage",
            &[other.to_string_lossy().into_owned()],
        );
        assert!(dirs.contains(&tmp.path().join("user/cache/game_list")));
    }

    #[test]
    fn test_cache_dirs_flatpak_root_first_for_flatpak_exe() {
        let executable = "flatpak:dev.eden_emu.eden";
        let dirs = game_list_cache_dirs_in(executable, &[]);
        assert_eq!(
            dirs.first(),
            Some(&emu_dirs::home_dir()
                .join(".var/app/dev.eden_emu.eden/cache/eden/game_list"))
        );
    }

    #[test]
    fn test_cache_dirs_cover_xdg_and_appimage_cache() {
        let dirs = game_list_cache_dirs_in("", &[]);
        let cache_home = emu_dirs::cache_home();
        assert!(dirs.contains(&cache_home.join("eden/game_list")));
        assert!(dirs.contains(&cache_home.join("AppImage-Cache/eden/game_list")));
    }
}




/// Game directories configured in every yuzu-family install's own
/// settings (`qt-config.ini` `[Paths] gamedirs`), portable `user/config`
/// trees included. The emulator library — not Ira's ROM roots — is the
/// source of truth for where these files live.
fn rom_game_dirs(executable: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for cfg_path in qt_config_paths(executable) {
        if let Ok(text) = std::fs::read_to_string(&cfg_path) {
            for dir in rom_game_dirs_in(&text) {
                if !dirs.contains(&dir) {
                    dirs.push(dir);
                }
            }
        }
    }
    dirs
}

/// The usable game directories in one qt-config's text: existing folders
/// from `gamedirs\<n>\path` keys — the `deep_scan`/`default` siblings and
/// the virtual SDMC/UserNAND/SysNAND entries never appear here.
fn rom_game_dirs_in(text: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for (section, key, value) in crate::qt_ini::QtIni::parse(text).entries {
        // QtIni lowercases sections and keys; real files write `Paths`.
        if section != "paths" || !key.starts_with("gamedirs\\") || !key.ends_with("\\path") {
            continue;
        }
        let dir = PathBuf::from(crate::qt_ini::percent_decode(&value));
        if dir.is_dir() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// Resolves a Switch ROM file name through every detected install's game
/// directories. Emulator-library titles are stored relative to those
/// directories, which exist whether or not Ira's own ROM roots are
/// configured. Absolute inputs pass through unhandled — the caller keeps
/// its own absolute-path branch.
pub fn resolve_rom(path: &str, executable: &str) -> Option<std::path::PathBuf> {
    resolve_rom_in(rom_game_dirs(executable), path)
}

fn resolve_rom_in(dirs: Vec<PathBuf>, path: &str) -> Option<std::path::PathBuf> {
    if path.is_empty() {
        return None;
    }
    let relative = std::path::Path::new(path);
    if relative.is_absolute() {
        return None;
    }
    dirs.into_iter()
        .map(|dir| dir.join(relative))
        .find(|candidate| candidate.is_file())
}

/// `qt-config.ini` of every yuzu-family install: each fork's shared XDG
/// config, flatpak app dirs, and portable `user/config` trees.
fn qt_config_paths(executable: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let push = |path: PathBuf, out: &mut Vec<PathBuf>| {
        if !out.contains(&path) {
            out.push(path);
        }
    };
    let exes = std::iter::once(executable.to_string())
        .chain(crate::switch_detect::detected_launch_commands());
    for exe in exes {
        if exe.is_empty() || exe.starts_with("flatpak:") {
            continue;
        }
        let path = std::path::Path::new(&exe);
        for root in [path.parent(), Some(path)].into_iter().flatten() {
            push(root.join("user").join("config").join("qt-config.ini"), &mut out);
        }
        if let Some(app) = crate::emu_dirs::flatpak_app_dir(&exe) {
            push(app.join("config").join("qt-config.ini"), &mut out);
        }
    }
    // The forks this codebase recognizes; Eden reads their directories too.
    for name in ["eden", "suyu", "citron", "sudachi", "yuzu"] {
        push(
            crate::emu_dirs::config_home().join(name).join("qt-config.ini"),
            &mut out,
        );
    }
    out
}

#[cfg(test)]
mod xci_rom_tests {
    use super::*;

    fn ini_with_dirs(dirs: &[&str]) -> String {
        let mut out = String::from("[Paths]\n");
        out.push_str("gamedirs\\size=5\n");
        for (i, dir) in dirs.iter().enumerate() {
            out.push_str(&format!("gamedirs\\{}\\path={}\n", i + 1, dir));
            out.push_str(&format!("gamedirs\\{}\\deep_scan\\default=true\n", i + 1));
            out.push_str(&format!("gamedirs\\{}\\deep_scan=false\n", i + 1));
        }
        out.push_str("gamedirs\\6\\path=SDMC\n");
        out
    }

    #[test]
    fn test_rom_game_dirs_in_reads_gamedir_paths_only() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("switch dumps");
        std::fs::create_dir_all(&real).unwrap();
        let text = ini_with_dirs(&[&real.to_string_lossy(), "/gone/nsp", "SDMC"]);
        let dirs = rom_game_dirs_in(&text);
        assert_eq!(dirs, vec![real]);
    }

    #[test]
    fn test_resolve_rom_in_finds_file_across_dirs_and_skips_absolute() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(b.join("Game [0100000000010000].nsp"), b"x").unwrap();

        let dirs = vec![a.clone(), b.clone()];
        assert_eq!(
            resolve_rom_in(dirs.clone(), "Game [0100000000010000].nsp"),
            Some(b.join("Game [0100000000010000].nsp"))
        );
        // Absolute paths and empties are the caller's business.
        assert_eq!(resolve_rom_in(dirs.clone(), "/abs/game.nsp"), None);
        assert_eq!(resolve_rom_in(dirs, ""), None);
    }
}
