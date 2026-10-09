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
use ira_platforms::switch::discover_installed_games as discover_switch_installed_games;
use ira_platforms::vita3k::discover_games_for_executable as discover_vita3k_games_for_executable;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct GameListProgress {
    pub status: String,
    pub completed: usize,
    pub total: usize,
}

/// Progress sink shared by every source scan in one list build.
type ProgressCallback = Arc<dyn Fn(GameListProgress) + Send + Sync>;
/// Steam discovery results: found games plus cached playtimes by app id.
type SteamDiscovery = (
    Vec<steam::SteamGame>,
    std::collections::HashMap<String, (f64, i64)>,
);

#[derive(Clone)]
struct ProgressReporter {
    inner: Arc<ReporterInner>,
    /// Which source this handle reports for; `None` for statuses sent
    /// before the sources are registered.
    source: Option<usize>,
}

struct ReporterInner {
    callback: ProgressCallback,
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
    fn new(callback: ProgressCallback, total: usize) -> Self {
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
    pub switch_installed_enabled: bool,
    pub switch_executable: String,
    pub steam_enabled: bool,
    pub auto_reload_steam: bool,
    pub auto_reload_roms: bool,
    pub auto_reload_shadps4: bool,
    pub auto_reload_rpcs3: bool,
    pub auto_reload_vita3k: bool,
    pub auto_reload_cemu: bool,
    pub auto_reload_azahar: bool,
    pub auto_reload_switch: bool,
    /// Whether compressed Switch containers (NSZ/XCZ) scan as ROMs.
    pub compressed_switch_roms: bool,
    /// Whether the ROM library scan runs: any ROM-folder console is enabled,
    /// and at startup additionally the auto-reload setting. Independent of
    /// the RetroAchievements toggle, which only governs enrichment.
    pub rom_scan_enabled: bool,
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
            switch_installed_enabled: cfg.console("switch").enabled,
            switch_executable: cfg.console("switch").executable.clone(),
            steam_enabled: cfg.steam_enabled,
            auto_reload_steam: cfg.auto_reload_steam,
            auto_reload_roms: cfg.auto_reload_roms,
            auto_reload_shadps4: cfg.auto_reload_shadps4,
            auto_reload_rpcs3: cfg.auto_reload_rpcs3,
            auto_reload_vita3k: cfg.auto_reload_vita3k,
            auto_reload_cemu: cfg.auto_reload_cemu,
            auto_reload_azahar: cfg.auto_reload_azahar,
            auto_reload_switch: cfg.auto_reload_switch,
            compressed_switch_roms: cfg.compressed_switch_roms,
            rom_scan_enabled: cfg.any_console_enabled(),
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
        options.switch_installed_enabled &= options.auto_reload_switch;
        options.rom_scan_enabled &= options.auto_reload_roms;
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
        let progress: ProgressCallback = Arc::new(move |update| {
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

    cleanup_stale_rom_entries(db, cfg);
    let db = db.clone();
    let save_dir = save_dir.to_string();
    let cfg = cfg.clone();
    let sort_mode = options.sort_mode;
    let sort_descending = options.sort_descending;
    let reporter = ProgressReporter::new(progress, count_sources(options));
    reporter.status(crate::tr!("Preparing game library…"));

    std::thread::scope(|s| {
        let steam_discovery = if options.steam_enabled {
            spawn_steam_discovery(s, &db, &reporter)
        } else {
            None
        };

        let native_handle = spawn_native_load(s, &db, &save_dir, &reporter);

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

        let switch_installed_handle = spawn_source_scan(
            s,
            SourceScan {
                enabled: options.switch_installed_enabled,
                source: "build_switch_installed_games",
                scanning: crate::tr!("Scanning installed Switch titles…"),
                loaded: crate::tr!("Loaded installed Switch titles"),
            },
            &db,
            &save_dir,
            &reporter,
            move |db, save_dir| {
                build_switch_installed_games(
                    db,
                    save_dir,
                    &options.switch_executable,
                    options.compressed_switch_roms,
                )
            },
        );

        let ra_handle = if options.rom_scan_enabled {
            spawn_rom_scan(s, &db, &save_dir, &cfg, &reporter)
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
        drain_pending_sources(
            &mut games,
            [
                PendingSource::Thread("PS4", ps4_handle),
                PendingSource::Thread("PS3", ps3_handle),
                PendingSource::Thread("Vita3K", vita3k_handle),
                PendingSource::Thread("Cemu", cemu_handle),
                PendingSource::Thread("Azahar", azahar_handle),
                PendingSource::Thread("Switch", switch_installed_handle),
                PendingSource::Ready(steam_games),
                PendingSource::Thread("RA", ra_handle),
            ],
        );

        sort_games(&mut games, sort_mode, sort_descending);

        games
    })
}

/// How many progress-reporting sources a run has: the saved-games load plus
/// one per enabled scan. Kept next to the spawn sites so the count stays in
/// sync when sources are added.
fn count_sources(options: &GameListOptions) -> usize {
    1 + usize::from(options.steam_enabled)
        + usize::from(options.shadps4_enabled)
        + usize::from(options.rpcs3_enabled)
        + usize::from(options.vita3k_enabled)
        + usize::from(options.cemu_enabled)
        + usize::from(options.azahar_enabled)
        + usize::from(options.switch_installed_enabled)
        + usize::from(options.rom_scan_enabled)
}

/// Steam discovery plus stale-entry cleanup, off the UI thread like every
/// other source scan.
fn spawn_steam_discovery<'scope>(
    s: &'scope std::thread::Scope<'scope, '_>,
    db: &db::DbConn,
    reporter: &ProgressReporter,
) -> Option<std::thread::ScopedJoinHandle<'scope, SteamDiscovery>> {
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
}

/// The DB load is the floor of the list: it holds every row no enabled scan
/// re-discovers (a disabled source has no scan at all), and rescans
/// re-discover the rest. Source appends must therefore replace, not add.
fn spawn_native_load<'scope>(
    s: &'scope std::thread::Scope<'scope, '_>,
    db: &db::DbConn,
    save_dir: &str,
    reporter: &ProgressReporter,
) -> std::thread::ScopedJoinHandle<'scope, Vec<Game>> {
    let db = db.clone();
    let save_dir = save_dir.to_string();
    let reporter = reporter.for_source(crate::tr!("Loading saved games…"));
    s.spawn(move || {
        let _s = tracing::info_span!("load_games_from_db").entered();
        reporter.status(crate::tr!("Loading saved games…"));
        let games = game_loader::load_saved_games(&db, &save_dir);
        reporter.finish(crate::tr!("Loaded saved games"));
        games
    })
}

