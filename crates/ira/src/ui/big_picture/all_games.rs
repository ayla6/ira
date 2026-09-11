//! The All Software page: every visible game on a square-capsule grid,
//! reached from the grid tile at the end of the home carousel. Controller
//! and keyboard drive an external selection highlight (the grid itself only
//! recycles cells); A launches, B returns home.

use crate::ui::css::*;
use super::marquee::Marquee;
use crate::ui::selection_ring::SelectionRing;
use crate::ui::state::SharedState;
use crate::ui::virtual_grid::VirtualGrid;
use crate::Game;
use adw::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;

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
    let last_row = (count - 1) / cols;
    let next_row = (row + dy as i64).clamp(0, last_row);
    // Horizontal moves wrap within the row: the row's right edge comes
    // around to its own left edge. A short last row wraps within the
    // cells it actually has.
    let row_len = (count - row * cols).min(cols);
    let next_col = if dx != 0 {
        (col + dx as i64).rem_euclid(row_len)
    } else {
        col
    };
    let next = next_row * cols + next_col;
    // A vertical move onto a short last row's missing column stays put
    // instead of dragging the selection sideways.
    (next < count && next != current as i64).then_some(next as usize)
}

/// The selection outline pokes this far past a tile's edge (the ring's
/// 11px outset plus a margin); the scroll rests that much above every row
/// boundary so the whole ring stays on screen instead of being shaved at
/// the viewport edge — the top row included.
const OUTLINE_ALLOWANCE: f64 = 18.0;

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

/// Which tab of the single big-picture page is showing: the recent
/// carousel, the game grid, or the groups tiles.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tab {
    Recent,
    Everything,
    Groups,
}

/// Reference (1080p) pixels at the current viewport scale, whole-pixel.
fn scaled_px(px: f64) -> i32 {
    (px * crate::ui::css::bp_scale()).round() as i32
}

