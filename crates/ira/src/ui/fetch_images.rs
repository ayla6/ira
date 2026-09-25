//! Missing-image fetching with a Nautilus-style progress indicator: while
//! a fetch runs, a strip slides in at the bottom of the sidebar (the same
//! reveal-bottom-bars treatment Nautilus gives its operation indicator)
//! showing progress; clicking it opens a popover with the current game,
//! a progress bar and a cancel button.
//!
//! The fetch re-runs the SGDB asset ensure for every matched game —
//! including games whose enrichment was skipped because they already had
//! art — so any asset file that went missing or never landed (squares
//! included) is downloaded.

use super::state::SharedState;
use crate::Game;
use adw::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// How long the strip stays revealed after the job finishes, mirroring
/// Nautilus's remove-finished timeout.
const HIDE_AFTER_MS: u64 = 3_000;

/// Progress report sent from the fetch thread to the indicator.
struct FetchUpdate {
    done: usize,
    total: usize,
    current: String,
    finished: bool,
    fetched: usize,
}

/// The bottom-of-sidebar control for one fetch job. Kept on `AppState` so
/// the settings action can start a job from anywhere.
#[derive(Clone)]
pub struct FetchIndicator {
    toolbar: adw::ToolbarView,
    toggle: gtk4::Button,
    ring: super::progress_ring::ProgressRing,
    short: gtk4::Label,
    popover: gtk4::Popover,
    status: gtk4::Label,
    details: gtk4::Label,
    bar: gtk4::ProgressBar,
    close_btn: gtk4::Button,
    cancel: Arc<AtomicBool>,
    running: Rc<Cell<bool>>,
}

/// Start fetching missing images for every matched game, revealing the
/// indicator. `Err` carries the reason it refused: a job already
/// running, or no game in the library it could fetch anything for.
pub fn start_missing_images_fetch(state: &SharedState) -> Result<(), String> {
    let Some(indicator) = state.borrow().fetch_progress.borrow().clone() else {
        return Err(crate::tr!("Nothing to fetch right now").to_string());
    };
    if indicator.running.get() {
        return Err(crate::tr!("A job is already running").to_string());
    }
    let games: Vec<Game> = {
        let s = state.borrow();
        s.games
            .iter()
            .filter(|g| {
                !g.sgdb_id.is_empty()
                    || matches!(
                        g.kind,
                        ira_models::GameKind::Ps4 | ira_models::GameKind::Switch
                    )
                    // Steam games have no Steam-CDN square; the SGDB steam
                    // endpoints serve it through the store id.
                    || (g.trophy_source.has_steam_enrichment() && !g.steam_api_id().is_empty())
                    // ScreenScraper-matched games without any of the above:
                    // the SS fallbacks below are their only source.
                    || !g.screenscraper_id.is_empty()
            })
            .cloned()
            .collect()
    };
    if games.is_empty() {
        return Err(crate::tr!("No games to fetch images for").to_string());
    }
    let total = games.len();
    indicator.cancel.store(false, Ordering::Relaxed);
    indicator.running.set(true);
    indicator.reveal(true);
    indicator.ring.reset();
    indicator.close_btn.set_sensitive(true);
    indicator.close_btn.set_icon_name("process-stop-symbolic");
    indicator.short.set_text(&crate::tr!("Fetching images…"));
    indicator
        .status
        .set_text(&crate::tr!("Fetching missing images…"));
    indicator.bar.set_fraction(0.0);

    let (tx, rx) = super::helpers::ui_channel::<FetchUpdate>();
    let (steam, sender, save_dir, db) = {
        let s = state.borrow();
        (
            s.steam.clone(),
            s.sender.clone(),
            s.save_dir.clone(),
            s.db.clone(),
        )
    };
    let cancel = Arc::clone(&indicator.cancel);
    let cfg = state.borrow().cfg.clone();
    let switch_exe = cfg.console("switch").executable.clone();
    std::thread::spawn(move || {
        let mut fetched = 0usize;
        for (done, game) in games.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            // Matched and steam-enriched games get the full SGDB ensure
            // (console games with their square kept native); unmatched PS4
            // and Switch titles get their ROM's native icon imported into
            // the square slot. ScreenScraper-matched games with neither
            // join the same pass so their SS fallbacks can land.
            let mut changed = false;
            if !game.sgdb_id.is_empty()
                || (game.trophy_source.has_steam_enrichment() && !game.steam_api_id().is_empty())
                || !game.screenscraper_id.is_empty()
            {
                let entry = match ira_db::find_by_db_id(&db, game.db_id) {
                    Ok(Some(entry)) => entry,
                    _ => continue,
                };
                let dir = ira_parser::entry_data_dir(&save_dir, &entry);
                let had = count_present(&dir);
                let (icon, hero, grid, logo, header, mut square) =
                    ensure_game_assets(&steam, &dir, &save_dir, &db, &cfg, game, &switch_exe);
                if square.is_empty()
                    && !matches!(
                        game.kind,
                        ira_models::GameKind::Ps4 | ira_models::GameKind::Switch
                    )
                {
                    // ensure_game_assets leaves squares to the native/SGDB
                    // paths it knows; the ScreenScraper box fills the rest.
                    // PS4/Switch squares are fully covered above (native,
                    // then SGDB), so they skip the second pass entirely.
                    square = ensure_game_square(&steam, &save_dir, &db, &cfg, game);
                }
                changed = count_present(&dir) > had;
                // Multi-disc art rides the same pass — the ensure
                // self-gates on disc count, presence, and match.
                if super::disc_art::ensure_game_discs(&steam, &db, &cfg, &save_dir, game) {
                    changed = true;
                }
                if changed {
                    let _ = sender.send(crate::AppMessage::SgdbAssetsDownloaded {
                        db_id: game.db_id,
                        sgdb_id: game.sgdb_id.clone(),
                        icon,
                        hero,
                        grid,
                        logo,
                        header,
                        square: square.clone(),
                    });
                }
            } else if matches!(
                game.kind,
                ira_models::GameKind::Ps4 | ira_models::GameKind::Switch
            ) {
                changed = !import_rom_square(&save_dir, &db, game, &cfg, &switch_exe)
                    .is_empty();
            }
            if changed {
                fetched += 1;
                let _ = sender.send(crate::AppMessage::SquareReady(game.db_id));
            }
            let _ = tx.try_send(FetchUpdate {
                done: done + 1,
                total,
                current: game.name.clone(),
                finished: false,
                fetched,
            });
        }
        let _ = tx.try_send(FetchUpdate {
            done: total,
            total,
            current: String::new(),
            finished: true,
            fetched,
        });
    });

    drain_updates(&indicator, rx);
    Ok(())
}

