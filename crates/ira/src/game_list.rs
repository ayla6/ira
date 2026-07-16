use ira_db as db;
use ira_config::Config;
use ira_models::{Game, SortMode};
use crate::game_loader;
use ira_parser as parser;
use ira_platforms::lutris::{load_lutris_games, LutrisGame};
use ira_platforms::ps4::{discover_games, load_shadps4_game, ShadPS4GameMeta};
use ira_platforms::retroachievements;
use ira_platforms::steam;

pub struct GameListOptions {
    pub lutris_enabled: bool,
    pub shadps4_enabled: bool,
    pub steam_enabled: bool,
    pub sort_mode: SortMode,
    pub sort_descending: bool,
}

fn normalize_title(s: &str) -> String {
    let lower = s.to_lowercase();
    let alnum: String = lower
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let words: Vec<&str> = alnum.split_whitespace().collect();
    let suffixes = ["the", "final", "cut", "edition", "complete", "definitive", "remastered", "hd"];
    let mut end = words.len();
    while end > 0 && suffixes.contains(&words[end - 1]) {
        end -= 1;
    }
    words[..end].join(" ")
}

fn auto_match_by_title(db: &db::DbConn, save_dir: &str, lutris_games: &[LutrisGame]) {
    let data_dir = std::path::Path::new(save_dir).join("data").join("steam");
    let mut title_map: Vec<(String, String)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&data_dir) {
        for entry in entries.flatten() {
            let app_id = match entry.file_name().to_str() {
                Some(s) if s.parse::<i64>().is_ok() => s.to_string(),
                _ => continue,
            };
            if db::find_by_steam_id(db, &app_id)
                .ok()
                .flatten()
                .map(|e| e.lutris_db_id.is_some())
                .unwrap_or(false)
            {
                continue;
            }
            if let Some(name) = parser::read_app_name(save_dir, &app_id) {
                title_map.push((normalize_title(&name), app_id));
            }
        }
    }

    let entries = db::load_all_games(db).unwrap_or_default();
    let linked: std::collections::HashSet<i64> = entries
        .iter()
        .filter_map(|e| e.lutris_db_id)
        .collect();
    let do_not_match: std::collections::HashSet<i64> = entries
        .iter()
        .filter(|e| {
            e.manual_unmatch || e.ignored == 1
        })
        .filter_map(|e| e.lutris_db_id)
        .collect();
    for lg in lutris_games {
        if linked.contains(&lg.id) || do_not_match.contains(&lg.id) {
            continue;
        }
        let norm = normalize_title(&lg.name);
        if norm.is_empty() {
            continue;
        }
        let match_id = title_map
            .iter()
            .find(|(t, _)| t == &norm)
            .map(|(_, id)| id.clone());
        if let Some(steam_id) = match_id {
            if let Ok(Some(entry)) = db::find_by_steam_id(db, &steam_id) {
                let _ = db::set_lutris_db_id(db, entry.id, lg.id);
                eprintln!("Auto-matched '{}' → steam_id {}", lg.name, steam_id);
            }
        }
    }
}

