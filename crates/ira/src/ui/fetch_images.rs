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
/// How often the indicator polls the job's progress channel.
const POLL_MS: u64 = 100;

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
/// indicator. A no-op while a fetch is already running.
pub fn start_missing_images_fetch(state: &SharedState) {
    let Some(indicator) = state.borrow().fetch_progress.borrow().clone() else {
        return;
    };
    if indicator.running.get() {
        return;
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
                    // endpoints serve it through the app id.
                    || (g.trophy_source.has_steam_enrichment() && !g.app_id.is_empty())
            })
            .cloned()
            .collect()
    };
    if games.is_empty() {
        return;
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

    let (tx, rx) = std::sync::mpsc::channel::<FetchUpdate>();
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
            // the square slot.
            let mut changed = false;
            if !game.sgdb_id.is_empty()
                || (game.trophy_source.has_steam_enrichment() && !game.app_id.is_empty())
            {
                let entry = match ira_db::find_by_db_id(&db, game.db_id) {
                    Ok(Some(entry)) => entry,
                    _ => continue,
                };
                let dir = ira_parser::entry_data_dir(&save_dir, &entry);
                let had = count_present(&dir);
                let (icon, hero, grid, logo, header, square) =
                    ensure_game_assets(&steam, &dir, &cfg, game, &switch_exe);
                changed = count_present(&dir) > had;
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
            let _ = tx.send(FetchUpdate {
                done: done + 1,
                total,
                current: game.name.clone(),
                finished: false,
                fetched,
            });
        }
        let _ = tx.send(FetchUpdate {
            done: total,
            total,
            current: String::new(),
            finished: true,
            fetched,
        });
    });

    drain_updates(&indicator, rx);
}

/// The popover's details line, at Nautilus's one-size-down markup:
/// counts first, current game after, trimmed by the label's ellipsize.
fn details_text(update: &FetchUpdate) -> String {
    let counts = format!("{} / {}", update.done, update.total);
    if update.current.is_empty() {
        format!("<span size='small'>{counts}</span>")
    } else {
        format!(
            "<span size='small'>{counts} · {}</span>",
            super::helpers::esc(&update.current)
        )
    }
}

/// Fill one game's square slot: the ROM's native icon for PS4/Switch games
/// (never SGDB art), the SGDB square for other matched games. Returns the
/// square path when a file landed on disk.
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
        fetch_sgdb_square(steam, save_dir, db, game.db_id, &game.sgdb_id)
    } else {
        String::new()
    }
}

/// Full SGDB asset ensure for one game. Matched games use their SGDB id;
/// steam-enriched games without a match are served by the SGDB steam
/// endpoints through their app id. PS4 and Switch titles never take SGDB
/// squares — that slot is the ROM's native icon — so SGDB fills the other
/// five slots and the native import fills the square.
pub(super) fn ensure_game_assets(
    steam: &ira_api::SteamDataClient,
    dir: &std::path::Path,
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
        && !game.app_id.is_empty();
    let id = if game.sgdb_id.is_empty() {
        if !steam_only {
            return (
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            );
        }
        ira_api::types::SgdbId::Steam(&game.app_id)
    } else {
        ira_api::types::SgdbId::Game(&game.sgdb_id)
    };
    let skip: &[ira_models::AssetType] = if console {
        &[ira_models::AssetType::Square]
    } else {
        &[]
    };
    let (icon, hero, grid, logo, header, square) = steam.ensure_sgdb_assets_in_dir(dir, id, skip);
    let square = if console {
        let native = native_square(dir, game, cfg, switch_exe);
        if native.is_empty() {
            // ROM icon extraction is not bulletproof (some NSP dumps yield
            // nothing) — SGDB square art beats a missing capsule.
            let (_, _, _, _, _, sgdb_square) = steam.ensure_sgdb_assets_in_dir(dir, id, &[]);
            sgdb_square
        } else {
            native
        }
    } else {
        square
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

/// Write the game's native ROM icon into `dir` as square.webp, replacing
/// any previous square (SGDB art included). When extraction fails, an
/// existing square file survives so custom art is never lost.
fn native_square(
    dir: &std::path::Path,
    game: &Game,
    cfg: &ira_config::Config,
    switch_exe: &str,
) -> String {
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

fn drain_updates(indicator: &FetchIndicator, rx: std::sync::mpsc::Receiver<FetchUpdate>) {
    let indicator = indicator.clone();
    let last = Rc::new(RefCell::new(FetchUpdate {
        done: 0,
        total: 0,
        current: String::new(),
        finished: false,
        fetched: 0,
    }));
    glib::timeout_add_local(Duration::from_millis(POLL_MS), move || {
        let mut fresh = false;
        let mut disconnected = false;
        loop {
            match rx.try_recv() {
                Ok(update) => {
                    *last.borrow_mut() = update;
                    fresh = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                // The thread may finish between two polls, leaving the
                // final updates buffered next to the disconnect: drain
                // everything first, render below, and only then tear down.
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if fresh {
            let update = last.borrow();
            let cancelled = indicator.cancel.load(Ordering::Relaxed);
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
        }
        if disconnected {
            indicator.running.set(false);
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
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