/// The popover's details line, at Nautilus's one-size-down markup:
/// counts first, current game after, trimmed by the label's ellipsize.
fn details_markup(done: usize, total: usize, current: &str) -> String {
    let counts = format!("{} / {}", done, total);
    if current.is_empty() {
        counts
    } else {
        format!("{} · {}", counts, super::helpers::esc(current))
    }
}

fn details_text(update: &FetchUpdate) -> String {
    details_markup(update.done, update.total, &update.current)
}

/// One job driving the sidebar strip, handed out by
/// [`begin_strip_job`]. Its updates re-resolve the *current* window's
/// strip on every call, so progress stays visible across a
/// hide-to-background rebuild — the old widget's labels would be dead.
#[derive(Clone)]
pub struct StripJob {
    cancel: Arc<AtomicBool>,
}

/// Claim the sidebar strip for a job: reveals it with the running
/// labels, arms the stop button, and returns the handle to report
/// progress through. `None` while another job is showing.
pub fn begin_strip_job(state: &SharedState, short: &str, status: &str) -> Option<StripJob> {
    let indicator = state.borrow().fetch_progress.borrow().clone()?;
    if indicator.running.get() {
        return None;
    }
    {
        let s = state.borrow();
        if s.strip_job_busy.get() {
            return None;
        }
        s.strip_job_busy.set(true);
    }
    indicator.running.set(true);
    indicator.cancel.store(false, Ordering::Relaxed);
    indicator.reveal(true);
    indicator.ring.reset();
    indicator.close_btn.set_sensitive(true);
    indicator.close_btn.set_icon_name("process-stop-symbolic");
    indicator.short.set_text(short);
    indicator.status.set_text(status);
    indicator.bar.set_fraction(0.0);
    Some(StripJob {
        cancel: Arc::clone(&indicator.cancel),
    })
}

