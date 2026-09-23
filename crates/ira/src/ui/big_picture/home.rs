//! The big picture home page: a horizontal carousel of recently played games plus
//! the grid tile that opens All Software. The selected tile wears a
//! single-line accent ring, with the game's name floating above it in a
//! tooltip pill that marquees when it overflows.

use super::marquee::Marquee;
use crate::ui::selection_ring::SelectionRing;
use super::view::BigPictureUi;
use crate::ui::css::*;
use crate::ui::recent_carousel::RecentRow;
use crate::ui::recent_row::build_cover;
use crate::ui::state::SharedState;
use crate::Game;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};


/// How many recent games the carousel keeps.
const RECENT_LIMIT: usize = 12;
/// Selection scroll animation length.
const SCROLL_MILLIS: u64 = 90;
/// How long native scrolling must stay quiet before the carousel snaps
/// to the nearest whole-cover boundary.
const SNAP_AFTER_MS: u64 = 350;
/// Layout spacer between the page top and the carousel: keeps the covers
/// clear of the floating pill (bubble + tail at the big picture title size),
/// which overlays the page and sizes itself from the cover's position.
const TITLE_AREA_HEIGHT: i32 = 68;

/// Widgets and selection state of the home carousel. `selected` runs over
/// the covers followed by the All Software tile (index == `games.len()`).
pub(super) struct HomeUi {
    page: gtk4::Overlay,
    row: RecentRow,
    scrolled: gtk4::ScrolledWindow,
    marquee: Marquee,
    ring: SelectionRing,
    /// The active capsule size; the carousel rebuilds when it changes.
    capsule: Rc<Cell<i32>>,
    /// The capsule and square-mode the current covers were built at: a
    /// viewport resize changes the capsule without touching the game
    /// list, and the unchanged-games guard alone would skip the rebuild.
    built_capsule: Cell<i32>,
    built_square: Cell<bool>,
    /// The row's edge spacing: covers sit this far below the scrolled
    /// window's top, so the tooltip measures the art top from there.
    spacing: i32,
    covers: RefCell<Vec<gtk4::Widget>>,
    games: RefCell<Vec<Game>>,
    /// The focused cover, if any. Scrolling the carousel by hand clears
    /// it; the arrows re-acquire from whatever is on screen.
    selected: Cell<Option<usize>>,
    /// The last selection before the carousel lost it (manual scrolling) —
    /// the arrows re-acquire here when it is still on screen.
    last_selected: Cell<Option<usize>>,
    scroll_anim: RefCell<Option<gtk4::TickCallbackId>>,
    /// The glide's latest goal, picked up every frame while it runs (same
    /// retarget-instead-of-restart as the game grid: repeats outpace one
    /// glide and restarting starves the camera).
    scroll_goal: Cell<f64>,
    /// The pending post-scroll settle snap, restarted on every native
    /// scroll event.
    snap_source: RefCell<Option<glib::SourceId>>,
    /// Games whose SGDB square is already being fetched in the background,
    /// so a refresh while the download runs does not re-queue them.
    square_queued: RefCell<HashSet<i64>>,
}

