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
const RECENT_LIMIT: usize = 16;
/// Selection scroll animation length.
const SCROLL_MILLIS: u64 = 90;
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
    selected: RefCell<usize>,
    scroll_anim: RefCell<Option<glib::SourceId>>,
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
                let desired = ((big.home.page.width() as f64 / 6.0).round() as i32).max(160);
                if desired != big.home.capsule.get() {
                    big.home.capsule.set(desired);
                    super::view::refresh(&scroll_state);
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
    }
    main.append(&scrolled);

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
        selected: RefCell::new(0),
        scroll_anim: RefCell::new(None),
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
        *ui.selected.borrow_mut() = 0;
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
    // resting on the tile follows the games count.
    let selected_index = *ui.selected.borrow();
    let at_tile = selected_index >= ui.games.borrow().len() && selected_index > 0;
    let previous = ui.games.borrow().get(selected_index).map(Game::grid_id);

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
        games.len()
    } else {
        games
            .iter()
            .position(|g| Some(g.grid_id()) == previous)
            .unwrap_or(0)
    };
    *ui.selected.borrow_mut() = selected;
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
/// already-selected cover plays. The All Software tile opens immediately.
fn on_cover_clicked(state: &SharedState, index: usize) {
    let is_tile = state
        .borrow()
        .big_picture
        .as_ref()
        .is_some_and(|big| index >= big.home.games.borrow().len());
    if is_tile {
        super::view::open_all(state);
        return;
    }
    let selected = state
        .borrow()
        .big_picture
        .as_ref()
        .map(|big| *big.home.selected.borrow())
        .unwrap_or(usize::MAX);
    if selected == index {
        super::view::confirm(state);
    } else {
        // First click focuses the cover the pointer is on; the camera
        // stays put (the cover is on screen by definition) and the second
        // click plays it.
        let Some(big) = state.borrow().big_picture.clone() else {
            return;
        };
        *big.home.selected.borrow_mut() = index;
        restyle_selection(&big);
        sync_title_text(&big.home);
        sync_title_position(&big);
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
    *ui.selected.borrow_mut() = next;
    apply_selection(&big);
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
    if let Err(error) = crate::ui::play_button::launch_game(state, game.db_id, game.variant_id) {
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

/// Slide the floating title so it rides just above the selected cover,
/// tail touching it — the same tile-relative distance the All Software
/// grid keeps.
fn sync_title_position(big: &Rc<BigPictureUi>) {
    let ui = &big.home;
    let selected = *ui.selected.borrow();
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
    // The pill points at icons, like the All Software grid does: while the
    // selected cover has no art yet (library still loading) it stays
    // hidden instead of floating over an empty frame.
    let has_art = ui
        .games
        .borrow()
        .get(selected)
        .is_none_or(|g| !g.grid_path.is_empty() || !g.square_path.is_empty());
    // The floats only track the selection while it sits fully inside the
    // scrolled viewport (ring outset included): a cover sliding off the
    // edge must not leave a detached pill behind, and the cover keeps its
    // own selected style meanwhile.
    let fully_visible = (|| {
        let cover = ui.covers.borrow().get(selected).cloned()?;
        let left = f64::from(
            cover
                .compute_point(&ui.scrolled, &gtk4::graphene::Point::zero())?
                .x(),
        );
        let view = ui.scrolled.width() as f64
            - f64::from(ui.scrolled.margin_start() + ui.scrolled.margin_end());
        Some(
            left - BP_RING_OUTSET >= 0.0 && left + cover.width() as f64 + BP_RING_OUTSET <= view,
        )
    })()
    .unwrap_or(false);
    if !fully_visible {
        ui.marquee.set_visible(false);
        ui.ring.set_visible(false);
        return;
    }
    // The ring marks the selection whether or not art has landed; only
    // the pill needs something to point at.
    ui.ring.set_visible(true);
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
    if !has_art {
        ui.marquee.set_visible(false);
        return;
    }
    ui.marquee.set_visible(true);
    ui.marquee
        .set_position(center, viewport, content_top - BP_RING_OUTSET, false);
}

/// Smooth-scroll the selected tile to the viewport center; the adjustment's
/// value_changed signal keeps the floating title glued to it.
fn update_scroll(big: &Rc<BigPictureUi>) {
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
    let icon = gtk4::Image::from_icon_name("view-grid-symbolic");
    icon.set_pixel_size(56);
    icon.set_opacity(0.7);
    icon.set_vexpand(true);
    icon.set_valign(gtk4::Align::Center);
    circle.append(&icon);
    vbox.append(&circle);

    let open_state = state.clone();
    let click = gtk4::GestureClick::new();
    click.connect_pressed(move |_, _, _, _| {
        super::view::open_all(&open_state);
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
