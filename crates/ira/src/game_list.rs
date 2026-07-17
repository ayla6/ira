use ira_db as db;
use ira_config::Config;
use ira_models::{Game, SortMode};
use crate::game_loader;
use ira_platforms::ps4::{discover_games, load_shadps4_game, ShadPS4GameMeta};
use ira_platforms::retroachievements;
use ira_platforms::steam;

pub struct GameListOptions {
    pub shadps4_enabled: bool,
    pub steam_enabled: bool,
    pub sort_mode: SortMode,
    pub sort_descending: bool,
}

pub fn build_game_list(db: &db::DbConn, save_dir: &str, cfg: &Config, options: &GameListOptions) -> Vec<Game> {
    let _span = tracing::info_span!("build_game_list").entered();

    let ra_any_console = cfg.any_console_enabled();
    let db = db.clone();
    let save_dir = save_dir.to_string();
    let cfg = cfg.clone();
    let sort_mode = options.sort_mode;
    let sort_descending = options.sort_descending;

    std::thread::scope(|s| {
        let steam_discovery = if options.steam_enabled {
            let db = db.clone();
            Some(s.spawn(move || {
                let _s = tracing::info_span!("steam_discover").entered();
                let steam_games = steam::discover_games();
                if !steam_games.is_empty() {
                    cleanup_steam_entries(&db, &steam_games);
                }
                let steam_playtimes = steam::read_all_playtimes();
                (steam_games, steam_playtimes)
            }))
        } else {
            None
        };

        let db_native = db.clone();
        let save_dir_native = save_dir.clone();
        let native_handle = s.spawn(move || {
            let _s = tracing::info_span!("load_games_from_db").entered();
            let mut games = game_loader::load_games(&db_native, &save_dir_native);
            games.retain(|g| g.kind != ira_models::GameKind::Steam
                && g.kind != ira_models::GameKind::Ps4
                && g.kind != ira_models::GameKind::Retro);
            games
        });

        let ps4_handle = if options.shadps4_enabled {
            let db_ps4 = db.clone();
            let save_dir_ps4 = save_dir.clone();
            Some(s.spawn(move || {
                let _s = tracing::info_span!("build_shadps4_games").entered();
                build_shadps4_games(&db_ps4, &save_dir_ps4)
            }))
        } else {
            None
        };

        let ra_handle = if ra_any_console {
            let db_ra = db.clone();
            let save_dir_ra = save_dir.clone();
            let cfg_ra = cfg.clone();
            Some(s.spawn(move || {
                let _s = tracing::info_span!("build_ra_games").entered();
                retroachievements::build_ra_games(&db_ra, &save_dir_ra, &cfg_ra, game_loader::load_game)
            }))
        } else {
            None
        };

        let steam_games = if let Some(h) = steam_discovery {
            match h.join() {
                Ok((games, playtimes)) => {
                    let _s = tracing::info_span!("build_steam_games").entered();
                    build_steam_games(&db, &save_dir, &games, &playtimes)
                }
                Err(_) => {
                    eprintln!("Steam discovery thread panicked");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        let mut games = match native_handle.join() {
            Ok(g) => g,
            Err(_) => {
                eprintln!("Native games thread panicked");
                Vec::new()
            }
        };

        if let Some(h) = ps4_handle {
            match h.join() {
                Ok(g) => games.extend(g),
                Err(_) => eprintln!("PS4 games thread panicked"),
            }
        }
        games.extend(steam_games);
        if let Some(h) = ra_handle {
            match h.join() {
                Ok(g) => games.extend(g),
                Err(_) => eprintln!("RA games thread panicked"),
            }
        }

        games.sort_by(|a, b| {
            let ord = sort_mode.compare(a, b);
            if sort_descending { ord.reverse() } else { ord }
        });

        games
    })
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