fn current_strip(state: &SharedState) -> Option<FetchIndicator> {
    state.borrow().fetch_progress.borrow().clone()
}

impl StripJob {
    /// The flag the job's worker thread polls between items.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// One step done of total, currently working on `current`. A
    /// cancelled job freezes its labels; later updates don't undo that.
    pub fn progress(&self, state: &SharedState, done: usize, total: usize, current: &str) {
        if self.cancelled() {
            return;
        }
        let Some(indicator) = current_strip(state) else {
            return;
        };
        let fraction = done as f64 / total.max(1) as f64;
        indicator.bar.set_fraction(fraction);
        indicator.ring.set_fraction(fraction);
        indicator.details.set_markup(&details_markup(done, total, current));
    }

    /// The job ended: a summary on the strip and the strip scheduled to
    /// slide away. A cancelled job skips the summary — the cancel click
    /// already froze the labels and scheduled the exit.
    pub fn finish(&self, state: &SharedState, short: &str, status: &str) {
        state.borrow().strip_job_busy.set(false);
        let Some(indicator) = current_strip(state) else {
            return;
        };
        indicator.running.set(false);
        if self.cancelled() {
            return;
        }
        indicator.ring.set_fraction(1.0);
        indicator.ring.animate_done("file-operation-finished-symbolic");
        indicator.short.set_text(short);
        indicator.status.set_text(status);
        indicator.bar.set_fraction(1.0);
        indicator.close_btn.set_sensitive(false);
        indicator.close_btn.set_icon_name("object-select-symbolic");
        let indicator = indicator.clone();
        glib::timeout_add_local_once(Duration::from_millis(HIDE_AFTER_MS), move || {
            indicator.popover.popdown();
            indicator.reveal(false);
            indicator.close_btn.set_sensitive(true);
        });
    }
}


/// Which ScreenScraper media feeds an Ira slot.
#[derive(Clone, Copy)]
pub(super) enum MediaKind {
    Box2d,
    Wheel,
    SteamGrid,
}

impl MediaKind {
    pub(super) fn url(self, game: &ira_api::screenscraper::ScrapedGame) -> Option<String> {
        match self {
            MediaKind::Box2d => game.box2d.clone(),
            MediaKind::Wheel => game.wheel.clone(),
            MediaKind::SteamGrid => game.steamgrid.clone(),
        }
    }
}

/// Download a PS1 game's ScreenScraper 2D box as image bytes — the
/// square source for rare games with no SGDB square.
pub(super) fn fetch_ss_square_bytes(
    steam: &ira_api::SteamDataClient,
    cfg: &ira_config::Config,
    db: &ira_db::DbConn,
    game: &Game,
) -> Option<Vec<u8>> {
    if !game.is_ps1() {
        return None;
    }
    fetch_ss_media_bytes(steam, cfg, db, game, MediaKind::Box2d)
}

/// One ScreenScraper media download by SS match id, decoded to lossless
/// WebP. `None` without a match, without credentials, without that art
/// on the entry, or when the bytes do not decode.
fn fetch_ss_media_bytes(
    steam: &ira_api::SteamDataClient,
    cfg: &ira_config::Config,
    db: &ira_db::DbConn,
    game: &Game,
    kind: MediaKind,
) -> Option<Vec<u8>> {
    if game.screenscraper_id.is_empty() {
        return None;
    }
    let creds = ira_api::ScraperCreds::from_account(
        cfg.screenscraper_id.clone(),
        cfg.screenscraper_password.clone(),
    );
    let region = super::disc_art::rom_region(db, game.db_id);
    let games = steam
        .screenscraper_game(&creds, &game.screenscraper_id, region)
        .ok()?;
    let url = games.iter().find_map(|game| kind.url(game))?;
    let bytes = steam.screenscraper_media(&url).ok()?;
    ira_parser::convert_bytes_to_lossless_webp(&bytes)
}

