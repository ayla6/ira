//! Ryujinx-family (Ryubing, Kenji-NX) title metadata. Each library game
//! gets `games/<16 lowercase hex>/gui/metadata.json` with its display
//! name; the base directory is a per-fork constant, so all likely names
//! are probed. Icons are read from the ROM at runtime and never cached,
//! which keeps this family a title-only source.
//!
//! The family cannot install titles to NAND, so its library is the set of
//! ROM files in the folders `Config.json` points at. The `games/`
//! metadata dirs are only a name source: they outlive deleted files and
//! would resurrect uninstalled titles.

use std::path::{Path, PathBuf};

use super::{SwitchInstalledGame, TitleCache};

/// The fork directory names worth probing, then the portable layout next
/// to each detected executable. Several Ryujinx-family installs can sit
/// side by side, each with its own `portable/games` library.
fn base_dirs_for(executable: &str) -> Vec<PathBuf> {
    base_dirs_in(executable, &crate::switch_detect::detected_launch_commands())
}

fn base_dirs_in(executable: &str, detected: &[String]) -> Vec<PathBuf> {
    let mut exes: Vec<&str> = vec![executable];
    exes.extend(detected.iter().map(String::as_str));

    let mut dirs = Vec::new();
    // Portable layouts of every detected install.
    for exe in &exes {
        if exe.is_empty() || exe.starts_with("flatpak:") {
            continue;
        }
        let exe_path = std::path::Path::new(exe);
        for root in [exe_path.parent(), Some(exe_path)].into_iter().flatten() {
            dirs.push(root.join("portable"));
        }
    }
    // Flatpak sandboxes redirect the config dir under the app id; the base
    // directory name inside it depends on the fork.
    for exe in &exes {
        if let Some(app) = crate::emu_dirs::flatpak_app_dir(exe) {
            for name in ["Ryubing", "Ryujinx", "Kenji-NX", "kenji-nx"] {
                dirs.push(app.join("config").join(name));
            }
        }
    }
    let config = crate::emu_dirs::config_home();
    for name in ["Ryubing", "Ryujinx", "Kenji-NX", "kenji-nx"] {
        dirs.push(config.join(name));
    }
    dirs
}

/// One cache per base directory found, best source first.
pub(super) fn title_caches(executable: &str) -> Vec<TitleCache> {
    title_caches_in(&base_dirs_for(executable))
}

/// The ROM files of every detected install's configured game directories,
/// deduplicated by base title id across installs and sorted by id so
/// scans are reproducible. Only titles a metadata dir names are reported:
/// an unnamed file has no identity beyond its file name, which the ROM
/// library scan handles better.
pub(super) fn library_games(executable: &str) -> Vec<SwitchInstalledGame> {
    library_games_in(&base_dirs_for(executable), &title_caches(executable))
}

/// The ROM container extensions Ryujinx offers in its file-type filter.
const ROM_EXTENSIONS: &[&str] = &["nsp", "pfs0", "xci", "nca", "nro", "nso"];

