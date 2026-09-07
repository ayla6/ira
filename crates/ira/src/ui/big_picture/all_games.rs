//! The All Software page: every visible game on a square-capsule grid,
//! reached from the grid tile at the end of the home carousel. Controller
//! and keyboard drive an external selection highlight (the grid itself only
//! recycles cells); A launches, B returns home.

use crate::ui::css::*;
use super::marquee::{Marquee, TAIL_HEIGHT};
use crate::ui::selection_ring::SelectionRing;
use crate::ui::state::SharedState;
use crate::ui::virtual_grid::VirtualGrid;
use crate::Game;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};

/// The game a big-picture grid action targets: launch uses the same (db, variant)
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
pub(super) fn grid_move(current: usize, count: usize, cols: usize, dx: i32, dy: i32) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let cols = cols.max(1) as i64;
    let count = count as i64;
    let col = (current as i64) % cols;
    let row = (current as i64) / cols;
    let next_col = (col + dx as i64).clamp(0, cols - 1);
    let last_row = (count - 1) / cols;
    let next_row = (row + dy as i64).clamp(0, last_row);
    let next = next_row * cols + next_col;
    // A short last row has no cell under many columns: a vertical move
    // onto a missing cell stays put instead of dragging the selection
    // sideways, and moving past either edge never wraps around.
    (next < count && next != current as i64).then_some(next as usize)
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

/// Which tab of the All Software page is showing: the game grid or the
/// groups list.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tab {
    Software,
    Groups,
}

/// The L/R shoulder badge beside the tabs: the connected pad's glyph art
/// when the icon set has one, the letter in a rounded box otherwise.
struct ShoulderBadge {
    slot: gtk4::Box,
    glyph: gtk4::Image,
    fallback: gtk4::Label,
}

impl ShoulderBadge {
    fn new() -> Self {
        let slot = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        slot.set_valign(gtk4::Align::Center);
        slot.set_margin_end(12);
        let glyph = gtk4::Image::new();
        glyph.set_halign(gtk4::Align::Center);
        glyph.set_valign(gtk4::Align::Center);
        let fallback = gtk4::Label::new(None);
        fallback.add_css_class(CSS_BP_SHOULDER);
        fallback.set_halign(gtk4::Align::Center);
        fallback.set_valign(gtk4::Align::Center);
        slot.append(&glyph);
        slot.append(&fallback);
        Self { slot, glyph, fallback }
    }

    fn refresh(&self, button: ira_input::GamepadButton, family: ira_input::ControllerFamily, scale: f64) {
        self.slot.set_size_request(
            (46.0 * scale).round().max(12.0) as i32,
            (38.0 * scale).round().max(10.0) as i32,
        );
        self.glyph.set_pixel_size((34.0 * scale).round().max(8.0) as i32);
        self.fallback.set_text(&crate::ui::input_profile_assets::source_badge(
            ira_input::InputSource::Button(button),
            family,
        ));
        crate::ui::input_profile_assets::set_source_asset(
            &self.glyph,
            &self.fallback,
            ira_input::InputSource::Button(button),
            family,
        );
    }
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
    /// The focused tile, if any. Scrolling the grid by hand clears it;
    /// the arrows re-acquire from whatever is on screen.
    selected: Cell<Option<usize>>,
    /// The highlighted game as a (db, variant) key, shared with the bind
    /// closure so cells can style themselves without a rebuild.
    selected_key: Rc<Cell<GameKey>>,
    /// The running scroll glide, so a new press replaces it mid-flight.
    scroll_anim: Rc<RefCell<Option<glib::SourceId>>>,
    opened: Cell<bool>,
    /// The "sorted by …" label in the header.
    ordering: gtk4::Label,
    software_tab: gtk4::Label,
    groups_tab: gtk4::Label,
    /// The L/R shoulder badges flanking the tabs.
    shoulder_l: ShoulderBadge,
    shoulder_r: ShoulderBadge,
    /// The connected pad's family, so the badges draw its glyphs.
    shoulder_family: Cell<ira_input::ControllerFamily>,
    shoulder_scale: Cell<f64>,
    /// Which tab is showing and, on the Groups tab, which group's games
    /// the grid holds (`None` = the groups tile grid itself).
    tab: Cell<Tab>,
    groups_view: Cell<Option<i64>>,
    groups_grid: super::groups::GroupsGrid,
}