/// Persist ScreenScraper bytes into the game's data dir under the
/// asset's own file base, next to whatever the other sources landed.
/// Returns the path when a file landed on disk.
fn persist_ss_media(
    dir: &std::path::Path,
    bytes: &[u8],
    asset: ira_models::AssetType,
) -> String {
    let webp = match ira_parser::convert_bytes_to_lossless_webp(bytes) {
        Some(webp) => webp,
        None => return String::new(),
    };
    let _ = std::fs::create_dir_all(dir);
    ira_parser::remove_image_variants(dir, asset.file_base());
    let dest = dir.join(format!("{}.webp", asset.file_base()));
    if std::fs::write(&dest, &webp).is_ok() {
        dest.to_string_lossy().into_owned()
    } else {
        String::new()
    }
}

/// Fill one game's square slot: the ROM's native icon for PS4/Switch games
/// (never SGDB art), the SGDB square for other matched games, and the
/// ScreenScraper 2D box for PS1 games the SGDB pass left empty. Returns
/// the square path when a file landed on disk.
pub(super) fn ensure_game_square(
    steam: &ira_api::SteamDataClient,
    save_dir: &str,
    db: &ira_db::DbConn,
    cfg: &ira_config::Config,
    game: &Game,
) -> String {
    if matches!(
        game.kind,
        ira_models::GameKind::Ps4 | ira_models::GameKind::Switch
    ) {
        let switch_exe = cfg.console("switch").executable.clone();
        let square = import_rom_square(save_dir, db, game, cfg, &switch_exe);
        if square.is_empty() && !game.sgdb_id.is_empty() {
            // ROM icon extraction is not bulletproof (some NSP dumps yield
            // nothing) — SGDB square art beats a missing capsule.
            fetch_sgdb_square(steam, save_dir, db, game.db_id, &game.sgdb_id)
        } else {
            square
        }
    } else if !game.sgdb_id.is_empty() {
        let square = fetch_sgdb_square(steam, save_dir, db, game.db_id, &game.sgdb_id);
        if square.is_empty() {
            fetch_ss_square_persisted(steam, save_dir, db, cfg, game)
        } else {
            square
        }
    } else {
        fetch_ss_square_persisted(steam, save_dir, db, cfg, game)
    }
}

/// ScreenScraper square for games with no SGDB square (or no SGDB match
/// at all): PS1 games with an SS match only, into an empty slot.
fn fetch_ss_square_persisted(
    steam: &ira_api::SteamDataClient,
    save_dir: &str,
    db: &ira_db::DbConn,
    cfg: &ira_config::Config,
    game: &Game,
) -> String {
    if !game.square_path.is_empty() {
        return String::new();
    }
    let Ok(Some(entry)) = ira_db::find_by_db_id(db, game.db_id) else {
        return String::new();
    };
    let dir = ira_parser::entry_data_dir(save_dir, &entry);
    if ira_parser::find_image_file(&dir, ira_models::AssetType::Square.file_base()).is_some() {
        return String::new();
    }
    let Some(bytes) = fetch_ss_square_bytes(steam, cfg, db, game) else {
        return String::new();
    };
    persist_ss_media(&dir, &bytes, ira_models::AssetType::Square)
}

/// ScreenScraper wheel logo / steam-grid header for games whose SGDB
/// left that slot empty, into an empty slot. `asset` selects both the
/// media kind and the file base.
fn fetch_ss_slot_persisted(
    steam: &ira_api::SteamDataClient,
    save_dir: &str,
    db: &ira_db::DbConn,
    cfg: &ira_config::Config,
    game: &Game,
    asset: ira_models::AssetType,
    kind: MediaKind,
) -> String {
    let slot_path = game.asset_path(asset);
    if !slot_path.is_empty() {
        return String::new();
    }
    let Ok(Some(entry)) = ira_db::find_by_db_id(db, game.db_id) else {
        return String::new();
    };
    let dir = ira_parser::entry_data_dir(save_dir, &entry);
    if ira_parser::find_image_file(&dir, asset.file_base()).is_some() {
        return String::new();
    }
    let Some(bytes) = fetch_ss_media_bytes(steam, cfg, db, game, kind) else {
        return String::new();
    };
    persist_ss_media(&dir, &bytes, asset)
}

