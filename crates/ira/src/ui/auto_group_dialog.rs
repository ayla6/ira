//! Create or edit an auto group: a name and a stack of rule rows. Each
//! row picks a dimension — genre, series, studio, console, release
//! window, playtime bounds — and the control for that dimension's
//! values. Rules AND together; the values inside one rule are any-of.
//! Save rewrites the stored rule set; membership follows on the next
//! rebuild, because it is derived, never stored.

use adw::prelude::*;
use ira_models::{AutoCriterion, AutoDimension, AutoGroup};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use super::state::SharedState;

pub(super) fn show_auto_group_create(state: &SharedState) {
    show(state, None);
}

pub(super) fn show_auto_group_edit(state: &SharedState, memory_id: i64) {
    match super::auto_groups::find_auto_group(state, memory_id) {
        Some(group) => show(state, Some(group)),
        None => eprintln!("Auto group {memory_id} vanished before its editor opened"),
    }
}

fn show(state: &SharedState, existing: Option<AutoGroup>) {
    let window = state.borrow().window.clone();

    let dialog = adw::Dialog::new();
    dialog.set_title(&crate::tr!("Auto group"));
    dialog.set_content_width(540);
    dialog.set_content_height(560);

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let title = gtk4::Label::new(Some(&crate::tr!("Auto group")));
    title.add_css_class("heading");
    header.set_title_widget(Some(&title));
    toolbar.add_top_bar(&header);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let name_row = adw::EntryRow::new();
    name_row.set_title(&crate::tr!("Name"));
    if let Some(group) = &existing {
        name_row.set_text(&group.name);
    }
    content.append(&name_row);

    let rules_heading = gtk4::Label::new(Some(&crate::tr!("Rules")));
    rules_heading.set_xalign(0.0);
    rules_heading.add_css_class(super::css::CSS_DIM_LABEL);
    content.append(&rules_heading);

    let rules_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    content.append(&rules_box);

    // One value menu per dimension, fetched once per dialog: the pickers
    // offer what the library actually has on record (plus the console
    // set, which is closed).
    let menus = Rc::new(dimension_menus(state));

    let editors: Rc<RefCell<Vec<CriterionEditor>>> = Rc::new(RefCell::new(Vec::new()));
    if let Some(group) = &existing {
        for criterion in &group.criteria {
            let editor = CriterionEditor::new(&menus, Some(criterion));
            rules_box.append(&editor.root);
            editors.borrow_mut().push(editor);
        }
    }
    {
        let editors = editors.clone();
        let rules_box = rules_box.clone();
        let menus = menus.clone();
        let add = gtk4::Button::with_label(&crate::tr!("Add rule"));
        add.add_css_class(super::css::CSS_FLAT);
        add.set_halign(gtk4::Align::Start);
        add.connect_clicked(move |_| {
            let editor = CriterionEditor::new(&menus, None);
            rules_box.append(&editor.root);
            editors.borrow_mut().push(editor);
        });
        content.append(&add);
    }

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_child(Some(&content));
    scrolled.set_vexpand(true);
    toolbar.set_content(Some(&scrolled));

    let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    buttons.set_halign(gtk4::Align::End);
    buttons.set_margin_top(6);
    buttons.set_margin_bottom(6);
    buttons.set_margin_start(12);
    buttons.set_margin_end(12);
    let cancel = gtk4::Button::with_label(&crate::tr!("Cancel"));
    cancel.add_css_class(super::css::CSS_FLAT);
    let save = gtk4::Button::with_label(&crate::tr!("Save"));
    save.add_css_class(super::css::CSS_SUGGESTED_ACTION);
    buttons.append(&cancel);
    buttons.append(&save);
    toolbar.add_bottom_bar(&buttons);

    dialog.set_child(Some(&toolbar));
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }

    {
        let dialog = dialog.clone();
        let (state, editors, name_row) = (state.clone(), editors, name_row.clone());
        save.connect_clicked(move |_| {
            let name = name_row.text().trim().to_string();
            if name.is_empty() {
                return;
            }
            let criteria: Vec<AutoCriterion> =
                editors.borrow().iter().map(|e| e.collect()).collect();
            let db = state.borrow().db.clone();
            match &existing {
                Some(group) => {
                    let db_id = super::auto_groups::to_db_id(group.id);
                    if let Err(e) = ira_db::update_auto_group(&db, db_id, &name, &criteria) {
                        eprintln!("Failed to update the auto group: {e}");
                        return;
                    }
                    let mut s = state.borrow_mut();
                    if let Some(stored) = s.auto_groups.iter_mut().find(|g| g.id == group.id) {
                        stored.name = name;
                        stored.criteria = criteria;
                    }
                }
                None => {
                    let db_id = match ira_db::create_auto_group(&db, &name, &criteria) {
                        Ok(id) => id,
                        Err(e) => {
                            eprintln!("Failed to create the auto group: {e}");
                            return;
                        }
                    };
                    state.borrow_mut().auto_groups.push(AutoGroup {
                        id: super::auto_groups::to_memory_id(db_id),
                        name,
                        criteria,
                    });
                }
            }
            dialog.close();
            super::sidebar::rebuild_sidebar(&state);
        });
    }

    dialog.present(Some(&window));
}