fn library_games_in(base_dirs: &[PathBuf], caches: &[TitleCache]) -> Vec<SwitchInstalledGame> {
    let mut out: Vec<SwitchInstalledGame> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for base in base_dirs {
        for dir in config_game_dirs(base) {
            for rom in rom_files(&dir) {
                let stem = rom
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                // Update NSPs normalize onto their base title here, exactly
                // like they do in the ROM file scan.
                let Some(title_id) = super::rom::title_id_from_nsp(&rom)
                    .or_else(|| super::rom::title_id_from_filename(&stem))
                else {
                    continue;
                };
                let Some(title) = caches
                    .iter()
                    .find_map(|cache| cache.title_for(&title_id))
                    .filter(|title| !title.is_empty())
                    .map(str::to_string)
                else {
                    continue;
                };
                if seen.insert(title_id.clone()) {
                    out.push(SwitchInstalledGame {
                        title_id,
                        title,
                        // This family never caches icons; the ROM file's own
                        // control NCA supplies them at load time.
                        icon: None,
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| a.title_id.cmp(&b.title_id));
    out
}

/// `Config.json`'s `game_dirs` of one install: the folders whose ROM files
/// make up its library.
fn config_game_dirs(base: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(base.join("Config.json")) else {
        return Vec::new();
    };
    let Ok(config) = serde_json::from_str::<RyujinxConfig>(&text) else {
        return Vec::new();
    };
    config
        .game_dirs
        .into_iter()
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .collect()
}

#[derive(serde::Deserialize)]
struct RyujinxConfig {
    #[serde(default)]
    game_dirs: Vec<String>,
}

/// Every file under `dir` (recursively) whose extension Ryujinx treats as
/// a game container. Unreadable directories yield nothing.
fn rom_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            out.extend(rom_files(&path));
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| ROM_EXTENSIONS.iter().any(|known| e.eq_ignore_ascii_case(known)))
        {
            out.push(path);
        }
    }
    out
}

/// The Ryujinx-family NAND content directory: installed titles' NCAs sit
/// in `bis/system/Contents/Registered`, under each install's base dir.
pub(super) fn bis_registered_dirs(executable: &str) -> Vec<PathBuf> {
    base_dirs_for(executable)
        .into_iter()
        .map(|base| base.join("bis/system/Contents/Registered"))
        .collect()
}

fn title_caches_in(base_dirs: &[PathBuf]) -> Vec<TitleCache> {
    let mut caches = Vec::new();
    for base in base_dirs {
        let games = base.join("games");
        let Ok(entries) = std::fs::read_dir(&games) else {
            continue;
        };
        let mut cache = TitleCache::empty();
        for entry in entries.flatten() {
            if !entry
                .file_type()
                .map(|t| t.is_dir())
                .unwrap_or(false)
            {
                continue;
            }
            let id = entry.file_name().to_string_lossy().into_owned();
            if !super::is_title_id(&id) {
                continue;
            }
            let Ok(text) =
                std::fs::read_to_string(entry.path().join("gui").join("metadata.json"))
            else {
                continue;
            };
            cache.insert(&id, metadata_title(&text).unwrap_or_default(), None);
        }
        if !cache.is_empty() {
            caches.push(cache);
        }
    }
    caches
}

/// The `title` field of Ryujinx's snake_case `metadata.json`.
fn metadata_title(json: &str) -> Option<String> {
    let parsed: MetadataJson = serde_json::from_str(json).ok()?;
    parsed
        .title
        .filter(|title| !title.trim().is_empty())
        .map(|title| title.trim().to_string())
}

#[derive(serde::Deserialize)]
struct MetadataJson {
    #[serde(default)]
    title: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_title_caches_reads_portable_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let gui = tmp
            .path()
            .join("portable/games/0100000000010000/gui");
        std::fs::create_dir_all(&gui).unwrap();
        std::fs::write(gui.join("metadata.json"), r#"{"title": "Super Mario Odyssey"}"#).unwrap();

        // A game directory without metadata and a non-title directory are
        // ignored.
        std::fs::create_dir_all(tmp.path().join("portable/games/0100000000010800/gui")).unwrap();
        std::fs::create_dir_all(tmp.path().join("portable/games/not-a-title")).unwrap();

        let caches = title_caches_in(&[tmp.path().join("portable")]);
        assert_eq!(caches.len(), 1);
        let meta = caches.first().unwrap();
        assert_eq!(
            meta.title_for("0100000000010000"),
            Some("Super Mario Odyssey")
        );
    }

    #[test]
    fn test_metadata_title_reads_snake_case_field() {
        assert_eq!(
            metadata_title(r#"{"title": "Zelda", "favorite": false}"#),
            Some("Zelda".to_string())
        );
        assert_eq!(metadata_title(r#"{"favorite": false}"#), None);
        assert_eq!(metadata_title("not json"), None);
        assert_eq!(metadata_title(r#"{"title": "  "}"#), None);
    }

    /// One install: `Config.json` points at a folder holding a base NSP
    /// (with ticket, so the id comes from the container too), an update
    /// NSP for the same title, a metadata dir naming the base title, and
    /// the cases that must never surface.
    fn library_fixture(tmp: &std::path::Path) -> PathBuf {
        let base = tmp.join("Ryubing");
        let games = tmp.join("roms");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&games).unwrap();
        std::fs::write(
            base.join("Config.json"),
            format!(r#"{{"game_dirs": ["{}"]}}"#, games.display()),
        )
        .unwrap();
        let metadata = base.join("games/010051f0207b2000/gui");
        std::fs::create_dir_all(&metadata).unwrap();
        std::fs::write(
            metadata.join("metadata.json"),
            r#"{"title": "Tomodachi Life: Living the Dream"}"#,
        )
        .unwrap();

        let base_nsp = games.join("Tomodachi Life [010051F0207B2000][v0][Base].nsp");
        std::fs::write(&base_nsp, nsp_fixture(&["010051f0207b20000000000000000003.tik"])).unwrap();
        // An update file normalizes onto the base title id.
        let update = games.join("Tomodachi Life [010051F0207B2800][v65536].nsp");
        std::fs::write(&update, nsp_fixture(&["010051f0207b28000000000000000003.tik"])).unwrap();
        // No title id anywhere, no metadata: never reported.
        std::fs::write(games.join("Unknown Game.nsp"), b"not an nsp").unwrap();
        // A file with an id but no metadata name: never reported.
        std::fs::write(games.join("[0100000000010000].xci"), b"not an nsp").unwrap();
        // A stale metadata dir without any backing file: never reported.
        let stale = base.join("games/0100000000010800/gui");
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("metadata.json"), r#"{"title": "Ghost"}"#).unwrap();
        base
    }

    /// Minimal PFS0 with one string-table entry, enough for the ticket
    /// name reader.
    fn nsp_fixture(names: &[&str]) -> Vec<u8> {
        let mut table = Vec::new();
        let offsets: Vec<u32> = names
            .iter()
            .map(|name| {
                let offset = table.len() as u32;
                table.extend_from_slice(name.as_bytes());
                table.push(0);
                offset
            })
            .collect();
        let mut out = Vec::new();
        out.extend_from_slice(b"PFS0");
        out.extend_from_slice(&(names.len() as u32).to_le_bytes());
        out.extend_from_slice(&(table.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for offset in &offsets {
            out.extend_from_slice(&0u64.to_le_bytes());
            out.extend_from_slice(&0u64.to_le_bytes());
            out.extend_from_slice(&offset.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
        }
        out.extend_from_slice(&table);
        out
    }

    #[test]
    fn test_library_games_scans_configured_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let base = library_fixture(tmp.path());

        let games = library_games_in(&[base], &title_caches_in(&[tmp.path().join("Ryubing")]));

        // The base and the update file collapse into one entry, named by
        // the metadata dir; the unnamed file, the nameless id and the
        // stale metadata dir never surface.
        assert_eq!(
            games
                .iter()
                .map(|g| (g.title_id.as_str(), g.title.as_str()))
                .collect::<Vec<_>>(),
            vec![("010051f0207b2000", "Tomodachi Life: Living the Dream")]
        );
        assert!(games[0].icon.is_none());
    }

    #[test]
    fn test_config_game_dirs_missing_or_broken_config() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("Ryujinx");
        std::fs::create_dir_all(&base).unwrap();
        assert!(config_game_dirs(&base).is_empty());
        std::fs::write(base.join("Config.json"), "not json").unwrap();
        assert!(config_game_dirs(&base).is_empty());
    }

    #[test]
    fn test_rom_files_walks_recursively_and_filters_extensions() {
        let tmp = tempfile::tempdir().unwrap();
        let games = tmp.path().join("library");
        std::fs::create_dir_all(games.join("sub")).unwrap();
        std::fs::write(games.join("a.nsp"), b"").unwrap();
        std::fs::write(games.join("b.XCI"), b"").unwrap();
        std::fs::write(games.join("sub").join("c.nro"), b"").unwrap();
        std::fs::write(games.join("notes.txt"), b"").unwrap();

        let mut found: Vec<String> = rom_files(&games)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        found.sort();
        assert_eq!(found, vec!["a.nsp", "b.XCI", "c.nro"]);
    }
}
