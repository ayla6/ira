//! The couch's per-game options menu: a centered panel over a dimmed page
//! that assigns the focused game to groups, creates groups, and cycles the
//! All Software ordering. While the menu is open the navigation router
//! feeds it Up/Down/Confirm/Back; the mouse clicks rows directly, and a
//! click on the dimmed area closes it.

use crate::ui::css::*;
use crate::ui::state::SharedState;
use gtk4::prelude::*;
use gtk4::Widget;
use ira_models::Group;
use std::cell::{Cell, RefCell};

/// One actionable row of the open menu.
#[derive(Clone)]
enum MenuRow {
    /// Toggle the focused game's membership of a group.
    Group { id: i64 },
    /// Create a fresh group and put the game in it.
    NewGroup,
    /// Advance the All Software sort order.
    Sort,
}

pub(super) struct GameMenu {
    /// The menu surface: dim layer with the panel floating on top.
    root: gtk4::Overlay,
    panel: gtk4::Box,
    rows: RefCell<Vec<MenuRow>>,
    selection: Cell<usize>,
    /// The game the menu was opened for.
    game: RefCell<Option<crate::Game>>,
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
        panel.set_halign(gtk4::Align::Center);
        panel.set_valign(gtk4::Align::Center);
        panel.set_size_request(620, -1);
        panel.set_margin_bottom(24);

        let root = gtk4::Overlay::new();
        root.set_child(Some(&dim));
        root.add_overlay(&panel);
        root.set_visible(false);
        Self {
            root,
            panel,
            rows: RefCell::new(Vec::new()),
            selection: Cell::new(0),
            game: RefCell::new(None),
        }
    }

    pub(super) fn root(&self) -> &Widget {
        self.root.upcast_ref()
    }

    pub(super) fn is_open(&self) -> bool {
        self.root.is_visible()
    }

    /// Open the menu for `game`: one checked row per group, then New
    /// Group, then the sort cycler.
    pub(super) fn open(&self, state: &SharedState, game: &crate::Game) {
        *self.game.borrow_mut() = Some(game.clone());
        self.selection.set(0);
        self.rebuild(state);
        self.root.set_visible(true);
    }

    pub(super) fn close(&self) {
        self.root.set_visible(false);
        *self.game.borrow_mut() = None;
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

    /// Run the selected row's action and keep the menu open (rows read as
    /// toggles); without a game the menu just closes.
    pub(super) fn activate(&self, state: &SharedState) {
        let action = {
            let rows = self.rows.borrow();
            rows.get(self.selection.get()).cloned()
        };
        let Some(action) = action else {
            return;
        };
        let Some(game) = self.game.borrow().clone() else {
            self.close();
            return;
        };
        let db = state.borrow().db.clone();
        match action {
            MenuRow::Group { id } => {
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
            }
            MenuRow::NewGroup => {
                let existing: Vec<String> = state
                    .borrow()
                    .groups
                    .iter()
                    .map(|g| g.name.clone())
                    .collect();
                let mut n = 1;
                while existing.iter().any(|name| name == &format!("Group {n}")) {
                    n += 1;
                }
                match ira_db::create_group(&db, &format!("Group {n}")) {
                    Ok(id) => {
                        let result = ira_db::add_game_to_group(&db, game.db_id, id);
                        if let Err(e) = result {
                            eprintln!("Failed to add game to group: {e}");
                        }
                        let group = Group { id, name: format!("Group {n}") };
                        state.borrow_mut().groups.push(group);
                    }
                    Err(e) => eprintln!("Failed to create group: {e}"),
                }
            }
            MenuRow::Sort => super::all_games::cycle_sort(state),
        }
        self.rebuild(state);
    }

    /// Rebuild the rows for the current game: groups (checked when the
    /// game is a member), New Group, and the sort cycler.
    fn rebuild(&self, state: &SharedState) {
        let Some(game) = self.game.borrow().clone() else {
            return;
        };
        let (groups, db) = {
            let s = state.borrow();
            (s.groups.clone(), s.db.clone())
        };
        let member_of: Vec<i64> = ira_db::get_groups_for_game(&db, game.db_id)
            .unwrap_or_default()
            .iter()
            .map(|g| g.id)
            .collect();
        crate::ui::helpers::clear_children(&self.panel);

        let header = gtk4::Label::new(Some(&game.name));
        header.set_xalign(0.0);
        header.add_css_class(CSS_BP_PAGE_TITLE);
        crate::ui::helpers::crisp_label(&header);
        self.panel.append(&header);

        let mut rows = Vec::new();
        for group in &groups {
            let check = if member_of.contains(&group.id) { "✓" } else { "·" };
            let index = rows.len();
            self.append_row(state, &format!("{check}  {}", group.name), index);
            rows.push(MenuRow::Group { id: group.id });
        }
        let index = rows.len();
        self.append_row(state, &crate::tr!("New Group"), index);
        rows.push(MenuRow::NewGroup);
        let (mode, descending) = {
            let s = state.borrow();
            (s.cfg.sort_mode, s.cfg.sort_descending)
        };
        let label = format!(
            "{}{}",
            crate::tr!("Sort: {}").replacen("{}", mode.display_label(), 1),
            if descending { " ↓" } else { "" }
        );
        let index = rows.len();
        self.append_row(state, &label, index);
        rows.push(MenuRow::Sort);
        *self.rows.borrow_mut() = rows;
    }

    fn append_row(&self, state: &SharedState, text: &str, index: usize) {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        row.add_css_class(CSS_BP_MENU_ROW);
        if self.selection.get() == index {
            row.add_css_class(CSS_BP_MENU_ROW_SELECTED);
        }
        let label = gtk4::Label::new(Some(text));
        label.set_xalign(0.0);
        crate::ui::helpers::crisp_label(&label);
        row.append(&label);
        let click_state = state.clone();
        let click = gtk4::GestureClick::new();
        click.connect_pressed(move |_, _, _, _| {
            if let Some(big) = click_state.borrow().big_picture.clone() {
                big.game_menu.selection.set(index);
                big.game_menu.activate(&click_state);
            }
        });
        row.add_controller(click);
        self.panel.append(&row);
    }
}
