mod bench;
mod config;
mod db;
mod gamesetup;
mod images;
mod lutris;
mod parser;
mod steam;
mod strings;
mod ui;
mod watcher;

use crate::parser::Game;
use crate::steam::SteamClient;
use crate::ui::{build_ui, handle_app_message, restore_content, SharedState};
use crate::watcher::AchievementWatcher;
use gtk4::glib;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};

pub enum AppMessage {
    EnrichedGame(Game),
    NewGame(Game),
    WatcherGameUpdated(Game),
    AddGameError(String),
    GameRemoved { app_id: String },
    GameStopped(i64),
    /// Fired by the LutrisWatcher when pga.db changes (debounced).
    /// Carries (lutris_id, playtime, lastplayed) for every Lutris game.
    LutrisDataChanged(Vec<(i64, f64, i64)>),
    /// Initial game list loaded in the background.
    GamesLoaded(Vec<Game>),
}

// === Pipe-based wakeup: eliminates 50ms polling ===
//
// AppSender wraps an mpsc::Sender and a pipe write-end. On send(), it pushes
// the message through the channel AND writes a byte to the pipe, which wakes
// up the main loop's GSource watching the read-end. Zero idle CPU.

extern "C" {
    fn g_unix_fd_source_new(fd: i32, condition: u32) -> *mut std::ffi::c_void;
}

/// Data passed to the GSource callback. Boxed and leaked — the destroy
/// callback frees it when the source is removed (on app exit).
struct MainLoopData {
    read_fd: i32,
    receiver: RefCell<mpsc::Receiver<AppMessage>>,
    state: SharedState,
}

/// Trampoline for the GSource callback.
/// Note: g_unix_fd_source_new's dispatch calls the callback as
/// GUnixFDSourceFunc(fd, condition, user_data), not GSourceFunc(user_data).
unsafe extern "C" fn source_trampoline(
    _fd: i32,
    _condition: u32,
    data: glib::ffi::gpointer,
) -> glib::ffi::gboolean {
    let data: &MainLoopData = &*(data as *const MainLoopData);
    // Drain the pipe so the source doesn't re-fire immediately
    let mut buf = [0u8; 256];
    while libc::read(data.read_fd, buf.as_mut_ptr() as *mut _, 256) > 0 {}
    // Drain all pending messages
    while let Ok(msg) = data.receiver.borrow_mut().try_recv() {
        handle_app_message(&data.state, msg);
    }
    glib::ffi::G_SOURCE_CONTINUE
}

unsafe extern "C" fn source_destroy(data: glib::ffi::gpointer) {
    let _ = Box::from_raw(data as *mut MainLoopData);
}

pub struct AppSender {
    tx: mpsc::Sender<AppMessage>,
    fd: std::os::unix::io::RawFd,
}

impl Clone for AppSender {
    fn clone(&self) -> Self {
        let new_fd = unsafe { libc::dup(self.fd) };
        Self { tx: self.tx.clone(), fd: new_fd }
    }
}

impl AppSender {
    pub fn send(&self, msg: AppMessage) -> Result<(), mpsc::SendError<AppMessage>> {
        let result = self.tx.send(msg);
        if result.is_ok() {
            let byte = [1u8; 1];
            unsafe { libc::write(self.fd, byte.as_ptr() as *const _, 1); }
        }
        result
    }
}

impl Drop for AppSender {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd); }
    }
}

unsafe impl Send for AppSender {}

fn main() {
    let app = adw::Application::new(
        Some("com.github.achievement.viewer"),
        gio::ApplicationFlags::empty(),
    );

    let state_holder: Rc<RefCell<Option<SharedState>>> = Rc::new(RefCell::new(None));

    app.connect_activate({
        let state_holder = state_holder.clone();
        move |app| {
            if let Some(state) = state_holder.borrow().as_ref() {
                let win = state.borrow().window.clone();
                win.present();
                restore_content(state);
                return;
            }
            let state = activate(app);
            *state_holder.borrow_mut() = Some(state);
        }
    });

    app.run();
}

