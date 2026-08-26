use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const CEMU_FLATPAK_ID: &str = "info.cemu.Cemu";

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn app_path_for(executable: &str, suffix: &str) -> PathBuf {
    let base = executable
        .strip_prefix("flatpak:")
        .map(|id| home_dir().join(".var").join("app").join(id))
        .unwrap_or_default();
    if !base.as_os_str().is_empty() {
        return base.join(suffix);
    }
    PathBuf::from(suffix)
}

fn portable_dir_for(executable: &str) -> Option<PathBuf> {
    if executable.is_empty() || executable.starts_with("flatpak:") {
        return None;
    }
    let path = Path::new(executable);
    [path.parent(), Some(path)]
        .into_iter()
        .flatten()
        .map(|root| root.join("portable"))
        .find(|path| path.is_dir())
}

pub fn cemu_config_dir() -> PathBuf {
    cemu_config_dir_for("")
}

pub fn cemu_config_dir_for(executable: &str) -> PathBuf {
    if executable.starts_with("flatpak:") {
        return app_path_for(executable, "config/Cemu");
    }
    if let Some(portable) = portable_dir_for(executable) {
        return portable;
    }
    xdg::BaseDirectories::new()
        .get_config_home()
        .unwrap_or_else(|| home_dir().join(".config"))
        .join("Cemu")
}

pub fn cemu_data_dir_for(executable: &str) -> PathBuf {
    if executable.starts_with("flatpak:") {
        return app_path_for(executable, "data/Cemu");
    }
    if let Some(portable) = portable_dir_for(executable) {
        return portable;
    }
    xdg::BaseDirectories::new()
        .get_data_home()
        .unwrap_or_else(|| home_dir().join(".local").join("share"))
        .join("Cemu")
}

fn settings_path_for(executable: &str) -> PathBuf {
    cemu_config_dir_for(executable).join("settings.xml")
}

fn setting_text<'a>(node: roxmltree::Node<'a, 'a>, name: &str) -> Option<String> {
    node.children()
        .find(|child| child.has_tag_name(name))
        .and_then(|child| child.text())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn with_settings<T>(executable: &str, f: impl FnOnce(roxmltree::Document<'_>) -> T) -> Option<T> {
    let data = std::fs::read_to_string(settings_path_for(executable)).ok()?;
    let document = roxmltree::Document::parse(&data).ok()?;
    Some(f(document))
}

pub fn mlc_path_for(executable: &str) -> PathBuf {
    let default = cemu_data_dir_for(executable).join("mlc01");
    with_settings(executable, |document| {
        document
            .descendants()
            .find(|node| node.has_tag_name("content"))
            .and_then(|content| setting_text(content, "mlc_path"))
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| default.clone())
    })
    .unwrap_or(default)
}

pub fn configured_game_paths_for(executable: &str) -> Vec<PathBuf> {
    with_settings(executable, |document| {
        document
            .descendants()
            .find(|node| node.has_tag_name("GamePaths"))
            .into_iter()
            .flat_map(|paths| paths.children().filter(|node| node.has_tag_name("Entry")))
            .filter_map(|entry| entry.text())
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
            .collect()
    })
    .unwrap_or_default()
}

#[derive(Debug, Clone)]
pub struct CemuGame {
    pub title_id: String,
    pub title: String,
    pub game_path: PathBuf,
    pub icon_path: PathBuf,
}

fn parse_title_xml(path: &Path) -> Option<(String, String)> {
    let data = std::fs::read_to_string(path).ok()?;
    let document = roxmltree::Document::parse(&data).ok()?;
    let root = document.root_element();
    let title_id = root
        .children()
        .find(|node| node.has_tag_name("title_id"))
        .and_then(|node| node.text())
        .and_then(|id| u64::from_str_radix(id.trim().trim_start_matches("0x"), 16).ok())?;
    let title = root
        .children()
        .find(|node| node.has_tag_name("longname_en"))
        .or_else(|| {
            root.children()
                .find(|node| node.has_tag_name("shortname_en"))
        })
        .and_then(|node| node.text())
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    Some((format!("{title_id:016x}"), title))
}

fn scan_title(path: &Path) -> Option<CemuGame> {
    let meta_path = path.join("meta/meta.xml");
    let app_path = path.join("code/app.xml");
    let (title_id, title) = parse_title_xml(&meta_path).or_else(|| parse_title_xml(&app_path))?;
    Some(CemuGame {
        title_id,
        title,
        game_path: path.to_path_buf(),
        icon_path: path.join("meta/iconTex.tga"),
    })
}