/// Full SGDB asset ensure for one game. Matched games use their SGDB id;
/// steam-enriched games without a match are served by the SGDB steam
/// endpoints through their app id. PS4 and Switch titles never take SGDB
/// squares — that slot is the ROM's native icon — so SGDB fills the other
/// five slots and the native import fills the square. Empty logo and
/// header slots fall back to ScreenScraper's wheel and steam-grid art,
/// last in the order after every SGDB source misses.
pub(super) fn ensure_game_assets(
    steam: &ira_api::SteamDataClient,
    dir: &std::path::Path,
    save_dir: &str,
    db: &ira_db::DbConn,
    cfg: &ira_config::Config,
    game: &Game,
    switch_exe: &str,
) -> (String, String, String, String, String, String) {
    let console = matches!(
        game.kind,
        ira_models::GameKind::Ps4 | ira_models::GameKind::Switch
    );
    let steam_only = game.sgdb_id.is_empty()
        && game.trophy_source.has_steam_enrichment()
        && !game.steam_api_id().is_empty();
    // SGDB's Steam endpoints need the store id, never the platform one.
    let store_id = game.steam_api_id().to_string();
    // No SGDB identity at all still leaves the ScreenScraper fallbacks
    // below something to do — an SS-matched game with neither (Saikai)
    // gets its wheel, steam-grid and box art here instead of being
    // skipped.
    let sgdb_id = if !game.sgdb_id.is_empty() {
        Some(ira_api::types::SgdbId::Game(game.sgdb_id.as_str()))
    } else if steam_only {
        Some(ira_api::types::SgdbId::Steam(store_id.as_str()))
    } else {
        None
    };
    let skip: &[ira_models::AssetType] = if console {
        &[ira_models::AssetType::Square]
    } else {
        &[]
    };
    let (icon, hero, grid, logo, header, square) = match sgdb_id {
        Some(id) => steam.ensure_sgdb_assets_in_dir(dir, id, skip),
        // Nothing SGDB can serve: the SS fallbacks below still run.
        None => (
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ),
    };
    let square = if console {
        let native = native_square(dir, game, cfg, switch_exe);
        if native.is_empty() {
            // ROM icon extraction is not bulletproof (some NSP dumps yield
            // nothing) — SGDB square art beats a missing capsule.
            match sgdb_id {
                Some(id) => {
                    let (_, _, _, _, _, sgdb_square) =
                        steam.ensure_sgdb_assets_in_dir(dir, id, &[]);
                    sgdb_square
                }
                None => String::new(),
            }
        } else {
            native
        }
    } else {
        square
    };
    let logo = if logo.is_empty() {
        fetch_ss_slot_persisted(
            steam,
            save_dir,
            db,
            cfg,
            game,
            ira_models::AssetType::Logo,
            MediaKind::Wheel,
        )
    } else {
        logo
    };
    let header = if header.is_empty() {
        fetch_ss_slot_persisted(
            steam,
            save_dir,
            db,
            cfg,
            game,
            ira_models::AssetType::Header,
            MediaKind::SteamGrid,
        )
    } else {
        header
    };
    (icon, hero, grid, logo, header, square)
}

/// Download the matched game's SGDB square (and any other missing SGDB
/// asset alongside it; cached files are reused). Returns the square path.
pub(super) fn fetch_sgdb_square(
    steam: &ira_api::SteamDataClient,
    save_dir: &str,
    db: &ira_db::DbConn,
    db_id: i64,
    sgdb_id: &str,
) -> String {
    let Ok(Some(entry)) = ira_db::find_by_db_id(db, db_id) else {
        return String::new();
    };
    let dir = ira_parser::entry_data_dir(save_dir, &entry);
    let (_, _, _, _, _, square) =
        steam.ensure_sgdb_assets_in_dir(&dir, ira_api::types::SgdbId::Game(sgdb_id), &[]);
    square
}

/// Import a console game's ROM icon into its data dir as square.webp —
/// the same native art the icon slot starts from, kept in its own slot so
/// SGDB's small chat icon never replaces it here.
pub(super) fn import_rom_square(
    save_dir: &str,
    db: &ira_db::DbConn,
    game: &Game,
    cfg: &ira_config::Config,
    switch_exe: &str,
) -> String {
    let Ok(Some(entry)) = ira_db::find_by_db_id(db, game.db_id) else {
        return String::new();
    };
    let dir = ira_parser::entry_data_dir(save_dir, &entry);
    native_square(&dir, game, cfg, switch_exe)
}

