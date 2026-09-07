//! The Groups tab of All Software: the library's groups as selectable
//! rows with game counts, plus a New Group action. Opening a group
//! filters the page's game grid to its members (the grid itself stays in
//! `all_games`; this module only owns the list).

use crate::ui::css::*;
use crate::ui::state::SharedState;
use gtk4::prelude::*;

use ira_models::Group;
use std::cell::{Cell, RefCell};

pub(super) struct GroupsUi {
    list: gtk4::ScrolledWindow,
    rows: gtk4::Box,
    selection: Cell<usize>,
    /// The groups the visible rows were built from, so the selection can
    /// resolve to a group without reading widgets back.
    groups: RefCell<Vec<Group>>,
}

impl GroupsUi {
    pub(super) fn widget(&self) -> &gtk4::ScrolledWindow {
        &self.list
    }

    pub(super) fn build(state: &SharedState) -> Self {
        let rows = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        rows.set_margin_top(8);
        rows.set_margin_bottom(16);
        rows.set_margin_start(28);
        rows.set_margin_end(28);
        let list = gtk4::ScrolledWindow::new();
        list.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        list.set_vexpand(true);
        list.set_child(Some(&rows));
        let ui = Self {
            list,
            rows,
            selection: Cell::new(0),
            groups: RefCell::new(Vec::new()),
        };
        ui.reload(state);
        ui
    }

    /// Rebuild the rows from the shared groups list: one row per group
    /// with its member count, then the New Group action.
    pub(super) fn reload(&self, state: &SharedState) {
        let (groups, db) = {
            let s = state.borrow();
            (s.groups.clone(), s.db.clone())
        };
        crate::ui::helpers::clear_children(&self.rows);
        for (index, group) in groups.iter().enumerate() {
            let count = ira_db::get_game_ids_in_group(&db, group.id)
                .map(|ids| ids.len())
                .unwrap_or(0);
            let row = group_row(group, count, state, Some(group.id));
            self.style_row(&row, index == self.selection.get());
            self.rows.append(&row);
        }
        *self.groups.borrow_mut() = groups.clone();
        self.clamp_selection();
        let new_row = group_row(
            &Group {
                id: -1,
                name: crate::tr!("New Group"),
            },
            usize::MAX,
            state,
            None,
        );
        let new_selected = self.selection.get() == groups.len();
        self.style_row(&new_row, new_selected);
        self.rows.append(&new_row);
    }

    fn style_row(&self, row: &gtk4::Box, selected: bool) {
        if selected {
            row.add_css_class(CSS_BP_GROUP_ROW_SELECTED);
        } else {
            row.remove_css_class(CSS_BP_GROUP_ROW_SELECTED);
        }
    }

    /// Restyle rows after a selection move without a rebuild.
    fn repaint_selection(&self) {
        let mut index = 0usize;
        let mut child = self.rows.first_child();
        while let Some(row) = child {
            self.style_row(row.downcast_ref::<gtk4::Box>().unwrap(), index == self.selection.get());
            child = row.next_sibling();
            index += 1;
        }
    }

    pub(super) fn move_selection(&self, delta: i32) {
        // Rows: one per group plus the New Group action.
        let count = self.groups.borrow().len() + 1;
        let next = (self.selection.get() as i64 + delta as i64).clamp(0, count as i64 - 1) as usize;
        if next != self.selection.get() {
            self.selection.set(next);
            self.repaint_selection();
        }
    }

    /// The group the selection rests on, or `None` on the New Group
    /// action.
    pub(super) fn selected_group(&self) -> Option<Group> {
        let groups = self.groups.borrow();
        groups.get(self.selection.get()).cloned()
    }

    /// Whether the selection rests on the New Group action.
    pub(super) fn selection_is_new_group(&self) -> bool {
        self.selection.get() == self.groups.borrow().len()
    }

    fn clamp_selection(&self) {
        let max = self.groups.borrow().len();
        if self.selection.get() > max {
            self.selection.set(max);
        }
    }
}

fn group_row(
    group: &Group,
    count: usize,
    state: &SharedState,
    open_id: Option<i64>,
) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    row.add_css_class(CSS_BP_GROUP_ROW);

    let name_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    let name = gtk4::Label::new(Some(&group.name));
    name.set_xalign(0.0);
    name.set_hexpand(true);
    name.add_css_class(CSS_BP_PAGE_TITLE);
    crate::ui::helpers::crisp_label(&name);
    name_box.append(&name);
    row.append(&name_box);

    if count != usize::MAX {
        let count_label = gtk4::Label::new(Some(&crate::tr!("{} games").replacen(
            "{}",
            &count.to_string(),
            1,
        )));
        count_label.set_valign(gtk4::Align::Center);
        count_label.add_css_class(CSS_BP_PAGE_SUBTITLE);
        crate::ui::helpers::crisp_label(&count_label);
        row.append(&count_label);
    }

    let icon = gtk4::Image::from_icon_name(if open_id.is_some() {
        "go-next-symbolic"
    } else {
        "list-add-symbolic"
    });
    icon.set_pixel_size(20);
    icon.set_valign(gtk4::Align::Center);
    row.append(&icon);

    unsafe {
        row.set_data::<Cell<i64>>("group-id", Cell::new(group.id));
    }
    let click_state = state.clone();
    let clicked_id = group.id;
    let click = gtk4::GestureClick::new();
    click.connect_pressed(move |_, _, _, _| {
        super::all_games::AllSoftwareUi::group_row_activated(&click_state, clicked_id);
    });
    row.add_controller(click);
    row
}
