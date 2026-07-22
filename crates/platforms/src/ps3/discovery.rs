use std::path::{Path, PathBuf};

use crate::ps4::{parse_psf, psf_get_title, psf_get_title_id};

use super::paths::{games_dir, disc_dir};

/// A discovered RPCS3 game.
#[derive(Debug, Clone)]
pub struct Rpcs3Game {
    /// TITLE_ID from PARAM.SFO (e.g. "NPUB30698", "BLES01234").
    pub serial: String,
    /// NPCommId from TROPDIR folder name (e.g. "NPWR00906_00").
    /// Empty if the game has no trophies.
    pub npwr_id: String,
    pub title: String,
    /// Path to the game's root folder (containing PARAM.SFO).
    pub game_path: PathBuf,
}

/// Scan a game folder for its PARAM.SFO, parse it, and find the NPWR ID from TROPDIR.
/// Returns None if PARAM.SFO is missing or has no TITLE_ID.
fn scan_game_folder(game_path: &Path) -> Option<Rpcs3Game> {
    let param_sfo = game_path.join("PARAM.SFO");
    if !param_sfo.is_file() {
        return None;
    }

    let psf = match parse_psf(&param_sfo) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("RPCS3: skip {}: {}", game_path.display(), e);
            return None;
        }
    };

    let title = psf_get_title(&psf);
    let serial = psf_get_title_id(&psf);
    if serial.is_empty() {
        return None;
    }

    // NPWR ID comes from the TROPDIR subfolder name (e.g. TROPDIR/NPWR00906_00/).
    // No npbind.dat parsing needed — unlike shadPS4, the folder name IS the NPCommId.
    let npwr_id = find_npwr_id(&game_path.join("TROPDIR"));

    Some(Rpcs3Game {
        serial,
        npwr_id,
        title,
        game_path: game_path.to_path_buf(),
    })
}

/// Find the first NPCommId folder under TROPDIR/ (e.g. "NPWR00906_00").
fn find_npwr_id(tropdir: &Path) -> String {
    let entries = match std::fs::read_dir(tropdir) {
        Ok(e) => e,
        Err(_) => return String::new(),
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with("NPWR") {
            return name_str.into_owned();
        }
    }
    String::new()
}

/// Discover all installed RPCS3 games.
///
/// Scans:
///   dev_hdd0/game/   — PSN games (each subfolder has PARAM.SFO at root)
///   dev_hdd0/disc/   — disc games (each subfolder has PS3_GAME/PARAM.SFO)
pub fn discover_games() -> Vec<Rpcs3Game> {
    let mut games = Vec::new();

    // PSN games
    let psn_dir = games_dir();
    if let Ok(entries) = std::fs::read_dir(&psn_dir) {
        games.extend(entries.flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .filter_map(|e| scan_game_folder(&e.path())));
    }

    // Disc games (folder structure: disc/<name>/PS3_GAME/PARAM.SFO)
    let disc_dir = disc_dir();
    if let Ok(entries) = std::fs::read_dir(&disc_dir) {
        games.extend(entries.flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.path().join("PS3_GAME"))
            .filter(|p| p.join("PARAM.SFO").is_file())
            .filter_map(|ref p| scan_game_folder(p)));
    }

    games
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_npwr_id_missing_dir() {
        let result = find_npwr_id(Path::new("/nonexistent/TROPDIR"));
        assert!(result.is_empty());
    }

    #[test]
    fn test_find_npwr_id_finds_first() {
        let tmp = tempfile::tempdir().unwrap();
        let tropdir = tmp.path().join("TROPDIR");
        std::fs::create_dir_all(tropdir.join("NPWR00906_00")).unwrap();
        std::fs::create_dir_all(tropdir.join("NPWR01234_00")).unwrap();
        std::fs::create_dir_all(tropdir.join("other")).unwrap();

        let result = find_npwr_id(&tropdir);
        assert!(result.starts_with("NPWR"));
    }

    #[test]
    fn test_find_npwr_id_no_npwr_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let tropdir = tmp.path().join("TROPDIR");
        std::fs::create_dir_all(tropdir.join("other")).unwrap();

        let result = find_npwr_id(&tropdir);
        assert!(result.is_empty());
    }

    #[test]
    fn test_scan_game_folder_missing_param_sfo() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(scan_game_folder(tmp.path()).is_none());
    }
}
