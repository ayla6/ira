use crate::db;
use crate::models::Game;
use crate::parser;
use crate::platforms::lutris::{load_lutris_games, LutrisGame};
use crate::platforms::ps4::{discover_games, load_shadps4_game};

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
            e.manual_unmatch.unwrap_or(0) == 1 || e.ignored.unwrap_or(0) == 1
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

pub fn build_game_list(db: &db::DbConn, save_dir: &str, shadps4_enabled: bool) -> Vec<Game> {
    let lutris_games = match load_lutris_games() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("Failed to read Lutris DB (falling back to DB-only list): {}", e);
            return parser::load_games(db, save_dir);
        }
    };

    for lg in &lutris_games {
        if lg.service_id.is_empty() {
            continue;
        }
        let entry = if lg.service == "steam" {
            db::find_by_steam_id(db, &lg.service_id).ok().flatten()
        } else if lg.service == "gog" {
            db::find_gog_by_product_id(db, &lg.service_id).ok().flatten()
        } else {
            None
        };
        if let Some(entry) = entry {
            if entry.lutris_db_id.is_none() {
                let _ = db::set_lutris_db_id(db, entry.id, lg.id);
            }
        }
    }

    auto_match_by_title(db, save_dir, &lutris_games);

    let entries = db::load_all_games(db).unwrap_or_default();
    let ignored_ids = db::get_ignored_lutris_ids(db);
    let hidden_lutris_ids = db::get_hidden_lutris_ids(db);
    let mut by_lutris: std::collections::HashMap<i64, crate::models::GameEntry> = entries
        .into_iter()
        .filter_map(|e| e.lutris_db_id.map(|id| (id, e)))
        .collect();

    let mut games = Vec::with_capacity(lutris_games.len());
    for lg in &lutris_games {
        if ignored_ids.contains(&lg.id) {
            continue;
        }
        if let Some(entry) = by_lutris.remove(&lg.id) {
            match parser::load_game(&entry, save_dir) {
                Ok(mut game) => {
                    game.lutris_id = lg.id;
                    game.slug = lg.slug.clone();
                    game.playtime = lg.playtime;
                    game.lastplayed = lg.lastplayed;
                    game.lutris_name = lg.name.clone();
                    if game.name.is_empty() || game.name.starts_with("App ID:") {
                        game.name = lg.name.clone();
                    }
                    games.push(game);
                }
                Err(e) => eprintln!("Skipping {} ({}): {}", lg.name, lg.slug, e),
            }
        } else {
            let mut game = crate::models::unmatched_game(lg.id, &lg.name, &lg.slug, lg.playtime, lg.lastplayed);
            if hidden_lutris_ids.contains(&lg.id) {
                game.hidden = true;
            }
            games.push(game);
        }
    }
    games.sort_by(|a, b| a.sort_key().cmp(b.sort_key()));

    if shadps4_enabled {
        games.extend(build_shadps4_games(&db, save_dir));
    }
    games.sort_by(|a, b| a.sort_key().cmp(b.sort_key()));
    games
}

fn build_shadps4_games(db: &db::DbConn, save_dir: &str) -> Vec<Game> {
    let shad_games = discover_games();
    let mut games = Vec::new();

    for shad in &shad_games {
        let entry = db::find_by_steam_id(db, &shad.npwr_id).ok().flatten()
            .or_else(|| db::find_by_kind_platform(db, "ps4", &shad.serial).ok().flatten());
        let (db_id, title, hidden, logo_position, logo_size, sort_title, sgdb_id, shadps4_version, last_played) = match entry {
            Some(e) => (e.id, e.title, e.hidden, e.logo_position, e.logo_size, e.sort_title, e.sgdb_id.clone().unwrap_or_default(), e.shadps4_version.clone().unwrap_or_default(), e.last_played),
            None => {
                match db::add_game(db, "ps4", &shad.npwr_id, &shad.serial, &shad.title) {
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
            &title,
            hidden,
            &logo_position,
            logo_size,
            &sort_title,
            &sgdb_id,
            &shadps4_version,
            last_played,
            save_dir,
        );
        games.push(game);
    }

    games
}