pub(super) fn build(state: &SharedState, status: &super::status::StatusBar) -> (gtk4::Box, AllSoftwareUi) {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    page.add_css_class(CSS_BP_ALL_PAGE);

    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    header.set_margin_top(18);
    header.set_margin_bottom(6);
    header.set_margin_start(28);
    header.set_margin_end(28);
    // One header line: the ordering and its button on the left, the
    // Software/Groups tabs centered between the expanding spacers, and
    // the shell's status cluster (clock, battery) docked right.
    let ordering = gtk4::Label::new(None);
    ordering.set_valign(gtk4::Align::Center);
    ordering.add_css_class(CSS_BP_PAGE_SUBTITLE);
    crate::ui::helpers::crisp_label(&ordering);
    header.append(&ordering);
    // The sort button opens the sort menu (Options does the same when no
    // game is focused).
    let sort_btn = gtk4::Button::from_icon_name("view-sort-descending-symbolic");
    sort_btn.add_css_class(CSS_FLAT);
    sort_btn.set_focusable(false);
    sort_btn.set_valign(gtk4::Align::Center);
    sort_btn.set_tooltip_text(Some(&crate::tr!("Sort by")));
    {
        let sort_state = state.clone();
        sort_btn.connect_clicked(move |_| {
            if let Some(big) = sort_state.borrow().big_picture.clone() {
                big.game_menu.open(&sort_state, super::game_menu::MenuKind::Sort);
            }
        });
    }
    header.append(&sort_btn);
    let lead = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    lead.set_hexpand(true);
    header.append(&lead);
    let shoulder_l = ShoulderBadge::new();
    let shoulder_r = ShoulderBadge::new();
    let software_tab = gtk4::Label::new(Some(&crate::tr!("Software")));
    let groups_tab = gtk4::Label::new(Some(&crate::tr!("Groups")));
    for tab_label in [&software_tab, &groups_tab] {
        tab_label.add_css_class(CSS_BP_TAB);
        crate::ui::helpers::crisp_label(tab_label);
        tab_label.set_valign(gtk4::Align::Center);
    }
    // The page builds on the Software tab; set_tab only fires on a
    // switch, so the initial active state must be set here.
    software_tab.add_css_class(CSS_BP_TAB_ACTIVE);
    header.append(&shoulder_l.slot);
    header.append(&software_tab);
    header.append(&groups_tab);
    header.append(&shoulder_r.slot);
    let trail = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    trail.set_hexpand(true);
    header.append(&trail);
    // The shell's status rail lives here while this page shows (see
    // `view::build_root`); drop its rail padding so it sits inline.
    status.widget().add_css_class(CSS_BP_STATUS_INLINE);
    header.append(status.widget());
    {
        let tab_state = state.clone();
        software_tab.set_cursor_from_name(Some("pointer"));
        let software_click = gtk4::GestureClick::new();
        software_click.connect_pressed(move |_, _, _, _| {
            if let Some(big) = tab_state.borrow().big_picture.clone() {
                big.all.set_tab(&tab_state, Tab::Software);
            }
        });
        software_tab.add_controller(software_click);
    }
    {
        let tab_state = state.clone();
        groups_tab.set_cursor_from_name(Some("pointer"));
        let groups_click = gtk4::GestureClick::new();
        groups_click.connect_pressed(move |_, _, _, _| {
            if let Some(big) = tab_state.borrow().big_picture.clone() {
                big.all.set_tab(&tab_state, Tab::Groups);
            }
        });
        groups_tab.add_controller(groups_click);
    }

    let grid = VirtualGrid::new(240);
    grid.set_square(true);
    // Six tiles per row at every resolution, Switch-style: the tile size
    // scales with the viewport and the margins derive from the tile.
    grid.set_fixed_cols(6);
    // Recycled cells must never paint outside the viewport, over the page
    // header above.
    grid.set_overflow(gtk4::Overflow::Hidden);
    let store = gio::ListStore::new::<crate::ui::game_item::GameItem>();
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
    // The ring must never paint past the grid area: a mid-glide place
    // used to smear its side edges up over the header.
    grid_overlay.set_clip_overlay(&ring, true);
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

    let groups = super::groups::GroupsGrid::build(state);
    page.append(groups.widget());

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
    // Wheel and touchpad scrolling over the grid deselect: the pointer
    // has taken over, and the arrows re-acquire from the visible rows.
    {
        let wheel_state = state.clone();
        let wheel = gtk4::EventControllerScroll::new(
            gtk4::EventControllerScrollFlags::VERTICAL
                | gtk4::EventControllerScrollFlags::HORIZONTAL,
        );
        wheel.set_propagation_phase(gtk4::PropagationPhase::Capture);
        wheel.connect_scroll(move |_, _, _| {
            if let Some(big) = wheel_state.borrow().big_picture.clone() {
                big.all.clear_selection();
            }
            glib::Propagation::Proceed
        });
        scrolled.add_controller(wheel);
    }

    let ui = AllSoftwareUi {
        page,
        scrolled,
        grid,
        tooltip,
        empty: empty.clone(),
        games: RefCell::new(Vec::new()),
        selected: Cell::new(None),
        selected_key,
        scroll_anim: Rc::new(RefCell::new(None)),
        ring,
        overlay: grid_overlay,
        opened: Cell::new(false),
        ordering,
        shoulder_l,
        shoulder_r,
        shoulder_family: Cell::new(ira_input::ControllerFamily::Xbox),
        shoulder_scale: Cell::new(0.0),
        software_tab,
        groups_tab,
        tab: Cell::new(Tab::Software),
        groups_view: Cell::new(None),
        groups_grid: groups,
    };
    ui.update_ordering_label(state);
    (ui.page.clone(), ui)
}