/// The tabs' order, used for wrapping with the shoulders and for picking
/// the slide's direction.
const TAB_ORDER: [Tab; 3] = [Tab::Recent, Tab::Everything, Tab::Groups];

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
        let glyph = gtk4::Image::new();
        let fallback = gtk4::Label::new(None);
        fallback.add_css_class(CSS_BP_SHOULDER);
        slot.append(&glyph);
        slot.append(&fallback);
        Self { slot, glyph, fallback }
    }

    fn refresh(&self, button: ira_input::GamepadButton, family: ira_input::ControllerFamily, scale: f64) {
        // The slot hugs its content: a GtkBox lays children out one after
        // another on its main axis, so a slot wider than the art would not
        // center it — the leftover width would sit between only one badge
        // and the tab picker, and the two shoulders would hang at different
        // distances from it.
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

/// Widgets and selection state of the big-picture page: one screen with
/// Recent / Everything / Groups tabs over a shared header.
pub(super) struct AllSoftwareUi {
    page: gtk4::Box,
    surfaces: gtk4::Stack,
    recent_page: gtk4::Widget,
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
    /// The last selection before the grid lost it (manual scrolling) —
    /// the arrows re-acquire here, keeping the column in view.
    last_selected: Cell<Option<usize>>,
    /// The running scroll glide, so a new press replaces it mid-flight.
    scroll_anim: Rc<RefCell<Option<gtk4::TickCallbackId>>>,
    /// The "sorted by …" label in the header.
    ordering: gtk4::Label,
    /// The header line and its wings; their margins and spacings are
    /// re-applied with the viewport scale.
    header: gtk4::CenterBox,
    header_start: gtk4::Box,
    header_center: gtk4::Box,
    /// The sort button's icon, scaled with the viewport like the rails'.
    sort_icon: gtk4::Image,
    tabs: adw::ToggleGroup,
    /// The L/R shoulder badges flanking the tabs.
    shoulder_l: ShoulderBadge,
    shoulder_r: ShoulderBadge,
    /// The connected pad's family, so the badges draw its glyphs.
    shoulder_family: Cell<ira_input::ControllerFamily>,
    shoulder_scale: Cell<f64>,
    /// The scale the tab picker's icons were sized at.
    tab_icon_scale: Cell<f64>,
    /// Which tab is showing and, on the Groups tab, which group's games
    /// the grid holds (`None` = the groups tile grid itself).
    tab: Cell<Tab>,
    groups_view: Cell<Option<i64>>,
    pub(super) groups_grid: super::groups::GroupsGrid,
}

pub(super) fn build(
    state: &SharedState,
    recent_page: &gtk4::Overlay,
) -> (gtk4::Box, AllSoftwareUi) {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    page.add_css_class(CSS_BP_ALL_PAGE);

    // One header line, truly centered: the ordering and its button in the
    // start wing, the tab picker in the middle (flanked by the pad's
    // shoulder glyphs), and nothing on the right — the status rail floats
    // above that empty corner (see `view::build_root`).
    let header = gtk4::CenterBox::new();
    header.set_margin_top(scaled_px(18.0));
    header.set_margin_bottom(scaled_px(6.0));
    header.set_margin_start(scaled_px(28.0));
    header.set_margin_end(scaled_px(28.0));
    let ordering = gtk4::Label::new(None);
    ordering.set_valign(gtk4::Align::Center);
    ordering.add_css_class(CSS_BP_PAGE_SUBTITLE);
    crate::ui::helpers::crisp_label(&ordering);
    // The sort button opens the sort menu (Options does the same when no
    // game is focused); on the Groups tab it orders the tiles instead.
    let sort_icon = gtk4::Image::from_icon_name("view-sort-descending-symbolic");
    let sort_btn = gtk4::Button::new();
    sort_btn.set_child(Some(&sort_icon));
    sort_btn.add_css_class(CSS_FLAT);
    sort_btn.set_focusable(false);
    sort_btn.set_valign(gtk4::Align::Center);
    sort_btn.set_tooltip_text(Some(&crate::tr!("Sort by")));
    {
        let sort_state = state.clone();
        sort_btn.connect_clicked(move |_| {
            let Some(big) = sort_state.borrow().big_picture.clone() else {
                return;
            };
            let kind = if big.all.tab.get() == Tab::Groups {
                super::game_menu::MenuKind::GroupOrder
            } else {
                super::game_menu::MenuKind::Sort
            };
            big.game_menu.open(&sort_state, kind);
        });
    }
    let start = gtk4::Box::new(gtk4::Orientation::Horizontal, scaled_px(10.0));
    start.append(&sort_btn);
    start.append(&ordering);
    header.set_start_widget(Some(&start));
    let shoulder_l = ShoulderBadge::new();
    let shoulder_r = ShoulderBadge::new();
    let tabs = adw::ToggleGroup::new();
    tabs.add(
        adw::Toggle::builder()
            .name("recent")
            .label(crate::tr!("Recent"))
            .icon_name("document-open-recent-symbolic")
            .build(),
    );
    tabs.add(
        adw::Toggle::builder()
            .name("everything")
            .label(crate::tr!("Everything"))
            .icon_name("view-grid-symbolic")
            .build(),
    );
    tabs.add(
        adw::Toggle::builder()
            .name("groups")
            .label(crate::tr!("Groups"))
            .icon_name("folder-symbolic")
            .build(),
    );
    tabs.add_css_class(CSS_BP_TABS);
    tabs.set_active_name(Some("recent"));
    // The first paint must already sit centered: the styles the pango
    // metrics are read from only settle once the page maps.
    tabs.connect_map(|w| {
        if let Some(tabs) = w.downcast_ref::<adw::ToggleGroup>() {
            center_tab_icons(tabs);
        }
    });
    let center = gtk4::Box::new(gtk4::Orientation::Horizontal, scaled_px(14.0));
    center.set_valign(gtk4::Align::Center);
    center.append(&shoulder_l.slot);
    center.append(&tabs);
    center.append(&shoulder_r.slot);
    header.set_center_widget(Some(&center));
    {
        let tab_state = state.clone();
        tabs.connect_active_name_notify(move |_| {
            let Some(big) = tab_state.borrow().big_picture.clone() else {
                return;
            };
            let tab = match big.all.tabs.active_name().as_deref() {
                Some("groups") => Tab::Groups,
                Some("everything") => Tab::Everything,
                _ => Tab::Recent,
            };
            big.all.set_tab(&tab_state, tab);
        });
    }
    page.append(&header);

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

    let groups = super::groups::GroupsGrid::build(state);

    // The three tab surfaces in one stack: the recent carousel, the game
    // grid, and the groups tiles. The switch is a quick crossfade: the
    // pages never move, so every viewport keeps clipping exactly like it
    // does at rest. A slide is what dragged the covers' clipped-off
    // halves through the visible area — the pages travel beneath the
    // stack's one stationary edge, so the parts a viewport had cut away
    // reappear mid-switch, full square icons and all.
    let surfaces = gtk4::Stack::new();
    surfaces.set_transition_type(gtk4::StackTransitionType::Crossfade);
    surfaces.set_transition_duration(130);
    surfaces.set_overflow(gtk4::Overflow::Hidden);
    surfaces.set_vexpand(true);
    surfaces.add_titled(recent_page, Some("recent"), "recent");
    surfaces.add_named(&grid_overlay, Some("everything"));
    surfaces.add_named(groups.widget(), Some("groups"));
    surfaces.set_visible_child(recent_page);
    page.append(&surfaces);

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
        surfaces,
        recent_page: recent_page.clone().upcast::<gtk4::Widget>(),
        scrolled,
        grid,
        tooltip,
        empty: empty.clone(),
        games: RefCell::new(Vec::new()),
        selected: Cell::new(None),
        selected_key,
        last_selected: Cell::new(None),
        scroll_anim: Rc::new(RefCell::new(None)),
        ring,
        overlay: grid_overlay,
        ordering,
        header,
        header_start: start,
        header_center: center,
        sort_icon,
        tabs,
        shoulder_l,
        shoulder_r,
        shoulder_family: Cell::new(ira_input::ControllerFamily::Xbox),
        shoulder_scale: Cell::new(0.0),
        tab_icon_scale: Cell::new(0.0),
        tab: Cell::new(Tab::Recent),
        groups_view: Cell::new(None),
        groups_grid: groups,
    };
    ui.update_ordering_label(state);
    (ui.page.clone(), ui)
}

