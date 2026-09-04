//! The All Software page: every visible game on a square-capsule grid,
//! reached from the grid tile at the end of the home carousel. Controller
//! and keyboard drive an external selection highlight (the grid itself only
//! recycles cells); A launches, B returns home.

use super::css::*;
use super::big_picture_marquee::Marquee;
use super::state::SharedState;
use super::virtual_grid::VirtualGrid;
use crate::Game;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicI64, Ordering};

/// The game a couch grid action targets: launch uses the same (db, variant)
/// pair the desktop grid stores on its cells.
type GameKey = (i64, i64);

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

/// The vertical scroll that makes the row holding `index` fully visible.
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
    if top < value {
        top
    } else if bottom > value + page {
        bottom - page
    } else {
        value
    }
}

/// Widgets and selection state of the All Software page.
pub(super) struct AllSoftwareUi {
    page: gtk4::Box,
    scrolled: gtk4::ScrolledWindow,
    grid: VirtualGrid,
    /// The selected game's name, Switch-style, centered above the grid;
    /// marquees when a long name overflows.
    name: Marquee,
    empty: gtk4::Label,
    games: RefCell<Vec<Game>>,
    selected: Cell<usize>,
    /// The highlighted game as a (db, variant) key, shared with the bind
    /// closure so cells can style themselves without a rebuild.
    selected_key: Rc<Cell<GameKey>>,
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
    let back = gtk4::Button::from_icon_name("go-previous-symbolic");
    back.add_css_class(CSS_FLAT);
    back.set_tooltip_text(Some(&crate::tr!("Back")));
    back.set_valign(gtk4::Align::Center);
    {
        let back_state = state.clone();
        back.connect_clicked(move |_| super::big_picture_view::show_home(&back_state));
    }
    header.append(&back);
    let icon = gtk4::Image::from_icon_name("games-symbolic");
    icon.set_pixel_size(24);
    icon.set_valign(gtk4::Align::Center);
    header.append(&icon);
    let title = gtk4::Label::new(Some(&crate::tr!("All Software")));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.add_css_class(CSS_BP_PAGE_TITLE);
    header.append(&title);
    let ordering = gtk4::Label::new(Some(&crate::tr!("By name")));
    ordering.set_valign(gtk4::Align::Center);
    ordering.add_css_class(CSS_DIM_LABEL);
    header.append(&ordering);
    page.append(&header);

    let name = Marquee::new();
    name.widget().set_halign(gtk4::Align::Center);
    name.set_visible(false);
    page.append(name.widget());

    let grid = VirtualGrid::new(220);
    grid.set_square(true);
    let store = gio::ListStore::new::<super::game_item::GameItem>();
    grid.set_model(&store);
    let selected_key = Rc::new(Cell::new((0, 0)));
    let (setup, bind, unbind) =
        cell_factories(state, grid.item_size_cell(), Rc::clone(&selected_key));
    grid.set_factory(setup, bind, unbind);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scrolled.set_vexpand(true);
    scrolled.set_child(Some(&grid));
    page.append(&scrolled);

    let empty = gtk4::Label::new(Some(&crate::tr!("No games yet")));
    empty.add_css_class(CSS_DIM_LABEL);
    empty.set_vexpand(true);
    empty.set_visible(false);
    page.append(&empty);

    let ui = AllSoftwareUi {
        page,
        scrolled,
        grid,
        name,
        empty,
        games: RefCell::new(Vec::new()),
        selected: Cell::new(0),
        selected_key,
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
        self.name.set_visible(!games.is_empty());
        if let Some(game) = games.get(selected) {
            self.name.set_text(&game.name);
        }
        *self.games.borrow_mut() = games.to_vec();
    }

    /// First open: park the selection and the scroll at the top. Later
    /// opens keep both.
    pub(super) fn ensure_opened(&self) {
        if self.opened.get() {
            self.scroll_to_selected();
            return;
        }
        self.opened.set(true);
        self.apply_selection(0);
    }

    pub(super) fn move_selection(&self, dx: i32, dy: i32) {
        let (cols, _, _item_h, _sp) = self.grid.current_layout();
        let count = self.games.borrow().len();
        if let Some(next) = grid_move(self.selected.get(), count, cols as usize, dx, dy) {
            self.apply_selection(next);
        }
    }

    /// Point the highlight at `index`: update the shared key, restyle the
    /// visible cells, name the game, and scroll the row into view.
    fn apply_selection(&self, index: usize) {
        let game = self.games.borrow().get(index).map(|g| (game_key(g), g.clone()));
        let Some((key, game)) = game else {
            return;
        };
        self.selected.set(index);
        self.selected_key.set(key);
        self.grid.rebind_visible();
        self.name.set_text(&game.name);
        self.scroll_to_selected();
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
        let (cols, _, item_h, sp) = self.grid.current_layout();
        let adj = self.scrolled.vadjustment();
        let target = scroll_target(
            self.selected.get(),
            cols as usize,
            (item_h + sp) as f64,
            sp as f64,
            adj.value(),
            adj.page_size(),
        );
        adj.set_value(target);
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

    let art = if game.square_path.is_empty() {
        &game.grid_path
    } else {
        &game.square_path
    };
    if art.is_empty() {
        pic.set_paintable(Some(&ira_images::ScaledPaintable::new_empty(
            cover_width,
            cover_height,
        )));
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
    fn test_scroll_target_follows_selection() {
        let row_h = 100.0;
        // Three rows fit in the 300px page at once.
        assert_eq!(scroll_target(0, COLS, row_h, 8.0, 0.0, 300.0), 0.0);
        assert_eq!(scroll_target(6, COLS, row_h, 8.0, 0.0, 300.0), 0.0);
        // Row 2 (index 10) pokes out: scroll just past its top.
        assert_eq!(scroll_target(10, COLS, row_h, 8.0, 0.0, 300.0), 8.0);
        assert_eq!(scroll_target(2, COLS, row_h, 8.0, 208.0, 300.0), 8.0);
        // Already visible: the scroll stays where it is.
        assert_eq!(scroll_target(12, COLS, row_h, 8.0, 208.0, 300.0), 208.0);
    }
}
