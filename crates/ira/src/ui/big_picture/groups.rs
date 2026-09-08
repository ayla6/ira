//! The Groups tab's tile grid: one tile per group — a collage of its
//! members' squares over the group's name — plus a leading New Group
//! tile. Tiles size with the viewport like the game grid's. The selection
//! is ours (the FlowBox's own is off), driven by the same router that
//! moves the game grid.

use crate::ui::css::*;
use crate::ui::state::SharedState;
use crate::Game;
use gtk4::prelude::*;
use ira_models::Group;
use std::cell::{Cell, RefCell};

/// How many tiles sit in one row, fixed like the game grid's columns.
pub(super) const COLS: usize = 5;

/// The flow's own edge margins and child spacing; the tile sizing keeps
/// in step with them.
const FLOW_MARGIN: i32 = 28;
const FLOW_SPACING: i32 = 18;

/// The smallest slot allocation the ring will draw from; anything smaller
/// is a pre-layout placeholder, never a tile (tiles are 160px at least).
const MIN_RING_TILE: i32 = 8;

pub(super) struct GroupsGrid {
    /// The scrolled tiles with the selection ring floating over them; the
    /// widget this page shows.
    overlay: gtk4::Overlay,
    scrolled: gtk4::ScrolledWindow,
    flow: gtk4::FlowBox,
    ring: crate::ui::selection_ring::SelectionRing,
    selection: Cell<usize>,
    /// Current tile edge length (slots are square); 0 until the window
    /// reports a real size.
    tile: Cell<i32>,
    /// Each tile's square slot in tile order, for anchoring the ring.
    slots: RefCell<Vec<gtk4::Box>>,
}

impl GroupsGrid {
    pub(super) fn widget(&self) -> &gtk4::Overlay {
        &self.overlay
    }