/// Write the game's native ROM icon into `dir` as square.webp — but only
/// into a genuinely empty slot. A square that already exists wins, custom
/// art included: the native import never replaces it, and a missing
/// square_small is derived from the existing square by the image loader
/// instead of any refetch.
fn native_square(
    dir: &std::path::Path,
    game: &Game,
    cfg: &ira_config::Config,
    switch_exe: &str,
) -> String {
    if let Some(existing) =
        ira_parser::find_image_file(dir, ira_models::AssetType::Square.file_base())
    {
        return existing.to_string_lossy().into_owned();
    }
    let bytes = super::image_manager_helpers::native_icon_bytes(
        game,
        cfg,
        &cfg.azahar_executable,
        &cfg.cemu_executable,
        switch_exe,
    );
    if let Some(bytes) = bytes {
        let _ = std::fs::create_dir_all(dir);
        ira_parser::remove_image_variants(dir, ira_models::AssetType::Square.file_base());
        let dest = dir.join(format!("{}.webp", ira_models::AssetType::Square.file_base()));
        if std::fs::write(&dest, &bytes).is_ok() {
            return dest.to_string_lossy().into_owned();
        }
    }
    ira_parser::find_image_file(dir, ira_models::AssetType::Square.file_base())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn count_present(dir: &std::path::Path) -> usize {
    ira_models::AssetType::all()
        .iter()
        .filter(|at| ira_parser::find_image_file(dir, at.file_base()).is_some())
        .count()
}

fn drain_updates(indicator: &FetchIndicator, rx: async_channel::Receiver<FetchUpdate>) {
    let indicator = indicator.clone();
    let last = Rc::new(RefCell::new(FetchUpdate {
        done: 0,
        total: 0,
        current: String::new(),
        finished: false,
        fetched: 0,
    }));
    super::helpers::watch_channel(rx, move |update| {
        let cancelled = indicator.cancel.load(Ordering::Relaxed);
        *last.borrow_mut() = update;
        let update = last.borrow();
        if update.finished {
            // A cancelled run froze on the click; keep that picture and
            // only schedule the strip's exit, like Nautilus.
            if !cancelled {
                indicator.ring.set_fraction(1.0);
                indicator
                    .ring
                    .animate_done("file-operation-finished-symbolic");
                indicator.short.set_text(
                    &crate::tr!("{} games needed new art")
                        .replacen("{}", &update.fetched.to_string(), 1),
                );
                indicator
                    .status
                    .set_text(&crate::tr!("Images fetched"));
                indicator.details.set_markup(&details_text(&update));
                indicator.bar.set_fraction(1.0);
                indicator.close_btn.set_sensitive(false);
                indicator
                    .close_btn
                    .set_icon_name("object-select-symbolic");
            }
            indicator.running.set(false);
            let indicator = indicator.clone();
            glib::timeout_add_local_once(Duration::from_millis(HIDE_AFTER_MS), move || {
                indicator.popover.popdown();
                indicator.reveal(false);
                indicator.close_btn.set_sensitive(true);
            });
            return glib::ControlFlow::Break;
        }
        // After a cancel the click already froze the labels; later
        // in-flight updates don't undo that picture.
        if !cancelled {
            let fraction = update.done as f64 / update.total.max(1) as f64;
            indicator.bar.set_fraction(fraction);
            indicator.ring.set_fraction(fraction);
            indicator.details.set_markup(&details_text(&update));
        }
        glib::ControlFlow::Continue
    });
}

/// Progress updates from an edit-save image conversion, rendered on the
/// sidebar strip. Clone the handle into the background thread.
#[derive(Clone)]
pub struct SaveProgress {
    tx: async_channel::Sender<SaveUpdate>,
}

#[derive(Clone)]
struct SaveUpdate {
    done: usize,
    total: usize,
    current: String,
    finished: bool,
}

impl SaveProgress {
    /// One image of the job has landed (copied, converted, thumb built).
    pub fn update(&self, done: usize, total: usize, current: &str) {
        let _ = self.tx.try_send(SaveUpdate {
            done,
            total,
            current: current.to_string(),
            finished: false,
        });
    }

    /// The job is done — everything that will be saved is saved.
    pub fn finish(&self) {
        let _ = self.tx.try_send(SaveUpdate {
            done: 0,
            total: 0,
            current: String::new(),
            finished: true,
        });
    }
}

/// Reveal the sidebar strip for an edit-save image conversion and start
/// rendering its progress. Returns None while the strip is busy with
/// another job — the conversion still runs, it just reports nowhere.
pub fn begin_image_save(state: &SharedState, title: &str) -> Option<SaveProgress> {
    let indicator = state.borrow().fetch_progress.borrow().clone()?;
    if indicator.running.get() {
        return None;
    }
    indicator.running.set(true);
    indicator.reveal(true);
    indicator.ring.reset();
    // A save conversion cannot be canceled: the old art is already being
    // replaced, so stopping midway would leave mixed assets behind.
    indicator.close_btn.set_sensitive(false);
    indicator.close_btn.set_icon_name("document-save-symbolic");
    indicator.short.set_text(&crate::tr!("Saving images…"));
    indicator
        .status
        .set_text(&crate::tr!("Saving images for {}").replacen("{}", title, 1));
    indicator.bar.set_fraction(0.0);

    let (tx, rx) = super::helpers::ui_channel::<SaveUpdate>();
    let indicator = indicator.clone();
    super::helpers::watch_channel(rx, move |update| {
        if update.finished {
            indicator.ring.set_fraction(1.0);
            indicator.ring.animate_done("object-select-symbolic");
            indicator.short.set_text(&crate::tr!("Images saved"));
            indicator
                .status
                .set_text(&crate::tr!("Edited images saved"));
            indicator.bar.set_fraction(1.0);
            let indicator = indicator.clone();
            glib::timeout_add_local_once(Duration::from_millis(HIDE_AFTER_MS), move || {
                indicator.popover.popdown();
                indicator.reveal(false);
                indicator.close_btn.set_sensitive(true);
            });
            return glib::ControlFlow::Break;
        }
        let fraction = update.done as f64 / update.total.max(1) as f64;
        indicator.bar.set_fraction(fraction);
        indicator.ring.set_fraction(fraction);
        indicator.details.set_text(&format!(
            "{} / {} · {}",
            update.done, update.total, update.current
        ));
        glib::ControlFlow::Continue
    });
    Some(SaveProgress { tx })
}

impl FetchIndicator {
    fn reveal(&self, on: bool) {
        self.toolbar.set_reveal_bottom_bars(on);
    }

    /// The bottom bar to add to the sidebar's toolbar view. Hidden until
    /// [`start_missing_images_fetch`] reveals it.
    pub fn widget(&self) -> gtk4::Widget {
        self.toggle.clone().upcast()
    }

    /// Build the strip for `toolbar`: a full-width toggle showing fetch
    /// progress; clicking opens the popover with details and cancel. The
    /// strip's bottom bar uses the flat style so it carries no shadow over
    /// the content above.
    pub fn build(toolbar: &adw::ToolbarView) -> Self {
        toolbar.set_bottom_bar_style(adw::ToolbarStyle::Flat);
        let toggle = gtk4::Button::new();
        toggle.add_css_class("flat");
        toggle.set_hexpand(true);
        // ToolbarView bottom bars inherit bold toolbar label styling; the
        // strip reads like sidebar content instead, with the toolbar's own
        // 6px inset (Nautilus relies on the same padding).
        toggle.add_css_class("fetch-strip");
        toggle.set_margin_start(6);
        toggle.set_margin_end(6);
        toggle.set_margin_top(6);
        toggle.set_margin_bottom(6);

        let ring = super::progress_ring::ProgressRing::new();
        ring.attach_widget(&toggle);
        let ring_icon = gtk4::Image::new();
        ring_icon.set_pixel_size(14);
        ring_icon.set_margin_start(3);
        ring_icon.set_paintable(Some(&ring));
        let short = gtk4::Label::new(None);
        short.set_xalign(0.0);
        short.set_hexpand(true);
        short.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        let inner = gtk4::Box::new(gtk4::Orientation::Horizontal, 9);
        inner.set_margin_start(3);
        inner.append(&ring_icon);
        inner.append(&short);
        toggle.set_child(Some(&inner));

        let popover = gtk4::Popover::new();
        // Nautilus points the popover sideways out of the sidebar.
        popover.set_position(gtk4::PositionType::Right);
        popover.set_parent(&toggle);

        // The popover row mirrors Nautilus's progress-info widget: status
        // label, bar under it, dim numeric details below that, and the
        // circular stop button spanning the rows on the right.
        let status = gtk4::Label::new(None);
        status.set_width_request(300);
        status.set_hexpand(true);
        status.set_margin_bottom(6);
        status.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        status.set_max_width_chars(40);
        status.set_xalign(0.0);
        let details = gtk4::Label::new(None);
        details.set_wrap(true);
        details.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        details.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        details.set_xalign(0.0);
        details.set_use_markup(true);
        details.add_css_class(super::css::CSS_DIM_LABEL);
        details.add_css_class(super::css::CSS_SMALL_TEXT);
        details.add_css_class("numeric");
        let bar = gtk4::ProgressBar::new();
        bar.set_valign(gtk4::Align::Center);
        bar.set_margin_start(2);
        bar.set_margin_bottom(4);
        bar.set_hexpand(true);
        bar.set_pulse_step(0.05);

        let close_btn = gtk4::Button::new();
        close_btn.set_icon_name("process-stop-symbolic");
        close_btn.add_css_class(super::css::CSS_CIRCULAR);
        close_btn.set_valign(gtk4::Align::Center);
        close_btn.set_margin_start(20);
        close_btn.set_tooltip_text(Some(&crate::tr!("Cancel")));

        let grid = gtk4::Grid::new();
        grid.set_margin_start(6);
        grid.set_margin_end(6);
        grid.set_margin_top(6);
        grid.set_margin_bottom(6);
        grid.attach(&status, 0, 0, 1, 1);
        grid.attach(&bar, 0, 1, 1, 1);
        grid.attach(&close_btn, 1, 0, 1, 3);
        grid.attach(&details, 0, 2, 1, 1);
        // Nautilus wraps the widget in a never-hscroll ScrolledWindow with
        // a natural-height cap: the viewport pins the popover's width to
        // the status label's request instead of the longest game title.
        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_hscrollbar_policy(gtk4::PolicyType::Never);
        scroll.set_max_content_height(270);
        scroll.set_propagate_natural_height(true);
        scroll.set_child(Some(&grid));
        popover.set_child(Some(&scroll));

        let indicator = Self {
            toolbar: toolbar.clone(),
            toggle: toggle.clone(),
            ring,
            short,
            popover,
            status,
            details,
            bar,
            close_btn,
            cancel: Arc::new(AtomicBool::new(false)),
            running: Rc::new(Cell::new(false)),
        };

        let popover_c = indicator.popover.clone();
        toggle.connect_clicked(move |_| {
            if popover_c.is_visible() {
                popover_c.popdown();
            } else {
                popover_c.popup();
            }
        });

        let cancel_flag = Arc::clone(&indicator.cancel);
        let details_c = indicator.details.clone();
        let close_c = indicator.close_btn.clone();
        let strip_c = indicator.clone();
        indicator
            .close_btn
            .connect_clicked(move |_| {
                cancel_flag.store(true, Ordering::Relaxed);
                // Nautilus freezes the widget and swaps the strip's ring
                // for the stop icon right away; only the details line
                // reads "Cancelled" — the operation text stays put.
                details_c.set_markup(&format!(
                    "<span size='small'>{}</span>",
                    crate::tr!("Cancelled")
                ));
                close_c.set_sensitive(false);
                // The ring crossfades into Nautilus's cancelled icon.
                strip_c
                    .ring
                    .animate_done("file-operation-cancelled-symbolic");
                let strip = strip_c.clone();
                glib::timeout_add_local_once(Duration::from_millis(HIDE_AFTER_MS), move || {
                    strip.popover.popdown();
                    strip.reveal(false);
                    strip.close_btn.set_sensitive(true);
                });
            });

        indicator
    }
}
