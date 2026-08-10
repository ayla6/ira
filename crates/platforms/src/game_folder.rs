use std::path::{Path, PathBuf};

/// Try to detect the game folder from the executable path, steamcmd
/// install_dir, and game title. Returns the detected folder or None.
///
/// Strategies (tried in order):
/// 1. If exe is inside `default_game_folder`, use the first subdirectory.
/// 2. Walk up from exe looking for game root markers.
/// 3. Match `install_dir` in `default_game_folder`.
/// 4. Match `title` (normalized) in `default_game_folder`.
pub fn detect_game_folder(
    exe_path: &str,
    default_game_folder: &str,
    install_dir: &str,
    title: &str,
) -> Option<PathBuf> {
    if !exe_path.is_empty() {
        if let Some(folder) = detect_from_exe(exe_path, default_game_folder) {
            return Some(folder);
        }
    }
    if !install_dir.is_empty() && !default_game_folder.is_empty() {
        let candidate = Path::new(default_game_folder).join(install_dir);
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    if !title.is_empty() && !default_game_folder.is_empty() {
        if let Some(folder) = match_by_title(default_game_folder, title) {
            return Some(folder);
        }
    }
    None
}

/// If the exe is inside `default_game_folder`, the game folder is the
/// first subdirectory after `default_game_folder`.
/// Otherwise, walk up from the exe looking for game root markers.
fn detect_from_exe(exe_path: &str, default_game_folder: &str) -> Option<PathBuf> {
    let exe = Path::new(exe_path);
    let default_dir = Path::new(default_game_folder);

    if !default_game_folder.is_empty() && exe.starts_with(default_dir) {
        let rel = exe.strip_prefix(default_dir).ok()?;
        let first = rel.components().next()?;
        let folder = default_dir.join(first);
        if folder.is_dir() {
            return Some(folder);
        }
    }

    walk_up_for_game_root(exe)
}

/// Walk up from the exe path looking for a directory that contains
/// game root markers. Markers: `steam_appid.txt`, `steam_settings/`,
/// `_CommonRedist/`, or a `.dll` file directly in the directory.
fn walk_up_for_game_root(exe: &Path) -> Option<PathBuf> {
    let mut current = exe.parent()?;
    for _ in 0..10 {
        if is_game_root(current) {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
    None
}

fn is_game_root(dir: &Path) -> bool {
    if dir.join("steam_appid.txt").exists() {
        return true;
    }
    if dir.join("steam_settings").is_dir() {
        return true;
    }
    if dir.join("_CommonRedist").is_dir() {
        return true;
    }
    if std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|e| {
            e.path()
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("dll"))
        })
    }) {
        return true;
    }
    false
}

/// Match a game title against directories in `default_game_folder`.
/// Tries exact (case-insensitive), normalized, and acronym matching.
fn match_by_title(default_game_folder: &str, title: &str) -> Option<PathBuf> {
    let base = Path::new(default_game_folder);
    let entries = std::fs::read_dir(base).ok()?;
    let dirs: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .collect();

    let title_lower = title.to_lowercase();
    if let Some(dir) = dirs.iter().find(|d| d.to_lowercase() == title_lower) {
        return Some(base.join(dir));
    }

    let title_norm = normalize(title);
    if !title_norm.is_empty() {
        if let Some(dir) = dirs.iter().find(|d| normalize(d) == title_norm) {
            return Some(base.join(dir));
        }
    }

    let acronym = make_acronym(title);
    if acronym.len() >= 2 {
        if let Some(dir) = dirs.iter().find(|d| normalize(d) == acronym) {
            return Some(base.join(dir));
        }
    }

    None
}