/// The value menu for each value dimension: the names the cache has on
/// record, plus the console set, which is closed.
fn dimension_menus(state: &SharedState) -> HashMap<AutoDimension, Vec<String>> {
    let db = state.borrow().db.clone();
    let mut menus = HashMap::new();
    let load = |kind: &str, role: Option<&str>| {
        ira_db::distinct_entity_names(&db, kind, role).unwrap_or_default()
    };
    menus.insert(AutoDimension::Genre, load(ira_db::KIND_GENRE, None));
    menus.insert(AutoDimension::Family, load(ira_db::KIND_FAMILY, None));
    menus.insert(
        AutoDimension::Developer,
        load(ira_db::KIND_COMPANY, Some("is_developer")),
    );
    menus.insert(
        AutoDimension::Publisher,
        load(ira_db::KIND_COMPANY, Some("is_publisher")),
    );
    menus.insert(
        AutoDimension::Console,
        ira_models::all_consoles()
            .map(|c| c.display_name.to_string())
            .collect(),
    );
    menus
}

/// One rule row: the dimension picker on top, that dimension's control
/// underneath.
struct CriterionEditor {
    root: gtk4::Box,
    dropdown: gtk4::DropDown,
    /// The check rows of the current value dimension, name in list order.
    value_checks: Rc<RefCell<Vec<(String, gtk4::CheckButton)>>>,
    from_pick: Rc<super::date_pick::DatePick>,
    to_pick: Rc<super::date_pick::DatePick>,
    min_hours: gtk4::SpinButton,
    max_hours: gtk4::SpinButton,
}