impl AllSoftwareUi {
    /// Name the current ordering in the header.
    pub(super) fn update_ordering_label(&self, state: &SharedState) {
        let (mode, descending) = {
            let s = state.borrow();
            (s.cfg.sort_mode, s.cfg.sort_descending)
        };
        let arrow = if descending { " ↓" } else { "" };
        self.ordering
            .set_text(&format!("{}{arrow}", mode.display_label()));
    }

    /// The game the selection currently rests on, if any.
    pub(super) fn selected_game(&self) -> Option<Game> {
        let index = self.selected.get()?;
        self.games.borrow().get(index).cloned()
    }

    /// Whether the Groups tab's tile grid is showing (opening a group
    /// hands the surface back to the game grid).
    pub(super) fn in_groups_tiles(&self) -> bool {
        self.tab.get() == Tab::Groups && self.groups_view.get().is_none()
    }

    /// Scale the header's shoulder badges with the viewport (1.0 = 1920
    /// wide), like the rails' icons.
    pub(super) fn set_icon_scale(&self, scale: f64) {
        self.shoulder_scale.set(scale);
        self.refresh_shoulders();
    }

    /// The connected pad's family changed: redraw the badges' glyphs.
    pub(super) fn set_shoulder_family(&self, family: ira_input::ControllerFamily) {
        if self.shoulder_family.get() == family {
            return;
        }
        self.shoulder_family.set(family);
        self.refresh_shoulders();
    }

    fn refresh_shoulders(&self) {
        let family = self.shoulder_family.get();
        let scale = self.shoulder_scale.get().max(1.0);
        self.shoulder_l
            .refresh(ira_input::GamepadButton::LeftShoulder, family, scale);
        self.shoulder_r
            .refresh(ira_input::GamepadButton::RightShoulder, family, scale);
    }

    /// Switch tabs. The Groups tab reloads its tiles so groups created
    /// elsewhere show up; a group's game view never survives the switch —
    /// the shoulders always land on the tab's own surface.
    pub(super) fn set_tab(&self, state: &SharedState, tab: Tab) {
        if self.tab.get() == tab && self.groups_view.get().is_none() {
            return;
        }
        self.tab.set(tab);
        self.groups_view.set(None);
        self.software_tab.remove_css_class(CSS_BP_TAB_ACTIVE);
        self.groups_tab.remove_css_class(CSS_BP_TAB_ACTIVE);
        match tab {
            Tab::Software => &self.software_tab,
            Tab::Groups => &self.groups_tab,
        }
        .add_css_class(CSS_BP_TAB_ACTIVE);
        if tab == Tab::Groups {
            self.groups_grid.reload(state);
        }
        self.apply_mode(state);
    }

    /// Step to the other tab (the shoulders), Switch-style.
    pub(super) fn switch_tab(&self, state: &SharedState, _delta: i32) {
        let next = match self.tab.get() {
            Tab::Software => Tab::Groups,
            Tab::Groups => Tab::Software,
        };
        self.set_tab(state, next);
    }