pub fn build_game_list(db: &db::DbConn, save_dir: &str, cfg: &Config, options: &GameListOptions) -> Vec<Game> {
    let steam_games = if options.steam_enabled { steam::discover_games() } else { Vec::new() };
    if options.steam_enabled && !steam_games.is_empty() {
        cleanup_steam_entries(db, &steam_games);
    }

    let steam_playtimes = if options.steam_enabled { steam::read_all_playtimes() } else { std::collections::HashMap::new() };

    let ra_any_console = cfg.any_console_enabled();

    let mut games = if options.lutris_enabled {
        let mut g = build_lutris_games(db, save_dir);
        g.extend(load_non_lutris_games(db, save_dir));
        g
    } else {
        load_non_lutris_games(db, save_dir)
    };

    games.retain(|g| {
        if !options.steam_enabled && g.kind == ira_models::GameKind::Steam { return false; }
        if !options.shadps4_enabled && g.kind == ira_models::GameKind::Ps4 { return false; }
        if !ra_any_console && g.kind == ira_models::GameKind::Retro { return false; }
        true
    });
    if options.shadps4_enabled {
        games.retain(|g| g.kind != ira_models::GameKind::Ps4);
    }
    if options.steam_enabled {
        games.retain(|g| g.kind != ira_models::GameKind::Steam);
    }
    if ra_any_console {
        games.retain(|g| g.kind != ira_models::GameKind::Retro);
    }
    games.sort_by(|a, b| {
        let ord = options.sort_mode.compare(a, b);
        if options.sort_descending { ord.reverse() } else { ord }
    });

    if options.shadps4_enabled {
        games.extend(build_shadps4_games(db, save_dir));
    }
    if options.steam_enabled {
        games.extend(build_steam_games(db, save_dir, &steam_games, &steam_playtimes));
    }
    if ra_any_console {
        games.extend(retroachievements::build_ra_games(db, save_dir, cfg, game_loader::load_game));
    }
    games.sort_by(|a, b| {
        let ord = options.sort_mode.compare(a, b);
        if options.sort_descending { ord.reverse() } else { ord }
    });

    games
}

fn build_lutris_games(db: &db::DbConn, save_dir: &str) -> Vec<Game> {
    let lutris_games = match load_lutris_games() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("Failed to read Lutris DB (falling back to DB-only list): {}", e);
            return game_loader::load_games(db, save_dir);
        }
    };

    for lg in &lutris_games {
        if lg.service_id.is_empty() {
            continue;
        }
        let entry = if ira_models::GameKind::from_string(&lg.service) == ira_models::GameKind::Steam {
            db::find_by_steam_id(db, &lg.service_id).ok().flatten()
        } else if lg.service == "gog" {
            db::find_gog_by_product_id(db, &lg.service_id).ok().flatten()
        } else {
            None
        };
        if let Some(entry) = entry {
            if entry.lutris_db_id.is_none() && !entry.manual_unmatch {
                let _ = db::set_lutris_db_id(db, entry.id, lg.id);
            }
        }
    }

    auto_match_by_title(db, save_dir, &lutris_games);

    let entries = db::load_all_games(db).unwrap_or_default();
    let ignored_ids = db::get_ignored_lutris_ids(db);
    let hidden_lutris_ids = db::get_hidden_lutris_ids(db);
    let mut by_lutris: std::collections::HashMap<i64, ira_models::GameEntry> = entries
        .into_iter()
        .filter_map(|e| e.lutris_db_id.map(|id| (id, e)))
        .collect();

    let mut games = Vec::with_capacity(lutris_games.len());
    for lg in &lutris_games {
        if ignored_ids.contains(&lg.id) {
            continue;
        }
        if let Some(entry) = by_lutris.remove(&lg.id) {
            match game_loader::load_game(&entry, save_dir) {
                Ok(mut game) => {
                    game.lutris_id = lg.id;
                    game.slug = lg.slug.clone();
                    game.playtime = lg.playtime;
                    game.last_played = lg.lastplayed;
                    game.lutris_name = lg.name.clone();
                    if game.name.is_empty() || game.name.starts_with("App ID:") {
                        game.name = lg.name.clone();
                    }
                    games.push(game);
                }
                Err(e) => eprintln!("Skipping {} ({}): {}", lg.name, lg.slug, e),
            }
        } else {
            let mut game = ira_models::unmatched_game(lg.id, &lg.name, &lg.slug, lg.playtime, lg.lastplayed);
            if hidden_lutris_ids.contains(&lg.id) {
                game.hidden = true;
            }
            games.push(game);
        }
    }
    games
}

