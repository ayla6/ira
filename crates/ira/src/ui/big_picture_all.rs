//! The All Software page: every visible game on a square-capsule grid,
//! reached from the grid tile at the end of the home carousel. Controller
//! and keyboard drive an external selection highlight (the grid itself only
//! recycles cells); A launches, B returns home.

use super::css::*;
use super::big_picture_marquee::{Marquee, TAIL_HEIGHT};
use super::selection_ring::SelectionRing;
use super::state::SharedState;
use super::virtual_grid::VirtualGrid;
use crate::Game;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};

/// The game a couch grid action targets: launch uses the same (db, variant)
/// pair the desktop grid stores on its cells.
type GameKey = (i64, i64);

/// Width the floating name tooltip clips its text to.
const TOOLTIP_MAX_WIDTH: f64 = 480.0;
/// Selection scroll animation length (matches the home carousel).
const SCROLL_MILLIS: u64 = 90;

fn game_key(game: &Game) -> GameKey {
    (game.db_id, game.variant_id.unwrap_or(0))
}

/// Where pressing (dx, dy) from `current` lands in a `cols`-wide grid of
/// `count` items; None when the grid is empty or the press is a no-op.
/// Horizontal moves stay on the row, vertical moves keep the column, and
/// overshoot into a short last row slides onto its last item.
fn grid_move(current: usize, count: usize, cols: usize, dx: i32, dy: i32) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let cols = cols.max(1) as i64;
    let col = (current as i64) % cols;
    let row = (current as i64) / cols;
    let next_col = (col + dx as i64).clamp(0, cols - 1);
    let next_row = (row + dy as i64).max(0);
    let next = (next_row * cols + next_col).clamp(0, count as i64 - 1) as usize;
    (next != current).then_some(next)
}

/// The selection outline pokes this far past a tile's edge; the scroll
/// rests that much above every row boundary so the top row's outline
/// stays on screen instead of being shaved at the viewport edge.
const OUTLINE_ALLOWANCE: f64 = 7.0;

/// The vertical scroll that keeps the selected row in view, resting only
/// on whole-row boundaries (minus the outline's room): the viewport's top
/// row is never left half cut nor missing its outline. A hidden row
/// enters by the least whole rows that fit it — stepping down moves about
/// a row and a half, never a jump to the top.
fn scroll_target(
    index: usize,
    cols: usize,
    row_h: f64,
    top_pad: f64,
    value: f64,
    page: f64,
) -> f64 {
    let cols = cols.max(1);
    let top = top_pad + (index / cols) as f64 * row_h;
    let bottom = top + row_h;
    let rows = if top < value {
        ((top - OUTLINE_ALLOWANCE - top_pad) / row_h).floor().max(0.0)
    } else if bottom + OUTLINE_ALLOWANCE > value + page {
        (((bottom + OUTLINE_ALLOWANCE - page - top_pad) / row_h).ceil()).max(0.0)
    } else {
        return value;
    };
    (top_pad + rows * row_h - OUTLINE_ALLOWANCE).max(0.0)
}

/// Widgets and selection state of the All Software page.
pub(super) struct AllSoftwareUi {
    page: gtk4::Box,
    scrolled: gtk4::ScrolledWindow,
    grid: VirtualGrid,
    /// The selected game's name as a tooltip pill floating above its tile.
    tooltip: Marquee,
    /// The drawn selection ring floating over the selected tile.
    ring: SelectionRing,
    /// The overlay both float in — their coordinates are overlay-relative.
    overlay: gtk4::Overlay,
    empty: gtk4::Label,
    games: RefCell<Vec<Game>>,
    selected: Cell<usize>,
    /// The highlighted game as a (db, variant) key, shared with the bind
    /// closure so cells can style themselves without a rebuild.
    selected_key: Rc<Cell<GameKey>>,
    /// The running scroll glide, so a new press replaces it mid-flight.
    scroll_anim: Rc<RefCell<Option<glib::SourceId>>>,
    opened: Cell<bool>,
}