fn activate(app: &adw::Application) -> SharedState {
    let cfg = config::load_config();

    let db = db::init_db(&format!("{}/gse.db", ui::SAVE_DIR));

    // Migrate data/{app_id}/ → data/steam/{app_id}/
    migrate_data_dir(ui::SAVE_DIR);

    // Populate achievement sources from existing steam/gog save dirs (first run).
    if db::load_all_games(&db).map(|v| v.is_empty()).unwrap_or(true) {
        populate_db_from_dirs(&db, ui::SAVE_DIR);
    }

    let steam = Arc::new(SteamClient::new(
        cfg.steam_api_key.clone(),
        cfg.steam_griddb_api_key.clone(),
        &format!("{}/data", ui::SAVE_DIR),
    ));

    // Create pipe for main-loop wakeup — eliminates 50ms polling entirely.
    let mut pipe_fds = [0i32; 2];
    unsafe {
        libc::pipe2(pipe_fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let read_fd = pipe_fds[0];
    let write_fd = pipe_fds[1];

    let (tx, rx) = mpsc::channel::<AppMessage>();
    let sender = AppSender { tx, fd: write_fd };

    let cfg_for_watcher = Arc::new(cfg.clone());
    let watcher = match AchievementWatcher::new(cfg_for_watcher, sender.clone(), ui::SAVE_DIR.to_string()) {
        Ok(w) => Some(w),
        Err(e) => {
            eprintln!("Live achievement watching unavailable: {}", e);
            None
        }
    };

    let game_names = watcher.as_ref().map(|w| w.game_names()).unwrap_or_else(|| {
        Arc::new(Mutex::new(std::collections::HashMap::new()))
    });

    // Watch Lutris pga.db for playtime/lastplayed changes (external launches,
    // game stops, etc.). Zero CPU when idle — inotify event-driven.
    let lutris_watcher = match crate::lutris::LutrisWatcher::new(sender.clone()) {
        Ok(w) => Some(w),
        Err(e) => {
            eprintln!("Lutris DB watching unavailable: {}", e);
            None
        }
    };

    // Build UI with empty game list — window appears immediately.
    // Games are loaded in a background thread and populated via GamesLoaded.
    let state = build_ui(
        app,
        Vec::new(),
        cfg,
        steam.clone(),
        watcher.clone(),
        db.clone(),
        sender.clone(),
        game_names,
    );

    state.borrow_mut().lutris_watcher = lutris_watcher;

    // Attach the pipe-based GSource to the main context.
    // When AppSender::send() writes to the pipe, the main loop wakes up
    // and drains the channel — zero idle CPU, no polling.
    {
        let data = Box::new(MainLoopData {
            read_fd,
            receiver: RefCell::new(rx),
            state: state.clone(),
        });
        let data_ptr = Box::into_raw(data) as *mut std::ffi::c_void;
        unsafe {
        let source = g_unix_fd_source_new(read_fd, glib::ffi::G_IO_IN);
        let func_ptr: unsafe extern "C" fn(i32, u32, glib::ffi::gpointer) -> glib::ffi::gboolean = source_trampoline;
        glib::ffi::g_source_set_callback(
            source as *mut glib::ffi::GSource,
            std::mem::transmute(func_ptr),
            data_ptr,
            Some(source_destroy),
        );
            glib::ffi::g_source_attach(source as *mut glib::ffi::GSource, std::ptr::null_mut());
            glib::ffi::g_source_unref(source as *mut glib::ffi::GSource);
        }
    }

    // Load games in background — the heavy filesystem/DB work.
    {
        let db = db.clone();
        let sender = sender.clone();
        std::thread::spawn(move || {
            let games = build_game_list(&db, ui::SAVE_DIR);
            let _ = sender.send(AppMessage::GamesLoaded(games));
        });
    }

    if std::env::var("AV_BENCH").is_ok() {
        bench::run_bench(state.clone());
    }

    state
}

/// Normalize a game title for fuzzy matching: lowercase, strip non-alphanumerics,
/// collapse whitespace, remove common suffixes like "- The Final Cut".
fn normalize_title(s: &str) -> String {
    let lower = s.to_lowercase();
    let alnum: String = lower
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let words: Vec<&str> = alnum.split_whitespace().collect();
    // Drop trailing words that are often edition/version suffixes.
    let suffixes = ["the", "final", "cut", "edition", "complete", "definitive", "remastered", "hd"];
    let mut end = words.len();
    while end > 0 && suffixes.contains(&words[end - 1]) {
        end -= 1;
    }
    words[..end].join(" ")
}

/// Try to auto-match unmatched Lutris games to existing save dirs by title.
/// Scans `data/<app_id>/appdetails.json` for the real game name, then matches
/// by normalized title (one contains the other).
fn auto_match_by_title(db: &db::DbConn, save_dir: &str, lutris_games: &[lutris::LutrisGame]) {
    // Build a map of normalized_title → steam_id from existing save dirs.
    let data_dir = std::path::Path::new(save_dir).join("data").join("steam");
    let mut title_map: Vec<(String, String)> = Vec::new(); // (normalized, steam_id)
    if let Ok(entries) = std::fs::read_dir(&data_dir) {
        for entry in entries.flatten() {
            let app_id = match entry.file_name().to_str() {
                Some(s) if s.parse::<i64>().is_ok() => s.to_string(),
                _ => continue,
            };
            // Skip dirs already linked to a Lutris game.
            if db::find_by_steam_id(db, &app_id)
                .ok()
                .flatten()
                .map(|e| e.lutris_db_id.is_some())
                .unwrap_or(false)
            {
                continue;
            }
            if let Some(name) = crate::parser::read_app_name(save_dir, &app_id) {
                title_map.push((normalize_title(&name), app_id));
            }
        }
    }

    // For each unmatched Lutris game, try to find a matching save dir by title.
    // Skip games that were manually unmatched or ignored.
    let entries = db::load_all_games(db).unwrap_or_default();
    let linked: std::collections::HashSet<i64> = entries
        .iter()
        .filter_map(|e| e.lutris_db_id)
        .collect();
    let do_not_match: std::collections::HashSet<i64> = entries
        .iter()
        .filter(|e| {
            // Check manual_unmatch or ignored columns
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
        // Exact normalized match only — substring matching is too loose.
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

/// Build the game list with Lutris as the source of truth.
///
/// Each Lutris game is joined to our DB (by `lutris_db_id`) to find its matched
/// achievement source. Matched games load achievements from the save dir;
/// unmatched ones appear with just their Lutris metadata until the user matches
/// them. Lutris games with a `service` (steam/gog) are auto-linked to existing
/// achievement sources by `service_id`.
fn build_game_list(db: &db::DbConn, save_dir: &str) -> Vec<Game> {
    let lutris_games = match lutris::load_lutris_games() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("Failed to read Lutris DB (falling back to DB-only list): {}", e);
            return parser::load_games(db, save_dir);
        }
    };

    // Auto-link Lutris service games to existing achievement sources.
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

    // Auto-match remaining unmatched Lutris games by title against existing
    // save dirs (which have appdetails.json with the real game name).
    auto_match_by_title(db, save_dir, &lutris_games);

    // Join: Lutris games (source of truth) ← our DB (achievement matching).
    let entries = db::load_all_games(db).unwrap_or_default();
    let ignored_ids = db::get_ignored_lutris_ids(db);
    let hidden_lutris_ids = db::get_hidden_lutris_ids(db);
    let mut by_lutris: HashMap<i64, db::GameEntry> = entries
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
            // Unmatched Lutris game — no achievement source yet.
            let mut game = parser::unmatched_game(lg.id, &lg.name, &lg.slug, lg.playtime, lg.lastplayed);
            // Apply hidden state from lutris_meta (for games with no DB row)
            if hidden_lutris_ids.contains(&lg.id) {
                game.hidden = true;
            }
            games.push(game);
        }
    }
    games.sort_by(|a, b| a.sort_key().cmp(b.sort_key()));
    games
}

fn populate_db_from_dirs(db: &db::DbConn, save_dir: &str) {
    let steam_dir = format!("{}/steam", save_dir);
    if let Ok(entries) = std::fs::read_dir(&steam_dir) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let app_id = match entry.file_name().to_str() {
                Some(s) if s.parse::<i64>().is_ok() => s.to_string(),
                _ => continue,
            };
            let title = crate::parser::read_app_name(save_dir, &app_id).unwrap_or_default();
            let _ = db::add_game(db, "steam", &app_id, &app_id, &title);
        }
    }

    let gog_dir = format!("{}/gog", save_dir);
    if let Ok(galaxy_entries) = std::fs::read_dir(&gog_dir) {
        for galaxy_entry in galaxy_entries.flatten() {
            if !galaxy_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let galaxy_path = galaxy_entry.path();
            if let Ok(product_entries) = std::fs::read_dir(&galaxy_path) {
                for product_entry in product_entries.flatten() {
                    if !product_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        continue;
                    }
                    let product_dir = product_entry.path();
                    let product_id = match product_entry.file_name().to_str() {
                        Some(s) if s.parse::<i64>().is_ok() => s.to_string(),
                        _ => continue,
                    };
                    let app_id = match std::fs::read_to_string(product_dir.join("steam_appid.txt")) {
                        Ok(s) => s.trim().to_string(),
                        Err(_) => continue,
                    };
                    if app_id.parse::<i64>().is_err() {
                        continue;
                    }
                    let title = crate::parser::read_app_name(save_dir, &app_id).unwrap_or_default();
                    let _ = db::add_game(db, "gog", &app_id, &product_id, &title);
                }
            }
        }
    }
}

/// Move data/{app_id}/ directories to data/steam/{app_id}/.
/// Idempotent: skips dirs that are already migrated or not numeric.
fn migrate_data_dir(save_dir: &str) {
    let data_dir = std::path::Path::new(save_dir).join("data");
    let steam_dir = data_dir.join("steam");

    let entries = match std::fs::read_dir(&data_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let _ = std::fs::create_dir_all(&steam_dir);

    for entry in entries.flatten() {
        let name = match entry.file_name().to_str() {
            Some(s) => s.to_string(),
            None => continue,
        };
        // Skip the new subdirectories
        if name == "steam" || name == "steamgriddb" {
            continue;
        }
        // Only migrate numeric dirs (app IDs)
        if name.parse::<i64>().is_err() {
            continue;
        }
        let src = entry.path();
        let dest = steam_dir.join(&name);
        if dest.exists() {
            continue;
        }
        if let Err(e) = std::fs::rename(&src, &dest) {
            eprintln!("Migration: could not move {} → {}: {}", src.display(), dest.display(), e);
        }
    }
}