fn scan_dir(path: &Path, depth: u32, results: &mut Vec<CemuGame>) {
    if depth > 6 {
        return;
    }
    if path.join("meta/meta.xml").is_file() || path.join("code/app.xml").is_file() {
        if let Some(game) = scan_title(path) {
            results.push(game);
        }
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries
        .flatten()
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
    {
        scan_dir(&entry.path(), depth + 1, results);
    }
}

pub fn discover_games() -> Vec<CemuGame> {
    discover_games_for_executable("")
}

pub fn discover_games_for_executable(executable: &str) -> Vec<CemuGame> {
    // A missing native executable must not stop discovery: game paths live
    // in Cemu's own settings (portable or XDG), which is readable even when
    // the launch command is stale (renamed AppImage, moved binary…).
    if !executable.is_empty()
        && !executable.starts_with("flatpak:")
        && executable.contains('/')
        && !Path::new(executable).is_file()
    {
        eprintln!(
            "Cemu executable not found, scanning its settings anyway: {}",
            executable
        );
    }
    let mlc = mlc_path_for(executable);
    let mut roots = configured_game_paths_for(executable);
    roots.extend([
        mlc.join("usr/title"),
        mlc.join("sys/title/00050010"),
        mlc.join("sys/title/00050030"),
    ]);

    let mut games = Vec::new();
    for root in roots {
        scan_dir(&root, 0, &mut games);
    }

    // The same title can be reachable from several roots (a configured game
    // path and the mlc storage); a title ID uniquely identifies a Wii U
    // title, so dedupe on it instead of the path to avoid duplicating the
    // game in the library. Configured game paths are scanned first, so the
    // user-chosen location wins.
    let mut seen = HashSet::new();
    games.retain(|game| seen.insert(game.title_id.clone()));
    games
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_title_xml() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("meta.xml");
        std::fs::write(&path, r#"<menu><title_id>0005000010101D00</title_id><longname_en>Test Game</longname_en></menu>"#).unwrap();
        assert_eq!(
            parse_title_xml(&path).unwrap(),
            ("0005000010101d00".to_string(), "Test Game".to_string())
        );
    }

    /// A stale launch command (renamed AppImage, moved binary) must not
    /// hide games: discovery falls back to the portable settings next to it.
    #[test]
    fn test_discover_cemu_stale_executable_still_scans_settings() {
        let tmp = tempfile::tempdir().unwrap();
        // The configured executable itself does not exist.
        let exe = tmp.path().join("cemu");
        assert!(!exe.exists());
        let portable = tmp.path().join("portable");
        std::fs::create_dir_all(&portable).unwrap();
        let games_dir = tmp.path().join("games");
        std::fs::create_dir_all(&games_dir).unwrap();
        std::fs::write(
            portable.join("settings.xml"),
            format!(
                "<CemuSettings><GamePaths><Entry>{}</Entry></GamePaths></CemuSettings>",
                games_dir.display()
            ),
        )
        .unwrap();
        std::fs::create_dir_all(games_dir.join("MyGame/meta")).unwrap();
        std::fs::write(
            games_dir.join("MyGame/meta/meta.xml"),
            r#"<menu><title_id>0005000010101D00</title_id><longname_en>Test Game</longname_en></menu>"#,
        )
        .unwrap();

        let games = discover_games_for_executable(&exe.to_string_lossy());

        assert_eq!(games.len(), 1);
        assert_eq!(games[0].title_id, "0005000010101d00");
    }

    #[test]
    fn test_cemu_portable_root() {
        let tmp = tempfile::tempdir().unwrap();
        let executable = tmp.path().join("Cemu");
        std::fs::create_dir_all(executable.parent().unwrap().join("portable")).unwrap();
        assert!(cemu_config_dir_for(&executable.to_string_lossy()).ends_with("portable"));
        assert!(cemu_data_dir_for(&executable.to_string_lossy()).ends_with("portable"));
    }

    #[test]
    fn test_scan_cemu_title_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join("title");
        std::fs::create_dir_all(game_dir.join("meta")).unwrap();
        std::fs::write(
            game_dir.join("meta/meta.xml"),
            r#"<menu><title_id>0005000010101D00</title_id><longname_en>Test Game</longname_en></menu>"#,
        )
        .unwrap();

        let mut games = Vec::new();
        scan_dir(tmp.path(), 0, &mut games);
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].title_id, "0005000010101d00");
        assert_eq!(games[0].title, "Test Game");
    }

    fn write_title(dir: &Path, title_id: &str) {
        std::fs::create_dir_all(dir.join("meta")).unwrap();
        std::fs::write(
            dir.join("meta/meta.xml"),
            format!(
                r#"<menu><title_id>{title_id}</title_id><longname_en>Test Game</longname_en></menu>"#
            ),
        )
        .unwrap();
    }

    /// Fixture: a portable Cemu install whose settings point at a games
    /// folder holding one title, while the mlc storage holds a copy of the
    /// same title under its content directory.
    fn cemu_fixture_with_duplicated_title() -> (tempfile::TempDir, String) {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("Cemu");
        std::fs::write(&exe, b"").unwrap();
        let portable = tmp.path().join("portable");
        std::fs::create_dir_all(&portable).unwrap();
        let games_dir = tmp.path().join("games");
        std::fs::create_dir_all(&games_dir).unwrap();
        std::fs::write(
            portable.join("settings.xml"),
            format!(
                "<CemuSettings><GamePaths><Entry>{}</Entry></GamePaths></CemuSettings>",
                games_dir.display()
            ),
        )
        .unwrap();
        write_title(&games_dir.join("MyGame"), "0005000010101D00");
        write_title(
            &portable.join("mlc01/usr/title/00050000/10101d00"),
            "0005000010101D00",
        );
        (tmp, exe.to_string_lossy().into_owned())
    }

    #[test]
    fn test_discover_cemu_dedupes_same_title_across_roots() {
        let (_tmp, exe) = cemu_fixture_with_duplicated_title();

        let games = discover_games_for_executable(&exe);

        assert_eq!(games.len(), 1);
        assert_eq!(games[0].title_id, "0005000010101d00");
        assert!(games[0].game_path.ends_with("MyGame"));
    }

    #[test]
    fn test_discover_cemu_keeps_distinct_titles_across_roots() {
        let (tmp, exe) = cemu_fixture_with_duplicated_title();
        write_title(&tmp.path().join("games/OtherGame"), "00050000101CD00");

        let games = discover_games_for_executable(&exe);

        assert_eq!(games.len(), 2);
    }
}