pub(super) fn build(state: &SharedState) -> (gtk4::Box, AllSoftwareUi) {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    page.add_css_class(CSS_BP_ALL_PAGE);

    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    header.set_margin_top(18);
    header.set_margin_bottom(6);
    header.set_margin_start(28);
    header.set_margin_end(28);
    // Controllers back out with B; the mouse needs a visible way home.
    // It must not take focus: the page switch would otherwise grab it and
    // paint the button as highlighted the moment the page opens.
    let back = gtk4::Button::from_icon_name("go-previous-symbolic");
    back.add_css_class(CSS_FLAT);
    back.set_focusable(false);
    back.set_tooltip_text(Some(&crate::tr!("Back")));
    back.set_valign(gtk4::Align::Center);
    {
        let back_state = state.clone();
        back.connect_clicked(move |_| super::big_picture_view::show_home(&back_state));
    }
    header.append(&back);
    let icon = gtk4::Image::from_icon_name("view-grid-symbolic");
    icon.set_pixel_size(24);
    icon.set_valign(gtk4::Align::Center);
    header.append(&icon);
    let title = gtk4::Label::new(Some(&crate::tr!("All Software")));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.add_css_class(CSS_BP_PAGE_TITLE);
    super::helpers::crisp_label(&title);
    header.append(&title);
    let ordering = gtk4::Label::new(Some(&crate::tr!("By name")));
    ordering.set_valign(gtk4::Align::Center);
    ordering.add_css_class(CSS_BP_PAGE_SUBTITLE);
    super::helpers::crisp_label(&ordering);
    header.append(&ordering);
    page.append(&header);

    let grid = VirtualGrid::new(240);
    grid.set_square(true);
    // Six tiles per row at every resolution, Switch-style: the tile size
    // scales with the viewport and the margins derive from the tile.
    grid.set_fixed_cols(6);
    // Recycled cells must never paint outside the viewport, over the page
    // header above.
    grid.set_overflow(gtk4::Overflow::Hidden);
    let store = gio::ListStore::new::<super::game_item::GameItem>();
    grid.set_model(&store);
    let selected_key = Rc::new(Cell::new((0, 0)));
    let (setup, bind, unbind) =
        cell_factories(state, grid.item_size_cell(), Rc::clone(&selected_key));
    grid.set_factory(setup, bind, unbind);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scrolled.set_vexpand(true);
    // No side margins here: the grid spaces its own edges exactly like
    // its tile gaps, so the vertical and horizontal margins all come
    // from one value.
    scrolled.set_child(Some(&grid));

    // The floating name tooltip rides the scroll over the grid, below the
    // selected tile. It draws over the games, so it lives in an overlay
    // that neither measures nor gets clipped by the viewport.
    let tooltip = Marquee::new(0, TOOLTIP_MAX_WIDTH);
    // Fill the overlay: the tooltip may float anywhere over the grid, and
    // its paint only exists inside its own allocation.
    tooltip.set_hexpand(true);
    tooltip.set_vexpand(true);
    tooltip.set_halign(gtk4::Align::Fill);
    tooltip.set_valign(gtk4::Align::Fill);
    let grid_overlay = gtk4::Overlay::new();
    grid_overlay.set_child(Some(&scrolled));
    let ring = SelectionRing::new();
    grid_overlay.add_overlay(&ring);
    grid_overlay.set_measure_overlay(&ring, false);
    grid_overlay.add_overlay(tooltip.widget());
    grid_overlay.set_measure_overlay(tooltip.widget(), false);
    // Every grid allocation re-anchors the floating overlays: without this
    // a relayout moves the tiles and leaves the tooltip/ring at the stale
    // spot until the next scroll or selection change.
    grid.set_post_layout_fn({
        let overlay_state = state.clone();
        Rc::new(move || {
            if let Some(big) = overlay_state.borrow().big_picture.clone() {
                big.all.update_tooltip();
            }
        })
    });
    grid_overlay.set_clip_overlay(tooltip.widget(), false);
    page.append(&grid_overlay);

    let empty = gtk4::Label::new(Some(&crate::tr!("No games yet")));
    empty.add_css_class(CSS_BP_PAGE_SUBTITLE);
    empty.set_vexpand(true);
    empty.set_visible(false);
    page.append(&empty);

    // The tooltip tracks the scroll; the adjustment's changed signal
    // covers the first layout (when the page opens), which is what puts
    // the tooltip in the right place from its first paint.
    {
        let scroll_state = state.clone();
        let adj = scrolled.vadjustment();
        let track = move |_: &gtk4::Adjustment| {
            if let Some(big) = scroll_state.borrow().big_picture.clone() {
                big.all.update_tooltip();
            }
        };
        adj.connect_value_changed(track.clone());
        adj.connect_changed(track);
    }

    let ui = AllSoftwareUi {
        page,
        scrolled,
        grid,
        tooltip,
        empty: empty.clone(),
        games: RefCell::new(Vec::new()),
        selected: Cell::new(0),
        selected_key,
        scroll_anim: Rc::new(RefCell::new(None)),
        ring,
        overlay: grid_overlay,
        opened: Cell::new(false),
    };
    (ui.page.clone(), ui)
}

