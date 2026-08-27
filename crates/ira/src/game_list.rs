use crate::game_loader;
use ira_config::Config;
use ira_db as db;
use ira_models::{Game, GameEntry, GameKind, SortMode};
use ira_platforms::azahar::discover_games_for_executable as discover_azahar_games_for_executable;
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
use std::sync::{Arc, Mutex};

const ROM_MIGRATION_THRESHOLD_SECONDS: i64 = 5 * 60;

#[derive(Clone, Debug)]
pub struct GameListProgress {
    pub status: String,
    pub completed: usize,
    pub total: usize,
}

#[derive(Clone)]
struct ProgressReporter {
    inner: Arc<ReporterInner>,
    /// Which source this handle reports for; `None` for statuses sent
    /// before the sources are registered.
    source: Option<usize>,
}

struct ReporterInner {
    callback: Arc<dyn Fn(GameListProgress) + Send + Sync>,
    total: usize,
    completed: AtomicUsize,
    /// One entry per source in spawn order. Sources run concurrently, so a
    /// bare "last message wins" label can name a source that finished long
    /// ago; the displayed status is always the oldest source still working.
    sources: Mutex<Vec<SourceProgress>>,
    fallback: Mutex<String>,
}

struct SourceProgress {
    status: String,
    done: bool,
}

impl ProgressReporter {
    fn new(callback: Arc<dyn Fn(GameListProgress) + Send + Sync>, total: usize) -> Self {
        Self {
            inner: Arc::new(ReporterInner {
                callback,
                total,
                completed: AtomicUsize::new(0),
                sources: Mutex::new(Vec::new()),
                fallback: Mutex::new(String::new()),
            }),
            source: None,
        }
    }

    /// Registers a source in spawn order and returns a handle that
    /// attributes its statuses and finishes to it.
    fn for_source(&self, initial: impl Into<String>) -> Self {
        let mut sources = self.inner.sources.lock().unwrap();
        sources.push(SourceProgress {
            status: initial.into(),
            done: false,
        });
        Self {
            inner: self.inner.clone(),
            source: Some(sources.len() - 1),
        }
    }

    fn status(&self, status: impl Into<String>) {
        match self.source {
            Some(idx) => self.inner.sources.lock().unwrap()[idx].status = status.into(),
            None => *self.inner.fallback.lock().unwrap() = status.into(),
        }
        self.push();
    }

    fn finish(&self, status: impl Into<String>) {
        let completed = self.inner.completed.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(idx) = self.source {
            let mut sources = self.inner.sources.lock().unwrap();
            sources[idx].done = true;
            sources[idx].status = status.into();
        }
        let status = self.displayed_status();
        (self.inner.callback)(GameListProgress {
            status,
            completed,
            total: self.inner.total,
        });
    }

    fn push(&self) {
        let status = self.displayed_status();
        (self.inner.callback)(GameListProgress {
            status,
            completed: self.inner.completed.load(Ordering::Relaxed),
            total: self.inner.total,
        });
    }