impl CriterionEditor {
    fn new(
        menus: &Rc<HashMap<AutoDimension, Vec<String>>>,
        initial: Option<&AutoCriterion>,
    ) -> Self {
        let initial = initial.cloned().unwrap_or_default();
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        root.add_css_class(super::css::CSS_BOXED_LIST);
        root.set_margin_top(6);
        root.set_margin_bottom(6);

        let top = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let dimensions = AutoDimension::ALL;
        let model = gtk4::StringList::new(&[]);
        for dimension in dimensions {
            model.append(dimension.display_label());
        }
        let dropdown = gtk4::DropDown::new(Some(model), None::<gtk4::Expression>);
        dropdown.set_hexpand(true);
        if let Some(position) = dimensions.iter().position(|d| *d == initial.dimension) {
            dropdown.set_selected(position as u32);
        }
        top.append(&dropdown);
        root.append(&top);

        let value_checks: Rc<RefCell<Vec<(String, gtk4::CheckButton)>>> =
            Rc::new(RefCell::new(Vec::new()));
        let from_pick = Rc::new(super::date_pick::DatePick::new(&initial.from));
        from_pick.set_tooltip(&crate::tr!("From"));
        let to_pick = Rc::new(super::date_pick::DatePick::new(&initial.to));
        to_pick.set_tooltip(&crate::tr!("To"));
        let min_hours = hours_spin(initial.min_hours);
        min_hours.set_tooltip_text(Some(&crate::tr!("Played at least this many hours; 0 is no floor")));
        let max_hours = hours_spin(initial.max_hours);
        max_hours.set_tooltip_text(Some(&crate::tr!("Played at most this many hours; 0 is no ceiling")));

        // The value picker: a popover of check buttons over the
        // dimension's names; the button label counts the picked ones.
        let value_label = gtk4::Label::new(Some(&crate::tr!("Pick values")));
        let list = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        let value_button = {
            let button = gtk4::MenuButton::new();
            button.set_child(Some(&value_label));
            button.add_css_class(super::css::CSS_FLAT);
            let scrolled = gtk4::ScrolledWindow::new();
            scrolled.set_child(Some(&list));
            scrolled.set_max_content_height(320);
            scrolled.set_propagate_natural_height(true);
            let popover_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            popover_box.set_margin_top(6);
            popover_box.set_margin_bottom(6);
            popover_box.set_margin_start(6);
            popover_box.set_margin_end(6);
            popover_box.append(&scrolled);
            let popover = gtk4::Popover::new();
            popover.set_child(Some(&popover_box));
            button.set_popover(Some(&popover));
            button
        };

        let set_label = {
            let value_checks = value_checks.clone();
            let value_label = value_label.clone();
            move || {
                let picked = value_checks
                    .borrow()
                    .iter()
                    .filter(|(_, check)| check.is_active())
                    .count();
                value_label.set_text(&if picked == 0 {
                    crate::tr!("Pick values").to_string()
                } else {
                    crate::tr!("{} picked").replacen("{}", &picked.to_string(), 1)
                });
            }
        };
        let fill_values = {
            let value_checks = value_checks.clone();
            let list = list.clone();
            let set_label = set_label.clone();
            let menus = menus.clone();
            move |dimension: AutoDimension| {
                let names = menus.get(&dimension).cloned().unwrap_or_default();
                let picked: HashSet<String> = value_checks
                    .borrow()
                    .iter()
                    .filter(|(_, check)| check.is_active())
                    .map(|(name, _)| name.clone())
                    .collect();
                value_checks.borrow_mut().clear();
                for widget in list_children(&list) {
                    list.remove(&widget);
                }
                for name in &names {
                    let check = gtk4::CheckButton::with_label(name);
                    check.set_active(picked.contains(name));
                    let recount = set_label.clone();
                    check.connect_toggled(move |_| recount());
                    list.append(&check);
                    value_checks.borrow_mut().push((name.clone(), check));
                }
                if names.is_empty() {
                    let empty = gtk4::Label::new(Some(&crate::tr!(
                        "Nothing on record for this rule yet"
                    )));
                    empty.add_css_class(super::css::CSS_DIM_LABEL);
                    list.append(&empty);
                }
                set_label();
            }
        };

        let stack = gtk4::Stack::new();
        stack.add_named(&value_button, Some("values"));
        let dates = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        dates.append(from_pick.button());
        dates.append(to_pick.button());
        stack.add_named(&dates, Some("released"));
        let hours = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        hours.append(&min_hours);
        hours.append(&max_hours);
        stack.add_named(&hours, Some("playtime"));
        root.append(&stack);

        {
            let stack = stack.clone();
            let fill_values = fill_values.clone();
            dropdown.connect_selected_notify(move |dropdown| {
                let dimension = AutoDimension::ALL
                    .get(dropdown.selected() as usize)
                    .copied()
                    .unwrap_or_default();
                match dimension {
                    AutoDimension::Released => stack.set_visible_child(&dates),
                    AutoDimension::Playtime => stack.set_visible_child(&hours),
                    _ => {
                        fill_values(dimension);
                        stack.set_visible_child(&value_button);
                    }
                }
            });
        }
        // Fire once so the initial dimension's control is the visible one.
        dropdown.emit_by_name::<()>("selection-notify", &[]);

        Self {
            root,
            dropdown,
            value_checks,
            from_pick,
            to_pick,
            min_hours,
            max_hours,
        }
    }

    fn collect(&self) -> AutoCriterion {
        let dimension = AutoDimension::ALL
            .get(self.dropdown.selected() as usize)
            .copied()
            .unwrap_or_default();
        let mut criterion = AutoCriterion {
            dimension,
            ..Default::default()
        };
        match dimension {
            AutoDimension::Released => {
                criterion.from = self.from_pick.typed().unwrap_or_default();
                criterion.to = self.to_pick.typed().unwrap_or_default();
            }
            AutoDimension::Playtime => {
                let min = self.min_hours.value();
                let max = self.max_hours.value();
                criterion.min_hours = (min > 0.0).then_some(min);
                criterion.max_hours = (max > 0.0).then_some(max);
            }
            _ => {
                criterion.values = self
                    .value_checks
                    .borrow()
                    .iter()
                    .filter(|(_, check)| check.is_active())
                    .map(|(name, _)| name.clone())
                    .collect();
            }
        }
        criterion
    }
}

fn hours_spin(initial: Option<f64>) -> gtk4::SpinButton {
    let spin = gtk4::SpinButton::with_range(0.0, 9999.0, 0.5);
    spin.set_value(initial.unwrap_or(0.0));
    spin.set_digits(1);
    spin
}

fn list_children(list: &gtk4::Box) -> Vec<gtk4::Widget> {
    let mut out = Vec::new();
    let mut child = list.first_child();
    while let Some(widget) = child {
        let next = widget.next_sibling();
        out.push(widget);
        child = next;
    }
    out
}
