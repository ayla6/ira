//! The big-picture couch view: a controller- and keyboard-driven horizontal
//! carousel of the most recently played games. Replaces the desktop window
//! content when Ira runs in big-picture mode (`--big-picture`,
//! `IRA_BIG_PICTURE=1`, or under Gamescope).

use super::big_picture_input::NavCommand;
use super::css::*;
use super::recent_carousel::RecentRow;
use super::recent_row::build_cover;
use super::state::SharedState;
use crate::Game;
use adw::prelude::*;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Carousel cover height; the covers are 2:3 portraits of this.
const COVER_HEIGHT: i32 = 320;
/// How many recent games the carousel keeps.
const RECENT_LIMIT: usize = 16;
/// Selection scroll animation length.
const SCROLL_MILLIS: u64 = 160;

/// Widgets and selection state of the couch view, kept on `AppState` behind
/// an `Rc` so every handler — refreshes, navigation, the scroll ticker —
/// sees the same selection state instead of a deep-cloned snapshot.
pub struct BigPictureUi {
    row: RecentRow,
    scrolled: gtk4::ScrolledWindow,
    caption: gtk4::Label,
    covers: RefCell<Vec<gtk4::Widget>>,
    games: RefCell<Vec<Game>>,
    selected: RefCell<usize>,
    scroll_anim: RefCell<Option<glib::SourceId>>,
    /// Games whose SGDB square is already being fetched in the background,
    /// so a refresh while the download runs does not re-queue them.
    square_queued: RefCell<HashSet<i64>>,
}

/// Build the couch window (fullscreen is applied by main.rs) and take over
/// the shared state's window reference.
pub(super) fn build_window(state: &SharedState, app: &adw::Application) {
    let window = adw::ApplicationWindow::new(app);
    window.set_title(Some(&crate::tr!("Ira")));
    window.set_size_request(900, 650);

    super::css::init_styles();

    let square_mode = state.borrow().cfg.big_picture_square_capsules;
    let (root, ui) = build_root(square_mode);
    // Key events land on the toplevel whenever nothing else holds focus, so
    // the keyboard handler lives there rather than on a child widget.
    wire_keyboard(state, &window);
    // The desktop window's close wiring never runs in this mode, so honor
    // the close-to-background setting here: without it a compositor close
    // would destroy the window and leave the process running headless.
    {
        let close_state = state.clone();
        window.connect_close_request(move |_| {
            let close_to_background = close_state.borrow().cfg.close_to_background;
            if close_to_background {
                super::background::show_close_choice_dialog(&close_state);
                glib::Propagation::Stop
            } else {
                if let Some(app) = close_state.borrow().window.application() {
                    app.quit();
                }
                glib::Propagation::Proceed
            }
        });
    }

    {
        let mut s = state.borrow_mut();
        s.window = window.clone();
        s.big_picture = Some(Rc::new(ui));
    }
    window.set_content(Some(&root));
    window.present();

    refresh(state);
    super::big_picture_input::start(state);
}

fn build_root(square: bool) -> (gtk4::Overlay, BigPictureUi) {
    let overlay = gtk4::Overlay::new();

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.add_css_class(CSS_BP_ROOT);

    let spring_top = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    spring_top.set_vexpand(true);
    root.append(&spring_top);

    let width = capsule_width(square);
    let spacing = super::virtual_grid::VirtualGrid::grid_spacing_for_item_w(width);
    let row = RecentRow::new(spacing, COVER_HEIGHT);
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Never);
    scrolled.set_valign(gtk4::Align::Start);
    // Side margins keep the selected cover's outline on screen at the ends
    // of the row.
    scrolled.set_margin_start(16);
    scrolled.set_margin_end(16);
    scrolled.add_css_class(CSS_RECENT_SCROLL);
    scrolled.set_child(Some(&row));
    root.append(&scrolled);

    let caption = gtk4::Label::new(None);
    caption.set_xalign(0.0);
    caption.set_margin_start(28);
    caption.set_margin_top(24);
    caption.set_margin_bottom(32);
    caption.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    caption.set_visible(false);
    caption.add_css_class(CSS_BP_CAPTION);
    root.append(&caption);

    let spring_bottom = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    spring_bottom.set_vexpand(true);
    root.append(&spring_bottom);

    root.set_hexpand(true);
    root.set_valign(gtk4::Align::Fill);
    overlay.add_overlay(&root);

    let ui = BigPictureUi {
        row,
        scrolled,
        caption,
        covers: RefCell::new(Vec::new()),
        games: RefCell::new(Vec::new()),
        selected: RefCell::new(0),
        scroll_anim: RefCell::new(None),
        square_queued: RefCell::new(HashSet::new()),
    };
    (overlay, ui)
}

