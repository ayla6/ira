//! Ryujinx-family (Ryubing, Kenji-NX) title metadata. Each library game
//! gets `games/<16 lowercase hex>/gui/metadata.json` with its display
//! name; the base directory is a per-fork constant, so all likely names
//! are probed. Icons are read from the ROM at runtime and never cached,
//! which keeps this family a title-only source.

use std::path::PathBuf;

use super::TitleCache;

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
}
