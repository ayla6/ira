use crate::game_loader;
use ira_config::Config;
use ira_db as db;
use ira_models::{Game, GameEntry, GameKind, SortMode};
use ira_platforms::cemu::discover_games_for_executable as discover_cemu_games_for_executable;
use ira_platforms::ps3::{
    discover_games_for_executable as discover_rpcs3_games_for_executable, load_rpcs3_game,
    Rpcs3GameMeta,
};
use ira_platforms::ps4::{discover_games_for_executable, load_shadps4_game, ShadPS4GameMeta};
use ira_platforms::retroachievements;
use ira_platforms::steam;
use ira_platforms::vita3k::discover_games_for_executable as discover_vita3k_games_for_executable;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct GameListProgress {
    pub status: String,
    pub completed: usize,
    pub total: usize,
}

#[derive(Clone)]
struct ProgressReporter {
    callback: Arc<dyn Fn(GameListProgress) + Send + Sync>,
    completed: Arc<AtomicUsize>,
    total: usize,
}

impl ProgressReporter {
    fn new(callback: Arc<dyn Fn(GameListProgress) + Send + Sync>, total: usize) -> Self {
        Self {
            callback,
            completed: Arc::new(AtomicUsize::new(0)),
            total,
        }
    }

    fn status(&self, status: impl Into<String>) {
        (self.callback)(GameListProgress {
            status: status.into(),
            completed: self.completed.load(Ordering::Relaxed),
            total: self.total,
        });
    }

    fn finish(&self, status: impl Into<String>) {
        let completed = self.completed.fetch_add(1, Ordering::Relaxed) + 1;
        (self.callback)(GameListProgress {
            status: status.into(),
            completed,
            total: self.total,
        });
    }
}

pub struct GameListOptions {
    pub shadps4_enabled: bool,
    pub rpcs3_enabled: bool,
    pub shadps4_executable: String,
    pub rpcs3_executable: String,
    pub vita3k_enabled: bool,
    pub vita3k_executable: String,
    pub cemu_enabled: bool,
    pub cemu_executable: String,
    pub steam_enabled: bool,
    pub sort_mode: SortMode,
    pub sort_descending: bool,
}

impl GameListOptions {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            shadps4_enabled: cfg.shadps4_enabled,
            rpcs3_enabled: cfg.rpcs3_enabled,
            shadps4_executable: cfg.shadps4_executable.clone(),
            rpcs3_executable: cfg.rpcs3_executable.clone(),
            vita3k_enabled: cfg.vita3k_enabled,
            vita3k_executable: cfg.vita3k_executable.clone(),
            cemu_enabled: cfg.cemu_enabled,
            cemu_executable: cfg.cemu_executable.clone(),
            steam_enabled: cfg.steam_enabled,
            sort_mode: cfg.sort_mode,
            sort_descending: cfg.sort_descending,
        }
    }
}

pub fn start_game_list_load(
    db: db::DbConn,
    save_dir: String,
    cfg: Config,
    sender: ira_models::AppSender,
) {
    let options = GameListOptions::from_config(&cfg);
    std::thread::spawn(move || {
        let progress_sender = sender.clone();
        let progress: Arc<dyn Fn(GameListProgress) + Send + Sync> = Arc::new(move |update| {
            let _ = progress_sender.send(ira_models::AppMessage::GamesLoadProgress {
                status: update.status,
                completed: update.completed,
                total: update.total,
            });
        });
        let games = build_game_list(&db, &save_dir, &cfg, &options, progress);
        let _ = sender.send(ira_models::AppMessage::GamesLoaded(games));
    });
}

