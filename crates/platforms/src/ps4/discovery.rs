use std::path::{Path, PathBuf};

use crate::ps4::{parse_npbind, parse_psf, psf_get_title, psf_get_title_id, shadps4_user_dir_for};

/// Read install_dirs from shadPS4 config.json
pub fn read_install_dirs() -> Vec<PathBuf> {
    read_install_dirs_for_executable("")
}

pub fn read_install_dirs_for_executable(executable: &str) -> Vec<PathBuf> {
    let config_path = shadps4_user_dir_for(executable).join("config.json");
    let data = match std::fs::read_to_string(&config_path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let json: serde_json::Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    json.get("General")
        .and_then(|g| g.get("install_dirs"))
        .and_then(|d| d.as_array())
        .into_iter()
        .flat_map(|arr| arr.iter())
        .filter(|dir| {
            dir.get("enabled")
                .and_then(|e| e.as_bool())
                .unwrap_or(false)
        })
        .filter_map(|dir| dir.get("path").and_then(|p| p.as_str()).map(PathBuf::from))
        .collect()
}

/// A discovered shadPS4 game.
pub struct ShadPS4Game {
    pub serial: String,
    pub npwr_id: String,
    pub title: String,
    pub game_path: PathBuf,
    pub user_dir: PathBuf,
}

/// Recursively scan a directory for game folders (containing sce_sys/param.sfo).
/// Max depth: 5 levels. Skips dirs ending in -UPDATE or -patch.
fn scan_dir(path: &Path, results: &mut Vec<PathBuf>, depth: u32) {
    if depth > 5 {
        return;
    }
    let param_sfo = path.join("sce_sys").join("param.sfo");
    if param_sfo.is_file() {
        results.push(path.to_path_buf());
        return;
    }
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.ends_with("-UPDATE") || name_str.ends_with("-patch") {
            continue;
        }
        scan_dir(&entry.path(), results, depth + 1);
    }
}

/// Recursively scan a directory for game folders (containing sce_sys/param.sfo).
/// Max depth: 5 levels. Skips dirs ending in -UPDATE or -patch.
pub fn scan_dir_for_test(path: &Path, results: &mut Vec<PathBuf>) {
    scan_dir(path, results, 0);
}

/// Discover all installed shadPS4 games.
pub fn discover_games() -> Vec<ShadPS4Game> {
    discover_games_for_executable("")
}

pub fn discover_games_for_executable(executable: &str) -> Vec<ShadPS4Game> {
    let install_dirs = read_install_dirs_for_executable(executable);
    let user_dir = shadps4_user_dir_for(executable);
    let mut game_dirs = Vec::new();
    for dir in &install_dirs {
        scan_dir(dir, &mut game_dirs, 0);
    }

    let mut games = Vec::new();
    for game_path in game_dirs {
        let param_sfo = game_path.join("sce_sys").join("param.sfo");
        let psf = match parse_psf(&param_sfo) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("shadPS4: skip {}: {}", game_path.display(), e);
                continue;
            }
        };

        let title = psf_get_title(&psf);
        let serial = psf_get_title_id(&psf);
        if serial.is_empty() {
            continue;
        }

        // Get NPWR ID from npbind.dat
        let npbind_path = game_path.join("sce_sys").join("npbind.dat");
        let npwr_id = match parse_npbind(&npbind_path) {
            Ok(ids) => ids.into_iter().next().unwrap_or_default(),
            Err(_) => {
                // Fallback: try strings extraction
                let npwr = std::fs::read(&npbind_path)
                    .ok()
                    .and_then(|data| {
                        String::from_utf8_lossy(&data)
                            .lines()
                            .find(|l| l.starts_with("NPWR"))
                            .map(|s| s.trim().to_string())
                    })
                    .unwrap_or_default();
                if npwr.is_empty() {
                    eprintln!("shadPS4: no NPWR ID for {}", serial);
                    continue;
                }
                npwr
            }
        };

        games.push(ShadPS4Game {
            serial,
            npwr_id,
            title,
            game_path,
            user_dir: user_dir.clone(),
        });
    }

    games
}