pub(super) fn build(state: &SharedState, square_mode: bool) -> (gtk4::Overlay, HomeUi) {
    // The floating title overlays the whole page — the same recipe as the
    // All Software grid — so its distance to the selected cover comes from
    // the cover's real position instead of a fixed rail's.
    let page = gtk4::Overlay::new();
    page.set_hexpand(true);

    // One sixth of the viewport: the capsule's size at any resolution.
    // 320 is the 1080p reference, used until the window reports a width.
    let capsule = Rc::new(Cell::new(320));

    let main = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let spring_top = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    spring_top.set_vexpand(true);
    main.append(&spring_top);

    let title_area = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    title_area.set_height_request(TITLE_AREA_HEIGHT);
    main.append(&title_area);

    let width = capsule_width(square_mode, capsule.get());
    let spacing = crate::ui::virtual_grid::VirtualGrid::grid_spacing_for_item_w(width);
    let row = RecentRow::new(spacing, capsule.get());
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Never);
    scrolled.set_valign(gtk4::Align::Start);
    // Side margins keep the selected cover's outline on screen at the ends
    // of the row.
    scrolled.set_margin_start(16);
    scrolled.set_margin_end(16);
    scrolled.add_css_class(CSS_RECENT_SCROLL);
    scrolled.set_child(Some(&row));
    // The floating title rides the scroll; the adjustment's changed signal
    // covers the first layout, when the page learns the viewport width.
    {
        let scroll_state = state.clone();
        let adj = scrolled.hadjustment();
        let track = move |_: &gtk4::Adjustment| {
            if let Some(big) = scroll_state.borrow().big_picture.clone() {
                // Rebuild the carousel when the viewport (and with it the
                // capsule size) changes; a no-op while it stays put.
                // Deferred to an idle: this signal fires mid-allocation, and
                // refresh reloads the stylesheet — re-styling widgets while
                // the layout pass is still walking them measures and
                // allocates them against mixed old/new styles.
                let desired = ((big.home.page.width() as f64 / 6.0).round() as i32).max(160);
                if desired != big.home.capsule.get() {
                    big.home.capsule.set(desired);
                    let refresh_state = scroll_state.clone();
                    glib::idle_add_local_once(move || {
                        super::view::refresh(&refresh_state);
                    });
                }
                sync_title_position(&big);
                // The rebuild above can run while the row is still
                // mid-allocation, so positions mapped through it are one
                // pass stale; re-anchor after the layout cycle settles.
                let idle_state = scroll_state.clone();
                glib::idle_add_local_once(move || {
                    if let Some(big) = idle_state.borrow().big_picture.clone() {
                        sync_title_position(&big);
                    }
                });
            }
        };
        adj.connect_value_changed(track.clone());
        adj.connect_changed(track);
        // Native scrolling (wheel, touchpad pan, kinetic glide) lands
        // wherever physics leaves it, and a cover sliced by the viewport
        // edge reads as a broken tile. Once the scrolling settles, snap
        // to the nearest whole-cover boundary.
        {
            let snap_state = state.clone();
            let snap_adj = scrolled.hadjustment();
            snap_adj.connect_value_changed(move |_| {
                if let Some(big) = snap_state.borrow().big_picture.clone() {
                    big.home.queue_boundary_snap(&snap_state);
                }
            });
        }
    }
    main.append(&scrolled);

    // Wheel and touchpad scrolling over the carousel deselect, exactly
    // like the All Software grid: the pointer has taken over, and the
    // arrows re-acquire from whatever is on screen. Without this the
    // settle snap kept yanking the row back to the stale selection.
    {
        let wheel_state = state.clone();
        let wheel = gtk4::EventControllerScroll::new(
            gtk4::EventControllerScrollFlags::VERTICAL | gtk4::EventControllerScrollFlags::HORIZONTAL,
        );
        wheel.set_propagation_phase(gtk4::PropagationPhase::Capture);
        wheel.connect_scroll(move |_, _, _| {
            if let Some(big) = wheel_state.borrow().big_picture.clone() {
                big.home.clear_selection();
            }
            glib::Propagation::Proceed
        });
        scrolled.add_controller(wheel);
    }

    let spring_bottom = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    spring_bottom.set_vexpand(true);
    main.append(&spring_bottom);
    page.set_child(Some(&main));

    let marquee = Marquee::new(0, capsule.get() as f64 * 2.0);
    // Fill the page: the pill floats anywhere over it.
    marquee.set_hexpand(true);
    marquee.set_halign(gtk4::Align::Fill);
    marquee.set_valign(gtk4::Align::Fill);
    let ring = SelectionRing::new();
    page.add_overlay(&ring);
    page.set_measure_overlay(&ring, false);
    // Same as the All Software grid: a mid-glide ring must not smear
    // past the carousel's own area.
    page.set_clip_overlay(&ring, true);
    page.add_overlay(marquee.widget());
    page.set_measure_overlay(marquee.widget(), false);


    let ui = HomeUi {
        page: page.clone(),
        row,
        scrolled,
        marquee,
        ring,
        capsule: Rc::clone(&capsule),
        built_capsule: Cell::new(0),
        built_square: Cell::new(false),
        spacing,
        covers: RefCell::new(Vec::new()),
        games: RefCell::new(Vec::new()),
        selected: Cell::new(None),
        last_selected: Cell::new(None),
        scroll_anim: RefCell::new(None),
        scroll_goal: Cell::new(0.0),
        snap_source: RefCell::new(None),
        square_queued: RefCell::new(HashSet::new()),
    };
    (page, ui)
}

