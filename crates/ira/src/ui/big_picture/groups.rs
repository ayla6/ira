//! The Groups tab's tile grid: one tile per group — a collage of its
//! members' covers over the group's name — plus a leading New Group tile.
//! The selection is ours (the FlowBox's own is off), driven by the same
//! router that moves the game grid.

use crate::ui::css::*;
use crate::ui::state::SharedState;
use crate::Game;
use gtk4::prelude::*;
use ira_models::Group;
use std::cell::Cell;

/// How many tiles sit in one row, fixed like the game grid's columns.
pub(super) const COLS: usize = 5;

pub(super) struct GroupsGrid {
    scrolled: gtk4::ScrolledWindow,
    flow: gtk4::FlowBox,
    selection: Cell<usize>,
}

impl GroupsGrid {
    pub(super) fn widget(&self) -> &gtk4::ScrolledWindow {
        &self.scrolled
    }

    pub(super) fn build(state: &SharedState) -> Self {
        let flow = gtk4::FlowBox::new();
        flow.set_homogeneous(true);
        flow.set_min_children_per_line(COLS as u32);
        flow.set_max_children_per_line(COLS as u32);
        flow.set_selection_mode(gtk4::SelectionMode::None);
        flow.set_row_spacing(18);
        flow.set_column_spacing(18);
        flow.set_margin_top(12);
        flow.set_margin_bottom(20);
        flow.set_margin_start(28);
        flow.set_margin_end(28);
        flow.set_valign(gtk4::Align::Start);
        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&flow));
        let grid = Self {
            scrolled,
            flow,
            selection: Cell::new(0),
        };
        grid.reload(state);
        grid
    }

    /// Rebuild the tiles: New Group first, then one per group. Counts and
    /// collages only consider games the library shows — hidden games are
    /// not advertised by a group tile.
    pub(super) fn reload(&self, state: &SharedState) {
        let (groups, db, show_hidden) = {
            let s = state.borrow();
            (s.groups.clone(), s.db.clone(), s.cfg.show_hidden_games)
        };
        let visible_games: Vec<Game> = state
            .borrow()
            .games
            .iter()
            .filter(|g| !g.hidden || show_hidden)
            .cloned()
            .collect();
        crate::ui::helpers::clear_children(&self.flow);
        self.append_new_group_tile(state);
        for group in &groups {
            let covers: Vec<&Game> = ira_db::get_game_ids_in_group(&db, group.id)
                .unwrap_or_default()
                .iter()
                .filter_map(|id| visible_games.iter().find(|g| g.db_id == *id))
                .take(4)
                .collect();
            self.append_group_tile(state, group, &covers);
        }
        self.clamp_selection(state);
        self.repaint_selection();
    }

    fn append_new_group_tile(&self, state: &SharedState) {
        let tile = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        let slot = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        slot.add_css_class(CSS_BP_GROUP_SLOT);
        slot.set_size_request(240, 240);
        let icon = gtk4::Image::from_icon_name("list-add-symbolic");
        icon.set_pixel_size(72);
        icon.set_halign(gtk4::Align::Center);
        icon.set_valign(gtk4::Align::Center);
        slot.append(&icon);
        tile.append(&slot);
        let name = gtk4::Label::new(Some(&crate::tr!("New Group")));
        name.add_css_class(CSS_BP_PAGE_SUBTITLE);
        name.set_halign(gtk4::Align::Center);
        crate::ui::helpers::crisp_label(&name);
        tile.append(&name);
        let child = gtk4::FlowBoxChild::new();
        child.set_focusable(false);
        child.set_child(Some(&tile));
        let click_state = state.clone();
        let click = gtk4::GestureClick::new();
        click.connect_pressed(move |_, _, _, _| {
            if let Some(big) = click_state.borrow().big_picture.clone() {
                big.all.groups_grid_selected(&click_state, 0);
            }
        });
        child.add_controller(click);
        self.flow.append(&child);
    }

    fn append_group_tile(
        &self,
        state: &SharedState,
        group: &Group,
        covers: &[&Game],
    ) {
        let tile = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        let slot = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        slot.add_css_class(CSS_BP_GROUP_SLOT);
        slot.set_size_request(240, 240);
        // A 2x2 collage of the group's first four covers, or a placeholder
        // icon for an empty group.
        if covers.is_empty() {
            let icon = gtk4::Image::from_icon_name("view-grid-symbolic");
            icon.set_pixel_size(64);
            icon.set_halign(gtk4::Align::Center);
            icon.set_valign(gtk4::Align::Center);
            slot.append(&icon);
        } else {
            let collage = gtk4::Grid::new();
            collage.set_row_homogeneous(true);
            collage.set_column_homogeneous(true);
            for (i, game) in covers.iter().enumerate() {
                let pic = gtk4::Picture::new();
                pic.set_content_fit(gtk4::ContentFit::Cover);
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
                collage.attach(&pic, (i % 2) as i32, (i / 2) as i32, 1, 1);
            }
            slot.append(&collage);
        }
        tile.append(&slot);
        let name = gtk4::Label::new(Some(&group.name));
        name.add_css_class(CSS_BP_PAGE_SUBTITLE);
        name.set_halign(gtk4::Align::Center);
        name.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        crate::ui::helpers::crisp_label(&name);
        tile.append(&name);

        let child = gtk4::FlowBoxChild::new();
        child.set_focusable(false);
        child.set_child(Some(&tile));
        let click_state = state.clone();
        let group_index = self.group_index_of(state, group.id);
        let click = gtk4::GestureClick::new();
        click.connect_pressed(move |_, _, _, _| {
            if let Some(big) = click_state.borrow().big_picture.clone() {
                big.all
                    .groups_grid_selected(&click_state, 1 + group_index);
            }
        });
        child.add_controller(click);
        self.flow.append(&child);
    }

    fn group_index_of(&self, state: &SharedState, group_id: i64) -> usize {
        state
            .borrow()
            .groups
            .iter()
            .position(|g| g.id == group_id)
            .unwrap_or(0)
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

    /// Paint the selection onto the FlowBox child; focusing it scrolls
    /// the tile into view.
    fn repaint_selection(&self) {
        let selected = self.selection.get();
        let mut index = 0usize;
        let mut child = self.flow.first_child();
        while let Some(node) = child {
            if let Some(flow_child) = node.downcast_ref::<gtk4::FlowBoxChild>() {
                if index == selected {
                    flow_child.add_css_class(CSS_BP_GROUP_TILE_SELECTED);
                    flow_child.grab_focus();
                } else {
                    flow_child.remove_css_class(CSS_BP_GROUP_TILE_SELECTED);
                }
                index += 1;
            }
            child = node.next_sibling();
        }
    }

    /// Keep the selection within the tiles after the groups change.
    pub(super) fn clamp_selection(&self, state: &SharedState) {
        let max = self.tile_count(state).saturating_sub(1);
        if self.selection.get() > max {
            self.selection.set(max);
        }
    }
}