impl AllSoftwareUi {
    /// The tab whose surface is showing (a group's game view counts as
    /// Everything for input purposes).
    pub(super) fn tab(&self) -> Tab {
        self.tab.get()
    }

    /// Name the current ordering in the header: the game sort on
    /// Everything, the tile order on Groups, nothing on Recent.
    pub(super) fn update_ordering_label(&self, state: &SharedState) {
        if let Some(parent) = self.ordering.parent() {
            // The sort button rides the same wing; Recent shows neither.
            parent.set_visible(self.tab.get() != Tab::Recent);
        }
        match self.tab.get() {
            Tab::Recent => {
                self.ordering.set_text("");
                self.ordering.set_visible(false);
            }
            Tab::Everything => {
                let (mode, descending) = {
                    let s = state.borrow();
                    (s.cfg.sort_mode, s.cfg.sort_descending)
                };
                let arrow = if descending { " ↓" } else { "" };
                self.ordering
                    .set_text(&format!("{}{arrow}", mode.display_label()));
                self.ordering.set_visible(true);
            }
            Tab::Groups => {
                let order = state.borrow().cfg.group_order;
                self.ordering.set_text(order.display_label());
                self.ordering.set_visible(true);
            }
        }
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

    /// Scale the header's chrome with the viewport (1.0 = 1920 wide), like
    /// the rails' icons.
    pub(super) fn set_icon_scale(&self, scale: f64) {
        self.shoulder_scale.set(scale);
        self.refresh_shoulders();
        self.scale_tab_icons(scale);
        self.sort_icon.set_pixel_size(scaled_px(22.0));
        self.header.set_margin_top(scaled_px(18.0));
        self.header.set_margin_bottom(scaled_px(6.0));
        self.header.set_margin_start(scaled_px(28.0));
        self.header.set_margin_end(scaled_px(28.0));
        self.header_start.set_spacing(scaled_px(10.0));
        self.header_center.set_spacing(scaled_px(14.0));
    }

    /// The tab picker's icons default to 16px regardless of the viewport;
    /// size them with the same scale the rest of the header uses.
    fn scale_tab_icons(&self, scale: f64) {
        if (self.tab_icon_scale.get() - scale).abs() < 0.01 {
            return;
        }
        self.tab_icon_scale.set(scale);
        let pixel = (22.0 * scale).round().max(8.0) as i32;
        let mut stack = vec![self.tabs.clone().upcast::<gtk4::Widget>()];
        while let Some(widget) = stack.pop() {
            if let Some(image) = widget.downcast_ref::<gtk4::Image>() {
                image.set_pixel_size(pixel);
            }
            let mut child = widget.first_child();
            while let Some(node) = child {
                stack.push(node.clone());
                child = node.next_sibling();
            }
        }
        center_tab_icons(&self.tabs);
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
        let scale = self.shoulder_scale.get();
        self.shoulder_l
            .refresh(ira_input::GamepadButton::LeftShoulder, family, scale);
        self.shoulder_r
            .refresh(ira_input::GamepadButton::RightShoulder, family, scale);
    }

    /// Switch tabs. The Groups tab reloads its tiles so groups created
    /// elsewhere show up; a group's game view never survives the switch —
    /// the shoulders and the toggle group always land on the tab's own
    /// surface.
    pub(super) fn set_tab(&self, state: &SharedState, tab: Tab) {
        if self.tab.get() == tab && self.groups_view.get().is_none() {
            return;
        }
        self.tab.set(tab);
        self.groups_view.set(None);
        // A glide caught mid-flight by the slide shows a half-scrolled
        // tile at the viewport edge; land both surfaces before sliding.
        self.finish_scroll();
        super::home::finish_scroll(state);
        // Keep the toggle group in step; its notify loops back here and
        // early-returns.
        let name = match tab {
            Tab::Recent => "recent",
            Tab::Everything => "everything",
            Tab::Groups => "groups",
        };
        if self.tabs.active_name().as_deref() != Some(name) {
            self.tabs.set_active_name(Some(name));
        }
        if tab == Tab::Groups {
            self.groups_grid.reload(state);
        }
        self.apply_mode(state);
    }

    /// Step to the neighbouring tab (the shoulders), wrapping around.
    pub(super) fn switch_tab(&self, state: &SharedState, delta: i32) {
        let current = TAB_ORDER
            .iter()
            .position(|t| *t == self.tab.get())
            .unwrap_or(0) as i64;
        let next = (current + delta as i64).rem_euclid(TAB_ORDER.len() as i64) as usize;
        self.set_tab(state, TAB_ORDER[next]);
    }

    /// Back out one level on this page: a group's games return to the
    /// groups tiles. Returns false when the page has nothing to pop and
    /// the caller should quit.
    pub(super) fn on_back(&self, state: &SharedState) -> bool {
        if self.tab.get() == Tab::Groups && self.groups_view.get().is_some() {
            self.groups_view.set(None);
            // Popping the group view is not a category change: no slide.
            self.without_slide(|s| s.apply_mode(state));
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
                // The group starts at its top: jump, don't glide from
                // wherever the groups tiles were scrolled to.
                self.scrolled.vadjustment().set_value(0.0);
                // Same surface swap, not a category change: no slide.
                self.without_slide(|s| s.apply_mode(state));
            }
            None => super::view::name_new_group(state, None),
        }
    }

