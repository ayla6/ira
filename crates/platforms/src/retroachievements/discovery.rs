use std::path::{Path, PathBuf};

use ira_config::Config;
use ira_models::{Game, GameKind, TrophySource};
use crate::consoles::{CONSOLES, ConsoleDef};
use crate::retroachievements::api::{RaClient, RaGameEntry};

struct ActiveConsole {
    def: &'static ConsoleDef,
    folder: String,
}

fn active_consoles(cfg: &Config) -> Vec<ActiveConsole> {
    let mut consoles = Vec::new();
    for def in CONSOLES {
        let cc = cfg.console(def.id);
        if cc.enabled && !cc.folder.is_empty() {
            consoles.push(ActiveConsole {
                def,
                folder: cc.folder.clone(),
            });
        }
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
    db: &ira_db::DbConn,
    save_dir: &str,
    cfg: &Config,
    load_game: impl Fn(&ira_models::GameEntry, &str) -> Result<ira_models::Game, String>,
) -> Vec<Game> {
    let has_credentials = !cfg.ra_username.is_empty() && !cfg.ra_token.is_empty();
    let client = if cfg.ra_enabled && has_credentials {
        RaClient::from_config(cfg)
    } else {
        None
    };

    let consoles = active_consoles(cfg);
    let mut games = Vec::new();

    for console in &consoles {
        let ra_games_raw = match &client {
            Some(c) => match c.fetch_console_games(save_dir, console.def.ra_console_id) {
                Ok(g) => g,
                Err(e) => {
                    eprintln!("RA: failed to fetch game list for {}: {}", console.def.id, e);
                    continue;
                }
            },
            None => Vec::new(),
        };
        let ra_games: Vec<RaGameEntry> = ra_games_raw
            .into_iter()
            .filter(|g| !g.title.contains('~') && !g.title.contains("[Subset"))
            .collect();

        let roms = scan_roms(&console.folder, console.def.extensions);

        for (rom_name, rom_path) in &roms {
            let rom_path_str = rom_path.to_string_lossy().into_owned();

            let existing = ira_db::find_by_rom_path(db, &rom_path_str)
                .ok()
                .flatten();

            let game = match existing {
                Some(e) => {
                    if e.rom_path.is_empty() {
                        let _ = ira_db::set_rom_path(db, e.id, &rom_path_str);
                    }
                    let mut g = load_game(&e, save_dir)
                        .unwrap_or_else(|_| Game {
                            app_id: e.steam_id.clone(),
                            kind: GameKind::Retro,
                            trophy_source: e.trophy_source,
                            platform_id: e.platform_id.clone(),
                            db_id: e.id,
                            name: if e.title.is_empty() { rom_name.clone() } else { e.title.clone() },
                            ..Default::default()
                        });
                    g.game_path = rom_path_str.clone();
                    g.rom_path = rom_path_str;
                    g
                }
                None => {
                    let serial = crate::rom_serial::read_serial(rom_path);

                    let matched_id = if client.is_some() {
                        match_rom_to_game(rom_name, &ra_games)
                    } else {
                        None
                    };
                    let (app_id, title, trophy_source) = match matched_id {
                        Some(id) => {
                            let t = ra_games
                                .iter()
                                .find(|g| g.id == id)
                                .map(|g| g.title.clone())
                                .unwrap_or_else(|| rom_name.clone());
                            (id.to_string(), t, TrophySource::Ra)
                        }
                        None => (serial.clone().unwrap_or_else(|| rom_name.clone()), rom_name.clone(), TrophySource::Empty),
                    };

                    let existing_by_id = ira_db::find_by_game_id(db, &app_id).ok().flatten();
                    if let Some(e) = existing_by_id {
                        if e.rom_path.is_empty() {
                            let _ = ira_db::set_rom_path(db, e.id, &rom_path_str);
                        }
                        let mut g = load_game(&e, save_dir)
                            .unwrap_or_else(|_| Game {
                                app_id: e.game_id.clone(),
                                kind: GameKind::Retro,
                                trophy_source: e.trophy_source,
                                platform_id: e.platform_id.clone(),
                                db_id: e.id,
                                name: if e.title.is_empty() { title.clone() } else { e.title.clone() },
                                ..Default::default()
                            });
                        g.game_path = rom_path_str.clone();
                        g.rom_path = rom_path_str;
                        g
                    } else {
                        match ira_db::add_game(db, GameKind::Retro, trophy_source, "", &app_id, console.def.id, &title) {
                            Ok(id) => {
                                let _ = ira_db::set_rom_path(db, id, &rom_path_str);
                                Game {
                                    app_id: app_id.clone(),
                                    kind: GameKind::Retro,
                                    trophy_source,
                                    platform_id: console.def.id.to_string(),
                                    db_id: id,
                                    name: title.clone(),
                                    game_path: rom_path_str.clone(),
                                    rom_path: rom_path_str,
                                    ..Default::default()
                                }
                            }
                            Err(e) => {
                                eprintln!("RA: failed to add {} to DB: {}", rom_name, e);
                                continue;
                            }
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