/// Capsule width for the current mode: square art or 2:3 portraits.
fn capsule_width(square: bool, capsule: i32) -> i32 {
    if square {
        capsule
    } else {
        (capsule as f64 * 2.0 / 3.0) as i32
    }
}

/// The `_small` square thumbnail beside a full square image, when one is on
/// disk. Same directory, generated alongside the download or edit save.
fn small_square_path(square_path: &str) -> Option<String> {
    let dir = std::path::Path::new(square_path).parent()?;
    ira_parser::find_image_file(dir, "square_small").map(|p| p.to_string_lossy().into_owned())
}

/// Repopulate the carousel from the shared game list. Message handlers call
/// it whenever the game list or its artwork changes; the rebuild is skipped
/// when nothing visible differs.
pub(super) fn refresh(state: &SharedState) {
    let Some(big) = state.borrow().big_picture.clone() else {
        return;
    };
    let ui = &big.home;
    let games = crate::ui::helpers::recently_played(state, RECENT_LIMIT);

    // Pre-load stage: with no games yet, the stage stays empty — no
    // covers, no All Software tile, nothing floating over it. Everything
    // appears together once the library lands.
    if games.is_empty() {
        ui.row.clear_covers();
        ui.covers.borrow_mut().clear();
        *ui.games.borrow_mut() = Vec::new();
        ui.selected.set(None);
        ui.marquee.set_visible(false);
        ui.ring.set_visible(false);
        // Zero means "nothing built", so the first real refresh rebuilds.
        ui.built_capsule.set(0);
        ui.built_square.set(false);
        return;
    }

    // The capsule follows the viewport (one sixth of its width); 320 is
    // the reference until the window reports a size. The row's cover
    // height is kept in step so covers stay square.
    let square_mode = state.borrow().cfg.big_picture_square_capsules;
    let viewport = state.borrow().window.width();
    let capsule = if viewport >= 960 {
        (viewport as f64 / 6.0).round() as i32
    } else {
        ui.capsule.get().max(320)
    };

    // The achievement watcher re-reports games constantly; rebuilding the
    // whole carousel for each report churns covers for no visible change.
    // Only rebuild when the carousel's content — or the size it is built
    // at — actually differs.
    let unchanged = ui.built_capsule.get() == capsule
        && ui.built_square.get() == square_mode
        && {
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
    // resting on the tile follows the games count. A selection the pointer
    // cleared stays cleared.
    let selected_index = ui.selected.get();
    let at_tile = matches!(selected_index, Some(i) if i >= ui.games.borrow().len() && i > 0);
    let previous = selected_index.and_then(|i| {
        ui.games
            .borrow()
            .get(i)
            .map(Game::grid_id)
    });
    let was_empty = ui.games.borrow().is_empty();

    ui.row.clear_covers();

    ui.capsule.set(capsule);
    ui.built_capsule.set(capsule);
    ui.built_square.set(square_mode);
    ui.row.set_cover_height(capsule);
    let width = capsule_width(square_mode, capsule);
    let mut covers = Vec::with_capacity(games.len() + 1);
    for (index, game) in games.iter().enumerate() {
        // Square mode: cover-fit the art into a square capsule. A game
        // whose square.webp has not landed yet falls back to its vertical
        // capsule, scaled to cover (centered, overflow cropped). The
        // small square variant decodes much faster than the full art and
        // the capsule never draws larger than it.
        let art: String = if square_mode && !game.square_path.is_empty() {
            small_square_path(&game.square_path).unwrap_or_else(|| game.square_path.clone())
        } else {
            game.grid_path.clone()
        };
        let cover = build_cover(state, game, &art, width, capsule, square_mode, move |state| {
            on_cover_clicked(state, index)
        });
        ui.row.append_cover(&cover);
        covers.push(cover);
    }
    let tile = build_all_tile(state, capsule);
    ui.row.append_cover(&tile);
    covers.push(tile);
    *ui.covers.borrow_mut() = covers;

    let selected = if at_tile {
        Some(games.len())
    } else {
        match selected_index {
            // The same game if it survives; otherwise its old slot clamped
            // into the new list, so the camera never jumps to the start.
            Some(i) => games
                .iter()
                .position(|g| Some(g.grid_id()) == previous)
                .or_else(|| Some(i.min(games.len() - 1))),
            None => None,
        }
    };
    // First population parks on the first cover, like the grid parks on
    // its first tile; only a surface entry re-parks a cleared selection.
    if was_empty {
        ui.selected.set(Some(0));
    } else {
        ui.selected.set(selected);
    }
    if let Some(selected) = ui.selected.get() {
        ui.last_selected.set(Some(selected));
    }
    *ui.games.borrow_mut() = games;
    ui.marquee.set_max_width(width as f64 * 2.0);
    apply_selection(&big);
    // Queued after the swap so it sees the freshly loaded list.
    queue_missing_squares(state, ui, square_mode);
}

/// Re-measure the floating title after the big-picture scale changed (see
/// `Marquee::revalidate_text`).
pub(super) fn revalidate_pill(big: &Rc<BigPictureUi>) {
    big.home.marquee.revalidate_text();
}

/// Mouse navigation on a cover: the first click selects, clicking the
/// already-selected cover plays. The All Software tile jumps to the
/// Everything tab.
fn on_cover_clicked(state: &SharedState, index: usize) {
    let is_tile = state
        .borrow()
        .big_picture
        .as_ref()
        .is_some_and(|big| index >= big.home.games.borrow().len());
    if is_tile {
        if let Some(big) = state.borrow().big_picture.clone() {
            big.all.set_tab(state, super::all_games::Tab::Everything);
        }
        return;
    }
    let selected = state
        .borrow()
        .big_picture
        .as_ref()
        .and_then(|big| big.home.selected.get());
    if selected == Some(index) {
        super::home::launch_selected(state);
    } else {
        // First click focuses the cover the pointer is on; the camera
        // stays put (the cover is on screen by definition) and the second
        // click plays it.
        let Some(big) = state.borrow().big_picture.clone() else {
            return;
        };
        big.home.selected.set(Some(index));
        big.home.last_selected.set(Some(index));
        restyle_selection(&big);
        sync_title_text(&big.home);
        sync_title_position(&big);
    }
}

pub(super) fn move_selection(state: &SharedState, delta: i32, engage: bool) {
    let Some(big) = state.borrow().big_picture.clone() else {
        return;
    };
    let ui = &big.home;
    // One past the last game is the All Software tile.
    let count = ui.games.borrow().len() + 1;
    if count <= 1 {
        return;
    }
    // The arrows moved the selection with nothing focused — the user
    // scrolled the carousel by hand, which deselects. Re-acquire on a
    // visible cover first (the old one when it is still on screen), with
    // the camera parked: the step below moves from somewhere on screen
    // and its own glide carries the camera.
    if ui.selected.get().is_none() {
        let Some((first, last)) = ui.visible_range() else {
            return;
        };
        let start = reacquire_index(ui.last_selected.get(), first, last);
        ui.selected.set(Some(start));
        ui.last_selected.set(Some(start));
        restyle_selection(&big);
        sync_title_text(ui);
        sync_title_position(&big);
    }
    let Some(next) = next_selection(ui.selected.get().unwrap_or(0), count, delta, engage) else {
        return;
    };
    ui.selected.set(Some(next));
    ui.last_selected.set(Some(next));
    apply_selection(&big);
}

/// Where an arrow press resumes after hand-scrolling left nothing
/// selected: the old selection clamped into whatever the view currently
/// shows, or the first visible cover with no history. The camera never
/// jumps to where the selection used to be.
fn reacquire_index(last: Option<usize>, first_visible: usize, last_visible: usize) -> usize {
    match last {
        Some(old) => old.clamp(first_visible, last_visible),
        None => first_visible,
    }
}

/// End the carousel's scroll glide on a whole-cover boundary, so a tab
/// slide never shows a half-scrolled cover at the viewport edge: the
/// incoming page's edge always starts on a full cover.
pub(super) fn finish_scroll(state: &SharedState) {
    let Some(big) = state.borrow().big_picture.clone() else {
        return;
    };
    let ui = &big.home;
    if let Some(id) = ui.scroll_anim.borrow_mut().take() {
        id.remove();
    }
    let adj = ui.scrolled.hadjustment();
    let target = match ui.selected.get() {
        Some(selected) => whole_cover_target(ui, &adj, selected),
        None => nearest_boundary_target(ui, &adj),
    };
    if let Some(target) = target {
        adj.set_value(target);
    }
}

/// Restart the settle timer: native scrolling is still moving.
impl HomeUi {
    /// The first and last indexes (covers, then the All Software tile)
    /// at least partly on screen. None when nothing is laid out to see.
    fn visible_range(&self) -> Option<(usize, usize)> {
        let adj = self.scrolled.hadjustment();
        let (value, page) = (adj.value(), adj.page_size());
        let count = self.games.borrow().len() + 1;
        let mut first = None;
        let mut last = 0;
        for index in 0..count {
            let Some((x, w)) = self.row.cover_geometry(index) else {
                continue;
            };
            if x + w > value && x < value + page {
                if first.is_none() {
                    first = Some(index);
                }
                last = index;
            }
        }
        first.map(|first| (first, last))
    }

    /// The user scrolled the carousel by hand: the focus goes away
    /// entirely, so the settle snap rests where the pointer left the row
    /// instead of dragging it back to the stale selection. The arrows
    /// bring the focus back on a visible cover (see `move_selection`).
    fn clear_selection(&self) {
        if self.selected.get().is_none() {
            return;
        }
        self.selected.set(None);
        for cover in self.covers.borrow().iter() {
            cover.remove_css_class(CSS_BP_SELECTED);
        }
        self.row.set_selected_cover(None);
        self.marquee.set_visible(false);
        self.ring.set_visible(false);
    }

    fn queue_boundary_snap(&self, state: &SharedState) {
        if let Some(id) = self.snap_source.borrow_mut().take() {
            id.remove();
        }
        let pending = state.clone();
        let id = glib::timeout_add_local(Duration::from_millis(SNAP_AFTER_MS), move || {
            if let Some(big) = pending.borrow().big_picture.clone() {
                // Returning Break destroys this source, so the stored
                // handle dies with it — drop it first or the next restart
                // removes a dead source and glib aborts. It must precede
                // the snap: a moved adjustment re-queues a fresh source
                // through the hook, and clearing after would eat that one.
                big.home.snap_source.borrow_mut().take();
                snap_to_boundary(&big.home);
            }
            glib::ControlFlow::Break
        });
        *self.snap_source.borrow_mut() = Some(id);
    }
}

/// Jump to the whole-cover boundary nearest where native scrolling
/// stopped, so a resting carousel never shows a cover sliced by the
/// viewport edge. Anchored to the scroll itself, never to the selection:
/// a hand-scrolled carousel has none (wheel scrolling deselects), and
/// pulling toward a stale selection is what dragged the row back.
fn snap_to_boundary(ui: &HomeUi) {
    let adj = ui.scrolled.hadjustment();
    if let Some(target) = nearest_boundary_target(ui, &adj) {
        adj.set_value(target);
    }
}

/// `current` stepped by `delta`. A wrap is the edge tile's privilege and
/// happens once, on a fresh press (`allow_wrap`): from anywhere else the
/// row moves one step, and a held direction that reaches an end meets a
/// wall instead of cycling around forever.
fn next_selection(
    current: usize,
    count: usize,
    delta: i32,
    allow_wrap: bool,
) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let next = current as i64 + delta as i64;
    if allow_wrap {
        Some(next.rem_euclid(count as i64) as usize)
    } else {
        Some(next.clamp(0, count as i64 - 1) as usize)
    }
}