fn spawn_rom_scan<'scope>(
    s: &'scope std::thread::Scope<'scope, '_>,
    db: &db::DbConn,
    save_dir: &str,
    cfg: &Config,
    reporter: &ProgressReporter,
) -> Option<std::thread::ScopedJoinHandle<'scope, Vec<Game>>> {
    let db = db.clone();
    let save_dir = save_dir.to_string();
    let cfg = cfg.clone();
    let reporter = reporter.for_source(crate::tr!("Scanning ROM library…"));
    Some(s.spawn(move || {
        let _s = tracing::info_span!("build_ra_games").entered();
        reporter.status(crate::tr!("Scanning ROM library…"));
        let status_reporter = reporter.clone();
        let games = retroachievements::build_ra_games(
            &db,
            &save_dir,
            &cfg,
            game_loader::load_game_fast,
            move |status| status_reporter.status(status),
        );
        reporter.finish(crate::tr!("Loaded ROM library"));
        games
    }))
}

fn drain_pending_sources(games: &mut Vec<Game>, sources: [PendingSource<'_>; 8]) {
    for source in sources {
        match source {
            PendingSource::Ready(source_games) => {
                append_source_games(games, source_games);
            }
            PendingSource::Thread(name, handle) => {
                let Some(handle) = handle else { continue };
                match handle.join() {
                    Ok(source_games) => append_source_games(games, source_games),
                    Err(_) => eprintln!("{name} games thread panicked"),
                }
            }
        }
    }
}

fn sort_games(games: &mut [Game], sort_mode: SortMode, sort_descending: bool) {
    games.sort_by(|a, b| {
        let ord = sort_mode.compare(a, b);
        if sort_descending { ord.reverse() } else { ord }
    });
}

/// Merge one scan source into the accumulated list. Replacement is
/// unconditional on (db_id, variant_id): a full scan re-discovers rows the
/// DB load already produced (same db_id — the DB is unique per source id),
/// and appending them again would duplicate every entry. Scanned data wins
/// because it is fresher; variants (same db_id, different variant_id) stay
/// separate entries and replaced entries keep their position.
fn append_source_games(games: &mut Vec<Game>, discovered: Vec<Game>) {
    // Index the accumulated list once: the merge is linear overall instead
    // of a scan per discovered game.
    let mut index: std::collections::HashMap<(i64, Option<i64>), usize> = games
        .iter()
        .enumerate()
        .map(|(i, existing)| ((existing.db_id, existing.variant_id), i))
        .collect();
    for game in discovered {
        if let Some(&i) = index.get(&(game.db_id, game.variant_id)) {
            games[i] = game;
            continue;
        }
        index.insert((game.db_id, game.variant_id), games.len());
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

/// A ROM whose file disappeared keeps its row: clearing the path hides the
/// game from loads, but its title, images, hashes and play history stay so
/// the ROM scan can reattach the same content when the file comes back —
/// even renamed or from another folder.
fn cleanup_stale_rom_entries(db: &db::DbConn, cfg: &Config) {
    let entries = match db::load_games_by_kinds(db, &[GameKind::Retro, GameKind::Switch]) {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("Failed to load games for ROM cleanup: {error}");
            return;
        }
    };

    // Disc paths for rows without a primary ROM path, in one query instead
    // of one per row. A failure aborts the pass: like the per-row lookup
    // before it, an unreadable discs table must keep games, never clear.
    let empty_ids: Vec<i64> = entries
        .iter()
        .filter(|entry| entry.rom_path.is_empty())
        .map(|entry| entry.id)
        .collect();
    let discs_by_game = match db::get_discs_for_games(db, &empty_ids) {
        Ok(map) => map,
        Err(error) => {
            eprintln!("Failed to load discs for ROM cleanup: {error}");
            return;
        }
    };

    for entry in entries
        .iter()
        // Switch ROM entries share the ROM library's lifecycle; the two
        // kinds are exactly what the ROM scan manages.
        // (Already kind-filtered above; the match stays as documentation.)
        .filter(|entry| matches!(entry.kind, GameKind::Retro | GameKind::Switch))
        // Switch titles of an emulator's own library have no ROM file by
        // design — their identity is the emulator's title id — so the
        // stale-ROM cleanup must never treat them as vanished ROMs.
        .filter(|entry| {
            !(entry.platform_id == "switch"
                && entry.rom_path.is_empty()
                && ira_platforms::switch::is_title_id(&entry.native_id))
        })
        .filter(|entry| {
            let discs = discs_by_game.get(&entry.id);
            !rom_entry_has_file(cfg, entry, discs.map(Vec::as_slice).unwrap_or(&[]))
        })
    {
        if !entry.rom_path.is_empty() {
            if let Err(error) = db::set_rom_path(db, entry.id, "") {
                eprintln!("Failed to clear stale ROM path {}: {error}", entry.id);
            }
        }
        if let Err(error) = db::delete_discs(db, entry.id) {
            eprintln!("Failed to clear stale ROM discs {}: {error}", entry.id);
        }
    }
}

fn rom_entry_has_file(cfg: &Config, entry: &GameEntry, discs: &[ira_models::GameDisc]) -> bool {
    let paths: Vec<&str> = if entry.rom_path.is_empty() {
        discs.iter().map(|disc| disc.rom_path.as_str()).collect()
    } else {
        vec![entry.rom_path.as_str()]
    };

    if paths.is_empty() {
        return false;
    }

    let mut checked_path = false;
    for path in paths {
        let Some(path) = cfg.resolve_rom_path(entry.platform_id.id(), path) else {
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
    logo_position: ira_models::LogoPosition,
    logo_size: i32,
    sort_title: String,
    sgdb_id: String,
    shadps4_version: String,
    last_played: i64,
    rom_path: String,
}

impl ConsoleDbMeta {
    fn from_entry(e: &GameEntry, include_version: bool) -> Self {
        Self {
            db_id: e.id,
            title: e.title.clone(),
            hidden: e.hidden,
            logo_position: e.logo_position,
            logo_size: e.logo_size,
            sort_title: e.sort_title.clone(),
            sgdb_id: e.sgdb_id.clone().unwrap_or_default(),
            shadps4_version: if include_version {
                e.shadps4_version.clone()
            } else {
                String::new()
            },
            last_played: e.last_played,
            rom_path: e.rom_path.clone(),
        }
    }

    fn new_db_entry(id: i64, title: String) -> Self {
        Self {
            db_id: id,
            title,
            hidden: false,
            logo_position: ira_models::LogoPosition::BottomLeft,
            logo_size: 50,
            sort_title: String::new(),
            sgdb_id: String::new(),
            shadps4_version: String::new(),
            last_played: 0,
            rom_path: String::new(),
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
/// Resolution goes through `find_by_native_id(native, platform)` only;
/// upgraded installs may see one-time duplicate library entries for such
/// rows, which the user-level duplicate-game merge tooling reconciles.
/// Logs DB errors instead of
/// silently swallowing them. `trophy_id` carries the achievement identity
/// (NPWR on Sony consoles, empty elsewhere); `native_id` the product id.
fn find_or_create_console_entry(
    db: &db::DbConn,
    kind: GameKind,
    trophy_id: &str,
    native_id: &str,
    title: &str,
    include_version: bool,
) -> Option<ConsoleDbMeta> {
    let platform_id = kind.as_str();
    let entry = match db::find_by_native_id(db, native_id, platform_id) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("DB error looking up {kind} game {native_id}: {e}");
            None
        }
    };

    match entry {
        Some(e) => Some(ConsoleDbMeta::from_entry(&e, include_version)),
        None => {
            match db::add_game(
                db,
                db::NewGame {
                    kind,
                    trophy_source: ira_models::TrophySource::Empty,
                    steam_id: "",
                    trophy_id,
                    native_id,
                    platform_id: kind.as_str(),
                    title,
                },
            ) {
                Ok(id) => Some(ConsoleDbMeta::new_db_entry(id, title.to_string())),
                Err(e) => {
                    eprintln!("{kind}: failed to add {native_id} to DB: {e}");
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
/// DB metadata to `into_game`. The discovered location is persisted on the
/// row and rows the scan no longer finds are marked vanished, so DB-only
/// loads can tell deleted games from installed ones.
fn scan_console_games<D>(
    db: &db::DbConn,
    kind: GameKind,
    include_version: bool,
    discover: impl FnOnce(&str) -> Vec<D>,
    identity: impl Fn(&D) -> (&str, &str, &str, &std::path::Path),
    into_game: impl Fn(&D, ConsoleDbMeta) -> Game,
    executable: &str,
) -> Vec<Game> {
    let discovered = discover(executable);
    // One lookup for the kind's rows instead of one per discovered game.
    // The map is the scan's working copy: hits answer from it, misses
    // insert and join it, so later duplicates of the same install hit.
    // A failed preload leaves no map and each game takes the single-lookup
    // path, so a DB hiccup costs speed, never games.
    let mut known_entries = load_console_entry_map(db, kind);
    let mut seen: Vec<(String, String)> = Vec::with_capacity(discovered.len());
    let games = discovered
        .iter()
        .filter_map(|item| {
            let (trophy_id, native_id, title, game_path) = identity(item);
            let meta = match known_entries.as_mut() {
                Some(map) => find_or_create_console_entry_cached(
                    db,
                    kind,
                    trophy_id,
                    native_id,
                    title,
                    include_version,
                    map,
                ),
                None => find_or_create_console_entry(
                    db,
                    kind,
                    trophy_id,
                    native_id,
                    title,
                    include_version,
                ),
            }?;
            remember_console_location(db, kind, &meta, game_path);
            seen.push((native_id.to_string(), kind.as_str().to_string()));
            Some(into_game(item, meta))
        })
        .collect();
    reconcile_console_presence(db, kind, &seen);
    games
}

/// Every row of one console kind keyed by its native identity, or `None`
/// when the preload query itself failed (callers then use the single-lookup
/// path per game).
fn load_console_entry_map(
    db: &db::DbConn,
    kind: GameKind,
) -> Option<std::collections::HashMap<(String, ira_models::Platform), GameEntry>> {
    match db::load_games_by_kind(db, kind) {
        Ok(entries) => Some(
            entries
                .into_iter()
                .map(|entry| ((entry.native_id.clone(), entry.platform_id.clone()), entry))
                .collect(),
        ),
        Err(error) => {
            eprintln!("DB error preloading {kind} games: {error}");
            None
        }
    }
}

/// `find_or_create_console_entry` against a preloaded map: hits never touch
/// the DB, misses insert and join the map. On an insert conflict (a row that
/// appeared after the preload) the single lookup runs as fallback.
fn find_or_create_console_entry_cached(
    db: &db::DbConn,
    kind: GameKind,
    trophy_id: &str,
    native_id: &str,
    title: &str,
    include_version: bool,
    known_entries: &mut std::collections::HashMap<(String, ira_models::Platform), GameEntry>,
) -> Option<ConsoleDbMeta> {
    let platform_id = kind.as_str();
    let platform = ira_models::Platform::parse(platform_id);
    let key = (native_id.to_string(), platform.clone());
    if let Some(entry) = known_entries.get(&key) {
        return Some(ConsoleDbMeta::from_entry(entry, include_version));
    }
    match db::add_game(
        db,
        db::NewGame {
            kind,
            trophy_source: ira_models::TrophySource::Empty,
            steam_id: "",
            trophy_id,
            native_id,
            platform_id,
            title,
        },
    ) {
        Ok(id) => {
            let meta = ConsoleDbMeta::new_db_entry(id, title.to_string());
            // Keep the map total for duplicate installs of one scan: the
            // synthetic row mirrors a freshly inserted DB row exactly
            // (logo_size 50 is the schema default; GameEntry::default
            // carries 0), so a later duplicate reads what a re-SELECT
            // would have returned.
            known_entries.entry(key).or_insert_with(|| GameEntry {
                id,
                kind,
                trophy_source: ira_models::TrophySource::Empty,
                trophy_id: trophy_id.to_string(),
                native_id: native_id.to_string(),
                platform_id: platform.clone(),
                title: title.to_string(),
            logo_position: ira_models::LogoPosition::BottomLeft,
                logo_size: 50,
                ..Default::default()
            });
            Some(meta)
        }
        Err(error) => {
            eprintln!("{kind}: failed to add {native_id} to DB: {error}");
            // Conflict or race: the row may exist after all.
            match db::find_by_native_id(db, native_id, platform_id) {
                Ok(Some(entry)) => {
                    let meta = ConsoleDbMeta::from_entry(&entry, include_version);
                    known_entries.insert(key, entry);
                    Some(meta)
                }
                Ok(None) => None,
                Err(error) => {
                    eprintln!("DB error looking up {kind} game {native_id}: {error}");
                    None
                }
            }
        }
    }
}

/// Record where a scanned game lives. The DB-only startup load uses the
/// stored location to notice the game's files disappearing between scans.
fn remember_console_location(
    db: &db::DbConn,
    kind: GameKind,
    meta: &ConsoleDbMeta,
    game_path: &std::path::Path,
) {
    let game_path_str = game_path.to_string_lossy();
    if meta.rom_path == game_path_str {
        return;
    }
    if let Err(e) = db::set_rom_path(db, meta.db_id, &game_path_str) {
        eprintln!("Failed to persist {kind} game location: {e}");
    }
}

/// A scan that found at least one game of its kind is the presence
/// authority for that kind: rows it does not contain are deleted or moved
/// installs. They keep their row — playtime and trophies stay — but the
/// loader ignores them until a later scan finds the game again.
fn reconcile_console_presence(db: &db::DbConn, kind: GameKind, seen: &[(String, String)]) {
    if seen.is_empty() {
        // An empty result means the source could not be read at all (no
        // emulator, no config): nothing here proves any game vanished.
        return;
    }
    let entries = match db::load_games_by_kind(db, kind) {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("Failed to load games for {kind} presence reconcile: {error}");
            return;
        }
    };
    let seen: std::collections::HashSet<(&str, &str)> = seen
        .iter()
        .map(|(native_id, platform_id)| (native_id.as_str(), platform_id.as_str()))
        .collect();
    for entry in &entries {
        let present = seen.contains(&(entry.native_id.as_str(), entry.platform_id.id()));
        if present == entry.vanished {
            if let Err(error) = db::set_game_vanished(db, entry.id, !present) {
                eprintln!("Failed to mark {kind} game {}: {error}", entry.id);
            }
        }
    }
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
                shad.game_path.as_path(),
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
                ps3_game.game_path.as_path(),
            )
        },
        |ps3_game, meta| load_rpcs3_game(ps3_game, meta.db_id, &meta.into_rpcs3_meta(), save_dir),
        executable,
    )
}

/// Re-read a console row right after its metadata was resolved, so later
/// steps (location persistence, game build) see the current row.
fn read_console_entry(db: &db::DbConn, db_id: i64) -> Option<GameEntry> {
    db::find_by_db_id(db, db_id).unwrap_or_else(|e| {
        eprintln!("Failed to read game {db_id}: {e}");
        None
    })
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
    let meta = find_or_create_console_entry(db, kind, "", game_id, title, false)?;
    let entry = read_console_entry(db, meta.db_id)?;
    let mut game = match game_loader::load_game_fast(&entry, save_dir) {
        Ok(game) => game,
        Err(e) => {
            eprintln!("Failed to load game {}: {e}", meta.db_id);
            return None;
        }
    };
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

/// True when the data dir already holds an icon: scan-time extraction
/// must never replace downloaded or user-picked assets.
fn icon_exists(data_dir: &std::path::Path) -> bool {
    ira_parser::find_image_file(data_dir, ira_models::AssetType::Icon.file_base()).is_some()
}

/// Imports the title's iconTex.tga into `data/wiiu/{title_id}/` unless an
/// icon is already present. Returns an empty path when unavailable.
fn default_wiiu_icon(
    save_dir: &str,
    title_id: &str,
    game_path: &std::path::Path,
) -> std::path::PathBuf {
    let data_dir = ira_parser::wiiu_data_dir(save_dir, title_id);
    if icon_exists(&data_dir) {
        return std::path::PathBuf::new();
    }
    let icon_tga = game_path.join("meta").join("iconTex.tga");
    ira_parser::import_image_as_webp(&icon_tga, &data_dir, ira_models::AssetType::Icon.file_base()).unwrap_or_default()
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
    if icon_exists(&data_dir) {
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
    match ira_parser::find_image_file(&data_dir, ira_models::AssetType::Icon.file_base()) {
        Some(path) => path,
        None => std::path::PathBuf::new(),
    }
}

/// Builds games for the titles of the user's yuzu-family and
/// Ryujinx-family Switch emulators — both families at once, regardless of
/// which emulator is configured to launch ROM files. These titles have no
/// ROM file in the user's library: their identity is the title id, so a
/// game also present as a ROM file keeps its single entry and only gains
/// the emulator's clean title and icon — or, when the cache has none, the
/// title and icon decrypted from the installed control NCA.
fn build_switch_installed_games(
    db: &db::DbConn,
    save_dir: &str,
    executable: &str,
    compressed_switch_roms: bool,
) -> Vec<Game> {
    discover_switch_installed_games(executable, compressed_switch_roms)
        .iter()
        .filter_map(|installed| {
            let meta = find_or_create_console_entry(
                db,
                GameKind::Switch,
                "",
                &installed.title_id,
                &installed.title,
                false,
            )?;
            let entry = read_console_entry(db, meta.db_id)?;
            let data_dir = ira_parser::switch_data_dir(save_dir, meta.db_id);
            // Cache-miss fallback: the NAND-installed control NCA is
            // decrypted when the emulator cached no icon; its NACP also
            // supplies the application title when the cache had none.
            let nand = if installed.icon.is_none() && !icon_exists(&data_dir) {
                ira_platforms::switch::extract_installed_meta(executable, &installed.title_id)
            } else {
                None
            };
            let native_title = if installed.title.is_empty() {
                nand.as_ref().and_then(|control| control.title.clone())
            } else {
                Some(installed.title.clone())
            };
            let rename = switch_title_should_rename(&entry.title, &entry.rom_path);
            if let Some(native) = &native_title {
                if rename && entry.title != *native {
                    if let Err(e) = db::update_game_title(db, meta.db_id, native) {
                        eprintln!("Failed to set Switch title for game {}: {e}", meta.db_id);
                    }
                }
            }
            let mut game = match game_loader::load_game_fast(&entry, save_dir) {
                Ok(game) => game,
                Err(e) => {
                    eprintln!("Failed to load game {}: {e}", meta.db_id);
                    return None;
                }
            };
            if let Some(native) = &native_title {
                if rename
                    || game.name.is_empty()
                    || game_loader::is_placeholder_name(&game.name)
                {
                    game.set_name(native);
                }
            }
            // The icon comes from the source: installed NCAs are decrypted
            // with the user's keys, the emulator's cached JPEG only fills
            // in when that fails. SteamGridDB enrichment can still
            // override both later.
            default_switch_icon(
                save_dir,
                meta.db_id,
                installed.icon.as_deref(),
                nand.as_ref().and_then(|control| control.icon.as_deref()),
            );
            if let Some(path) =
                ira_parser::find_image_file(&data_dir, ira_models::AssetType::Icon.file_base())
            {
                game.icon_path = path.to_string_lossy().into_owned();
            }
            Some(game)
        })
        .collect()
}

/// True when the entry only carries a file-name-derived title: empty, a
/// placeholder, or exactly the ROM file's stem — the same rule the ROM
/// scan's Switch backfill uses.
fn switch_title_should_rename(current: &str, rom_path: &str) -> bool {
    let stem = std::path::Path::new(rom_path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    current.is_empty()
        || game_loader::is_placeholder_name(current)
        || (!stem.is_empty() && current == stem)
}

/// Writes the title's default icon into its data dir unless one is
/// already present: first the NAND-decrypted icon bytes, then the
/// emulator's cached icon JPEG. SteamGridDB enrichment can still
/// override both later.
fn default_switch_icon(
    save_dir: &str,
    db_id: i64,
    cached: Option<&std::path::Path>,
    nand: Option<&[u8]>,
) {
    let data_dir = ira_parser::switch_data_dir(save_dir, db_id);
    if icon_exists(&data_dir) {
        return;
    }
    if let Some(bytes) = nand {
        match ira_parser::encode_bytes_to_lossless_webp(bytes) {
            Some(webp) => {
                if std::fs::create_dir_all(&data_dir).is_ok()
                    && std::fs::write(data_dir.join("icon.webp"), webp).is_err()
                {
                    eprintln!("Failed to write Switch icon for game {db_id}");
                }
                return;
            }
            None => eprintln!("Failed to decode installed Switch icon for game {db_id}"),
        }
    }
    let Some(cached) = cached else {
        return;
    };
    if ira_parser::import_image_as_webp(cached, &data_dir, ira_models::AssetType::Icon.file_base()).is_none() {
        eprintln!("Failed to import Switch icon for game {db_id}");
    }
}

fn cleanup_steam_entries(db: &db::DbConn, discovered: &[steam::SteamGame]) {
    let discovered_ids: std::collections::HashSet<String> =
        discovered.iter().map(|g| g.app_id.clone()).collect();

    let all_entries = match db::load_games_by_kind(db, GameKind::Steam) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("DB error loading all games for Steam cleanup: {e}");
            return;
        }
    };
    for entry in &all_entries {
        if !discovered_ids.contains(&entry.steam_id) {
            if let Err(e) = db::remove_game(db, entry.id) {
                eprintln!(
                    "DB error removing stale Steam entry {}: {e}",
                    entry.steam_id
                );
            }
        }
    }
}

/// Look up a Steam row by app id, inserting it on first discovery.
/// Returns `None` (having logged) when the DB is unreadable.
fn find_or_create_steam_entry(db: &db::DbConn, app_id: &str, name: &str) -> Option<GameEntry> {
    match db::find_by_steam_id(db, app_id, GameKind::Steam.as_str()) {
        Ok(Some(entry)) => Some(entry),
        Ok(None) => {
            if let Err(e) = db::add_game(
                db,
                db::NewGame {
                    kind: GameKind::Steam,
                    trophy_source: ira_models::TrophySource::SteamNative,
                    steam_id: app_id,
                    trophy_id: "",
                    native_id: app_id,
                    platform_id: GameKind::Steam.as_str(),
                    title: name,
                },
            ) {
                eprintln!("Steam: failed to add {app_id} to DB: {e}");
                return None;
            }
            match db::find_by_steam_id(db, app_id, GameKind::Steam.as_str()) {
                Ok(Some(entry)) => Some(entry),
                Ok(None) => {
                    eprintln!("Steam: entry for {app_id} vanished after insert");
                    None
                }
                Err(e) => {
                    eprintln!("DB error re-looking up Steam app {app_id}: {e}");
                    None
                }
            }
        }
        Err(e) => {
            eprintln!("DB error looking up Steam app {app_id}: {e}");
            None
        }
    }
}

fn build_steam_games(
    db: &db::DbConn,
    save_dir: &str,
    steam_games: &[steam::SteamGame],
    playtimes: &std::collections::HashMap<String, (f64, i64)>,
) -> Vec<Game> {
    // One lookup for the Steam rows instead of one per discovered game.
    // Hits answer from the map; misses run the single-lookup path and join
    // it, so duplicate manifests of one app id hit.
    let mut known_steam_games: std::collections::HashMap<String, GameEntry> =
        match db::load_games_by_kind(db, GameKind::Steam) {
            Ok(entries) => entries
                .into_iter()
                .map(|entry| (entry.steam_id.clone(), entry))
                .collect(),
            Err(e) => {
                eprintln!("DB error preloading Steam games: {e}");
                std::collections::HashMap::new()
            }
        };
    let mut games = Vec::new();

    for sg in steam_games {
        if sg.app_id.is_empty() {
            continue;
        }

        let entry = match known_steam_games.get(&sg.app_id) {
            Some(entry) => entry.clone(),
            None => {
                let Some(entry) = find_or_create_steam_entry(db, &sg.app_id, &sg.name) else {
                    continue;
                };
                known_steam_games.insert(sg.app_id.clone(), entry.clone());
                entry
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

    /// A progress sink plus the collected updates, for `build_game_list`.
    fn test_progress_collector() -> (ProgressCallback, Arc<Mutex<Vec<GameListProgress>>>) {
        let updates = Arc::new(Mutex::new(Vec::new()));
        let sink = updates.clone();
        (
            Arc::new(move |update| {
                sink.lock().unwrap().push(update);
            }),
            updates,
        )
    }

    /// A Retro row whose ROM file is missing, with a `roms/` tree the config
    /// points at. The row survives with its metadata; only the stale path is
    /// cleared, so the game leaves the list but can reattach later.
    fn stale_rom_fixture() -> (tempfile::TempDir, db::DbConn, i64, Config) {
        let tmp = tempfile::tempdir().unwrap();
        let db = db::init_db(&tmp.path().join("ira.db").to_string_lossy());
        let game_id = db::add_game(
            &db,
            db::NewGame {
                kind: GameKind::Retro,
                trophy_source: ira_models::TrophySource::Empty,
                steam_id: "",
                trophy_id: "",
                native_id: "stale-game",
                platform_id: "saturn",
                title: "Stale game",
            },
        )
        .unwrap();
        db::set_rom_path(&db, game_id, "missing.rom").unwrap();
        std::fs::create_dir_all(tmp.path().join("roms/saturn")).unwrap();
        let cfg = Config {
            roms_folder: tmp.path().join("roms").to_string_lossy().into_owned(),
            ..Config::default()
        };
        (tmp, db, game_id, cfg)
    }

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
        // Config::default() enables every ROM-folder console, so the ROM
        // library scan is on independent of the RA integration flag.
        assert!(options.rom_scan_enabled);
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
        assert!(!options.rom_scan_enabled);
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

        append_source_games(&mut games, discovered);

        assert_eq!(games.len(), 3);
        assert_eq!(games[0].name, "Refreshed game");
        assert_eq!(games[1].name, "Saved variant");
        assert_eq!(games[2].name, "New game");
    }

    /// A rescan re-discovers DB rows the native load already listed (same
    /// db_id — the DB is unique per source id). The merge must therefore be
    /// unconditional: appending after a full scan replaces the DB-built
    /// entry — base and variant alike — instead of duplicating it.
    #[test]
    fn test_append_source_games_replaces_existing_db_and_variant_ids() {
        let mut games = vec![
            Game {
                db_id: 7,
                name: "DB-built".to_string(),
                ..Default::default()
            },
            Game {
                db_id: 7,
                variant_id: Some(1),
                name: "DB-built variant".to_string(),
                ..Default::default()
            },
        ];
        let discovered = vec![
            Game {
                db_id: 7,
                name: "Scanned".to_string(),
                ..Default::default()
            },
            Game {
                db_id: 7,
                variant_id: Some(1),
                name: "Scanned variant".to_string(),
                ..Default::default()
            },
        ];

        append_source_games(&mut games, discovered);

        assert_eq!(games.len(), 2);
        assert_eq!(games[0].db_id, 7);
        assert_eq!(games[0].variant_id, None);
        assert_eq!(games[0].name, "Scanned");
        assert_eq!(games[1].db_id, 7);
        assert_eq!(games[1].variant_id, Some(1));
        assert_eq!(games[1].name, "Scanned variant");
    }

    /// Games from unrelated DB rows never collide: distinct db_ids are both
    /// kept, in append order.
    #[test]
    fn test_append_source_games_keeps_distinct_db_ids() {
        let mut games = vec![Game {
            db_id: 1,
            name: "First".to_string(),
            ..Default::default()
        }];
        let discovered = vec![Game {
            db_id: 2,
            name: "Second".to_string(),
            ..Default::default()
        }];

        append_source_games(&mut games, discovered);

        assert_eq!(games.len(), 2);
        assert_eq!(games[0].db_id, 1);
        assert_eq!(games[0].name, "First");
        assert_eq!(games[1].db_id, 2);
        assert_eq!(games[1].name, "Second");
    }

    #[test]
    fn test_cleanup_stale_rom_entries_keeps_row_for_missing_file() {
        let (_tmp, db, game_id, cfg) = stale_rom_fixture();

        cleanup_stale_rom_entries(&db, &cfg);

        // The row survives with its metadata; only the stale path goes, so
        // the game leaves the list but can be reattached when it returns.
        let entry = db::find_by_db_id(&db, game_id).unwrap().unwrap();
        assert!(entry.rom_path.is_empty());
        assert_eq!(entry.title, "Stale game");
        assert!(game_loader::load_saved_games(&db, &cfg.save_dir).is_empty());
    }

    #[test]
    fn test_cleanup_stale_rom_entries_keeps_play_history() {
        let (_tmp, db, game_id, cfg) = stale_rom_fixture();
        db::record_session(&db, game_id, None, 1000, 1301).unwrap();

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
    fn test_reconcile_console_presence_marks_unseen_rows_only() {
        let tmp = tempfile::tempdir().unwrap();
        let db = db::init_db(&tmp.path().join("ira.db").to_string_lossy());
        let seen_id = db::add_game(
            &db,
            db::NewGame {
                kind: GameKind::Ps4,
                trophy_source: ira_models::TrophySource::Empty,
                steam_id: "",
                trophy_id: "NPWR0001",
                native_id: "CUSA00001",
                platform_id: "ps4",
                title: "Present game",
            },
        )
        .unwrap();
        let gone_id = db::add_game(
            &db,
            db::NewGame {
                kind: GameKind::Ps4,
                trophy_source: ira_models::TrophySource::Empty,
                steam_id: "",
                trophy_id: "NPWR0002",
                native_id: "CUSA00002",
                platform_id: "ps4",
                title: "Deleted game",
            },
        )
        .unwrap();
        let other_kind_id = db::add_game(
            &db,
            db::NewGame {
                kind: GameKind::WiiU,
                trophy_source: ira_models::TrophySource::Empty,
                steam_id: "",
                trophy_id: "",
                native_id: "00050000",
                platform_id: "wiiu",
                title: "Other kind",
            },
        )
        .unwrap();

        reconcile_console_presence(
            &db,
            GameKind::Ps4,
            &[("CUSA00001".to_string(), "ps4".to_string())],
        );

        assert!(
            !db::find_by_db_id(&db, seen_id).unwrap().unwrap().vanished,
            "a seen row stays present"
        );
        assert!(
            db::find_by_db_id(&db, gone_id).unwrap().unwrap().vanished,
            "a row the scan no longer finds is vanished, but its row stays"
        );
        assert!(db::find_by_db_id(&db, gone_id).unwrap().is_some());
        assert!(
            !db::find_by_db_id(&db, other_kind_id)
                .unwrap()
                .unwrap()
                .vanished,
            "other kinds are not this scan's business"
        );

        // The game coming back clears the verdict.
        reconcile_console_presence(
            &db,
            GameKind::Ps4,
            &[
                ("CUSA00001".to_string(), "ps4".to_string()),
                ("CUSA00002".to_string(), "ps4".to_string()),
            ],
        );
        assert!(!db::find_by_db_id(&db, gone_id).unwrap().unwrap().vanished);
    }

    #[test]
    fn test_reconcile_console_presence_ignores_empty_scans() {
        // An empty discovery means the source was unreadable (no emulator,
        // no config) — it never proves any game vanished.
        let tmp = tempfile::tempdir().unwrap();
        let db = db::init_db(&tmp.path().join("ira.db").to_string_lossy());
        let game_id = db::add_game(
            &db,
            db::NewGame {
                kind: GameKind::Ps4,
                trophy_source: ira_models::TrophySource::Empty,
                steam_id: "",
                trophy_id: "NPWR0003",
                native_id: "CUSA00003",
                platform_id: "ps4",
                title: "Untitled install",
            },
        )
        .unwrap();

        reconcile_console_presence(&db, GameKind::Ps4, &[]);

        assert!(!db::find_by_db_id(&db, game_id).unwrap().unwrap().vanished);
    }

    #[test]
    fn test_load_saved_games_ignores_deleted_console_games() {
        let tmp = tempfile::tempdir().unwrap();
        let db = db::init_db(&tmp.path().join("ira.db").to_string_lossy());
        // Deleted before any scan recorded the location, but marked by the
        // presence reconcile: never shows.
        let swept = db::add_game(
            &db,
            db::NewGame {
                kind: GameKind::Ps4,
                trophy_source: ira_models::TrophySource::Empty,
                steam_id: "",
                trophy_id: "NPWR0004",
                native_id: "CUSA00004",
                platform_id: "ps4",
                title: "Catherine Full Body",
            },
        )
        .unwrap();
        db::set_game_vanished(&db, swept, true).unwrap();
        // Deleted after a scan recorded the location: the missing file is
        // enough, even without a reconcile verdict yet.
        let missing_path = db::add_game(
            &db,
            db::NewGame {
                kind: GameKind::WiiU,
                trophy_source: ira_models::TrophySource::Empty,
                steam_id: "",
                trophy_id: "",
                native_id: "000500001010cd00",
                platform_id: "wiiu",
                title: "Bayonetta",
            },
        )
        .unwrap();
        db::set_rom_path(
            &db,
            missing_path,
            tmp.path().join("deleted/bayonetta").to_string_lossy().as_ref(),
        )
        .unwrap();
        // A live install keeps showing even when marked hidden by the user.
        let live_dir = tmp.path().join("live-game");
        std::fs::create_dir_all(&live_dir).unwrap();
        let live = db::add_game(
            &db,
            db::NewGame {
                kind: GameKind::WiiU,
                trophy_source: ira_models::TrophySource::Empty,
                steam_id: "",
                trophy_id: "",
                native_id: "000500001011e800",
                platform_id: "wiiu",
                title: "Live game",
            },
        )
        .unwrap();
        db::set_rom_path(&db, live, live_dir.to_string_lossy().as_ref()).unwrap();

        let loaded = game_loader::load_saved_games(&db, &tmp.path().to_string_lossy());

        let ids: Vec<i64> = loaded.iter().map(|game| game.db_id).collect();
        assert_eq!(ids, vec![live], "only the live install loads");
    }

    #[test]
    fn test_build_game_list_startup_loads_saved_games() {
        let tmp = tempfile::tempdir().unwrap();
        let db = db::init_db(&tmp.path().join("ira.db").to_string_lossy());
        let game_id = db::add_game(
            &db,
            db::NewGame {
                kind: GameKind::Retro,
                trophy_source: ira_models::TrophySource::Empty,
                steam_id: "",
                trophy_id: "",
                native_id: "saved-game",
                platform_id: "saturn",
                title: "Saved game",
            },
        )
        .unwrap();
        db::set_rom_path(&db, game_id, "saved.rom").unwrap();
        let rom_root = tmp.path().join("roms/saturn");
        std::fs::create_dir_all(&rom_root).unwrap();
        std::fs::write(rom_root.join("saved.rom"), b"rom").unwrap();

        let (progress, updates) = test_progress_collector();
        let mut cfg = Config {
            save_dir: tmp.path().to_string_lossy().into_owned(),
            roms_folder: tmp.path().join("roms").to_string_lossy().into_owned(),
            ..Config::default()
        };
        // The machine's real installed Switch titles must not leak into
        // this test's library; with the startup options the ROM scan is
        // off anyway (auto-reload defaults to disabled).
        cfg.console_mut("switch").enabled = false;
        let options = GameListOptions::for_startup(&cfg);

        let games = build_game_list(
            &db,
            &cfg.save_dir,
            &cfg,
            &options,
            progress,
        );

        assert_eq!(games.len(), 1);
        assert_eq!(games[0].name, "Saved game");
        assert_eq!(updates.lock().unwrap().last().unwrap().completed, 1);
    }

    /// A rescan with every source integration disabled still lists the
    /// library: the DB load is the floor of the list, and a disabled source
    /// has no scan that could re-discover its rows.
    #[test]
    fn test_build_game_list_full_scan_keeps_disabled_source_games() {
        let tmp = tempfile::tempdir().unwrap();
        let db = db::init_db(&tmp.path().join("ira.db").to_string_lossy());
        let rom_id = db::add_game(
            &db,
            db::NewGame {
                kind: GameKind::Retro,
                trophy_source: ira_models::TrophySource::Empty,
                steam_id: "",
                trophy_id: "",
                native_id: "saved-rom",
                platform_id: "saturn",
                title: "Saved rom",
            },
        )
        .unwrap();
        db::set_rom_path(&db, rom_id, "saved.iso").unwrap();
        let rom_root = tmp.path().join("roms/saturn");
        std::fs::create_dir_all(&rom_root).unwrap();
        std::fs::write(rom_root.join("saved.iso"), b"rom").unwrap();
        db::add_game(
            &db,
            db::NewGame {
                kind: GameKind::Wine,
                trophy_source: ira_models::TrophySource::Nge,
                steam_id: "570",
                trophy_id: "",
                native_id: "",
                platform_id: "gog",
                title: "Saved gog game",
            },
        )
        .unwrap();

        let (progress, _updates) = test_progress_collector();
        let mut cfg = Config {
            save_dir: tmp.path().to_string_lossy().into_owned(),
            roms_folder: tmp.path().join("roms").to_string_lossy().into_owned(),
            ..Config::default()
        };
        // Every source integration is off; Saturn is the only enabled
        // console and its ROM must stay listed even though no scan covers
        // it. The fixture uses a scanned extension so the ROM scan itself
        // recognizes the file.
        cfg.steam_enabled = false;
        cfg.ra_enabled = false;
        for def in ira_models::all_consoles() {
            cfg.console_mut(def.id).enabled = def.id == "saturn";
        }
        let options = GameListOptions::from_config(&cfg);

        let games = build_game_list(&db, &cfg.save_dir, &cfg, &options, progress);

        let names: Vec<&str> = games.iter().map(|game| game.name.as_str()).collect();
        assert!(names.contains(&"Saved rom"), "{names:?}");
        assert!(names.contains(&"Saved gog game"), "{names:?}");
    }

    /// A rescan discovers new ROMs even with the RetroAchievements
    /// integration off: the ROM library scans offline, and the file lands
    /// in the library named after its file stem with no RA source attached.
    #[test]
    fn test_build_game_list_full_scan_discovers_roms_with_ra_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let db = db::init_db(&tmp.path().join("ira.db").to_string_lossy());
        let rom_root = tmp.path().join("roms/saturn");
        std::fs::create_dir_all(&rom_root).unwrap();
        std::fs::write(rom_root.join("Quack Shot.iso"), b"rom").unwrap();

        let (progress, _updates) = test_progress_collector();
        let mut cfg = Config {
            save_dir: tmp.path().to_string_lossy().into_owned(),
            roms_folder: tmp.path().join("roms").to_string_lossy().into_owned(),
            ..Config::default()
        };
        cfg.ra_enabled = false;
        for def in ira_models::all_consoles() {
            cfg.console_mut(def.id).enabled = def.id == "saturn";
        }
        let options = GameListOptions::from_config(&cfg);

        let games = build_game_list(&db, &cfg.save_dir, &cfg, &options, progress);

        assert_eq!(games.len(), 1);
        assert_eq!(games[0].name, "Quack Shot");
        assert_eq!(games[0].kind, GameKind::Retro);
        assert_eq!(games[0].trophy_source, ira_models::TrophySource::Empty);
    }
}
