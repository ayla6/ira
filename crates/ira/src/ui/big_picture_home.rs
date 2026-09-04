//! The couch home page: a horizontal carousel of recently played games plus
//! the grid tile that opens All Software. The selected tile pops forward
//! (an ease-out-back scale baked into the row's allocation transform) under
//! a stronger accent ring, with the game's name floating above it as a
//! marquee when it overflows.

use super::big_picture_marquee::Marquee;
use super::big_picture_view::BigPictureUi;
use super::css::*;
use super::recent_carousel::RecentRow;
use super::recent_row::build_cover;
use super::state::SharedState;
use crate::Game;
use adw::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Carousel cover height; the covers are 2:3 portraits of this, and the
/// All Software tile is this square.
const COVER_HEIGHT: i32 = 320;
/// How many recent games the carousel keeps.
const RECENT_LIMIT: usize = 16;
/// Selection scroll animation length.
const SCROLL_MILLIS: u64 = 160;
/// Selected tile scale and pop timing (ease-out-back overshoots slightly).
const POP_SCALE: f64 = 1.05;
const POP_IN_MS: u32 = 240;
const POP_OUT_MS: u32 = 140;
/// Height of the strip the floating title moves within.
const TITLE_AREA_HEIGHT: i32 = 56;
/// Per-cover paint-scale key. RecentRow reads it during allocation and
/// bakes it into the tile's transform, so the pop never touches layout.
const SCALE_KEY: &str = "bp-scale";

/// Widgets and selection state of the home carousel. `selected` runs over
/// the covers followed by the All Software tile (index == `games.len()`).
pub(super) struct HomeUi {
    row: RecentRow,
    scrolled: gtk4::ScrolledWindow,
    marquee: Marquee,
    covers: RefCell<Vec<gtk4::Widget>>,
    games: RefCell<Vec<Game>>,
    selected: RefCell<usize>,
    scroll_anim: RefCell<Option<glib::SourceId>>,
    /// Games whose SGDB square is already being fetched in the background,
    /// so a refresh while the download runs does not re-queue them.
    square_queued: RefCell<HashSet<i64>>,
    /// Selection pops still playing; skipped when the selection moves on.
    pops: RefCell<Vec<adw::Animation>>,
}

pub(super) fn build(state: &SharedState, square_mode: bool) -> (gtk4::Box, HomeUi) {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    page.set_hexpand(true);

    let spring_top = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    spring_top.set_vexpand(true);
    page.append(&spring_top);

    let marquee = Marquee::new(TITLE_AREA_HEIGHT);
    // Same side margins as the carousel below, so the title rail's
    // coordinates match the scroll viewport's and tiles center exactly.
    marquee.set_margin_start(16);
    marquee.set_margin_end(16);
    marquee.set_hexpand(true);
    page.append(marquee.widget());

    let width = capsule_width(square_mode);
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
    // The floating title rides the scroll: any adjustment move (animated
    // selection scroll, wheel, drag) re-centers it over the selected tile.
    {
        let scroll_state = state.clone();
        let adj = scrolled.hadjustment();
        adj.connect_value_changed(move |_| {
            if let Some(big) = scroll_state.borrow().big_picture.clone() {
                sync_title_position(&big);
            }
        });
    }
    page.append(&scrolled);

    let spring_bottom = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    spring_bottom.set_vexpand(true);
    page.append(&spring_bottom);

    let ui = HomeUi {
        row,
        scrolled,
        marquee,
        covers: RefCell::new(Vec::new()),
        games: RefCell::new(Vec::new()),
        selected: RefCell::new(0),
        scroll_anim: RefCell::new(None),
        square_queued: RefCell::new(HashSet::new()),
        pops: RefCell::new(Vec::new()),
    };
    (page, ui)
}

/// Capsule width for the current mode: square art or 2:3 portraits.
fn capsule_width(square: bool) -> i32 {
    if square {
        COVER_HEIGHT
    } else {
        (COVER_HEIGHT as f64 * 2.0 / 3.0) as i32
    }
}

/// Repopulate the carousel from the shared game list. Message handlers call
/// it whenever the game list or its artwork changes; the rebuild is skipped
/// when nothing visible differs.
pub(super) fn refresh(state: &SharedState) {
    let Some(big) = state.borrow().big_picture.clone() else {
        return;
    };
    let ui = &big.home;
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

    // Keep pointing at the same selection across rebuilds when it survives;
    // resting on the tile follows the games count.
    let selected_index = *ui.selected.borrow();
    let at_tile = selected_index >= ui.games.borrow().len() && selected_index > 0;
    let previous = ui.games.borrow().get(selected_index).map(Game::grid_id);

    ui.row.clear_covers();

    let width = capsule_width(square_mode);
    let mut covers = Vec::with_capacity(games.len() + 1);
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
        attach_scale(&cover);
        ui.row.append_cover(&cover);
        covers.push(cover);
    }
    let tile = build_all_tile(state);
    attach_scale(&tile);
    ui.row.append_cover(&tile);
    covers.push(tile);
    *ui.covers.borrow_mut() = covers;

    let selected = if at_tile {
        games.len()
    } else {
        games
            .iter()
            .position(|g| Some(g.grid_id()) == previous)
            .unwrap_or(0)
    };
    *ui.selected.borrow_mut() = selected;
    *ui.games.borrow_mut() = games;
    apply_selection(&big, None, false);
    // Queued after the swap so it sees the freshly loaded list.
    queue_missing_squares(state, ui, square_mode);
}