/// Launch the selected game through the shared launch path (which already
/// guards against double launches). A no-op when nothing is selected or on
/// the All Software tile.
pub(super) fn launch_selected(state: &SharedState) {
    let game = state
        .borrow()
        .big_picture
        .as_ref()
        .and_then(|big| {
            let ui = &big.home;
            let selected = ui.selected.get()?;
            ui.games.borrow().get(selected).cloned()
        });
    let Some(game) = game else {
        return;
    };
    if let Err(error) = crate::ui::play_button::launch_game(state, game.db_id, game.variant_id) {
        eprintln!("Failed to launch game: {error}");
        let _ = state
            .borrow()
            .sender
            .send(crate::AppMessage::AddGameError(error));
    }
}

/// True when the selection rests on the All Software tile rather than a
/// game cover. False with nothing selected at all.
pub(super) fn selection_is_tile(big: &Rc<BigPictureUi>) -> bool {
    matches!(
        big.home.selected.get(),
        Some(selected) if selected >= big.home.games.borrow().len()
    )
}

/// Selection visuals, floating title, and scroll.
/// The gamepad or keyboard moved the selection: restyle it and bring it
/// to the viewport center.
fn apply_selection(big: &Rc<BigPictureUi>) {
    restyle_selection(big);
    sync_title_text(&big.home);
    sync_title_position(big);
    update_scroll(big);
}

