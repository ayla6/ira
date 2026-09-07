//! The big-picture options menus: a full-height panel docked on the right
//! over a dimmed page, in two flavors — group assignment for a focused
//! game, and the All Software sort picker. While a menu is open the
//! navigation router feeds it Up/Down/Confirm/Back; the mouse clicks rows
//! directly, and a click on the dimmed area closes it.

use crate::ui::css::*;
use crate::ui::state::SharedState;
use gtk4::prelude::*;
use gtk4::Widget;
use std::cell::{Cell, RefCell};

/// What the open menu does.
#[derive(Clone)]
pub(super) enum MenuKind {
    /// Toggle the focused game's membership of each group, or file it
    /// into a new one.
    Groups(Box<crate::Game>),
    /// Pick the Everything tab's game ordering.
    Sort,
    /// Pick the Groups tab's tile ordering.
    GroupOrder,
    /// Ask before a group is deleted; carries its id.
    ConfirmDelete { id: i64 },
}

/// One actionable row of the open menu.
#[derive(Clone)]
enum MenuRow {
    Group { id: i64 },
    NewGroup,
    Sort(ira_models::SortMode),
    GroupOrder(ira_models::GroupOrder),
    DeleteGroup { id: i64 },
    Cancel,
}

pub(super) struct GameMenu {
    /// The menu surface: dim layer with the panel floating on top.
    root: gtk4::Overlay,
    panel: gtk4::Box,
    kind: RefCell<Option<MenuKind>>,
    rows: RefCell<Vec<MenuRow>>,
    selection: Cell<usize>,
}

impl GameMenu {
    pub(super) fn new(state: &SharedState) -> Self {
        let dim = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        dim.add_css_class(CSS_BP_MENU_DIM);
        {
            let close_state = state.clone();
            let click = gtk4::GestureClick::new();
            click.connect_pressed(move |_, _, _, _| {
                if let Some(big) = close_state.borrow().big_picture.clone() {
                    big.game_menu.close();
                }
            });
            dim.add_controller(click);
        }
        let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        panel.add_css_class(CSS_BP_MENU_PANEL);
        // Docked full-height on the right, Switch-options style.
        panel.add_css_class(CSS_BP_OPTION_PANEL);
        panel.set_halign(gtk4::Align::End);
        panel.set_valign(gtk4::Align::Fill);
        panel.set_size_request(560, -1);

        let root = gtk4::Overlay::new();
        // The overlay is a sibling of the bp-root Box, so it must carry the
        // big-picture class itself to inherit its font.
        root.add_css_class(CSS_BP_ROOT);
        root.set_child(Some(&dim));
        root.add_overlay(&panel);
        root.set_visible(false);
        Self {
            root,
            panel,
            kind: RefCell::new(None),
            rows: RefCell::new(Vec::new()),
            selection: Cell::new(0),
        }
    }

    pub(super) fn root(&self) -> &Widget {
        self.root.upcast_ref()
    }

    pub(super) fn is_open(&self) -> bool {
        self.root.is_visible()
    }

    /// Open a menu flavor and lay out its rows.
    pub(super) fn open(&self, state: &SharedState, kind: MenuKind) {
        *self.kind.borrow_mut() = Some(kind);
        self.selection.set(0);
        self.rebuild(state);
        self.root.set_visible(true);
    }

    pub(super) fn close(&self) {
        self.root.set_visible(false);
        *self.kind.borrow_mut() = None;
    }

    pub(super) fn move_selection(&self, state: &SharedState, delta: i32) {
        let count = self.rows.borrow().len();
        if count == 0 {
            return;
        }
        let next = (self.selection.get() as i64 + delta as i64).clamp(0, count as i64 - 1) as usize;
        if next != self.selection.get() {
            self.selection.set(next);
            self.rebuild(state);
        }
    }

    /// Run the selected row's action. Group rows stay open so membership
    /// reads as a checklist; picking a sort closes.
    pub(super) fn activate(&self, state: &SharedState) {
        let action = self.rows.borrow().get(self.selection.get()).cloned();
        let Some(action) = action else {
            return;
        };
        let db = state.borrow().db.clone();
        match action {
            MenuRow::Group { id } => {
                let Some(MenuKind::Groups(game)) = self.kind.borrow().clone() else {
                    return;
                };
                let member = ira_db::get_groups_for_game(&db, game.db_id)
                    .unwrap_or_default()
                    .iter()
                    .any(|g| g.id == id);
                let result = if member {
                    ira_db::remove_game_from_group(&db, game.db_id, id)
                } else {
                    ira_db::add_game_to_group(&db, game.db_id, id)
                };
                if let Err(e) = result {
                    eprintln!("Failed to update group membership: {e}");
                }
                self.rebuild(state);
            }
            MenuRow::NewGroup => {
                let Some(MenuKind::Groups(game)) = self.kind.borrow().clone() else {
                    return;
                };
                self.close();
                super::view::name_new_group(state, Some(*game));
            }
            MenuRow::Sort(mode) => {
                state.borrow_mut().cfg.sort_mode = mode;
                if let Err(e) = state.borrow().cfg.save() {
                    eprintln!("Failed to save sort order: {e}");
                }
                if let Some(big) = state.borrow().big_picture.clone() {
                    big.all.update_ordering_label(state);
                    big.all.refresh(state);
                }
                self.close();
            }
            MenuRow::GroupOrder(order) => {
                state.borrow_mut().cfg.group_order = order;
                if let Err(e) = state.borrow().cfg.save() {
                    eprintln!("Failed to save group order: {e}");
                }
                if let Some(big) = state.borrow().big_picture.clone() {
                    big.all.update_ordering_label(state);
                    big.all.groups_grid.reload(state);
                }
                self.close();
            }
            MenuRow::DeleteGroup { id } => {
                if let Err(e) = ira_db::delete_group(&db, id) {
                    eprintln!("Failed to delete group: {e}");
                }
                if let Some(big) = state.borrow().big_picture.clone() {
                    big.all.group_deleted(state, id);
                }
                self.close();
            }
            MenuRow::Cancel => {
                self.close();
            }
        }
    }