    /// Run a surface change with the tab fade suppressed — a group view
    /// swap is not a category change.
    fn without_slide(&self, f: impl FnOnce(&Self)) {
        self.surfaces.set_transition_type(gtk4::StackTransitionType::None);
        f(self);
        self.surfaces
            .set_transition_type(gtk4::StackTransitionType::Crossfade);
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
    /// The groups tab's X: ask before the group is gone for good.
    pub(super) fn groups_delete_selected(&self, state: &SharedState) {
        let Some(id) = self.groups_grid.selected_group_id(state) else {
            return;
        };
        let name = state
            .borrow()
            .groups
            .iter()
            .find(|g| g.id == id)
            .map(|g| g.name.clone())
            .unwrap_or_default();
        if let Some(big) = state.borrow().big_picture.clone() {
            big.game_menu.open(
                state,
                super::game_menu::MenuKind::ConfirmDelete { id, name },
            );
        }
    }

    /// A group was deleted: refresh the shared list and the tiles, and
    /// land back on the groups tiles.
    pub(super) fn group_deleted(&self, state: &SharedState, group_id: i64) {
        self.sync_groups(state);
        if self.groups_view.get() == Some(group_id) {
            self.groups_view.set(None);
        }
        self.groups_grid.reload(state);
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

    /// After a create, rename, or delete: sync the tiles and land the
    /// selection on the touched group — without opening it.
    pub(super) fn sync_and_focus_group(&self, state: &SharedState, group_id: i64) {
        self.sync_groups(state);
        self.groups_view.set(None);
        self.groups_grid.reload(state);
        if let Some(index) = state.borrow().groups.iter().position(|g| g.id == group_id) {
            self.groups_grid.select_tile(index + 1);
            self.groups_grid.repaint(state);
        }
        self.selected.set(None);
        self.apply_mode(state);
    }

    /// Lay out the page for the current tab and view: which surface
    /// shows, which button prompts the bottom rail has, and — on the game
    /// grid — a default selection so a freshly entered surface is never
    /// highlight-less.
    pub(super) fn apply_mode(&self, state: &SharedState) {
        let tiles = self.in_groups_tiles();
        let recent = self.tab.get() == Tab::Recent;
        let target: &gtk4::Widget = if recent {
            &self.recent_page
        } else if tiles {
            self.groups_grid.widget().upcast_ref()
        } else {
            self.overlay.upcast_ref()
        };
        self.surfaces.set_visible_child(target);
        self.refresh(state);
        if tiles {
            // The ring re-anchors itself from the tiles' own allocation
            // hooks; a tab slide reallocates the page and lands it.
            self.groups_grid.repaint(state);
        } else if !recent {
            self.ensure_default_selection();
        }
        self.update_ordering_label(state);
        if let Some(big) = state.borrow().big_picture.clone() {
            let prompts: Vec<(ira_input::GamepadButton, String)> = if recent {
                vec![(ira_input::GamepadButton::A, crate::tr!("Play"))]
            } else if tiles {
                vec![
                    (ira_input::GamepadButton::X, crate::tr!("Delete Group")),
                    (ira_input::GamepadButton::Start, crate::tr!("Rename")),
                    (ira_input::GamepadButton::A, crate::tr!("OK")),
                ]
            } else if self.groups_view.get().is_some() {
                vec![
                    (ira_input::GamepadButton::B, crate::tr!("Back")),
                    (ira_input::GamepadButton::Start, crate::tr!("Options")),
                    (ira_input::GamepadButton::A, crate::tr!("Play")),
                ]
            } else {
                vec![
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
        if self.selected.get().is_none() && !self.games.borrow().is_empty() {
            self.select(0);
        }
    }

    pub(super) fn refresh(&self, state: &SharedState) {
        if self.groups_grid.ensure_sized() {
            self.groups_grid.reload(state);
        }
        // Keep the tiles current while they show: games hidden or art
        // arriving changes what the collages advertise.
        if self.in_groups_tiles() {
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
        // The library landed with nothing selected: park on the first
        // tile.
        if was_empty && !games.is_empty() {
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
            let cols = cols.max(1) as usize;
            let adj = self.scrolled.vadjustment();
            let row_h = (item_h + sp) as f64;
            // The old selection comes back, keeping its column, with the
            // row clamped into whatever the view currently shows — the
            // camera never jumps to where the selection used to be.
            let first_row = (adj.value() / row_h).floor().max(0.0) as usize;
            let last_row = (((adj.value() + adj.page_size()) / row_h).ceil() as usize)
                .saturating_sub(1)
                .min((count - 1) / cols);
            let index = match self.last_selected.get() {
                Some(old) if old < count => {
                    let row = (old / cols).clamp(first_row, last_row);
                    (row * cols + old % cols).min(count - 1)
                }
                _ => (first_row.min(last_row) * cols).min(count - 1),
            };
            self.select(index);
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
        self.last_selected.set(Some(index));
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
        let ring_outset = crate::ui::css::bp_ring_outset();
        let tail = super::marquee::scaled_tail_height();
        let top_tip = tile_top - ring_outset;
        let bottom_tip = tile_top + item_h as f64 + ring_outset;
        let fits_above = top_tip - tail - pill_h >= scrolled_top + 2.0;
        let fits_below = bottom_tip + tail + pill_h <= scrolled_bottom - 4.0;
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
        self.animate_scroll_to(self.snapped_scroll_target(&adj));
    }

    /// The scroll value that rests exactly on the selected row — never
    /// between rows, which is what puts a half-visible tile at the
    /// viewport edge. Zero before the first layout: the page height then
    /// reads as zero, which would make every row look like it pokes out
    /// and dive the scroll deep into the grid.
    fn snapped_scroll_target(&self, adj: &gtk4::Adjustment) -> f64 {
        if adj.page_size() <= 1.0 {
            return 0.0;
        }
        let Some(selected) = self.selected.get() else {
            return adj.value();
        };
        let (cols, _, item_h, sp) = self.grid.current_layout();
        scroll_target(
            selected,
            cols as usize,
            (item_h + sp) as f64,
            sp as f64,
            adj.value(),
            adj.page_size(),
        )
    }

    /// End any scroll glide on the exact snapped target. A tab switch
    /// must not catch the grid mid-glide: the slide would show a
    /// half-scrolled tile at the viewport edge.
    pub(super) fn finish_scroll(&self) {
        if let Some(id) = self.scroll_anim.borrow_mut().take() {
            id.remove();
        }
        let adj = self.scrolled.vadjustment();
        adj.set_value(self.snapped_scroll_target(&adj));
    }

    /// Glide to `target` like the home carousel does; a press during the
    /// glide replaces it from wherever it currently is. The glide steps
    /// on frame-clock ticks — once per displayed frame, in sync with
    /// vsync — instead of 16ms timers that drift against the frames and
    /// read as choppy.
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
        let id = self.scrolled.add_tick_callback(move |_, _| {
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

/// Vertically center each tab's icon on its label's capital-letter band.
/// Plain box centering aligns the icon to the label's line box, and the
/// bundled font's glyphs sit low in that box (its tall CJK line
/// metrics), so the icon reads as floating above the text. Measure the
/// real geometry from pango — the font's baseline and cap height
/// against the rendered line height — instead of tuning pixels by eye.
fn center_tab_icons(tabs: &adw::ToggleGroup) {
    let mut stack = vec![tabs.clone().upcast::<gtk4::Widget>()];
    while let Some(widget) = stack.pop() {
        if let Some(label) = widget.downcast_ref::<gtk4::Label>() {
            center_icon_on_label(label);
        }
        let mut child = widget.first_child();
        while let Some(node) = child {
            stack.push(node.clone());
            child = node.next_sibling();
        }
    }
}

/// Drop the icon beside `label` so its center rests on the
/// capital-letter band (baseline − cap height / 2) — the band the eye
/// reads as "the text", identical for every word.
fn center_icon_on_label(label: &gtk4::Label) {
    let Some(image) = sibling_image(label) else {
        return;
    };
    let layout = label.layout();
    let (_, logical) = layout.extents();
    // "X" measures the font's cap band independent of the word: every
    // tab must agree, whatever the translation's descenders do.
    let probe = pango::Layout::new(&label.pango_context());
    probe.set_text("X");
    let (cap, _) = probe.extents();
    let Some(drop) = icon_drop_px(layout.baseline(), cap.height(), logical.height()) else {
        return; // styles not settled yet; the map pass redoes it
    };
    // A fill-aligned image paints centered in whatever box it is given,
    // so a margin moved it by only half; centered, the margin is the
    // exact distance.
    image.set_valign(gtk4::Align::Center);
    if image.margin_top() != drop {
        image.set_margin_top(drop);
    }
}

/// The image sharing `label`'s container — AdwButtonContent's box.
fn sibling_image(label: &gtk4::Label) -> Option<gtk4::Image> {
    let parent = label.parent()?.downcast::<gtk4::Box>().ok()?;
    let mut child = parent.first_child();
    while let Some(node) = child {
        if let Some(image) = node.downcast_ref::<gtk4::Image>() {
            return Some(image.clone());
        }
        child = node.next_sibling();
    }
    None
}

/// The margin dropping a centered icon onto the cap band, from pango
/// extents (PANGO_SCALE units). `None` while the styles have not
/// settled and the geometry would measure as zero.
fn icon_drop_px(baseline: i32, cap_height: i32, line_height: i32) -> Option<i32> {
    if line_height <= 0 || cap_height <= 0 {
        return None;
    }
    let drop = (i64::from(baseline) - i64::from(cap_height) / 2 - i64::from(line_height) / 2)
        as f64
        / f64::from(pango::SCALE);
    Some(drop.round().max(0.0) as i32)
}

#[cfg(test)]
mod tests {
    use super::{grid_move, icon_drop_px, scroll_target};

    const COLS: usize = 5;

    #[test]
    fn test_grid_move_wraps_within_the_row() {
        assert_eq!(grid_move(0, 10, COLS, 1, 0), Some(1));
        assert_eq!(grid_move(4, 10, COLS, 1, 0), Some(0), "row edge wraps left");
        assert_eq!(grid_move(5, 10, COLS, -1, 0), Some(9), "row start wraps right");
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
        // The short row wraps within its own two cells.
        assert_eq!(grid_move(6, 7, COLS, 1, 0), Some(5));
        assert_eq!(grid_move(5, 7, COLS, -1, 0), Some(6));
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
        // a WHOLE row minus the ring's room (90 = row 1's top at 108,
        // lowered 18 so its whole ring stays on screen).
        assert_eq!(scroll_target(10, COLS, row_h, top_pad, 0.0, 300.0), 90.0);
        // Row 1 (index 5) visible at that position: no move.
        assert_eq!(scroll_target(5, COLS, row_h, top_pad, 90.0, 300.0), 90.0);
        // Scrolling back up to row 0 lands on the first boundary, again
        // lowered by the ring's room, clamped to the top.
        assert_eq!(scroll_target(0, COLS, row_h, top_pad, 90.0, 300.0), 0.0);
        // Deep rows (row 5 = index 25) scroll to the boundary that fits.
        assert_eq!(scroll_target(25, COLS, row_h, top_pad, 90.0, 300.0), 390.0);
    }

    #[test]
    fn test_icon_drop_lands_icon_center_on_cap_band() {
        let k = pango::SCALE;
        // A 30px line box, baseline at 25px, 15px caps: the cap band
        // center sits at 25 − 7.5 = 17.5px, the box center at 15px →
        // 2.5px down.
        assert_eq!(icon_drop_px(25 * k, 15 * k, 30 * k), Some(3));
        // Symmetric metrics need no offset.
        assert_eq!(icon_drop_px(20 * k, 16 * k, 24 * k), Some(0));
        // A taller line box than the cap band can reach never pushes up.
        assert_eq!(icon_drop_px(20 * k, 16 * k, 40 * k), Some(0));
        // Unstyled geometry measures as zero: skip the pass.
        assert_eq!(icon_drop_px(0, 0, 0), None);
    }
}