/// Restyle the covers and re-anchor the floating title and ring to the
/// selection without touching the scroll position.
fn restyle_selection(big: &Rc<BigPictureUi>) {
    let ui = &big.home;
    let selected = ui.selected.get();
    let covers = ui.covers.borrow();
    for (index, cover) in covers.iter().enumerate() {
        if Some(index) == selected {
            cover.add_css_class(CSS_BP_SELECTED);
        } else {
            cover.remove_css_class(CSS_BP_SELECTED);
        }
    }
    let selected_cover = selected.and_then(|selected| covers.get(selected).cloned());
    drop(covers);
    ui.row.set_selected_cover(selected_cover.as_ref());
}

fn sync_title_text(ui: &HomeUi) {
    let Some(selected) = ui.selected.get() else {
        ui.marquee.set_visible(false);
        return;
    };
    let game = ui.games.borrow().get(selected).cloned();
    let title = match game {
        Some(game) => game.name,
        None => crate::tr!("Everything"),
    };
    ui.marquee.set_text(&title);
}

/// Slide the floating title so it rides just above the selected cover,
/// tail touching it — the same tile-relative distance the All Software
/// grid keeps.
fn sync_title_position(big: &Rc<BigPictureUi>) {
    let ui = &big.home;
    let Some(selected) = ui.selected.get() else {
        ui.marquee.set_visible(false);
        ui.ring.set_visible(false);
        return;
    };
    let Some((x, w)) = ui.row.cover_geometry(selected) else {
        // Nothing to anchor to (the stage is still empty): hide the
        // floats rather than leave them painted at a stale spot.
        ui.marquee.set_visible(false);
        ui.ring.set_visible(false);
        return;
    };
    ui.ring.set_visible(true);
    let adj = ui.scrolled.hadjustment();
    // Cover center in page coordinates: the scrolled margin plus content
    // x (cover_geometry includes the leading spacing) minus the scroll.
    let center = ui.scrolled.margin_start() as f64 + x + w / 2.0 - adj.value();
    // The cover's top edge, mapped into the page the marquee spans. The
    // row lays its covers one spacing below its own top.
    let cover_top = ui
        .scrolled
        .compute_point(&ui.page, &gtk4::graphene::Point::zero())
        .map(|point| point.y() as f64 + ui.spacing as f64)
        .unwrap_or(ui.spacing as f64);
    // The All Software tile's visible content is its centered half-size
    // circle, not the full capsule shell — anchor to the circle, or the
    // pill floats far above the thing it points at.
    let is_tile = selected >= ui.games.borrow().len();
    let capsule = ui.capsule.get() as f64;
    let content_top = cover_top
        + if is_tile {
            (capsule - capsule / 2.0) / 2.0
        } else {
            0.0
        };
    // Top bias: the tail tip rests just outside the selection ring, the
    // same distance the All Software grid keeps. The viewport is the
    // page's own width — final here, unlike the floating marquee's,
    // which lags one allocation pass behind (and starts at zero).
    let viewport = ui.page.width() as f64;
    // Any overlap keeps the floats: the camera clamps at the row's ends,
    // so the selected cover — the All Software tile above all — can rest
    // partway off the edge. Demanding full visibility there left the
    // selection markerless exactly when it was picked.
    let visible = (|| {
        let cover = ui.covers.borrow().get(selected).cloned()?;
        let left = f64::from(
            cover
                .compute_point(&ui.scrolled, &gtk4::graphene::Point::zero())?
                .x(),
        );
        let view = ui.scrolled.width() as f64
            - f64::from(ui.scrolled.margin_start() + ui.scrolled.margin_end());
        Some(left + cover.width() as f64 > 0.0 && left < view)
    })()
    .unwrap_or(false);
    if !visible {
        ui.marquee.set_visible(false);
        ui.ring.set_visible(false);
        return;
    }
    ui.marquee.set_visible(true);
    ui.ring.set_visible(true);
    ui.marquee
        .set_position(center, viewport, content_top - crate::ui::css::bp_ring_outset(), false);
    let ring_scale = viewport / 1920.0;
    // The ring hugs what is visibly selected: the full capsule on a game
    // cover, the centered half-size circle on the All Software tile — a
    // true circle there, since the tile's content is round.
    if is_tile {
        let circle = capsule / 2.0;
        ui.ring
            .place(center - circle / 2.0, content_top, circle, circle, ring_scale, true);
    } else {
        ui.ring
            .place(center - w / 2.0, cover_top, w, capsule, ring_scale, false);
    }
}