    /// Rebuild the rows for the menu's flavor.
    fn rebuild(&self, state: &SharedState) {
        crate::ui::helpers::clear_children(&self.panel);
        let kind = self.kind.borrow().clone();
        let Some(kind) = kind else {
            return;
        };
        match kind {
            MenuKind::Groups(game) => {
                let (groups, db) = {
                    let s = state.borrow();
                    (s.groups.clone(), s.db.clone())
                };
                let member_of: Vec<i64> = ira_db::get_groups_for_game(&db, game.db_id)
                    .unwrap_or_default()
                    .iter()
                    .map(|g| g.id)
                    .collect();
                let header = gtk4::Label::new(Some(&game.name));
                header.set_xalign(0.0);
                header.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                header.add_css_class(CSS_BP_MENU_TITLE);
                crate::ui::helpers::crisp_label(&header);
                self.panel.append(&header);

                let mut rows = Vec::new();
                for group in &groups {
                    let member = member_of.contains(&group.id);
                    let index = rows.len();
                    self.append_row(state, &group.name, index, member);
                    rows.push(MenuRow::Group { id: group.id });
                }
                let index = rows.len();
                self.append_row(state, &crate::tr!("New Group…"), index, false);
                rows.push(MenuRow::NewGroup);
                *self.rows.borrow_mut() = rows;
            }
            MenuKind::Sort => {
                let current = state.borrow().cfg.sort_mode;
                let header = gtk4::Label::new(Some(&crate::tr!("Sort by")));
                header.set_xalign(0.0);
                header.add_css_class(CSS_BP_MENU_TITLE);
                crate::ui::helpers::crisp_label(&header);
                self.panel.append(&header);

                let mut rows = Vec::new();
                for mode in ira_models::SortMode::ALL {
                    let index = rows.len();
                    self.append_row(state, mode.display_label(), index, *mode == current);
                    rows.push(MenuRow::Sort(*mode));
                }
                *self.rows.borrow_mut() = rows;
            }
            MenuKind::GroupOrder => {
                let current = state.borrow().cfg.group_order;
                let header = gtk4::Label::new(Some(&crate::tr!("Order groups by")));
                header.set_xalign(0.0);
                header.add_css_class(CSS_BP_MENU_TITLE);
                crate::ui::helpers::crisp_label(&header);
                self.panel.append(&header);

                let mut rows = Vec::new();
                for order in ira_models::GroupOrder::ALL {
                    let index = rows.len();
                    self.append_row(state, order.display_label(), index, *order == current);
                    rows.push(MenuRow::GroupOrder(*order));
                }
                *self.rows.borrow_mut() = rows;
            }
            MenuKind::ConfirmDelete { id } => {
                let header = gtk4::Label::new(Some(&crate::tr!("Delete group")));
                header.set_xalign(0.0);
                header.add_css_class(CSS_BP_MENU_TITLE);
                crate::ui::helpers::crisp_label(&header);
                self.panel.append(&header);

                let mut rows = Vec::new();
                let index = rows.len();
                self.append_row(state, &crate::tr!("Delete"), index, false);
                rows.push(MenuRow::DeleteGroup { id });
                let index = rows.len();
                self.append_row(state, &crate::tr!("Cancel"), index, false);
                rows.push(MenuRow::Cancel);
                *self.rows.borrow_mut() = rows;
            }
        }
    }

    /// One row: the current sort and the groups the game belongs to wear
    /// the accent instead of a checkmark glyph.
    fn append_row(&self, state: &SharedState, text: &str, index: usize, active: bool) {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        row.add_css_class(CSS_BP_MENU_ROW);
        if active {
            row.add_css_class(CSS_BP_MENU_ROW_ACTIVE);
        }
        if self.selection.get() == index {
            row.add_css_class(CSS_BP_MENU_ROW_SELECTED);
        }
        let label = gtk4::Label::new(Some(text));
        label.set_xalign(0.0);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        crate::ui::helpers::crisp_label(&label);
        row.append(&label);
        let click_state = state.clone();
        let click = gtk4::GestureClick::new();
        click.connect_pressed(move |_, _, _, _| {
            // Clone out of the borrow: activate saves the config through a
            // mutable borrow, which would panic under the held one.
            let big = click_state.borrow().big_picture.clone();
            if let Some(big) = big {
                big.game_menu.selection.set(index);
                big.game_menu.activate(&click_state);
            }
        });
        row.add_controller(click);
        self.panel.append(&row);
    }
}