impl AllSoftwareUi {
    /// The game the selection currently rests on, if any.
    pub(super) fn selected_game(&self) -> Option<Game> {
        self.games
            .borrow()
            .get(self.selected.get())
            .cloned()
    }

    /// Swap in the full alphabetized list. Rebuilds the store only when the
    /// content actually differs; the achievement watcher re-reports often.
    pub(super) fn refresh(&self, state: &SharedState) {
        let show_hidden = state.borrow().cfg.show_hidden_games;
        let mut games: Vec<Game> = state
            .borrow()
            .games
            .iter()
            .filter(|g| !g.hidden || show_hidden)
            .cloned()
            .collect();
        games.sort_by(|a, b| a.sort_key().cmp(b.sort_key()).then_with(|| a.db_id.cmp(&b.db_id)));

        let unchanged = {
            let current = self.games.borrow();
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
        self.sync_store(&games);
    }

    /// Replace the model contents and restyle around the surviving game.
    fn sync_store(&self, games: &[Game]) {
        let store = gio::ListStore::new::<super::game_item::GameItem>();
        for game in games {
            store.append(&super::game_item::GameItem::new(game));
        }
        // The model swap replays items_changed with removed != added, so the
        // grid clears its visible cells and rebinds from the new store.
        self.grid.set_model(&store);

        let selected = self
            .selected
            .get()
            .min(games.len().saturating_sub(1));
        self.selected.set(selected);
        if let Some(game) = games.get(selected) {
            self.selected_key.set(game_key(game));
        }
        self.empty.set_visible(games.is_empty());
        self.scrolled.set_visible(!games.is_empty());
        self.tooltip.set_visible(!games.is_empty());
        if let Some(game) = games.get(selected) {
            self.tooltip.set_text(&game.name);
        }
        *self.games.borrow_mut() = games.to_vec();
        self.update_tooltip();
    }

    /// First open: park the selection and the scroll at the top. Later
    /// opens keep both.
    pub(super) fn ensure_opened(&self) {
        if self.opened.get() {
            self.scroll_to_selected();
            return;
        }
        self.opened.set(true);
        // The pill's text was set while this page sat hidden — possibly at
        // a different couch scale — so its layout may measure stale.
        self.tooltip.revalidate_text();
        self.apply_selection(0);
    }

    /// Re-measure the pill after the couch scale changed (see
    /// `Marquee::revalidate_text`).
    pub(super) fn revalidate_tooltip(&self) {
        self.tooltip.revalidate_text();
    }

    pub(super) fn move_selection(&self, dx: i32, dy: i32) {
        let (cols, _, _item_h, _sp) = self.grid.current_layout();
        let count = self.games.borrow().len();
        if let Some(next) = grid_move(self.selected.get(), count, cols as usize, dx, dy) {
            self.apply_selection(next);
        }
    }

    /// Point the highlight at `index`: update the shared key, restyle the
    /// visible cells, float the name tooltip over the tile, and scroll the
    /// row into view.
    fn apply_selection(&self, index: usize) {
        let game = self.games.borrow().get(index).map(|g| (game_key(g), g.clone()));
        let Some((key, game)) = game else {
            return;
        };
        self.selected.set(index);
        self.selected_key.set(key);
        self.grid.rebind_visible();
        self.tooltip.set_text(&game.name);
        self.scroll_to_selected();
        self.update_tooltip();
    }

    /// Float the name tooltip over the selected tile — above its top edge
    /// normally, below it when the tile touches the viewport top — riding
    /// whatever scroll position the grid is at, and kept fully on screen.
    fn update_tooltip(&self) {
        if !self.tooltip.is_visible() {
            return;
        }
        let game = self.games.borrow().get(self.selected.get()).cloned();
        let Some(game) = game else {
            return;
        };
        // The pill points at icons: while a tile has no art yet (library
        // still loading) the tooltip stays hidden instead of floating
        // over an empty frame.
        let has_art = !game.square_path.is_empty() || !game.grid_path.is_empty();
        self.tooltip.set_visible(has_art);
        if !has_art {
            return;
        };
        let (_, item_w, item_h, _sp) = self.grid.current_layout();
        // The tooltip's clip width follows the tiles — 3 of them, the same
        // pixel width as 2 home capsules (the tiles are 2:3 apart). The
        // title scales with the tile (256px = the 1080p reference) so the
        // tooltip keeps its proportions at 720p, 1080p, 1440p…
        self.tooltip.set_max_width(item_w as f64 * 3.0);
        self.tooltip.set_text(&game.name);
        let Some((x, y)) = self.grid.cell_geometry(self.selected.get()) else {
            return;
        };
        // Store the position even when this widget has no width yet (the
        // page is still being mapped): the marquee falls back to its own
        // allocation when it paints, so the bubble lands right on first
        // open instead of waiting for a scroll event that may never come.
        // The anchor comes from the real cell widget mapped into the
        // page's coordinates — no content-vs-page arithmetic to get wrong.
        let cell_widget = self.grid.widget_for_index(self.selected.get());
        let (x, tile_top) = cell_widget
            .as_ref()
            .and_then(|cell| cell.compute_point(&self.overlay, &gtk4::graphene::Point::zero()))
            .map(|p| (p.x() as f64, p.y() as f64))
            .unwrap_or((x, y - self.scrolled.vadjustment().value()));
        let center = x + item_w as f64 / 2.0;
        // The tail tip rests just outside the selection ring on whichever
        // side the pill's actual height fits into the scrolled area —
        // above by preference, below near the top edge. Both checks are in
        // page coordinates against the scrolled area's bounds, so the pill
        // never slides under the page header.
        let pill_h = self.tooltip.pill_height() as f64;
        // The overlay is the parent of both the grid and the floating
        // widgets, so its width is final whenever this runs (the grid's
        // post-layout hook fires mid-pass, before the overlay children
        // are re-allocated). The tooltip's own width there would be one
        // pass stale — zero before the first one, which pinned the pill
        // to the left edge and scaled the ring down to a hairline.
        let viewport = self.overlay.width() as f64;
        let scrolled_top = self
            .scrolled
            .compute_point(&self.overlay, &gtk4::graphene::Point::zero())
            .map(|p| p.y() as f64)
            .unwrap_or(0.0);
        let scrolled_bottom = scrolled_top + self.scrolled.vadjustment().page_size();
        let top_tip = tile_top - BP_RING_OUTSET;
        let bottom_tip = tile_top + item_h as f64 + BP_RING_OUTSET;
        let fits_above = top_tip - TAIL_HEIGHT - pill_h >= scrolled_top + 2.0;
        let fits_below = bottom_tip + TAIL_HEIGHT + pill_h <= scrolled_bottom - 4.0;
        if fits_above || !fits_below {
            self.tooltip.set_position(center, viewport, top_tip, false);
        } else {
            self.tooltip.set_position(center, viewport, bottom_tip, true);
        }
        // The drawn selection ring hugs the tile's edges.
        let scale = viewport / 1920.0;
        self.ring
            .place(x, tile_top, item_w as f64, item_h as f64, scale, false);
    }

    /// Move the highlight onto a game by key (a mouse click on a cell
    /// selects what it clicked before launching it).
    pub(super) fn select_key(&self, key: GameKey) {
        let index = self
            .games
            .borrow()
            .iter()
            .position(|g| game_key(g) == key);
        if let Some(index) = index {
            self.apply_selection(index);
        }
    }

    fn scroll_to_selected(&self) {
        let adj = self.scrolled.vadjustment();
        // Before the first layout the page height reads as zero, which
        // makes every row look like it pokes out and dives the scroll
        // deep into the grid — an opening position must simply be zero.
        if adj.page_size() <= 1.0 {
            if let Some(id) = self.scroll_anim.borrow_mut().take() {
                id.remove();
            }
            adj.set_value(0.0);
            return;
        }
        let (cols, _, item_h, sp) = self.grid.current_layout();
        let target = scroll_target(
            self.selected.get(),
            cols as usize,
            (item_h + sp) as f64,
            sp as f64,
            adj.value(),
            adj.page_size(),
        );
        self.animate_scroll_to(target);
    }

    /// Glide to `target` like the home carousel does; a press during the
    /// glide replaces it from wherever it currently is.
    fn animate_scroll_to(&self, target: f64) {
        if let Some(id) = self.scroll_anim.borrow_mut().take() {
            id.remove();
        }
        let adj = self.scrolled.vadjustment();
        if (target - adj.value()).abs() < 0.5 {
            return;
        }
        let start = adj.value();
        let started = Instant::now();
        let anim = Rc::clone(&self.scroll_anim);
        let id = glib::timeout_add_local(Duration::from_millis(16), move || {
            let t = (started.elapsed().as_millis() as f64 / SCROLL_MILLIS as f64).min(1.0);
            let eased = 1.0 - (1.0 - t) * (1.0 - t);
            adj.set_value(start + (target - start) * eased);
            if t >= 1.0 {
                *anim.borrow_mut() = None;
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        *self.scroll_anim.borrow_mut() = Some(id);
    }
}

/// Cell factories for the couch grid: the same square-capsule recipe as the
/// desktop grid, minus the context menus, plus the external highlight.
fn cell_factories(
    state: &SharedState,
    item_size: Rc<Cell<(i32, i32)>>,
    selected_key: Rc<Cell<GameKey>>,
) -> (
    super::virtual_grid::SetupFn,
    super::virtual_grid::BindFn,
    super::virtual_grid::UnbindFn,
) {
    let setup_state = state.clone();
    let setup: super::virtual_grid::SetupFn = Rc::new(move || {
        build_cell(&setup_state)
    });
    let bind: super::virtual_grid::BindFn = Rc::new(move |widget, game| {
        bind_cell(widget, game, &item_size, &selected_key);
    });
    let unbind: super::virtual_grid::UnbindFn = Rc::new(|widget| {
        widget.remove_css_class(CSS_BP_SELECTED);
    });
    (setup, bind, unbind)
}

fn build_cell(state: &SharedState) -> gtk4::Widget {
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    vbox.set_valign(gtk4::Align::Start);
    vbox.set_halign(gtk4::Align::Center);
    vbox.add_css_class(CSS_COVER_ITEM);
    vbox.add_css_class(CSS_BP_SQ);
    vbox.set_overflow(gtk4::Overflow::Visible);

    let overlay = gtk4::Overlay::new();
    overlay.set_overflow(gtk4::Overflow::Visible);
    let pic = gtk4::Picture::new();
    pic.set_content_fit(gtk4::ContentFit::Cover);
    pic.add_css_class(CSS_GAME_COVER_PIC);
    overlay.set_child(Some(&pic));

    let name_label = gtk4::Label::new(None);
    name_label.set_wrap(true);
    name_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    name_label.set_max_width_chars(15);
    name_label.set_halign(gtk4::Align::Center);
    name_label.set_valign(gtk4::Align::Center);
    name_label.set_margin_start(6);
    name_label.set_margin_end(6);
    name_label.add_css_class(CSS_COVER_NAME_FALLBACK);
    super::helpers::crisp_label(&name_label);
    name_label.set_visible(false);
    overlay.add_overlay(&name_label);
    vbox.append(&overlay);

    unsafe { vbox.set_data::<AtomicI64>("game-db-id", AtomicI64::new(0)) };
    unsafe { vbox.set_data::<AtomicI64>("game-variant-id", AtomicI64::new(0)) };
    unsafe { vbox.set_data::<gtk4::Label>("name-label", name_label) };

    let click_state = state.clone();
    let click = gtk4::GestureClick::new();
    click.connect_pressed(move |gesture, _, _, _| {
        launch_from_cell(&click_state, &gesture.widget().unwrap());
    });
    vbox.add_controller(click);

    // Hovering a tile moves the selection onto it; the cell's game ids are
    // re-read on every enter because recycled cells rebind to new games.
    let hover_state = state.clone();
    let hover = gtk4::EventControllerMotion::new();
    hover.connect_enter(move |controller, _, _| {
        let Some(big) = hover_state.borrow().big_picture.clone() else {
            return;
        };
        let widget = controller.widget().unwrap();
        let cell_key = || -> Option<GameKey> {
            let db = unsafe { widget.data::<AtomicI64>("game-db-id") }
                .map(|p| unsafe { p.as_ref() }.load(Ordering::Relaxed))?;
            let variant = unsafe { widget.data::<AtomicI64>("game-variant-id") }
                .map(|p| unsafe { p.as_ref() }.load(Ordering::Relaxed))
                .unwrap_or(0);
            Some((db, variant))
        };
        if let Some(key) = cell_key() {
            big.all.select_key(key);
        }
    });
    vbox.add_controller(hover);

    vbox.upcast()
}

/// Bind one recycled cell: art, name fallback, size, and the highlight for
/// the externally selected game key.
fn bind_cell(
    widget: &gtk4::Widget,
    game: &Game,
    item_size: &Rc<Cell<(i32, i32)>>,
    selected_key: &Rc<Cell<GameKey>>,
) {
    let vbox = widget.downcast_ref::<gtk4::Box>().unwrap();
    let (cover_width, cover_height) = item_size.get();
    let overlay_widget = vbox.first_child().unwrap();
    let overlay = overlay_widget.downcast_ref::<gtk4::Overlay>().unwrap();
    let pic_widget = overlay.child().unwrap();
    let pic = pic_widget.downcast_ref::<gtk4::Picture>().unwrap();
    let name_label = unsafe { vbox.data::<gtk4::Label>("name-label") }
        .map(|ptr| unsafe { ptr.as_ref() }.clone());
    vbox.set_size_request(cover_width, cover_height);
    pic.set_size_request(cover_width, cover_height);

    if let Some(ptr) = unsafe { vbox.data::<AtomicI64>("game-db-id") } {
        unsafe { ptr.as_ref() }.store(game.db_id, Ordering::Relaxed);
    }
    if let Some(ptr) = unsafe { vbox.data::<AtomicI64>("game-variant-id") } {
        unsafe { ptr.as_ref() }.store(game.variant_id.unwrap_or(0), Ordering::Relaxed);
    }
    if selected_key.get() == game_key(game) {
        vbox.add_css_class(CSS_BP_SELECTED);
    } else {
        vbox.remove_css_class(CSS_BP_SELECTED);
    }

    // A recycled cell still shows the game it played in its last life;
    // blank it before the new art resolves or scrolling flashes wrong
    // covers everywhere.
    let blank = ira_images::ScaledPaintable::new_empty(cover_width, cover_height);
    pic.set_paintable(Some(&blank));

    let art = if game.square_path.is_empty() {
        &game.grid_path
    } else {
        &game.square_path
    };
    if art.is_empty() {
        if let Some(ref label) = name_label {
            label.set_text(&game.name);
            label.set_visible(true);
        }
        return;
    }
    if let Some(ref label) = name_label {
        label.set_visible(false);
    }
    let pic_weak = pic.downgrade();
    let expected = game_key(game);
    let path = art.clone();
    let set = move |texture: Option<gdk4::Texture>| {
        if let Some(pic) = pic_weak.upgrade() {
            // The cell may have been rebound to another game while the
            // texture was decoding; only paint if it is still ours.
            if cell_key(&pic) == expected {
                if let Some(texture) = texture {
                    pic.set_paintable(Some(&texture));
                }
            }
        }
    };
    match ira_images::cached_texture(art) {
        Some(texture) => set(Some(texture)),
        None => ira_images::load_texture_async_with_priority(
            &path,
            glib::Priority::DEFAULT,
            set,
        ),
    }
}

/// The (db, variant) key a cell currently displays, read back from its atoms.
fn cell_key(pic: &gtk4::Picture) -> GameKey {
    let vbox = pic
        .ancestor(gtk4::Box::static_type())
        .and_then(|w| w.downcast::<gtk4::Box>().ok());
    let Some(vbox) = vbox else {
        return (0, 0);
    };
    let read = |key: &str| {
        unsafe { vbox.data::<AtomicI64>(key) }
            .map(|ptr| unsafe { ptr.as_ref() }.load(Ordering::Relaxed))
            .unwrap_or(0)
    };
    (read("game-db-id"), read("game-variant-id"))
}

fn launch_from_cell(state: &SharedState, widget: &gtk4::Widget) {
    let Some((db_id, variant_id)) = read_cell_ids(widget) else {
        return;
    };
    let game = state
        .borrow()
        .games
        .iter()
        .find(|g| g.db_id == db_id && g.variant_id == variant_id.filter(|v| *v > 0))
        .cloned();
    if let Some(game) = game {
        // The click selects what it clicked, then launches it — so the
        // highlight never rests on a game the user did not point at.
        if let Some(big) = state.borrow().big_picture.clone() {
            big.all.select_key(game_key(&game));
        }
        launch(state, &game);
    }
}

fn read_cell_ids(widget: &gtk4::Widget) -> Option<(i64, Option<i64>)> {
    let db_id = unsafe { widget.data::<AtomicI64>("game-db-id") }
        .map(|ptr| unsafe { ptr.as_ref() }.load(Ordering::Relaxed))?;
    if db_id == 0 {
        return None;
    }
    let variant_id = unsafe { widget.data::<AtomicI64>("game-variant-id") }
        .map(|ptr| unsafe { ptr.as_ref() }.load(Ordering::Relaxed))
        .filter(|v| *v > 0);
    Some((db_id, variant_id))
}

fn launch(state: &SharedState, game: &Game) {
    if let Err(error) =
        super::play_button::launch_game(state, game.db_id, game.variant_id)
    {
        eprintln!("Failed to launch game: {error}");
        let _ = state
            .borrow()
            .sender
            .send(crate::AppMessage::AddGameError(error));
    }
}

#[cfg(test)]
mod tests {
    use super::{grid_move, scroll_target};

    const COLS: usize = 5;

    #[test]
    fn test_grid_move_horizontally_clamped_to_row() {
        assert_eq!(grid_move(0, 10, COLS, 1, 0), Some(1));
        assert_eq!(grid_move(4, 10, COLS, 1, 0), None, "row edge stays put");
        assert_eq!(grid_move(5, 10, COLS, -1, 0), None);
        assert_eq!(grid_move(5, 10, COLS, 1, 0), Some(6));
    }

    #[test]
    fn test_grid_move_keeps_column() {
        assert_eq!(grid_move(2, 12, COLS, 0, 1), Some(7));
        assert_eq!(grid_move(7, 12, COLS, 0, -1), Some(2));
        assert_eq!(grid_move(2, 12, COLS, 0, -3), None, "top edge stays put");
    }

    #[test]
    fn test_grid_move_slides_onto_short_last_row() {
        // 7 items: row 1 holds indexes 5 and 6. Down from 3 slides left
        // onto 6; down from 4 lands on 6 too.
        assert_eq!(grid_move(3, 7, COLS, 0, 1), Some(6));
        assert_eq!(grid_move(4, 7, COLS, 0, 1), Some(6));
        assert_eq!(grid_move(6, 7, COLS, 0, 1), None);
    }

    #[test]
    fn test_grid_move_empty_grid() {
        assert_eq!(grid_move(0, 0, COLS, 1, 0), None);
    }

    #[test]
    fn test_scroll_target_snaps_to_whole_rows() {
        let row_h = 100.0;
        let top_pad = 8.0;
        // Row 0 fully visible: no scroll.
        assert_eq!(scroll_target(0, COLS, row_h, top_pad, 0.0, 300.0), 0.0);
        // Row 2 (index 10) pokes 8px past the bottom: the scroll snaps up
        // a WHOLE row minus the outline's room (101 = row 1's top at 108,
        // raised 7 so its outline stays on screen).
        assert_eq!(scroll_target(10, COLS, row_h, top_pad, 0.0, 300.0), 101.0);
        // Row 1 (index 5) visible at that position: no move.
        assert_eq!(scroll_target(5, COLS, row_h, top_pad, 101.0, 300.0), 101.0);
        // Scrolling back up to row 0 lands on the first boundary, again
        // raised by the outline's room (8 - 7).
        assert_eq!(scroll_target(0, COLS, row_h, top_pad, 101.0, 300.0), 1.0);
        // Deep rows (row 5 = index 25) scroll to the boundary that fits.
        assert_eq!(scroll_target(25, COLS, row_h, top_pad, 101.0, 300.0), 401.0);
    }
}