    /// Back out one level on this page: a group's games return to the
    /// groups tiles. Returns false when the page has nothing to pop and
    /// the caller should leave for home.
    pub(super) fn on_back(&self, state: &SharedState) -> bool {
        if self.tab.get() == Tab::Groups && self.groups_view.get().is_some() {
            self.groups_view.set(None);
            self.apply_mode(state);
            true
        } else {
            false
        }
    }

    /// Activate the selected groups tile: New Group names (and creates)
    /// a group through the keyboard, a group opens its game view.
    pub(super) fn groups_open_selected(&self, state: &SharedState) {
        match self.groups_grid.selected_group_id(state) {
            Some(id) => {
                self.groups_view.set(Some(id));
                self.selected.set(None);
                self.apply_mode(state);
            }
            None => super::view::name_new_group(state, None),
        }
    }

    /// The mouse clicked a groups tile: first click focuses it, a click
    /// on the focused tile activates it.
    pub(super) fn groups_grid_selected(&self, state: &SharedState, tile: usize) {
        if self.groups_grid.selection() == tile {
            self.groups_open_selected(state);
        } else {
            self.groups_grid.select_tile(tile);
            self.groups_grid.repaint(state);
        }
    }

    /// Delete the group under the selection (the X button on the Groups
    /// tiles). Membership rows go with it; games stay.
    pub(super) fn groups_delete_selected(&self, state: &SharedState) {
        let Some(id) = self.groups_grid.selected_group_id(state) else {
            return;
        };
        let db = state.borrow().db.clone();
        if let Err(e) = ira_db::delete_group(&db, id) {
            eprintln!("Failed to delete group: {e}");
            return;
        }
        self.sync_groups(state);
        if self.groups_view.get() == Some(id) {
            self.groups_view.set(None);
        }
        self.apply_mode(state);
    }

    /// Rename the group under the selection through the keyboard.
    pub(super) fn groups_rename_selected(&self, state: &SharedState) {
        let Some(id) = self.groups_grid.selected_group_id(state) else {
            return;
        };
        let current = state
            .borrow()
            .groups
            .iter()
            .find(|g| g.id == id)
            .map(|g| g.name.clone())
            .unwrap_or_default();
        super::view::rename_group(state, id, &current);
    }

    /// Move the groups tile selection (arrows on the Groups tab).
    pub(super) fn groups_move(&self, state: &SharedState, dx: i32, dy: i32) {
        self.groups_grid.move_selection(state, dx, dy);
    }

    /// Pull the shared groups list back from the database.
    fn sync_groups(&self, state: &SharedState) {
        let db = state.borrow().db.clone();
        state.borrow_mut().groups = ira_db::get_all_groups(&db).unwrap_or_default();
    }

    /// After a create or rename: sync the tiles, land the selection on
    /// the touched group, and open its game view.
    pub(super) fn sync_and_focus_group(&self, state: &SharedState, group_id: i64) {
        self.sync_groups(state);
        self.groups_grid.reload(state);
        if let Some(index) = state.borrow().groups.iter().position(|g| g.id == group_id) {
            self.groups_grid.select_tile(index + 1);
            self.groups_grid.repaint(state);
        }
        self.groups_view.set(Some(group_id));
        self.selected.set(None);
        self.apply_mode(state);
    }

    /// Lay out the page for the current tab and view: which surface
    /// shows, which button prompts the bottom rail has, and — on the game
    /// grid — a default selection so a freshly entered surface is never
    /// highlight-less.
    pub(super) fn apply_mode(&self, state: &SharedState) {
        let tiles = self.in_groups_tiles();
        self.groups_grid.widget().set_visible(tiles);
        self.overlay.set_visible(!tiles);
        self.refresh(state);
        if tiles {
            self.groups_grid.repaint(state);
        } else {
            self.ensure_default_selection();
        }
        if let Some(big) = state.borrow().big_picture.clone() {
            let prompts: Vec<(ira_input::GamepadButton, String)> = if tiles {
                vec![
                    (ira_input::GamepadButton::X, crate::tr!("Delete Group")),
                    (ira_input::GamepadButton::B, crate::tr!("Back")),
                    (ira_input::GamepadButton::Start, crate::tr!("Rename")),
                    (ira_input::GamepadButton::A, crate::tr!("OK")),
                ]
            } else {
                vec![
                    (ira_input::GamepadButton::B, crate::tr!("Back")),
                    (ira_input::GamepadButton::Start, crate::tr!("Options")),
                    (ira_input::GamepadButton::A, crate::tr!("Play")),
                ]
            };
            let prompt_refs: Vec<(ira_input::GamepadButton, &str)> =
                prompts.iter().map(|(b, l)| (*b, l.as_str())).collect();
            big.set_prompts(&prompt_refs);
        }
    }