/// Mouse navigation on a cover: the first click selects, clicking the
/// already-selected cover plays. The All Software tile opens immediately.
fn on_cover_clicked(state: &SharedState, index: usize) {
    let is_tile = state
        .borrow()
        .big_picture
        .as_ref()
        .is_some_and(|big| index >= big.home.games.borrow().len());
    if is_tile {
        super::big_picture_view::open_all(state);
        return;
    }
    let selected = state
        .borrow()
        .big_picture
        .as_ref()
        .map(|big| *big.home.selected.borrow())
        .unwrap_or(usize::MAX);
    if selected == index {
        super::big_picture_view::confirm(state);
    } else {
        let Some(big) = state.borrow().big_picture.clone() else {
            return;
        };
        let previous = *big.home.selected.borrow();
        *big.home.selected.borrow_mut() = index;
        apply_selection(&big, Some(previous), true);
    }
}

pub(super) fn move_selection(state: &SharedState, delta: i32) {
    let Some(big) = state.borrow().big_picture.clone() else {
        return;
    };
    let ui = &big.home;
    // One past the last game is the All Software tile.
    let count = ui.games.borrow().len() + 1;
    let Some(next) = next_selection(*ui.selected.borrow(), count, delta) else {
        return;
    };
    let previous = *ui.selected.borrow();
    *ui.selected.borrow_mut() = next;
    apply_selection(&big, Some(previous), true);
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
/// guards against double launches). A no-op on the All Software tile.
pub(super) fn launch_selected(state: &SharedState) {
    let game = state
        .borrow()
        .big_picture
        .as_ref()
        .and_then(|big| {
            let ui = &big.home;
            ui.games.borrow().get(*ui.selected.borrow()).cloned()
        });
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

/// True when the selection rests on the All Software tile rather than a
/// game cover.
pub(super) fn selection_is_tile(big: &Rc<BigPictureUi>) -> bool {
    *big.home.selected.borrow() >= big.home.games.borrow().len()
}

/// Selection visuals, floating title, scroll, and the pop animation.
/// `previous` is the index that wore the highlight before; animations only
/// run when it is known (a real selection change, not a refresh).
fn apply_selection(big: &Rc<BigPictureUi>, previous: Option<usize>, animate: bool) {
    let ui = &big.home;
    let selected = *ui.selected.borrow();
    let covers = ui.covers.borrow();
    for (index, cover) in covers.iter().enumerate() {
        if index == selected {
            cover.add_css_class(CSS_BP_SELECTED);
        } else {
            cover.remove_css_class(CSS_BP_SELECTED);
        }
    }
    let previous_cover = previous.and_then(|index| covers.get(index).cloned());
    let selected_cover = covers.get(selected).cloned();
    drop(covers);
    ui.row.set_selected_cover(selected_cover.as_ref());

    skip_pops(ui);
    match (animate, previous_cover, selected_cover.as_ref()) {
        (true, Some(previous), Some(cover)) => {
            play_pop(ui, &previous, 1.0, adw::Easing::EaseOutCubic, POP_OUT_MS);
            play_pop(ui, cover, POP_SCALE, adw::Easing::EaseOutBack, POP_IN_MS);
        }
        (false, _, Some(cover)) => {
            // Selection restored without an event (refresh): snap, don't pop.
            if let Some(cell) = scale_of(cover) {
                cell.set(POP_SCALE);
            }
            ui.row.queue_allocate();
        }
        _ => {}
    }

    sync_title_text(ui);
    sync_title_position(big);
    update_scroll(big, animate);
}

fn sync_title_text(ui: &HomeUi) {
    let selected = *ui.selected.borrow();
    let game = ui.games.borrow().get(selected).cloned();
    let title = match game {
        Some(game) => game.name,
        None => crate::tr!("All Software"),
    };
    ui.marquee.set_text(&title);
}

/// Slide the floating title so it rides above the selected tile.
fn sync_title_position(big: &Rc<BigPictureUi>) {
    let ui = &big.home;
    let selected = *ui.selected.borrow();
    let Some((x, w)) = ui.row.cover_geometry(selected) else {
        return;
    };
    let adj = ui.scrolled.hadjustment();
    // Tile center in viewport coordinates: content x (cover_geometry
    // includes the leading spacing) minus the scroll. The marquee rail
    // carries the same side margins as the scrolled row, so the
    // coordinates line up with no adjustment.
    let center = x + w / 2.0 - adj.value();
    ui.marquee.set_position(center, adj.page_size());
}

/// Smooth-scroll the selected tile to the viewport center; the adjustment's
/// value_changed signal keeps the floating title glued to it.
fn update_scroll(big: &Rc<BigPictureUi>, animate: bool) {
    let ui = &big.home;
    let selected = *ui.selected.borrow();
    let Some((x, w)) = ui.row.cover_geometry(selected) else {
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
    let ticker_big = Rc::clone(big);
    let id = glib::timeout_add_local(Duration::from_millis(16), move || {
        let t = (started.elapsed().as_millis() as f64 / SCROLL_MILLIS as f64).min(1.0);
        let eased = 1.0 - (1.0 - t) * (1.0 - t);
        adj.set_value(start + (target - start) * eased);
        if t >= 1.0 {
            *ticker_big.home.scroll_anim.borrow_mut() = None;
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    *ui.scroll_anim.borrow_mut() = Some(id);
}

fn attach_scale(cover: &gtk4::Widget) {
    unsafe { cover.set_data::<Rc<Cell<f64>>>(SCALE_KEY, Rc::new(Cell::new(1.0))) };
}

fn scale_of(cover: &gtk4::Widget) -> Option<Rc<Cell<f64>>> {
    unsafe { cover.data::<Rc<Cell<f64>>>(SCALE_KEY) }.map(|ptr| unsafe { ptr.as_ref() }.clone())
}

fn skip_pops(ui: &HomeUi) {
    for anim in ui.pops.borrow_mut().drain(..) {
        anim.skip();
    }
}

/// Animate one tile's scale to `to`; the row repaints from the cell each
/// frame. `skip_pops` runs first so two animations never fight over a cell.
fn play_pop(ui: &HomeUi, cover: &gtk4::Widget, to: f64, easing: adw::Easing, ms: u32) {
    let Some(cell) = scale_of(cover) else {
        return;
    };
    let row = ui.row.downgrade();
    let anim_cell = Rc::clone(&cell);
    let target = adw::CallbackAnimationTarget::new(move |value| {
        anim_cell.set(value);
        if let Some(row) = row.upgrade() {
            row.queue_allocate();
        }
    });
    let anim = adw::TimedAnimation::new(cover, cell.get(), to, ms, target);
    anim.set_easing(easing);
    anim.play();
    ui.pops.borrow_mut().push(anim.upcast());
}

/// The grid tile at the end of the carousel that opens All Software. A
/// circle in a capsule shell so the selection ring and pop treat it like
/// any cover.
fn build_all_tile(state: &SharedState) -> gtk4::Widget {
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    vbox.set_valign(gtk4::Align::Start);
    vbox.set_halign(gtk4::Align::Center);
    vbox.add_css_class(CSS_COVER_ITEM);
    vbox.add_css_class(CSS_BP_ALL);
    vbox.set_size_request(COVER_HEIGHT, COVER_HEIGHT);
    vbox.set_overflow(gtk4::Overflow::Visible);

    let circle = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    circle.add_css_class(CSS_BP_ALL_TILE);
    circle.set_hexpand(true);
    circle.set_vexpand(true);
    circle.set_valign(gtk4::Align::Fill);
    let icon = gtk4::Image::from_icon_name("games-symbolic");
    icon.set_pixel_size(64);
    icon.set_opacity(0.7);
    icon.set_vexpand(true);
    icon.set_valign(gtk4::Align::Center);
    circle.append(&icon);
    vbox.append(&circle);

    let open_state = state.clone();
    let click = gtk4::GestureClick::new();
    click.connect_pressed(move |_, _, _, _| {
        super::big_picture_view::open_all(&open_state);
    });
    vbox.add_controller(click);
    vbox.upcast()
}

/// Start background fills of missing square.webp files so the carousel
/// fills in as art arrives. Matched games pull their SGDB square; console
/// games without one get their ROM's native icon imported. Each game is
/// queued once per session; the completion message drives the refresh.
fn queue_missing_squares(state: &SharedState, ui: &HomeUi, square_mode: bool) {
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
    std::thread::spawn(move || {
        for game in jobs {
            let square =
                super::fetch_images::ensure_game_square(&steam, &save_dir, &db, &cfg, &game);
            if !square.is_empty() {
                let _ = sender.send(crate::AppMessage::SquareReady(game.db_id));
            }
        }
    });
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