pub fn build_game_list(
    db: &db::DbConn,
    save_dir: &str,
    cfg: &Config,
    options: &GameListOptions,
    progress: Arc<dyn Fn(GameListProgress) + Send + Sync>,
) -> Vec<Game> {
    let _span = tracing::info_span!("build_game_list").entered();

    let ra_any_console = cfg.any_console_enabled();
    let db = db.clone();
    let save_dir = save_dir.to_string();
    let cfg = cfg.clone();
    let sort_mode = options.sort_mode;
    let sort_descending = options.sort_descending;
    let total_sources = 1
        + usize::from(options.steam_enabled)
        + usize::from(options.shadps4_enabled)
        + usize::from(options.rpcs3_enabled)
        + usize::from(options.vita3k_enabled)
        + usize::from(options.cemu_enabled)
        + usize::from(ra_any_console);
    let reporter = ProgressReporter::new(progress, total_sources);
    reporter.status("Preparing game library…");

    std::thread::scope(|s| {
        let steam_discovery = if options.steam_enabled {
            let db = db.clone();
            let reporter = reporter.clone();
            Some(s.spawn(move || {
                let _s = tracing::info_span!("steam_discover").entered();
                reporter.status("Scanning Steam games…");
                let steam_games = steam::discover_games();
                if !steam_games.is_empty() {
                    cleanup_steam_entries(&db, &steam_games);
                }
                let steam_playtimes = steam::read_all_playtimes();
                reporter.finish("Loaded Steam games");
                (steam_games, steam_playtimes)
            }))
        } else {
            None
        };

        let db_native = db.clone();
        let save_dir_native = save_dir.clone();
        let native_reporter = reporter.clone();
        let native_handle = s.spawn(move || {
            let _s = tracing::info_span!("load_games_from_db").entered();
            native_reporter.status("Loading saved games…");
            let games = game_loader::load_games(&db_native, &save_dir_native);
            native_reporter.finish("Loaded saved games");
            games
        });

        let ps4_handle = if options.shadps4_enabled {
            let db_ps4 = db.clone();
            let save_dir_ps4 = save_dir.clone();
            let reporter = reporter.clone();
            Some(s.spawn(move || {
                let _s = tracing::info_span!("build_shadps4_games").entered();
                reporter.status("Scanning shadPS4 games…");
                let games =
                    build_shadps4_games(&db_ps4, &save_dir_ps4, &options.shadps4_executable);
                reporter.finish("Loaded shadPS4 games");
                games
            }))
        } else {
            None
        };

        let ps3_handle = if options.rpcs3_enabled {
            let db_ps3 = db.clone();
            let save_dir_ps3 = save_dir.clone();
            let reporter = reporter.clone();
            Some(s.spawn(move || {
                let _s = tracing::info_span!("build_rpcs3_games").entered();
                reporter.status("Scanning RPCS3 games…");
                let games = build_rpcs3_games(&db_ps3, &save_dir_ps3, &options.rpcs3_executable);
                reporter.finish("Loaded RPCS3 games");
                games
            }))
        } else {
            None
        };

        let vita3k_handle = if options.vita3k_enabled {
            let db_vita = db.clone();
            let save_dir_vita = save_dir.clone();
            let reporter = reporter.clone();
            Some(s.spawn(move || {
                let _s = tracing::info_span!("build_vita3k_games").entered();
                reporter.status("Scanning Vita3K games…");
                let games =
                    build_vita3k_games(&db_vita, &save_dir_vita, &options.vita3k_executable);
                reporter.finish("Loaded Vita3K games");
                games
            }))
        } else {
            None
        };

        let cemu_handle = if options.cemu_enabled {
            let db_cemu = db.clone();
            let save_dir_cemu = save_dir.clone();
            let reporter = reporter.clone();
            Some(s.spawn(move || {
                let _s = tracing::info_span!("build_cemu_games").entered();
                reporter.status("Scanning Cemu games…");
                let games = build_cemu_games(&db_cemu, &save_dir_cemu, &options.cemu_executable);
                reporter.finish("Loaded Cemu games");
                games
            }))
        } else {
            None
        };

        let ra_handle = if ra_any_console {
            let db_ra = db.clone();
            let save_dir_ra = save_dir.clone();
            let cfg_ra = cfg.clone();
            let reporter = reporter.clone();
            Some(s.spawn(move || {
                let _s = tracing::info_span!("build_ra_games").entered();
                reporter.status("Scanning ROM library…");
                let status_reporter = reporter.clone();
                let games = retroachievements::build_ra_games(
                    &db_ra,
                    &save_dir_ra,
                    &cfg_ra,
                    game_loader::load_game_fast,
                    move |status| status_reporter.status(status),
                );
                reporter.finish("Loaded ROM library");
                games
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
        if let Some(h) = ps3_handle {
            match h.join() {
                Ok(g) => games.extend(g),
                Err(_) => eprintln!("PS3 games thread panicked"),
            }
        }
        if let Some(h) = vita3k_handle {
            match h.join() {
                Ok(g) => games.extend(g),
                Err(_) => eprintln!("Vita3K games thread panicked"),
            }
        }
        if let Some(h) = cemu_handle {
            match h.join() {
                Ok(g) => games.extend(g),
                Err(_) => eprintln!("Cemu games thread panicked"),
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
            if sort_descending {
                ord.reverse()
            } else {
                ord
            }
        });

        games
    })
}

/// Fields from a DB entry needed to build console game metadata.
struct ConsoleDbMeta {
    db_id: i64,
    title: String,
    hidden: bool,
    logo_position: String,
    logo_size: i32,
    sort_title: String,
    sgdb_id: String,
    shadps4_version: String,
    last_played: i64,
}

impl ConsoleDbMeta {
    fn from_entry(e: &GameEntry, include_version: bool) -> Self {
        Self {
            db_id: e.id,
            title: e.title.clone(),
            hidden: e.hidden,
            logo_position: e.logo_position.clone(),
            logo_size: e.logo_size,
            sort_title: e.sort_title.clone(),
            sgdb_id: e.sgdb_id.clone().unwrap_or_default(),
            shadps4_version: if include_version {
                e.shadps4_version.clone()
            } else {
                String::new()
            },
            last_played: e.last_played,
        }
    }

    fn new_db_entry(id: i64, title: String) -> Self {
        Self {
            db_id: id,
            title,
            hidden: false,
            logo_position: ira_models::LogoPosition::BottomLeft.to_string(),
            logo_size: 50,
            sort_title: String::new(),
            sgdb_id: String::new(),
            shadps4_version: String::new(),
            last_played: 0,
        }
    }
}

/// Look up or create a DB entry for a discovered console game.
/// Tries `find_by_game_id` first, then `find_by_kind_platform` as fallback.
/// Logs DB errors instead of silently swallowing them.
fn find_or_create_console_entry(
    db: &db::DbConn,
    kind: GameKind,
    npwr_id: &str,
    serial: &str,
    title: &str,
    include_version: bool,
) -> Option<ConsoleDbMeta> {
    let entry = match db::find_by_game_id(db, npwr_id, serial) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("DB error looking up {kind} game {serial}: {e}");
            None
        }
    };
    let entry = match entry {
        Some(e) => Some(e),
        None => match db::find_by_kind_platform(db, kind, serial) {
            Ok(Some(e)) => Some(e),
            Ok(None) => None,
            Err(e) => {
                eprintln!("DB error looking up {kind} game {serial} by kind/platform: {e}");
                None
            }
        },
    };

    match entry {
        Some(e) => Some(ConsoleDbMeta::from_entry(&e, include_version)),
        None => {
            match db::add_game(
                db,
                kind,
                ira_models::TrophySource::Empty,
                "",
                npwr_id,
                serial,
                title,
            ) {
                Ok(id) => Some(ConsoleDbMeta::new_db_entry(id, title.to_string())),
                Err(e) => {
                    eprintln!("{kind}: failed to add {serial} to DB: {e}");
                    None
                }
            }
        }
    }
}

fn build_shadps4_games(db: &db::DbConn, save_dir: &str, executable: &str) -> Vec<Game> {
    let shad_games = discover_games_for_executable(executable);
    let mut games = Vec::new();

    for shad in &shad_games {
        let Some(meta) = find_or_create_console_entry(
            db,
            GameKind::Ps4,
            &shad.npwr_id,
            &shad.serial,
            &shad.title,
            true,
        ) else {
            continue;
        };

        let game = load_shadps4_game(
            shad,
            meta.db_id,
            &ShadPS4GameMeta {
                title: meta.title,
                hidden: meta.hidden,
                logo_position: meta.logo_position,
                logo_size: meta.logo_size,
                sort_title: meta.sort_title,
                sgdb_id: meta.sgdb_id,
                shadps4_version: meta.shadps4_version,
                last_played: meta.last_played,
            },
            save_dir,
        );
        games.push(game);
    }

    games
}

fn build_rpcs3_games(db: &db::DbConn, save_dir: &str, executable: &str) -> Vec<Game> {
    let ps3_games = discover_rpcs3_games_for_executable(executable);
    let mut games = Vec::new();

    for ps3_game in &ps3_games {
        let Some(meta) = find_or_create_console_entry(
            db,
            GameKind::Ps3,
            &ps3_game.npwr_id,
            &ps3_game.serial,
            &ps3_game.title,
            false,
        ) else {
            continue;
        };

        let game = load_rpcs3_game(
            ps3_game,
            meta.db_id,
            &Rpcs3GameMeta {
                title: meta.title,
                hidden: meta.hidden,
                logo_position: meta.logo_position,
                logo_size: meta.logo_size,
                sort_title: meta.sort_title,
                sgdb_id: meta.sgdb_id,
                last_played: meta.last_played,
            },
            save_dir,
        );
        games.push(game);
    }

    games
}

fn load_special_game(
    db: &db::DbConn,
    save_dir: &str,
    kind: GameKind,
    game_id: &str,
    title: &str,
    game_path: &std::path::Path,
    icon_path: &std::path::Path,
) -> Option<Game> {
    let meta = find_or_create_console_entry(db, kind, game_id, game_id, title, false)?;
    let entry = db::find_by_db_id(db, meta.db_id).ok().flatten()?;
    let mut game = game_loader::load_game_fast(&entry, save_dir).ok()?;
    if (game.name.is_empty() || game.name.starts_with("App ID:")) && !title.is_empty() {
        game.set_name(title);
    }
    game.game_path = game_path.to_string_lossy().into_owned();
    if icon_path.is_file() {
        game.icon_path = icon_path.to_string_lossy().into_owned();
    }
    Some(game)
}

fn build_vita3k_games(db: &db::DbConn, save_dir: &str, executable: &str) -> Vec<Game> {
    discover_vita3k_games_for_executable(executable)
        .iter()
        .filter_map(|game| {
            load_special_game(
                db,
                save_dir,
                GameKind::PsVita,
                &game.title_id,
                &game.title,
                &game.game_path,
                &game.icon_path,
            )
        })
        .collect()
}

fn build_cemu_games(db: &db::DbConn, save_dir: &str, executable: &str) -> Vec<Game> {
    discover_cemu_games_for_executable(executable)
        .iter()
        .filter_map(|game| {
            load_special_game(
                db,
                save_dir,
                GameKind::WiiU,
                &game.title_id,
                &game.title,
                &game.game_path,
                &game.icon_path,
            )
        })
        .collect()
}

fn cleanup_steam_entries(db: &db::DbConn, discovered: &[steam::SteamGame]) {
    let discovered_ids: std::collections::HashSet<String> =
        discovered.iter().map(|g| g.app_id.clone()).collect();

    let all_entries = match db::load_all_games(db) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("DB error loading all games for Steam cleanup: {e}");
            return;
        }
    };
    for entry in &all_entries {
        if entry.kind == GameKind::Steam && !discovered_ids.contains(&entry.steam_id) {
            if let Err(e) = db::remove_game(db, entry.id) {
                eprintln!(
                    "DB error removing stale Steam entry {}: {e}",
                    entry.steam_id
                );
            }
        }
    }
}