/// Capsule width for the current mode: square art or 2:3 portraits.
fn capsule_width(square: bool) -> i32 {
    if square {
        COVER_HEIGHT
    } else {
        (COVER_HEIGHT as f64 * 2.0 / 3.0) as i32
    }
}

fn wire_keyboard(state: &SharedState, window: &adw::ApplicationWindow) {
    let state = state.clone();
    let key = gtk4::EventControllerKey::new();
    key.connect_key_pressed(move |_, key, _, modifiers| {
        match key {
            gdk4::Key::Left => handle_nav(&state, NavCommand::Left),
            gdk4::Key::Right => handle_nav(&state, NavCommand::Right),
            gdk4::Key::Return | gdk4::Key::KP_Enter | gdk4::Key::space => {
                handle_nav(&state, NavCommand::Confirm)
            }
            gdk4::Key::Escape => quit_app(&state),
            gdk4::Key::q if modifiers.contains(gdk4::ModifierType::CONTROL_MASK) => {
                quit_app(&state)
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    window.add_controller(key);
}

fn quit_app(state: &SharedState) {
    let window = state.borrow().window.clone();
    if let Some(app) = window.application() {
        app.quit();
    }
}

/// Repopulate the carousel from the shared game list. Cheap no-op outside
/// big-picture mode; message handlers call it whenever the game list or its
/// artwork changes.
pub(super) fn refresh(state: &SharedState) {
    let Some(ui) = state.borrow().big_picture.clone() else {
        return;
    };
    let games = super::helpers::recently_played(state, RECENT_LIMIT);

    // The achievement watcher re-reports games constantly; rebuilding the
    // whole carousel for each report churns covers for no visible change.
    // Only rebuild when the carousel's content actually differs.
    let square_mode = state.borrow().cfg.big_picture_square_capsules;
    let unchanged = {
        let current = ui.games.borrow();
        current.len() == games.len()
            && current.iter().zip(&games).all(|(a, b)| {
                a.grid_id() == b.grid_id()
                    && a.grid_path == b.grid_path
                    && a.square_path == b.square_path
                    && a.name == b.name
            })
    };
    if unchanged {
        return;
    }

    // Keep pointing at the same game across rebuilds when it survives.
    let previous = ui
        .games
        .borrow()
        .get(*ui.selected.borrow())
        .map(Game::grid_id);

    ui.row.clear_covers();

    let width = capsule_width(square_mode);
    let mut covers = Vec::with_capacity(games.len());
    for (index, game) in games.iter().enumerate() {
        // Square mode: cover-fit the art into a square capsule. A game
        // whose square.webp has not landed yet falls back to its vertical
        // capsule, scaled to cover (centered, overflow cropped).
        let art = if square_mode && !game.square_path.is_empty() {
            &game.square_path
        } else {
            &game.grid_path
        };
        let cover = build_cover(state, game, art, width, COVER_HEIGHT, square_mode, move |state| {
            on_cover_clicked(state, index)
        });
        ui.row.append_cover(&cover);
        covers.push(cover);
    }
    *ui.covers.borrow_mut() = covers;
    queue_missing_squares(state, &ui, square_mode);

    let selected = games
        .iter()
        .position(|g| Some(g.grid_id()) == previous)
        .unwrap_or(0);
    *ui.selected.borrow_mut() = selected;
    *ui.games.borrow_mut() = games;
    apply_selection(&ui, false);
}

/// Mouse navigation on a cover: the first click selects, clicking the
/// already-selected cover plays.
fn on_cover_clicked(state: &SharedState, index: usize) {
    let selected = state
        .borrow()
        .big_picture
        .as_ref()
        .map(|ui| *ui.selected.borrow())
        .unwrap_or(usize::MAX);
    if selected == index {
        confirm(state);
    } else {
        select(state, index);
    }
}

pub(super) fn handle_nav(state: &SharedState, command: NavCommand) {
    match command {
        NavCommand::Left => move_selection(state, -1),
        NavCommand::Right => move_selection(state, 1),
        NavCommand::Confirm => confirm(state),
    }
}

fn move_selection(state: &SharedState, delta: i32) {
    let Some(ui) = state.borrow().big_picture.clone() else {
        return;
    };
    let count = ui.games.borrow().len();
    let Some(next) = next_selection(*ui.selected.borrow(), count, delta) else {
        return;
    };
    *ui.selected.borrow_mut() = next;
    apply_selection(&ui, true);
}

fn select(state: &SharedState, index: usize) {
    let Some(ui) = state.borrow().big_picture.clone() else {
        return;
    };
    *ui.selected.borrow_mut() = index;
    apply_selection(&ui, true);
}

/// Clamp `current` by `delta` into `0..count`, or None when there is nothing
/// to move within.
fn next_selection(current: usize, count: usize, delta: i32) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let next = current as i64 + delta as i64;
    Some(next.clamp(0, count as i64 - 1) as usize)
}

/// Launch the selected game through the shared launch path (which already
/// guards against double launches).
fn confirm(state: &SharedState) {
    let game = state
        .borrow()
        .big_picture
        .as_ref()
        .and_then(|ui| ui.games.borrow().get(*ui.selected.borrow()).cloned());
    let Some(game) = game else {
        return;
    };
    if let Err(error) = super::play_button::launch_game(state, game.db_id, game.variant_id) {
        eprintln!("Failed to launch game: {error}");
        let _ = state
            .borrow()
            .sender
            .send(crate::AppMessage::AddGameError(error));
    }
}

/// Selection visuals, caption and scroll. Runs on the shared `Rc` handle so
/// no state borrow is held while widgets sync.
fn apply_selection(ui: &Rc<BigPictureUi>, animate: bool) {
    let selected = *ui.selected.borrow();
    let covers = ui.covers.borrow();
    for (index, cover) in covers.iter().enumerate() {
        if index == selected {
            cover.add_css_class(CSS_BP_SELECTED);
        } else {
            cover.remove_css_class(CSS_BP_SELECTED);
        }
    }
    let selected_cover = covers.get(selected).cloned();
    drop(covers);
    ui.row.set_selected_cover(selected_cover.as_ref());

    let game = ui.games.borrow().get(selected).cloned();
    update_caption(ui, game.as_ref());
    update_scroll(ui, animate);
}

/// The caption names the selected game and nothing else; it stays hidden
/// while there is nothing to name.
fn update_caption(ui: &BigPictureUi, game: Option<&Game>) {
    match game {
        Some(game) => {
            ui.caption.set_text(&game.name);
            ui.caption.set_visible(true);
        }
        None => ui.caption.set_visible(false),
    }
}

fn update_scroll(ui: &Rc<BigPictureUi>, animate: bool) {
    let Some((x, w)) = ui.row.cover_geometry(*ui.selected.borrow()) else {
        return;
    };
    let adj = ui.scrolled.hadjustment();
    let max = (adj.upper() - adj.page_size()).max(0.0);
    let target = (x + w / 2.0 - adj.page_size() / 2.0).clamp(0.0, max);
    if let Some(id) = ui.scroll_anim.borrow_mut().take() {
        id.remove();
    }
    if !animate {
        adj.set_value(target);
        return;
    }
    let start = adj.value();
    if (target - start).abs() < 0.5 {
        return;
    }
    let adj = adj.clone();
    let started = Instant::now();
    let ticker_ui = Rc::clone(ui);
    let id = glib::timeout_add_local(Duration::from_millis(16), move || {
        let t = (started.elapsed().as_millis() as f64 / SCROLL_MILLIS as f64).min(1.0);
        let eased = 1.0 - (1.0 - t) * (1.0 - t);
        adj.set_value(start + (target - start) * eased);
        if t >= 1.0 {
            *ticker_ui.scroll_anim.borrow_mut() = None;
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    *ui.scroll_anim.borrow_mut() = Some(id);
}

/// Start background fills of missing square.webp files so the carousel
/// fills in as art arrives. Matched games pull their SGDB square; console
/// games without one get their ROM's native icon imported. Each game is
/// queued once per session; the completion message drives the refresh.
fn queue_missing_squares(state: &SharedState, ui: &BigPictureUi, square_mode: bool) {
    if !square_mode {
        return;
    }
    let (steam, sender, save_dir, db, cfg) = {
        let s = state.borrow();
        (
            s.steam.clone(),
            s.sender.clone(),
            s.save_dir.clone(),
            s.db.clone(),
            s.cfg.clone(),
        )
    };
    let jobs: Vec<Game> = {
        let games = ui.games.borrow();
        let mut queued = ui.square_queued.borrow_mut();
        games
            .iter()
            .filter(|g| g.square_path.is_empty())
            .filter(|g| queued.insert(g.db_id))
            .cloned()
            .collect()
    };
    if jobs.is_empty() {
        return;
    }
    let switch_exe = cfg.console("switch").executable.clone();
    std::thread::spawn(move || {
        for game in jobs {
            let square = if !game.sgdb_id.is_empty() {
                fetch_sgdb_square(&steam, &save_dir, &db, game.db_id, &game.sgdb_id)
            } else if matches!(
                game.kind,
                ira_models::GameKind::Ps4 | ira_models::GameKind::Switch
            ) {
                import_rom_square(&save_dir, &db, &game, &cfg, &switch_exe)
            } else {
                String::new()
            };
            if !square.is_empty() {
                let _ = sender.send(crate::AppMessage::SquareReady(game.db_id));
            }
        }
    });
}

/// Download the matched game's SGDB square (and any other missing SGDB
/// asset alongside it; cached files are reused). Returns the square path.
fn fetch_sgdb_square(
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
    let (_, _, _, _, _, square) = steam.ensure_sgdb_assets_in_dir(&dir, sgdb_id);
    square
}

/// Import a console game's ROM icon into its data dir as square.webp —
/// the same native art the icon slot starts from, kept in its own slot so
/// SGDB's small chat icon never replaces it here.
fn import_rom_square(
    save_dir: &str,
    db: &ira_db::DbConn,
    game: &Game,
    cfg: &ira_config::Config,
    switch_exe: &str,
) -> String {
    let Some(bytes) = super::image_manager_helpers::native_icon_bytes(
        game,
        cfg,
        &cfg.azahar_executable,
        &cfg.cemu_executable,
        switch_exe,
    ) else {
        return String::new();
    };
    let Ok(Some(entry)) = ira_db::find_by_db_id(db, game.db_id) else {
        return String::new();
    };
    let dir = ira_parser::entry_data_dir(save_dir, &entry);
    if std::fs::create_dir_all(&dir).is_err() {
        return String::new();
    }
    let dest = dir.join("square.webp");
    if std::fs::write(&dest, &bytes).is_err() {
        return String::new();
    }
    dest.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::next_selection;

    #[test]
    fn test_next_selection_clamps_at_edges() {
        assert_eq!(next_selection(0, 5, -1), Some(0));
        assert_eq!(next_selection(0, 5, 1), Some(1));
        assert_eq!(next_selection(4, 5, 1), Some(4));
        assert_eq!(next_selection(2, 5, 2), Some(4));
        assert_eq!(next_selection(0, 0, 1), None);
    }
}