    /// Park the selection on the first tile when the grid has none —
    /// entering the page, opening a group, or switching tabs must always
    /// show a highlight. A selection cleared by manual scrolling stays
    /// cleared; only a surface (re)entry re-parks it.
    fn ensure_default_selection(&self) {
        if self.opened.get() && self.selected.get().is_none() && !self.games.borrow().is_empty() {
            self.select(0);
        }
    }

    pub(super) fn refresh(&self, state: &SharedState) {
        if self.groups_grid.ensure_sized() {
            self.groups_grid.reload(state);
        }
        let was_empty = self.games.borrow().is_empty();
        let (show_hidden, sort_mode, sort_descending) = {
            let s = state.borrow();
            (
                s.cfg.show_hidden_games,
                s.cfg.sort_mode,
                s.cfg.sort_descending,
            )
        };
        let members: Option<std::collections::HashSet<i64>> =
            self.groups_view.get().map(|group_id| {
                let db = state.borrow().db.clone();
                ira_db::get_game_ids_in_group(&db, group_id)
                    .unwrap_or_default()
                    .into_iter()
                    .collect()
            });
        let mut games: Vec<Game> = state
            .borrow()
            .games
            .iter()
            .filter(|g| !g.hidden || show_hidden)
            .filter(|g| {
                members
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&g.db_id))
            })
            .cloned()
            .collect();
        games.sort_by(|a, b| {
            let ord = sort_mode.compare(a, b).then_with(|| a.db_id.cmp(&b.db_id));
            if sort_descending {
                ord.reverse()
            } else {
                ord
            }
        });

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
        // The games changed under a selection: keep the focused key when
        // it survives the new list, drop the focus when it does not.
        if let Some(index) = self.selected.get() {
            let key = self
                .games
                .borrow()
                .get(index)
                .map(|g| (g.db_id, g.variant_id.unwrap_or(0)));
            let kept = key.and_then(|key| {
                games
                    .iter()
                    .position(|g| (g.db_id, g.variant_id.unwrap_or(0)) == key)
            });
            self.selected.set(kept);
            self.selected_key.set(key.unwrap_or((0, 0)));
        }
        self.sync_store(&games);
        // The library landed while the page was open with nothing selected
        // (its first open predated the games): park on the first tile.
        if was_empty && !games.is_empty() && self.opened.get() {
            self.select(0);
        }
    }

    fn sync_store(&self, games: &[Game]) {
        let store = gio::ListStore::new::<crate::ui::game_item::GameItem>();
        for game in games {
            store.append(&crate::ui::game_item::GameItem::new(game));
        }
        // The model swap replays items_changed with removed != added, so the
        // grid clears its visible cells and rebinds from the new store.
        self.grid.set_model(&store);

        if let Some(selected) = self.selected.get() {
            let selected = selected.min(games.len().saturating_sub(1));
            self.selected.set(Some(selected));
            if let Some(game) = games.get(selected) {
                self.selected_key.set(game_key(game));
            }
        }
        self.empty.set_visible(games.is_empty());
        if let Some(game) = games.get(self.selected.get().unwrap_or(0)) {
            self.tooltip.set_text(&game.name);
        }
        *self.games.borrow_mut() = games.to_vec();
        self.update_tooltip();
    }

    /// Opening the page: park the selection on the first tile when there
    /// is none and keep the scroll anchored. Later opens keep both.
    pub(super) fn ensure_opened(&self, state: &SharedState) {
        if !self.opened.get() {
            self.opened.set(true);
            // The pill's text was set while this page sat hidden — possibly
            // at a different big-picture scale — so its layout may measure
            // stale.
            self.tooltip.revalidate_text();
        }
        if self.selected.get().is_none() {
            self.select(0);
        }
        self.scroll_to_selected();
        self.update_tooltip();
        // The page is mid stack transition here: its first real allocation
        // still lies ahead, and the floats must re-anchor once it lands.
        let idle_state = state.clone();
        glib::idle_add_local_once(move || {
            if let Some(big) = idle_state.borrow().big_picture.clone() {
                big.all.update_tooltip();
            }
        });
    }

    /// Re-measure the pill after the big-picture scale changed (see
    /// `Marquee::revalidate_text`).
    pub(super) fn revalidate_tooltip(&self) {
        self.tooltip.revalidate_text();
    }

    /// The arrows moved the selection. With nothing focused yet — the user
    /// scrolled the grid by hand, which deselects — the selection comes
    /// back on the first fully visible tile before the move applies, so
    /// navigation never resumes from somewhere off screen.
    pub(super) fn move_selection(&self, dx: i32, dy: i32) {
        let (cols, _, item_h, sp) = self.grid.current_layout();
        let count = self.games.borrow().len();
        if count == 0 {
            return;
        }
        if self.selected.get().is_none() {
            let value = self.scrolled.vadjustment().value();
            let row = ((value / (item_h + sp) as f64).round().max(0.0) as usize)
                .min((count - 1) / cols.max(1) as usize);
            self.select((row * cols.max(1) as usize).min(count - 1));
        }
        let selected = self.selected.get().unwrap_or(0);
        if let Some(next) = grid_move(selected, count, cols as usize, dx, dy) {
            self.select(next);
        }
    }

    /// Point the highlight at `index` and bring it into view: the gamepad
    /// and keyboard path, where the camera follows the selection.
    fn select(&self, index: usize) {
        if self.apply_selection(index) {
            self.scroll_to_selected();
            self.update_tooltip();
        }
    }

    /// Restyle the grid around `index` and set the focused key. Returns
    /// false when the index has no game.
    fn apply_selection(&self, index: usize) -> bool {
        let game = self
            .games
            .borrow()
            .get(index)
            .map(|g| (game_key(g), g.clone()));
        let Some((key, game)) = game else {
            return false;
        };
        self.selected.set(Some(index));
        self.selected_key.set(key);
        self.grid.rebind_visible();
        self.tooltip.set_text(&game.name);
        true
    }

    /// The user scrolled the grid by hand: the focus goes away entirely.
    /// The arrows bring it back on the first visible tile (see
    /// `move_selection`).
    pub(super) fn clear_selection(&self) {
        if self.selected.get().is_none() {
            return;
        }
        self.selected.set(None);
        self.grid.rebind_visible();
        self.tooltip.set_visible(false);
        self.ring.set_visible(false);
    }

    /// Float the name tooltip over the selected tile — above its top edge
    /// normally, below it when the tile touches the viewport top — riding
    /// whatever scroll position the grid is at. The ring marks the
    /// selection whenever it is on screen, art or not (an artless tile
    /// shows its name label); only the pill needs art to point at.
    fn update_tooltip(&self) {
        let Some(selected) = self.selected.get() else {
            self.tooltip.set_visible(false);
            self.ring.set_visible(false);
            return;
        };
        let Some(game) = self.games.borrow().get(selected).cloned() else {
            return;
        };
        let (_, item_w, item_h, _sp) = self.grid.current_layout();
        let Some((x, y)) = self.grid.cell_geometry(selected) else {
            return;
        };
        // The anchor comes from the real cell widget mapped into the
        // overlay's coordinates; the fallback keeps the first paint
        // anchored when the page has no width yet.
        let cell_widget = self.grid.widget_for_index(selected);
        let (x, tile_top) = cell_widget
            .as_ref()
            .and_then(|cell| cell.compute_point(&self.overlay, &gtk4::graphene::Point::zero()))
            .map(|p| (p.x() as f64, p.y() as f64))
            .unwrap_or((x, y - self.scrolled.vadjustment().value()));
        let center = x + item_w as f64 / 2.0;
        let viewport = self.overlay.width() as f64;
        let scrolled_top = self
            .scrolled
            .compute_point(&self.overlay, &gtk4::graphene::Point::zero())
            .map(|p| p.y() as f64)
            .unwrap_or(0.0);
        let scrolled_bottom = scrolled_top + self.scrolled.vadjustment().page_size();
        // Any overlap keeps the floats: the scroll target itself rests the
        // selection's outline a few pixels past the viewport edge, so
        // demanding full visibility here would hide the ring right where
        // navigation parks it. Manual scrolling deselects instead of
        // hiding (see `clear_selection`).
        let in_view = tile_top + item_h as f64 > scrolled_top && tile_top < scrolled_bottom;
        self.ring.set_visible(in_view);
        self.tooltip.set_visible(in_view);
        if !in_view {
            return;
        }
        // The drawn selection ring hugs the tile's edges.
        let scale = viewport / 1920.0;
        self.ring
            .place(x, tile_top, item_w as f64, item_h as f64, scale, false);
        self.tooltip.set_max_width(item_w as f64 * 3.0);
        self.tooltip.set_text(&game.name);
        // The tail tip rests just outside the selection ring on whichever
        // side the pill's actual height fits into the scrolled area —
        // above by preference, below near the top edge.
        let pill_h = self.tooltip.pill_height() as f64;
        let top_tip = tile_top - BP_RING_OUTSET;
        let bottom_tip = tile_top + item_h as f64 + BP_RING_OUTSET;
        let fits_above = top_tip - TAIL_HEIGHT - pill_h >= scrolled_top + 2.0;
        let fits_below = bottom_tip + TAIL_HEIGHT + pill_h <= scrolled_bottom - 4.0;
        if fits_above || !fits_below {
            self.tooltip.set_position(center, viewport, top_tip, false);
        } else {
            self.tooltip.set_position(center, viewport, bottom_tip, true);
        }
    }

    /// Focus the clicked game without moving the camera. Returns whether
    /// the game was found.
    pub(super) fn focus_key(&self, key: GameKey) -> bool {
        let index = self
            .games
            .borrow()
            .iter()
            .position(|g| game_key(g) == key);
        index.is_some_and(|index| self.apply_selection(index))
    }

    /// Whether `key` is the highlighted game — a second mouse click on an
    /// already-focused cell launches it.
    pub(super) fn is_selected(&self, key: GameKey) -> bool {
        self.selected_key.get() == key
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
        let Some(selected) = self.selected.get() else {
            return;
        };
        let (cols, _, item_h, sp) = self.grid.current_layout();
        let target = scroll_target(
            selected,
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

/// Cell factories for the big picture grid: the same square-capsule recipe as the
/// desktop grid, minus the context menus, plus the external highlight.
fn cell_factories(
    state: &SharedState,
    item_size: Rc<Cell<(i32, i32)>>,
    selected_key: Rc<Cell<GameKey>>,
) -> (
    crate::ui::virtual_grid::SetupFn,
    crate::ui::virtual_grid::BindFn,
    crate::ui::virtual_grid::UnbindFn,
) {
    let setup_state = state.clone();
    let setup: crate::ui::virtual_grid::SetupFn = Rc::new(move || {
        build_cell(&setup_state)
    });
    let bind: crate::ui::virtual_grid::BindFn = Rc::new(move |widget, game| {
        bind_cell(widget, game, &item_size, &selected_key);
    });
    let unbind: crate::ui::virtual_grid::UnbindFn = Rc::new(|widget| {
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
    crate::ui::helpers::crisp_label(&name_label);
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
    let Some(game) = game else {
        return;
    };
    let Some(big) = state.borrow().big_picture.clone() else {
        return;
    };
    // First click focuses the pointed-at game (the scroll target leaves a
    // visible row alone, so the camera only moves if the tile was out of
    // view); the second click launches it.
    if big.all.is_selected(game_key(&game)) {
        launch(state, &game);
    } else {
        big.all.focus_key(game_key(&game));
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
        crate::ui::play_button::launch_game(state, game.db_id, game.variant_id)
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
        assert_eq!(grid_move(2, 12, COLS, 0, -1), None, "top edge stays put");
    }

    #[test]
    fn test_grid_move_never_slides_off_short_last_row() {
        // 7 items: row 1 holds indexes 5 and 6. Down from a column the
        // short row does not have stays put instead of dragging the
        // selection onto the row's only remaining cell.
        assert_eq!(grid_move(0, 7, COLS, 0, 1), Some(5));
        assert_eq!(grid_move(3, 7, COLS, 0, 1), None);
        assert_eq!(grid_move(4, 7, COLS, 0, 1), None);
        assert_eq!(grid_move(6, 7, COLS, 0, 1), None);
    }

    #[test]
    fn test_grid_move_stays_put_on_last_row() {
        assert_eq!(grid_move(7, 12, COLS, 0, 1), None);
        assert_eq!(grid_move(11, 12, COLS, 0, 1), None);
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