/// Normalize a string: lowercase, keep only alphanumeric, remove common suffixes.
fn normalize(s: &str) -> String {
    let lower = s.to_lowercase();
    let trimmed = lower
        .trim_end_matches(": deluxe edition")
        .trim_end_matches(": game of the year edition")
        .trim_end_matches("(gog.com)")
        .trim_end_matches("(steam)");
    trimmed.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Create an acronym from a title's words (e.g. "Metaphor ReFantazio" → "mr").
fn make_acronym(title: &str) -> String {
    title
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .filter(|c| c.is_alphabetic())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_detect_from_exe_in_default_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let games = tmp.path().join("games");
        let game_dir = games.join("MyGame");
        fs::create_dir_all(game_dir.join("bin")).unwrap();
        let exe = game_dir.join("bin").join("game.exe");
        fs::write(&exe, b"x").unwrap();

        let detected = detect_from_exe(&exe.to_string_lossy(), &games.to_string_lossy());
        assert_eq!(detected, Some(game_dir));
    }

    #[test]
    fn test_detect_from_exe_with_steam_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join("MyGame");
        fs::create_dir_all(game_dir.join("bin")).unwrap();
        fs::write(game_dir.join("steam_appid.txt"), "123").unwrap();
        let exe = game_dir.join("bin").join("game.exe");
        fs::write(&exe, b"x").unwrap();

        let detected = detect_from_exe(&exe.to_string_lossy(), "");
        assert_eq!(detected, Some(game_dir));
    }

    #[test]
    fn test_detect_from_exe_with_dll() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join("MyGame");
        fs::create_dir_all(game_dir.join("sub")).unwrap();
        fs::write(game_dir.join("steam_api64.dll"), b"x").unwrap();
        let exe = game_dir.join("sub").join("game.exe");
        fs::write(&exe, b"x").unwrap();

        let detected = detect_from_exe(&exe.to_string_lossy(), "");
        assert_eq!(detected, Some(game_dir));
    }

    #[test]
    fn test_match_by_title_exact() {
        let tmp = tempfile::tempdir().unwrap();
        let games = tmp.path().join("games");
        fs::create_dir_all(games.join("Hotline Miami")).unwrap();

        let detected = match_by_title(&games.to_string_lossy(), "Hotline Miami");
        assert_eq!(detected, Some(games.join("Hotline Miami")));
    }

    #[test]
    fn test_match_by_title_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let games = tmp.path().join("games");
        fs::create_dir_all(games.join("hotline miami")).unwrap();

        let detected = match_by_title(&games.to_string_lossy(), "Hotline Miami");
        assert_eq!(detected, Some(games.join("hotline miami")));
    }

    #[test]
    fn test_match_by_title_normalized() {
        let tmp = tempfile::tempdir().unwrap();
        let games = tmp.path().join("games");
        fs::create_dir_all(games.join("DanganronpaTriggerHappyHavoc")).unwrap();

        let detected = match_by_title(&games.to_string_lossy(), "Danganronpa: Trigger Happy Havoc");
        assert_eq!(detected, Some(games.join("DanganronpaTriggerHappyHavoc")));
    }

    #[test]
    fn test_match_by_title_no_match() {
        let tmp = tempfile::tempdir().unwrap();
        let games = tmp.path().join("games");
        fs::create_dir_all(games.join("OtherGame")).unwrap();

        let detected = match_by_title(&games.to_string_lossy(), "MyGame");
        assert!(detected.is_none());
    }

    #[test]
    fn test_detect_game_folder_with_install_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let games = tmp.path().join("games");
        fs::create_dir_all(games.join("P5R")).unwrap();

        let detected = detect_game_folder("", &games.to_string_lossy(), "P5R", "");
        assert_eq!(detected, Some(games.join("P5R")));
    }

    #[test]
    fn test_detect_game_folder_empty() {
        assert!(detect_game_folder("", "", "", "").is_none());
    }

    #[test]
    fn test_normalize_strips_suffixes() {
        assert_eq!(normalize("My Game: Deluxe Edition"), "mygame");
        assert_eq!(normalize("Game (GOG.com)"), "game");
    }

    #[test]
    fn test_make_acronym() {
        assert_eq!(make_acronym("Metaphor ReFantazio"), "mr");
        assert_eq!(make_acronym("Persona Royal"), "pr");
        assert_eq!(make_acronym("Hotline Miami"), "hm");
    }
}