/// The scroll value that shows only whole covers. The carousel never
/// rests with a cover sliced by the viewport edge — a half-visible cover
/// reads as a broken tile, and a tab slide revealing "the rest of it"
/// makes it worse — so the selected cover's centered position is snapped
/// to the whole-cover boundary nearest it.
fn whole_cover_target(ui: &HomeUi, adj: &gtk4::Adjustment, selected: usize) -> Option<f64> {
    let max = (adj.upper() - adj.page_size()).max(0.0);
    let (x, w) = ui.row.cover_geometry(selected)?;
    let centered = (x + w / 2.0 - adj.page_size() / 2.0).clamp(0.0, max);
    let Some((first_x, pitch)) = boundary_rail(ui) else {
        return Some(centered);
    };
    Some(boundary_target(first_x, pitch, centered, max))
}

/// The whole-cover boundary nearest where the scroll currently rests.
fn nearest_boundary_target(ui: &HomeUi, adj: &gtk4::Adjustment) -> Option<f64> {
    let max = (adj.upper() - adj.page_size()).max(0.0);
    let (first_x, pitch) = boundary_rail(ui)?;
    Some(boundary_target(first_x, pitch, adj.value(), max))
}

/// Where the cover boundaries sit: the row's first cover x and the step
/// between consecutive covers. None with fewer than two covers.
fn boundary_rail(ui: &HomeUi) -> Option<(f64, f64)> {
    let first_x = ui.row.cover_geometry(0)?.0;
    let pitch = ui.row.cover_geometry(1)?.0 - first_x;
    (pitch > 1.0).then_some((first_x, pitch))
}