fn build_steam_games(
    db: &db::DbConn,
    save_dir: &str,
    steam_games: &[steam::SteamGame],
    playtimes: &std::collections::HashMap<String, (f64, i64)>,
) -> Vec<Game> {
    let mut games = Vec::new();

    for sg in steam_games {
        if sg.app_id.is_empty() {
            continue;
        }

        let entry = match db::find_by_steam_id(db, &sg.app_id) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("DB error looking up Steam app {}: {e}", sg.app_id);
                continue;
            }
        };

        let entry = match entry {
            Some(e) => e,
            None => {
                if let Err(e) = db::add_game(
                    db,
                    GameKind::Steam,
                    ira_models::TrophySource::SteamNative,
                    &sg.app_id,
                    "",
                    &sg.app_id,
                    &sg.name,
                ) {
                    eprintln!("Steam: failed to add {} to DB: {e}", sg.app_id);
                    continue;
                }
                match db::find_by_steam_id(db, &sg.app_id) {
                    Ok(e) => match e {
                        Some(e) => e,
                        None => {
                            eprintln!("Steam: entry for {} vanished after insert", sg.app_id);
                            continue;
                        }
                    },
                    Err(e) => {
                        eprintln!("DB error re-looking up Steam app {}: {e}", sg.app_id);
                        continue;
                    }
                }
            }
        };

        match game_loader::load_game_fast(&entry, save_dir) {
            Ok(mut game) => {
                if (game.name.is_empty() || game.name.starts_with("App ID:")) && !sg.name.is_empty()
                {
                    game.set_name(&sg.name);
                }
                game.game_path = sg.install_dir.to_string_lossy().into_owned();
                if let Some(&(pt, lp)) = playtimes.get(&sg.app_id) {
                    game.playtime = pt;
                    game.last_played = lp;
                }
                games.push(game);
            }
            Err(e) => eprintln!("Steam: failed to load {}: {e}", sg.app_id),
        }
    }

    games
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn test_progress_reporter_tracks_completed_sources() {
        let updates = Arc::new(Mutex::new(Vec::new()));
        let updates_clone = updates.clone();
        let callback: Arc<dyn Fn(GameListProgress) + Send + Sync> = Arc::new(move |update| {
            updates_clone.lock().unwrap().push(update);
        });
        let reporter = ProgressReporter::new(callback, 2);

        reporter.status("Scanning ROMs");
        reporter.finish("Loaded N64");
        reporter.finish("Loaded NDS");

        let updates = updates.lock().unwrap();
        assert_eq!(updates[0].completed, 0);
        assert_eq!(updates[1].completed, 1);
        assert_eq!(updates[2].completed, 2);
        assert_eq!(updates[2].total, 2);
    }

    #[test]
    fn test_game_list_options_from_config_copies_source_settings() {
        let mut cfg = Config::default();
        cfg.steam_enabled = true;
        cfg.shadps4_enabled = true;
        cfg.shadps4_executable = "/tmp/shadps4".to_string();
        cfg.sort_descending = true;

        let options = GameListOptions::from_config(&cfg);

        assert!(options.steam_enabled);
        assert!(options.shadps4_enabled);
        assert_eq!(options.shadps4_executable, "/tmp/shadps4");
        assert!(options.sort_descending);
    }
}