    pub(super) fn build(state: &SharedState) -> Self {
        let flow = gtk4::FlowBox::new();
        flow.set_homogeneous(true);
        flow.set_min_children_per_line(COLS as u32);
        flow.set_max_children_per_line(COLS as u32);
        flow.set_selection_mode(gtk4::SelectionMode::None);
        flow.set_row_spacing(FLOW_SPACING as u32);
        flow.set_column_spacing(FLOW_SPACING as u32);
        flow.set_margin_top(12);
        flow.set_margin_bottom(20);
        flow.set_margin_start(FLOW_MARGIN);
        flow.set_margin_end(FLOW_MARGIN);
        flow.set_valign(gtk4::Align::Start);
        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&flow));
        // The selection ring floats over the tiles — the same frame the
        // game grid wears, not a CSS outline.
        let ring = crate::ui::selection_ring::SelectionRing::new();
        // The ring re-derives its rect from the selected slot's live
        // geometry every time it is drawn — no event ordering can park it
        // on a stale rect or a pre-layout placeholder size. False (an
        // unreal slot or hidden page) paints nothing that frame.
        let ring_state = state.clone();
        ring.set_repositioner(Some(Box::new(move || {
            let big = ring_state.borrow().big_picture.clone();
            big.is_some_and(|big| big.all.groups_grid.position_ring())
        })));
        let overlay = gtk4::Overlay::new();
        overlay.set_child(Some(&scrolled));
        overlay.add_overlay(&ring);
        overlay.set_measure_overlay(&ring, false);
        overlay.set_clip_overlay(&ring, true);
        // First allocation lands here: size the tiles from the flow's real
        // width and rebuild. Without this, a first open shows the fallback
        // 200px slots floating in oversized cells.
        {
            let size_state = state.clone();
            let track = move |_: &gtk4::Adjustment| {
                if let Some(big) = size_state.borrow().big_picture.clone() {
                    if big.all.groups_grid.ensure_sized() {
                        big.all.groups_grid.reload(&size_state);
                    }
                    big.all.groups_grid.position_ring();
                }
            };
            scrolled.vadjustment().connect_changed(track.clone());
            scrolled.vadjustment().connect_value_changed(track);
        }
        let grid = Self {
            overlay,
            scrolled,
            flow,
            ring,
            selection: Cell::new(0),
            tile: Cell::new(0),
            slots: RefCell::new(Vec::new()),
        };
        grid.reload(state);
        grid
    }

    /// Track the viewport: slots are square and sized like the game
    /// grid's tiles (the flow's real allocation split over the columns —
    /// a window-width read can disagree with the allocation and overflow).
    /// Returns true when the size moved and the tiles need a reload.
    pub(super) fn ensure_sized(&self) -> bool {
        let width = self.flow.width();
        if width < 600 {
            return false;
        }
        let desired = ((width - 2 * FLOW_MARGIN - (COLS as i32 - 1) * FLOW_SPACING) as f64
            / COLS as f64)
            .round() as i32;
        let desired = desired.clamp(160, 520);
        if self.tile.get() == desired {
            return false;
        }
        self.tile.set(desired);
        true
    }

    /// Rebuild the tiles: New Group first, then one per group in the
    /// configured order (alphabetical, or most games first). Counts and
    /// collages only consider games the library shows — hidden games are
    /// not advertised by a group tile.
    pub(super) fn reload(&self, state: &SharedState) {
        self.ensure_sized();
        let (groups, db, show_hidden, order) = {
            let s = state.borrow();
            (
                s.groups.clone(),
                s.db.clone(),
                s.cfg.show_hidden_games,
                s.cfg.group_order,
            )
        };
        let visible_games: Vec<Game> = state
            .borrow()
            .games
            .iter()
            .filter(|g| !g.hidden || show_hidden)
            .cloned()
            .collect();
        crate::ui::helpers::clear_children(&self.flow);
        self.slots.borrow_mut().clear();
        let tile = self.tile.get().max(200);
        self.append_new_group_tile(state, tile);
        // (member count, group, cover games) — the count feeds the size
        // ordering, the covers feed the collage.
        let mut entries: Vec<(usize, &Group, Vec<&Game>)> = groups
            .iter()
            .map(|group| {
                let members = ira_db::get_game_ids_in_group(&db, group.id).unwrap_or_default();
                let covers: Vec<&Game> = members
                    .iter()
                    .filter_map(|id| visible_games.iter().find(|g| g.db_id == *id))
                    .take(4)
                    .collect();
                (members.len(), group, covers)
            })
            .collect();
        if order == ira_models::GroupOrder::Size {
            entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
        }
        for (group, covers) in entries.into_iter().map(|(_, group, covers)| (group, covers)) {
            self.append_group_tile(state, group, &covers, tile);
        }
        self.clamp_selection(state);
        self.repaint_selection();
    }

    /// The shared tile shell: square slot over the name, wired with the
    /// click gesture (first click focuses, a click on the focused tile
    /// opens) and registered for selection painting. Returns the slot for
    /// the caller to fill.
    fn append_tile_shell(&self, state: &SharedState, label: &str, tile: i32) -> gtk4::Box {
        let tile_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        let slot = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        slot.add_css_class(CSS_BP_GROUP_SLOT);
        slot.set_size_request(tile, tile);
        tile_box.append(&slot);
        let name = gtk4::Label::new(Some(label));
        name.add_css_class(CSS_BP_PAGE_SUBTITLE);
        name.set_halign(gtk4::Align::Center);
        name.set_valign(gtk4::Align::Center);
        name.set_vexpand(true);
        name.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        crate::ui::helpers::crisp_label(&name);
        tile_box.append(&name);

        let index = self.slots.borrow().len();
        self.slots.borrow_mut().push(slot.clone());
        let click_state = state.clone();
        let click = gtk4::GestureClick::new();
        click.connect_pressed(move |_, _, _, _| {
            if let Some(big) = click_state.borrow().big_picture.clone() {
                big.all.groups_grid_selected(&click_state, index);
            }
        });
        tile_box.add_controller(click);
        let child = gtk4::FlowBoxChild::new();
        child.set_focusable(false);
        child.set_child(Some(&tile_box));
        self.flow.append(&child);
        slot
    }

    fn append_new_group_tile(&self, state: &SharedState, tile: i32) {
        let slot = self.append_tile_shell(state, &crate::tr!("New Group"), tile);
        let icon = gtk4::Image::from_icon_name("list-add-symbolic");
        icon.set_pixel_size((tile / 3).max(24));
        // Expand + center: a plain centered child of a fill-allocation
        // Box keeps the left edge, since its allocation is already its
        // natural size.
        icon.set_hexpand(true);
        icon.set_vexpand(true);
        icon.set_halign(gtk4::Align::Center);
        icon.set_valign(gtk4::Align::Center);
        slot.append(&icon);
    }

    fn append_group_tile(&self, state: &SharedState, group: &Group, covers: &[&Game], tile: i32) {
        let slot = self.append_tile_shell(state, &group.name, tile);
        // A 2x2 collage of the group's first four squares, or a
        // placeholder icon for an empty group.
        if covers.is_empty() {
            let icon = gtk4::Image::from_icon_name("view-grid-symbolic");
            icon.set_pixel_size((tile / 3).max(24));
            icon.set_hexpand(true);
            icon.set_vexpand(true);
            icon.set_halign(gtk4::Align::Center);
            icon.set_valign(gtk4::Align::Center);
            slot.append(&icon);
            return;
        }
        let collage = gtk4::Grid::new();
        collage.set_row_homogeneous(true);
        collage.set_column_homogeneous(true);
        // Breathing room between the quarters: the slot's background
        // reads through the gaps instead of back-to-back art.
        let gap = (tile as f64 * 0.015).round().max(2.0) as u32;
        collage.set_row_spacing(gap);
        collage.set_column_spacing(gap);
        // Always a 2x2: the group's covers take the first cells and the
        // missing ones stay invisible, so a 2-cover group is two quarters
        // in the top row — never two stretched halves.
        for i in 0..4 {
            let cell = match covers.get(i) {
                Some(game) => {
                    let pic = gtk4::Picture::new();
                    pic.set_content_fit(gtk4::ContentFit::Cover);
                    // GTK sizes a Picture to its paintable's natural size
                    // unless it may shrink; without this the quadrants
                    // blow up to 384px and, through the FlowBox's
                    // homogeneous cells, drag the whole grid past the
                    // viewport edge.
                    pic.set_can_shrink(true);
                    pic.set_halign(gtk4::Align::Fill);
                    pic.set_valign(gtk4::Align::Fill);
                    pic.add_css_class(CSS_GAME_COVER_PIC);
                    let path = if game.square_path.is_empty() {
                        &game.grid_path
                    } else {
                        &game.square_path
                    };
                    if !path.is_empty() {
                        if let Some(texture) = ira_images::cached_texture(path) {
                            pic.set_paintable(Some(&texture));
                        } else {
                            let pic_weak = pic.downgrade();
                            let path_owned = path.clone();
                            ira_images::load_texture_async_with_priority(
                                &path_owned,
                                glib::Priority::DEFAULT,
                                move |texture| {
                                    if let (Some(pic), Some(t)) = (pic_weak.upgrade(), texture) {
                                        pic.set_paintable(Some(&t));
                                    }
                                },
                            );
                        }
                    }
                    pic.upcast::<gtk4::Widget>()
                }
                None => gtk4::Box::new(gtk4::Orientation::Horizontal, 0).upcast::<gtk4::Widget>(),
            };
            collage.attach(&cell, (i % 2) as i32, (i / 2) as i32, 1, 1);
        }
        slot.append(&collage);
    }

    /// The selected tile's index (0 = New Group).
    pub(super) fn selection(&self) -> usize {
        self.selection.get()
    }

    /// Point the selection at a tile without activating it.
    pub(super) fn select_tile(&self, tile: usize) {
        self.selection.set(tile);
    }

    /// Re-clamp and repaint after an outside selection change.
    pub(super) fn repaint(&self, state: &SharedState) {
        self.clamp_selection(state);
        self.repaint_selection();
    }

    /// How many tiles exist: New Group plus one per group.
    pub(super) fn tile_count(&self, state: &SharedState) -> usize {
        state.borrow().groups.len() + 1
    }

    /// The group id of the selected tile; None on the New Group tile.
    pub(super) fn selected_group_id(&self, state: &SharedState) -> Option<i64> {
        let index = self.selection.get();
        if index == 0 {
            return None;
        }
        state.borrow().groups.get(index - 1).map(|g| g.id)
    }

    pub(super) fn move_selection(&self, state: &SharedState, dx: i32, dy: i32) {
        let count = self.tile_count(state);
        let next = super::all_games::grid_move(self.selection.get(), count, COLS, dx, dy);
        if let Some(next) = next {
            self.selection.set(next);
            self.repaint_selection();
        }
    }

    /// Paint the selection: float the ring over the selected tile's slot
    /// — the same frame the game grid wears — and keep the row on screen.
    fn repaint_selection(&self) {
        self.position_ring();
        self.scroll_selection_into_view();
    }

    /// Derive the ring's rect from the selected slot's live position.
    /// Called both from events (scroll, tab changes) and at draw time
    /// from the ring's own snapshot — the latter is the guarantee the
    /// frame is always painted from geometry that exists right now.
    /// False means there is nothing real to draw: an unmapped surface,
    /// or a slot without a full-size allocation (a slot mapped before
    /// its first allocation is exactly the tiny square ring).
    pub(super) fn position_ring(&self) -> bool {
        if self.overlay.width() <= 1 || self.overlay.height() <= 1 || !self.overlay.is_mapped() {
            return false;
        }
        let Some(slot) = self.slots.borrow().get(self.selection.get()).cloned() else {
            return false;
        };
        if !slot.is_mapped() || slot.width() < MIN_RING_TILE || slot.height() < MIN_RING_TILE {
            return false;
        }
        let Some(point) = slot.compute_point(&self.overlay, &gtk4::graphene::Point::zero())
        else {
            return false;
        };
        self.ring.set_rect(
            point.x() as f64,
            point.y() as f64,
            slot.width() as f64,
            slot.height() as f64,
            self.overlay.width() as f64 / 1920.0,
            false,
        );
        true
    }

    /// Nudge the scroll so the selected row shows. Rows pitch at the
    /// FlowBoxChild's allocated height (slot plus name label) plus the
    /// spacing; children are uniform so any child's height serves.
    fn scroll_selection_into_view(&self) {
        let adj = self.scrolled.vadjustment();
        let page = adj.page_size();
        if page <= 1.0 {
            return;
        }
        let Some(child) = self
            .flow
            .first_child()
            .and_then(|c| c.downcast::<gtk4::FlowBoxChild>().ok())
        else {
            return;
        };
        let child_h = child.height() as f64;
        if child_h <= 0.0 {
            return;
        }
        let row_top = self.flow.margin_top() as f64
            + (self.selection.get() / COLS) as f64 * (child_h + FLOW_SPACING as f64);
        let row_bottom = row_top + child_h;
        let target = if row_top < adj.value() {
            row_top - FLOW_MARGIN as f64
        } else if row_bottom > adj.value() + page {
            row_bottom - page + FLOW_MARGIN as f64
        } else {
            return;
        };
        adj.set_value(target.max(0.0));
    }

    /// Keep the selection within the tiles after the groups change.
    pub(super) fn clamp_selection(&self, state: &SharedState) {
        let max = self.tile_count(state).saturating_sub(1);
        if self.selection.get() > max {
            self.selection.set(max);
        }
    }
}