/// The whole-cover boundary nearest `anchor`, as a scroll value. The row's
/// leading spacing pad (scroll 0) is itself a resting spot: the first
/// boundary would plant cover 0 flush against the viewport edge, so any
/// anchor that rounds onto it rests at 0 instead — the inset view the
/// carousel opens with, and the only way back to it after scrolling away.
fn boundary_target(first_x: f64, pitch: f64, anchor: f64, max: f64) -> f64 {
    let step = ((anchor - first_x) / pitch).round();
    let target = if step <= 0.0 {
        0.0
    } else {
        first_x + step * pitch
    };
    target.clamp(0.0, max)
}

/// Smooth-scroll the selected tile into the viewport; the adjustment's
/// value_changed signal keeps the floating title glued to it. Steps on
/// frame-clock ticks, in sync with vsync, not on drifting timers.
fn update_scroll(big: &Rc<BigPictureUi>) {
    let ui = &big.home;
    let Some(selected) = ui.selected.get() else {
        return;
    };
    let adj = ui.scrolled.hadjustment();
    let Some(target) = whole_cover_target(ui, &adj, selected) else {
        return;
    };
    ui.scroll_goal.set(target);
    if ui.scroll_anim.borrow().is_some() {
        return;
    }
    let start = adj.value();
    if (target - start).abs() < 0.5 {
        return;
    }
    let adj = adj.clone();
    let started = Instant::now();
    let ticker_big = Rc::clone(big);
    let id = ui.scrolled.add_tick_callback(move |_, _| {
        let t = (started.elapsed().as_millis() as f64 / SCROLL_MILLIS as f64).min(1.0);
        let eased = 1.0 - (1.0 - t) * (1.0 - t);
        let g = ticker_big.home.scroll_goal.get();
        adj.set_value(start + (g - start) * eased);
        if t >= 1.0 {
            adj.set_value(g);
            *ticker_big.home.scroll_anim.borrow_mut() = None;
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    *ui.scroll_anim.borrow_mut() = Some(id);
}

/// The grid tile at the end of the carousel that opens All Software: a
/// half-size circle in the capsule shell, so the selection ring hugs the
/// circle instead of drawing a square around empty space.
fn build_all_tile(state: &SharedState, capsule: i32) -> gtk4::Widget {
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    vbox.set_valign(gtk4::Align::Start);
    vbox.set_halign(gtk4::Align::Center);
    vbox.add_css_class(CSS_COVER_ITEM);
    vbox.add_css_class(CSS_BP_ALL);
    vbox.set_size_request(capsule, capsule);
    vbox.set_overflow(gtk4::Overflow::Visible);

    let circle = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    circle.add_css_class(CSS_BP_ALL_TILE);
    circle.set_size_request(capsule / 2, capsule / 2);
    circle.set_halign(gtk4::Align::Center);
    circle.set_valign(gtk4::Align::Center);
    let icon = gtk4::Image::from_icon_name("go-next-symbolic");
    icon.set_pixel_size(56);
    icon.set_opacity(0.7);
    icon.set_vexpand(true);
    icon.set_valign(gtk4::Align::Center);
    circle.append(&icon);
    vbox.append(&circle);

    let open_state = state.clone();
    let click = gtk4::GestureClick::new();
    click.connect_pressed(move |_, _, _, _| {
        if let Some(big) = open_state.borrow().big_picture.clone() {
            big.all.set_tab(&open_state, super::all_games::Tab::Everything);
        }
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
                crate::ui::fetch_images::ensure_game_square(&steam, &save_dir, &db, &cfg, &game);
            if !square.is_empty() {
                let _ = sender.send(crate::AppMessage::SquareReady(game.db_id));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{boundary_target, next_selection, reacquire_index};

    #[test]
    fn test_next_selection_wraps_once_from_the_edge_on_a_fresh_press() {
        // Engaged steps wrap at the edge tiles.
        assert_eq!(next_selection(0, 5, -1, true), Some(4));
        assert_eq!(next_selection(4, 5, 1, true), Some(0));
        // Held repeats meet a wall there; interior steps just move one.
        assert_eq!(next_selection(0, 5, -1, false), Some(0));
        assert_eq!(next_selection(4, 5, 1, false), Some(4));
        assert_eq!(next_selection(0, 5, 1, false), Some(1));
        assert_eq!(next_selection(2, 5, 2, true), Some(4));
        assert_eq!(next_selection(0, 0, 1, true), None);
    }

    #[test]
    fn test_boundary_target_rests_at_the_leading_pad_near_the_start() {
        let first_x = 8.0;
        let pitch = 248.0;
        let max = 4_000.0;
        // Anything rounding onto the first boundary rests at 0, where the
        // row's leading pad insets the covers — the opening view.
        assert_eq!(boundary_target(first_x, pitch, 0.0, max), 0.0);
        assert_eq!(boundary_target(first_x, pitch, first_x, max), 0.0);
        assert_eq!(
            boundary_target(first_x, pitch, first_x + pitch / 2.0 - 1.0, max),
            0.0
        );
        // Past the first boundary's midpoint the second takes over.
        assert_eq!(
            boundary_target(first_x, pitch, first_x + pitch / 2.0 + 1.0, max),
            first_x + pitch
        );
        // Ordinary boundaries land on themselves.
        assert_eq!(
            boundary_target(first_x, pitch, first_x + 3.0 * pitch, max),
            first_x + 3.0 * pitch
        );
        // And the tail clamps at the scroll's end.
        assert_eq!(boundary_target(first_x, pitch, 10_000.0, max), max);
    }

    #[test]
    fn test_reacquire_index_clamps_the_old_selection_into_view() {
        // The old selection is on screen: it comes back exactly.
        assert_eq!(reacquire_index(Some(3), 1, 6), 3);
        // It is off screen: its slot clamps into the visible range.
        assert_eq!(reacquire_index(Some(0), 4, 8), 4);
        assert_eq!(reacquire_index(Some(9), 2, 7), 7);
        // No history: the first visible cover.
        assert_eq!(reacquire_index(None, 4, 8), 4);
    }
}