fn build_shadps4_games(db: &db::DbConn, save_dir: &str) -> Vec<Game> {
    let shad_games = discover_games();
    let mut games = Vec::new();

    for shad in &shad_games {
        let entry = db::find_by_game_id(db, &shad.npwr_id).ok().flatten()
            .or_else(|| db::find_by_kind_platform(db, ira_models::GameKind::Ps4, &shad.serial).ok().flatten());
        let (db_id, title, hidden, logo_position, logo_size, sort_title, sgdb_id, shadps4_version, last_played) = match entry {
            Some(e) => (e.id, e.title, e.hidden, e.logo_position, e.logo_size, e.sort_title, e.sgdb_id.clone().unwrap_or_default(), e.shadps4_version.clone(), e.last_played),
            None => {
                match db::add_game(db, ira_models::GameKind::Ps4, ira_models::TrophySource::Empty, "", &shad.npwr_id, &shad.serial, &shad.title) {
                    Ok(id) => (id, shad.title.clone(), false, "bottom-left".to_string(), 50, String::new(), String::new(), String::new(), 0),
                    Err(e) => {
                        eprintln!("shadPS4: failed to add {} to DB: {}", shad.serial, e);
                        continue;
                    }
                }
            }
        };

        let game = load_shadps4_game(
            shad,
            db_id,
            &ShadPS4GameMeta {
                title,
                hidden,
                logo_position,
                logo_size,
                sort_title,
                sgdb_id,
                shadps4_version,
                last_played,
            },
            save_dir,
        );
        games.push(game);
    }

    games
}

/// Load DB games that are NOT linked to Lutris (lutris_db_id is None).
/// Used when Lutris is disabled — Lutris-linked games are hidden but
/// their DB entries are preserved untouched.
fn load_non_lutris_games(db: &db::DbConn, save_dir: &str) -> Vec<Game> {
    let entries = db::load_all_games(db).unwrap_or_default();
    entries
        .into_iter()
        .filter(|e| e.lutris_db_id.is_none())
        .filter_map(|e| game_loader::load_game(&e, save_dir).ok())
        .collect()
}

fn cleanup_steam_entries(db: &db::DbConn, discovered: &[steam::SteamGame]) {
    let discovered_ids: std::collections::HashSet<String> = discovered.iter()
        .map(|g| g.app_id.clone())
        .collect();

    let all_entries = db::load_all_games(db).unwrap_or_default();
    for entry in &all_entries {
        if entry.kind == ira_models::GameKind::Steam && !discovered_ids.contains(&entry.steam_id) {
            let _ = db::remove_game(db, entry.id);
        }
    }
}

fn build_steam_games(db: &db::DbConn, save_dir: &str, steam_games: &[steam::SteamGame], playtimes: &std::collections::HashMap<String, (f64, i64)>) -> Vec<Game> {
    let mut games = Vec::new();

    for sg in steam_games {
        if sg.app_id.is_empty() {
            continue;
        }
        let entry = db::find_by_steam_id(db, &sg.app_id).ok().flatten();
        if entry.is_none() {
            let kind = ira_models::GameKind::Steam;
            let trophy_source = ira_models::TrophySource::SteamNative;
            if let Err(e) = db::add_game(db, kind, trophy_source, &sg.app_id, "", &sg.app_id, &sg.name) {
                eprintln!("Steam: failed to add {} to DB: {}", sg.app_id, e);
                continue;
            }
        }

        let db_entry = db::find_by_steam_id(db, &sg.app_id).ok().flatten();
        if let Some(e) = db_entry {
            match game_loader::load_game(&e, save_dir) {
                Ok(mut game) => {
                    if (game.name.is_empty() || game.name.starts_with("App ID:"))
                        && !sg.name.is_empty() {
                            game.name = sg.name.clone();
                        }
                    game.game_path = sg.install_dir.to_string_lossy().into_owned();
                    if let Some(&(pt, lp)) = playtimes.get(&sg.app_id) {
                        game.playtime = pt;
                        game.last_played = lp;
                    }
                    games.push(game);
                }
                Err(e) => eprintln!("Steam: failed to load {}: {}", sg.app_id, e),
            }
        }
    }

    games
}