    /// The oldest source that has not finished yet; once everything is
    /// done, the last "Loaded …" message.
    fn displayed_status(&self) -> String {
        let sources = self.inner.sources.lock().unwrap();
        sources
            .iter()
            .find(|source| !source.done)
            .or_else(|| sources.iter().rev().find(|source| source.done))
            .map(|source| source.status.clone())
            .unwrap_or_else(|| self.inner.fallback.lock().unwrap().clone())
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
    pub azahar_enabled: bool,
    pub azahar_executable: String,
    pub steam_enabled: bool,
    pub auto_reload_steam: bool,
    pub auto_reload_roms: bool,
    pub auto_reload_shadps4: bool,
    pub auto_reload_rpcs3: bool,
    pub auto_reload_vita3k: bool,
    pub auto_reload_cemu: bool,
    pub auto_reload_azahar: bool,
    pub ra_enabled: bool,
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
            azahar_enabled: cfg.azahar_enabled,
            azahar_executable: cfg.azahar_executable.clone(),
            steam_enabled: cfg.steam_enabled,
            auto_reload_steam: cfg.auto_reload_steam,
            auto_reload_roms: cfg.auto_reload_roms,
            auto_reload_shadps4: cfg.auto_reload_shadps4,
            auto_reload_rpcs3: cfg.auto_reload_rpcs3,
            auto_reload_vita3k: cfg.auto_reload_vita3k,
            auto_reload_cemu: cfg.auto_reload_cemu,
            auto_reload_azahar: cfg.auto_reload_azahar,
            ra_enabled: cfg.ra_enabled,
            sort_mode: cfg.sort_mode,
            sort_descending: cfg.sort_descending,
        }
    }

    fn for_startup(cfg: &Config) -> Self {
        let mut options = Self::from_config(cfg);
        options.steam_enabled &= options.auto_reload_steam;
        options.shadps4_enabled &= options.auto_reload_shadps4;
        options.rpcs3_enabled &= options.auto_reload_rpcs3;
        options.vita3k_enabled &= options.auto_reload_vita3k;
        options.cemu_enabled &= options.auto_reload_cemu;
        options.azahar_enabled &= options.auto_reload_azahar;
        options.ra_enabled &= options.auto_reload_roms;
        options
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GameListLoadMode {
    Startup,
    FullScan,
}

pub fn start_game_list_load(
    db: db::DbConn,
    save_dir: String,
    cfg: Config,
    sender: ira_models::AppSender,
) {
    start_game_list_load_with_mode(db, save_dir, cfg, sender, GameListLoadMode::FullScan);
}

pub fn start_saved_game_load(
    db: db::DbConn,
    save_dir: String,
    cfg: Config,
    sender: ira_models::AppSender,
) {
    start_game_list_load_with_mode(db, save_dir, cfg, sender, GameListLoadMode::Startup);
}

fn start_game_list_load_with_mode(
    db: db::DbConn,
    save_dir: String,
    cfg: Config,
    sender: ira_models::AppSender,
    mode: GameListLoadMode,
) {
    let options = match mode {
        GameListLoadMode::Startup => GameListOptions::for_startup(&cfg),
        GameListLoadMode::FullScan => GameListOptions::from_config(&cfg),
    };
    std::thread::spawn(move || {
        let progress_sender = sender.clone();
        let progress: Arc<dyn Fn(GameListProgress) + Send + Sync> = Arc::new(move |update| {
            let _ = progress_sender.send(ira_models::AppMessage::GamesLoadProgress {
                status: update.status,
                completed: update.completed,
                total: update.total,
            });
        });
        let games = build_game_list_with_mode(&db, &save_dir, &cfg, &options, progress, mode);
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
    build_game_list_with_mode(
        db,
        save_dir,
        cfg,
        options,
        progress,
        GameListLoadMode::FullScan,
    )
}

fn build_game_list_with_mode(
    db: &db::DbConn,
    save_dir: &str,
    cfg: &Config,
    options: &GameListOptions,
    progress: Arc<dyn Fn(GameListProgress) + Send + Sync>,
    mode: GameListLoadMode,
) -> Vec<Game> {
    let _span = tracing::info_span!("build_game_list").entered();

    cleanup_stale_rom_entries(db, cfg);
    let ra_any_console = options.ra_enabled && cfg.any_console_enabled();
    let merge_discovered_games = mode == GameListLoadMode::Startup;
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
        + usize::from(options.azahar_enabled)
        + usize::from(ra_any_console);
    let reporter = ProgressReporter::new(progress, total_sources);
    reporter.status(crate::tr!("Preparing game library…"));

    std::thread::scope(|s| {
        let steam_discovery = if options.steam_enabled {
            let db = db.clone();
            let reporter = reporter.for_source(crate::tr!("Scanning Steam games…"));
            Some(s.spawn(move || {
                let _s = tracing::info_span!("steam_discover").entered();
                reporter.status(crate::tr!("Scanning Steam games…"));
                let steam_games = steam::discover_games();
                if !steam_games.is_empty() {
                    cleanup_steam_entries(&db, &steam_games);
                }
                let steam_playtimes = steam::read_all_playtimes();
                reporter.finish(crate::tr!("Loaded Steam games"));
                (steam_games, steam_playtimes)
            }))
        } else {
            None
        };

        let db_native = db.clone();
        let save_dir_native = save_dir.clone();
        let native_reporter = reporter.for_source(crate::tr!("Loading saved games…"));
        let native_handle = s.spawn(move || {
            let _s = tracing::info_span!("load_games_from_db").entered();
            native_reporter.status(crate::tr!("Loading saved games…"));
            let games = if merge_discovered_games {
                game_loader::load_saved_games(&db_native, &save_dir_native)
            } else {
                game_loader::load_games(&db_native, &save_dir_native)
            };
            native_reporter.finish(crate::tr!("Loaded saved games"));
            games
        });

        let ps4_handle = spawn_source_scan(
            s,
            SourceScan {
                enabled: options.shadps4_enabled,
                source: "build_shadps4_games",
                scanning: crate::tr!("Scanning shadPS4 games…"),
                loaded: crate::tr!("Loaded shadPS4 games"),
            },
            &db,
            &save_dir,
            &reporter,
            move |db, save_dir| build_shadps4_games(db, save_dir, &options.shadps4_executable),
        );

        let ps3_handle = spawn_source_scan(
            s,
            SourceScan {
                enabled: options.rpcs3_enabled,
                source: "build_rpcs3_games",
                scanning: crate::tr!("Scanning RPCS3 games…"),
                loaded: crate::tr!("Loaded RPCS3 games"),
            },
            &db,
            &save_dir,
            &reporter,
            move |db, save_dir| build_rpcs3_games(db, save_dir, &options.rpcs3_executable),
        );

        let vita3k_handle = spawn_source_scan(
            s,
            SourceScan {
                enabled: options.vita3k_enabled,
                source: "build_vita3k_games",
                scanning: crate::tr!("Scanning Vita3K games…"),
                loaded: crate::tr!("Loaded Vita3K games"),
            },
            &db,
            &save_dir,
            &reporter,
            move |db, save_dir| build_vita3k_games(db, save_dir, &options.vita3k_executable),
        );

        let cemu_handle = spawn_source_scan(
            s,
            SourceScan {
                enabled: options.cemu_enabled,
                source: "build_cemu_games",
                scanning: crate::tr!("Scanning Cemu games…"),
                loaded: crate::tr!("Loaded Cemu games"),
            },
            &db,
            &save_dir,
            &reporter,
            move |db, save_dir| build_cemu_games(db, save_dir, &options.cemu_executable),
        );

        let azahar_handle = spawn_source_scan(
            s,
            SourceScan {
                enabled: options.azahar_enabled,
                source: "build_azahar_games",
                scanning: crate::tr!("Scanning Azahar games…"),
                loaded: crate::tr!("Loaded Azahar games"),
            },
            &db,
            &save_dir,
            &reporter,
            move |db, save_dir| build_azahar_games(db, save_dir, &options.azahar_executable),
        );

        let ra_handle = if ra_any_console {
            let db_ra = db.clone();
            let save_dir_ra = save_dir.clone();
            let cfg_ra = cfg.clone();
            let reporter = reporter.for_source(crate::tr!("Scanning ROM library…"));
            Some(s.spawn(move || {
                let _s = tracing::info_span!("build_ra_games").entered();
                reporter.status(crate::tr!("Scanning ROM library…"));
                let status_reporter = reporter.clone();
                let games = retroachievements::build_ra_games(
                    &db_ra,
                    &save_dir_ra,
                    &cfg_ra,
                    game_loader::load_game_fast,
                    move |status| status_reporter.status(status),
                );
                reporter.finish(crate::tr!("Loaded ROM library"));
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

        // Drain every source in declaration order so the pre-sort entry
        // order stays stable across runs; Steam was materialized earlier
        // and lands between the emulator scans and the ROM library.
        for source in [
            PendingSource::Thread("PS4", ps4_handle),
            PendingSource::Thread("PS3", ps3_handle),
            PendingSource::Thread("Vita3K", vita3k_handle),
            PendingSource::Thread("Cemu", cemu_handle),
            PendingSource::Thread("Azahar", azahar_handle),
            PendingSource::Ready(steam_games),
            PendingSource::Thread("RA", ra_handle),
        ] {
            match source {
                PendingSource::Ready(source_games) => {
                    append_source_games(&mut games, source_games, merge_discovered_games);
                }
                PendingSource::Thread(name, handle) => {
                    let Some(handle) = handle else { continue };
                    match handle.join() {
                        Ok(source_games) => {
                            append_source_games(&mut games, source_games, merge_discovered_games)
                        }
                        Err(_) => eprintln!("{name} games thread panicked"),
                    }
                }
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

fn append_source_games(games: &mut Vec<Game>, discovered: Vec<Game>, merge: bool) {
    for game in discovered {
        if merge {
            if let Some(existing) = games.iter_mut().find(|existing| {
                existing.db_id == game.db_id && existing.variant_id == game.variant_id
            }) {
                *existing = game;
                continue;
            }
        }
        games.push(game);
    }
}

/// One pending discovery source to drain after the parallel scans: either a
/// scan thread still waiting to be joined, or games that were materialized
/// earlier (Steam enriches its discovery results right after joining).
enum PendingSource<'scope> {
    Thread(
        &'static str,
        Option<std::thread::ScopedJoinHandle<'scope, Vec<Game>>>,
    ),
    Ready(Vec<Game>),
}

fn cleanup_stale_rom_entries(db: &db::DbConn, cfg: &Config) {
    let entries = match db::load_all_games(db) {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("Failed to load games for ROM cleanup: {error}");
            return;
        }
    };

    for entry in entries
        .iter()
        .filter(|entry| entry.kind == GameKind::Retro)
        .filter(|entry| !rom_entry_has_file(db, cfg, entry))
    {
        if migration_playtime_seconds(db, entry) > ROM_MIGRATION_THRESHOLD_SECONDS {
            if !entry.rom_path.is_empty() {
                if let Err(error) = db::set_rom_path(db, entry.id, "") {
                    eprintln!("Failed to clear stale ROM path {}: {error}", entry.id);
                }
            }
            if let Err(error) = db::delete_discs(db, entry.id) {
                eprintln!("Failed to clear stale ROM discs {}: {error}", entry.id);
            }
        } else if let Err(error) = db::remove_game(db, entry.id) {
            eprintln!("Failed to remove stale ROM entry {}: {error}", entry.id);
        }
    }
}

fn migration_playtime_seconds(db: &db::DbConn, entry: &GameEntry) -> i64 {
    let cached_seconds = (entry.playtime.max(0.0) * 3600.0).round() as i64;
    let sessions_seconds = match db::get_sessions_for_game(db, entry.id, None) {
        Ok(sessions) => sessions
            .into_iter()
            .map(|session| session.duration_seconds.max(0))
            .sum(),
        Err(error) => {
            eprintln!(
                "Failed to read play history for ROM entry {}: {error}",
                entry.id
            );
            return cached_seconds.max(ROM_MIGRATION_THRESHOLD_SECONDS + 1);
        }
    };

    cached_seconds.max(sessions_seconds)
}

fn rom_entry_has_file(db: &db::DbConn, cfg: &Config, entry: &GameEntry) -> bool {
    let paths = if entry.rom_path.is_empty() {
        match db::get_discs(db, entry.id) {
            Ok(discs) => discs.into_iter().map(|disc| disc.rom_path).collect(),
            Err(error) => {
                eprintln!("Failed to load discs for ROM entry {}: {error}", entry.id);
                return true;
            }
        }
    } else {
        vec![entry.rom_path.clone()]
    };

    if paths.is_empty() {
        return false;
    }

    let mut checked_path = false;
    for path in paths {
        let Some(path) = cfg.resolve_rom_path(&entry.platform_id, &path) else {
            continue;
        };
        checked_path = true;
        if path.is_file() {
            return true;
        }
    }

    !checked_path
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

    fn into_shadps4_meta(self) -> ShadPS4GameMeta {
        ShadPS4GameMeta {
            title: self.title,
            hidden: self.hidden,
            logo_position: self.logo_position,
            logo_size: self.logo_size,
            sort_title: self.sort_title,
            sgdb_id: self.sgdb_id,
            shadps4_version: self.shadps4_version,
            last_played: self.last_played,
        }
    }

    fn into_rpcs3_meta(self) -> Rpcs3GameMeta {
        Rpcs3GameMeta {
            title: self.title,
            hidden: self.hidden,
            logo_position: self.logo_position,
            logo_size: self.logo_size,
            sort_title: self.sort_title,
            sgdb_id: self.sgdb_id,
            last_played: self.last_played,
        }
    }
}

/// Look up or create a DB entry for a discovered console game.
/// Resolution goes through `find_by_game_id(npwr_id, serial)` only; the
/// historical kind/platform fallback served rows predating npwr_id storage
/// and was removed pre-release — upgraded installs may see one-time
/// duplicate library entries for such rows, which the user-level
/// duplicate-game merge tooling reconciles. Logs DB errors instead of
/// silently swallowing them.
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

/// One emulator-source scan descriptor: whether to run it and what to
/// report while it runs.
struct SourceScan {
    enabled: bool,
    source: &'static str,
    scanning: String,
    loaded: String,
}

/// Spawns one emulator-source scan on the scoped thread pool when enabled,
/// reporting start/finish through the shared reporter.
fn spawn_source_scan<'scope>(
    s: &'scope std::thread::Scope<'scope, '_>,
    scan: SourceScan,
    db: &db::DbConn,
    save_dir: &str,
    reporter: &ProgressReporter,
    build: impl FnOnce(&db::DbConn, &str) -> Vec<Game> + Send + 'scope,
) -> Option<std::thread::ScopedJoinHandle<'scope, Vec<Game>>> {
    if !scan.enabled {
        return None;
    }
    let db = db.clone();
    let save_dir = save_dir.to_string();
    let reporter = reporter.for_source(scan.scanning.clone());
    Some(s.spawn(move || {
        let _span = tracing::info_span!("source_scan", scan.source).entered();
        reporter.status(scan.scanning);
        let games = build(&db, &save_dir);
        reporter.finish(scan.loaded);
        games
    }))
}

/// Shared body of the shadPS4/RPCS3 scans: run `discover` over the emulator
/// executable, reconcile each hit against the DB, then hand the hit plus its
/// DB metadata to `into_game`.
fn scan_console_games<D>(
    db: &db::DbConn,
    kind: GameKind,
    include_version: bool,
    discover: impl FnOnce(&str) -> Vec<D>,
    identity: impl Fn(&D) -> (&str, &str, &str),
    into_game: impl Fn(&D, ConsoleDbMeta) -> Game,
    executable: &str,
) -> Vec<Game> {
    discover(executable)
        .iter()
        .filter_map(|item| {
            let (npwr_id, serial, title) = identity(item);
            let meta =
                find_or_create_console_entry(db, kind, npwr_id, serial, title, include_version)?;
            Some(into_game(item, meta))
        })
        .collect()
}

fn build_shadps4_games(db: &db::DbConn, save_dir: &str, executable: &str) -> Vec<Game> {
    scan_console_games(
        db,
        GameKind::Ps4,
        true,
        discover_games_for_executable,
        |shad| {
            (
                shad.npwr_id.as_str(),
                shad.serial.as_str(),
                shad.title.as_str(),
            )
        },
        |shad, meta| load_shadps4_game(shad, meta.db_id, &meta.into_shadps4_meta(), save_dir),
        executable,
    )
}

fn build_rpcs3_games(db: &db::DbConn, save_dir: &str, executable: &str) -> Vec<Game> {
    scan_console_games(
        db,
        GameKind::Ps3,
        false,
        discover_rpcs3_games_for_executable,
        |ps3_game| {
            (
                ps3_game.npwr_id.as_str(),
                ps3_game.serial.as_str(),
                ps3_game.title.as_str(),
            )
        },
        |ps3_game, meta| load_rpcs3_game(ps3_game, meta.db_id, &meta.into_rpcs3_meta(), save_dir),
        executable,
    )
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
    if (game.name.is_empty() || game_loader::is_placeholder_name(&game.name)) && !title.is_empty() {
        game.set_name(title);
    }
    // Persist the discovered location: everything built later straight from
    // the DB row (context menu, native-icon restore) relies on it.
    let game_path_str = game_path.to_string_lossy().into_owned();
    if entry.rom_path != game_path_str {
        if let Err(e) = db::set_rom_path(db, meta.db_id, &game_path_str) {
            eprintln!("Failed to persist console game location: {e}");
        }
    }
    game.game_path = game_path_str;
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
            // Cemu keeps each title's icon as meta/iconTex.tga inside the
            // game folder; import it into the data dir so it acts as the
            // default icon, with SteamGridDB enrichment overriding later.
            let icon = default_wiiu_icon(save_dir, &game.title_id, &game.game_path);
            load_special_game(
                db,
                save_dir,
                GameKind::WiiU,
                &game.title_id,
                &game.title,
                &game.game_path,
                &icon,
            )
        })
        .collect()
}

/// Imports the title's iconTex.tga into `data/wiiu/{title_id}/` unless an
/// icon is already present. Returns an empty path when unavailable.
fn default_wiiu_icon(
    save_dir: &str,
    title_id: &str,
    game_path: &std::path::Path,
) -> std::path::PathBuf {
    let data_dir = ira_parser::wiiu_data_dir(save_dir, title_id);
    if ira_parser::find_image_file(&data_dir, "icon").is_some() {
        return std::path::PathBuf::new();
    }
    let icon_tga = game_path.join("meta").join("iconTex.tga");
    ira_parser::import_image_as_webp(&icon_tga, &data_dir, "icon").unwrap_or_default()
}

fn build_azahar_games(db: &db::DbConn, save_dir: &str, executable: &str) -> Vec<Game> {
    discover_azahar_games_for_executable(executable)
        .iter()
        .filter_map(|game| {
            // 3DS SMDH icons live inside the ExeFS; extract them into the
            // game's data dir so they act as the default icon, with
            // SteamGridDB enrichment overriding later if matched.
            load_special_game(
                db,
                save_dir,
                GameKind::ThreeDS,
                &game.title_id,
                &game.title,
                &game.game_path,
                &default_azahar_icon(save_dir, &game.title_id, game.icon.as_deref()),
            )
        })
        .collect()
}

/// Extracts the SMDH icon into `data/3ds/{title_id}/icon.png` (converted to
/// lossless WebP) unless one is already present. Returns an empty path when
/// the ROM carries no readable icon.
fn default_azahar_icon(save_dir: &str, title_id: &str, icon: Option<&[u8]>) -> std::path::PathBuf {
    let Some(icon) = icon else {
        return std::path::PathBuf::new();
    };
    let data_dir = ira_parser::three_ds_data_dir(save_dir, title_id);
    if ira_parser::find_image_file(&data_dir, "icon").is_some() {
        return std::path::PathBuf::new();
    }
    if std::fs::create_dir_all(&data_dir).is_err() {
        return std::path::PathBuf::new();
    }
    let png = data_dir.join("icon.png");
    if let Err(e) = ira_parser::save_rgb565_png(&png, 48, 48, icon) {
        eprintln!("Failed to write 3DS icon for {title_id}: {e}");
        return std::path::PathBuf::new();
    }
    ira_parser::convert_to_lossless_webp(&png);
    match ira_parser::find_image_file(&data_dir, "icon") {
        Some(path) => path,
        None => std::path::PathBuf::new(),
    }
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
                if (game.name.is_empty() || game_loader::is_placeholder_name(&game.name))
                    && !sg.name.is_empty()
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
        let n64 = reporter.for_source("Scanning N64");
        let nds = reporter.for_source("Scanning NDS");

        n64.status("Scanning N64 ROMs");
        nds.status("Scanning NDS ROMs");
        // Both sources are running; the label names the oldest unfinished
        // one, not the last thread that happened to speak.
        assert_eq!(
            updates.lock().unwrap().last().unwrap().status,
            "Scanning N64 ROMs"
        );

        n64.finish("Loaded N64");
        let update = updates.lock().unwrap().last().unwrap().clone();
        assert_eq!(update.status, "Scanning NDS ROMs");
        assert_eq!(update.completed, 1);

        nds.finish("Loaded NDS");
        let update = updates.lock().unwrap().last().unwrap().clone();
        assert_eq!(update.status, "Loaded NDS");
        assert_eq!(update.completed, 2);
        assert_eq!(update.total, 2);
    }

    /// Scan-time icon extraction must never replace an icon that is
    /// already on disk — downloaded or user-picked assets always win.
    #[test]
    fn test_default_azahar_icon_keeps_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().to_str().unwrap();
        let title_id = "0004000000123400";
        let data_dir = ira_parser::three_ds_data_dir(save_dir, title_id);
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(data_dir.join("icon.webp"), b"keep").unwrap();

        let result = default_azahar_icon(save_dir, title_id, Some(&[0u8; 48 * 48 * 2]));

        assert!(result.as_os_str().is_empty());
        assert_eq!(std::fs::read(data_dir.join("icon.webp")).unwrap(), b"keep");
    }

    #[test]
    fn test_game_list_options_from_config_copies_source_settings() {
        let cfg = Config {
            steam_enabled: true,
            shadps4_enabled: true,
            shadps4_executable: "/tmp/shadps4".to_string(),
            sort_descending: true,
            ..Config::default()
        };

        let options = GameListOptions::from_config(&cfg);

        assert!(options.steam_enabled);
        assert!(!options.ra_enabled);
        assert!(options.shadps4_enabled);
        assert_eq!(options.shadps4_executable, "/tmp/shadps4");
        assert!(options.sort_descending);
    }

    #[test]
    fn test_game_list_options_for_startup_applies_each_reload_setting() {
        let cfg = Config {
            steam_enabled: true,
            shadps4_enabled: true,
            rpcs3_enabled: true,
            vita3k_enabled: true,
            cemu_enabled: true,
            azahar_enabled: true,
            ra_enabled: true,
            auto_reload_steam: false,
            auto_reload_roms: false,
            auto_reload_shadps4: true,
            auto_reload_rpcs3: false,
            auto_reload_vita3k: true,
            auto_reload_cemu: false,
            auto_reload_azahar: true,
            ..Config::default()
        };

        let options = GameListOptions::for_startup(&cfg);

        assert!(!options.steam_enabled);
        assert!(!options.ra_enabled);
        assert!(options.shadps4_enabled);
        assert!(!options.rpcs3_enabled);
        assert!(options.vita3k_enabled);
        assert!(!options.cemu_enabled);
        assert!(options.azahar_enabled);
    }

    #[test]
    fn test_append_source_games_replaces_saved_base_and_keeps_variants() {
        let mut games = vec![
            Game {
                db_id: 1,
                name: "Saved game".to_string(),
                ..Default::default()
            },
            Game {
                db_id: 1,
                variant_id: Some(2),
                name: "Saved variant".to_string(),
                ..Default::default()
            },
        ];
        let discovered = vec![
            Game {
                db_id: 1,
                name: "Refreshed game".to_string(),
                ..Default::default()
            },
            Game {
                db_id: 3,
                name: "New game".to_string(),
                ..Default::default()
            },
        ];

        append_source_games(&mut games, discovered, true);

        assert_eq!(games.len(), 3);
        assert_eq!(games[0].name, "Refreshed game");
        assert_eq!(games[1].name, "Saved variant");
        assert_eq!(games[2].name, "New game");
    }

    #[test]
    fn test_cleanup_stale_rom_entries_removes_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let db = db::init_db(&tmp.path().join("ira.db").to_string_lossy());
        let game_id = db::add_game(
            &db,
            GameKind::Retro,
            ira_models::TrophySource::Empty,
            "",
            "stale-game",
            "saturn",
            "Stale game",
        )
        .unwrap();
        db::set_rom_path(&db, game_id, "missing.rom").unwrap();
        let rom_root = tmp.path().join("roms/saturn");
        std::fs::create_dir_all(&rom_root).unwrap();
        let cfg = Config {
            roms_folder: tmp.path().join("roms").to_string_lossy().into_owned(),
            ..Config::default()
        };

        cleanup_stale_rom_entries(&db, &cfg);

        assert!(db::find_by_db_id(&db, game_id).unwrap().is_none());
    }

    #[test]
    fn test_cleanup_stale_rom_entries_preserves_play_history_for_migration() {
        let tmp = tempfile::tempdir().unwrap();
        let db = db::init_db(&tmp.path().join("ira.db").to_string_lossy());
        let game_id = db::add_game(
            &db,
            GameKind::Retro,
            ira_models::TrophySource::Empty,
            "",
            "stale-game",
            "saturn",
            "Stale game",
        )
        .unwrap();
        db::set_rom_path(&db, game_id, "missing.rom").unwrap();
        db::record_session(&db, game_id, None, 1000, 1301).unwrap();
        let rom_root = tmp.path().join("roms/saturn");
        std::fs::create_dir_all(&rom_root).unwrap();
        let cfg = Config {
            roms_folder: tmp.path().join("roms").to_string_lossy().into_owned(),
            ..Config::default()
        };

        cleanup_stale_rom_entries(&db, &cfg);

        let entry = db::find_by_db_id(&db, game_id).unwrap().unwrap();
        assert!(entry.rom_path.is_empty());
        assert_eq!(
            db::get_sessions_for_game(&db, game_id, None).unwrap().len(),
            1
        );
        assert!(game_loader::load_saved_games(&db, &cfg.save_dir).is_empty());
    }

    #[test]
    fn test_build_game_list_startup_loads_saved_games() {
        let tmp = tempfile::tempdir().unwrap();
        let db = db::init_db(&tmp.path().join("ira.db").to_string_lossy());
        let game_id = db::add_game(
            &db,
            GameKind::Retro,
            ira_models::TrophySource::Empty,
            "",
            "saved-game",
            "saturn",
            "Saved game",
        )
        .unwrap();
        db::set_rom_path(&db, game_id, "saved.rom").unwrap();
        let rom_root = tmp.path().join("roms/saturn");
        std::fs::create_dir_all(&rom_root).unwrap();
        std::fs::write(rom_root.join("saved.rom"), b"rom").unwrap();

        let updates = Arc::new(Mutex::new(Vec::new()));
        let updates_clone = updates.clone();
        let progress: Arc<dyn Fn(GameListProgress) + Send + Sync> = Arc::new(move |update| {
            updates_clone.lock().unwrap().push(update);
        });
        let cfg = Config {
            save_dir: tmp.path().to_string_lossy().into_owned(),
            roms_folder: tmp.path().join("roms").to_string_lossy().into_owned(),
            ..Config::default()
        };
        let options = GameListOptions::from_config(&cfg);

        let games = build_game_list_with_mode(
            &db,
            &cfg.save_dir,
            &cfg,
            &options,
            progress,
            GameListLoadMode::Startup,
        );

        assert_eq!(games.len(), 1);
        assert_eq!(games[0].name, "Saved game");
        assert_eq!(updates.lock().unwrap().last().unwrap().completed, 1);
    }
}
