use std::path::{Path, PathBuf};

use crate::ps4::{parse_psf, psf_get_title, psf_get_title_id};

use super::paths::{disc_dir_for, games_dir_for, rpcs3_config_dir_for_executable};

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
    pub config_dir: PathBuf,
}

/// None if PARAM.SFO is missing or has no TITLE_ID.
fn scan_game_folder(game_path: &Path, config_dir: &Path) -> Option<Rpcs3Game> {
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

    // NPWR ID is the TROPDIR subfolder name (e.g. TROPDIR/NPWR00906_00/).
    let npwr_id = find_npwr_id(&game_path.join("TROPDIR"));

    Some(Rpcs3Game {
        serial,
        npwr_id,
        title,
        game_path: game_path.to_path_buf(),
        config_dir: config_dir.to_path_buf(),
    })
}

fn find_npwr_id(tropdir: &Path) -> String {
    let entries = match std::fs::read_dir(tropdir) {
        Ok(e) => e,
        Err(_) => return String::new(),
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(is_dir)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("NPWR"))
        .collect();
    names.sort();
    names.into_iter().next().unwrap_or_default()
}

fn is_dir(entry: &std::fs::DirEntry) -> bool {
    entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
}

/// Discover all installed RPCS3 games: `dev_hdd0/game/` (PSN, PARAM.SFO at
/// root) and `dev_hdd0/disc/` (`<name>/PS3_GAME/PARAM.SFO`).
pub fn discover_games() -> Vec<Rpcs3Game> {
    discover_games_for_executable("")
}

pub fn discover_games_for_executable(executable: &str) -> Vec<Rpcs3Game> {
    let mut games = Vec::new();
    let config_dir = rpcs3_config_dir_for_executable(executable);

    let psn_dir = games_dir_for(executable);
    if let Ok(entries) = std::fs::read_dir(&psn_dir) {
        games.extend(
            entries
                .flatten()
                .filter(is_dir)
                .filter_map(|e| scan_game_folder(&e.path(), &config_dir)),
        );
    }

    let disc_dir = disc_dir_for(executable);
    if let Ok(entries) = std::fs::read_dir(&disc_dir) {
        games.extend(
            entries
                .flatten()
                .filter(is_dir)
                .map(|e| e.path().join("PS3_GAME"))
                .filter(|p| p.join("PARAM.SFO").is_file())
                .filter_map(|ref p| scan_game_folder(p, &config_dir)),
        );
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
        assert_eq!(result, "NPWR00906_00");
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
        assert!(scan_game_folder(tmp.path(), tmp.path()).is_none());
    }
}
