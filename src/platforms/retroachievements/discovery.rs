use std::path::{Path, PathBuf};

use crate::config::RaConfig;
use crate::models::{Game, RETRO, RA};
use crate::platforms::retroachievements::api::{RaClient, RaGameEntry};

pub const CONSOLE_PSX: u32 = 12;
pub const CONSOLE_PS2: u32 = 21;
pub const CONSOLE_PSP: u32 = 41;

const PSX_EXTENSIONS: &[&str] = &["bin", "cue", "chd", "pbp", "iso", "ecm"];
const PS2_EXTENSIONS: &[&str] = &["iso", "bin", "cue", "chd", "gz", "elf"];
const PSP_EXTENSIONS: &[&str] = &["iso", "cso", "pbp", "prx"];

struct ConsoleConfig {
    id: u32,
    name: &'static str,
    folder: String,
    extensions: &'static [&'static str],
}

fn console_configs(cfg: &RaConfig) -> Vec<ConsoleConfig> {
    let mut consoles = Vec::new();
    if cfg.psx_enabled && !cfg.psx_folder.is_empty() {
        consoles.push(ConsoleConfig {
            id: CONSOLE_PSX,
            name: "psx",
            folder: cfg.psx_folder.clone(),
            extensions: PSX_EXTENSIONS,
        });
    }
    if cfg.ps2_enabled && !cfg.ps2_folder.is_empty() {
        consoles.push(ConsoleConfig {
            id: CONSOLE_PS2,
            name: "ps2",
            folder: cfg.ps2_folder.clone(),
            extensions: PS2_EXTENSIONS,
        });
    }
    if cfg.psp_enabled && !cfg.psp_folder.is_empty() {
        consoles.push(ConsoleConfig {
            id: CONSOLE_PSP,
            name: "psp",
            folder: cfg.psp_folder.clone(),
            extensions: PSP_EXTENSIONS,
        });
    }
    consoles
}

fn normalize_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c == '_' || c == '.' { ' ' } else { c })
        .collect();
    let no_tags = remove_version_tags(&cleaned);
    let lower = no_tags.to_lowercase();
    let alnum: String = lower
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    alnum.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn remove_version_tags(s: &str) -> String {
    let mut result = String::new();
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            _ if depth == 0 => result.push(c),
            _ => {}
        }
    }
    result.trim().to_string()
}

fn scan_roms(folder: &str, extensions: &[&str]) -> Vec<(String, PathBuf)> {
    let mut roms = Vec::new();
    let path = Path::new(folder);
    if !path.is_dir() {
        return roms;
    }
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return roms,
    };
    for entry in entries.flatten() {
        let file_path = entry.path();
        if !file_path.is_file() {
            continue;
        }
        if let Some(ext) = file_path.extension().and_then(|e| e.to_str()) {
            if extensions.iter().any(|&e| e.eq_ignore_ascii_case(ext)) {
                let name = file_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                roms.push((name, file_path));
            }
        }
    }
    roms
}

fn match_rom_to_game(rom_name: &str, games: &[RaGameEntry]) -> Option<u32> {
    let rom_norm = normalize_name(rom_name);
    if rom_norm.is_empty() {
        return None;
    }

    for g in games {
        if normalize_name(&g.title) == rom_norm {
            return Some(g.id);
        }
    }

    None
}

pub fn build_ra_games(
    db: &crate::db::DbConn,
    save_dir: &str,
    cfg: &RaConfig,
) -> Vec<Game> {
    let has_credentials = !cfg.username.is_empty() && !cfg.token.is_empty();
    let client = if cfg.ra_enabled && has_credentials {
        RaClient::from_config(cfg)
    } else {
        None
    };

    let consoles = console_configs(cfg);
    let mut games = Vec::new();

    for console in &consoles {
        let ra_games = match &client {
            Some(c) => match c.fetch_console_games(save_dir, console.id) {
                Ok(g) => g,
                Err(e) => {
                    eprintln!("RA: failed to fetch game list for {}: {}", console.name, e);
                    continue;
                }
            },
            None => Vec::new(),
        };

        let roms = scan_roms(&console.folder, console.extensions);

        for (rom_name, rom_path) in &roms {
            let matched_id = if client.is_some() {
                match_rom_to_game(rom_name, &ra_games)
            } else {
                None
            };
            let (app_id, title, trophy_source) = match matched_id {
                Some(id) => {
                    let title = ra_games
                        .iter()
                        .find(|g| g.id == id)
                        .map(|g| g.title.clone())
                        .unwrap_or_else(|| rom_name.clone());
                    (id.to_string(), title, RA.to_string())
                }
                None => (rom_name.clone(), rom_name.clone(), String::new()),
            };

            let entry = crate::db::find_by_steam_id(db, &app_id)
                .ok()
                .flatten();

            let game = match entry {
                Some(e) => {
                    let mut g = crate::parser::load_game(&e, save_dir)
                        .unwrap_or_else(|_| Game {
                            app_id: app_id.clone(),
                            kind: RETRO.to_string(),
                            trophy_source: trophy_source.clone(),
                            platform_id: console.name.to_string(),
                            db_id: e.id,
                            name: if e.title.is_empty() { title.clone() } else { e.title.clone() },
                            ..Default::default()
                        });
                    g.game_path = rom_path.to_string_lossy().into_owned();
                    g
                }
                None => {
                    match crate::db::add_game(db, RETRO, &trophy_source, &app_id, console.name, &title) {
                        Ok(_id) => Game {
                            app_id: app_id.clone(),
                            kind: RETRO.to_string(),
                            trophy_source: trophy_source.clone(),
                            platform_id: console.name.to_string(),
                            db_id: _id,
                            name: title.clone(),
                            game_path: rom_path.to_string_lossy().into_owned(),
                            ..Default::default()
                        },
                        Err(e) => {
                            eprintln!("RA: failed to add {} to DB: {}", rom_name, e);
                            continue;
                        }
                    }
                }
            };
            games.push(game);
        }
    }

    games
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_name_basic() {
        assert_eq!(normalize_name("Final Fantasy VII (USA)"), "final fantasy vii");
    }

    #[test]
    fn test_normalize_name_underscores() {
        assert_eq!(normalize_name("Final_Fantasy.VII"), "final fantasy vii");
    }

    #[test]
    fn test_normalize_name_version_tags() {
        assert_eq!(normalize_name("Chrono Trigger [!]"), "chrono trigger");
    }

    #[test]
    fn test_normalize_name_empty() {
        assert_eq!(normalize_name(""), "");
    }

    #[test]
    fn test_match_rom_exact() {
        let games = vec![RaGameEntry {
            id: 1,
            title: "Final Fantasy VII".to_string(),
            image_icon: String::new(),
            image_url: String::new(),
            num_achievements: 0,
            points: 0,
        }];
        assert_eq!(match_rom_to_game("Final Fantasy VII", &games), Some(1));
    }

    #[test]
    fn test_match_rom_normalized() {
        let games = vec![RaGameEntry {
            id: 42,
            title: "Chrono Trigger".to_string(),
            image_icon: String::new(),
            image_url: String::new(),
            num_achievements: 0,
            points: 0,
        }];
        assert_eq!(match_rom_to_game("Chrono_Trigger", &games), Some(42));
    }

    #[test]
    fn test_match_rom_no_match() {
        let games = vec![RaGameEntry {
            id: 1,
            title: "Final Fantasy VII".to_string(),
            image_icon: String::new(),
            image_url: String::new(),
            num_achievements: 0,
            points: 0,
        }];
        assert_eq!(match_rom_to_game("Completely Different Game", &games), None);
    }
}
