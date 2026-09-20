//! Create or edit an auto group: a name, a root gate, and one card per
//! rule group, each editing its rules in place. Long value lists open
//! on the dialog's values page. Save rewrites the stored rule set;
//! membership follows on the next rebuild, because it is derived,
//! never stored.

use adw::prelude::*;
use ira_models::{AutoDimension, AutoGroup};
use std::rc::Rc;
use super::auto_group_editor::GroupsUi;
use super::auto_group_values_page::{ValueMenus, ValueOption, ValuesPage};
use super::state::SharedState;

const MAIN_PAGE: &str = "main";
const VALUES_PAGE: &str = "values";

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
    dialog.set_content_width(560);
    dialog.set_content_height(600);

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let back = gtk4::Button::from_icon_name("go-previous-symbolic");
    back.add_css_class(super::css::CSS_FLAT);
    back.set_visible(false);
    header.pack_start(&back);
    let title = gtk4::Label::new(Some(&crate::tr!("Auto group")));
    title.add_css_class("heading");
    header.set_title_widget(Some(&title));
    toolbar.add_top_bar(&header);

    // Two pages: the editor and the value picker. The header's back
    // button and the title travel with the switch.
    let stack = gtk4::Stack::new();
    stack.set_transition_type(gtk4::StackTransitionType::SlideLeft);
    stack.set_vhomogeneous(false);
    stack.set_vexpand(true);

    let values_page = Rc::new(ValuesPage::new());
    {
        let stack = stack.clone();
        let back_button = back.clone();
        let title = title.clone();
        let main_title = crate::tr!("Auto group");
        let values_title = crate::tr!("Pick values");
        values_page.set_navigation(Rc::new(move |on_values: bool| {
            stack.set_visible_child_name(if on_values {
                VALUES_PAGE
            } else {
                MAIN_PAGE
            });
            back_button.set_visible(on_values);
            title.set_text(if on_values { &values_title } else { &main_title });
        }));
        {
            let values_page = values_page.clone();
            back.connect_clicked(move |_| values_page.close());
        }
    }

    // The editor page: the name, the root gate, one card per rule
    // group — all inside a scrolled window, since a few groups already
    // outgrow the dialog.
    let main_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    main_page.set_margin_top(12);
    main_page.set_margin_bottom(12);
    main_page.set_margin_start(12);
    main_page.set_margin_end(12);

    let name_group = adw::PreferencesGroup::new();
    let name_row = adw::EntryRow::new();
    name_row.set_title(&crate::tr!("Name"));
    if let Some(group) = &existing {
        name_row.set_text(&group.name);
    }
    name_group.add(&name_row);
    main_page.append(&name_group);

    let menus = Rc::new(dimension_menus(state));
    let groups = GroupsUi::new(&menus, &values_page, existing.as_ref());
    main_page.append(&groups.root);

    let main_scroll = gtk4::ScrolledWindow::new();
    main_scroll.set_child(Some(&main_page));
    main_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    main_scroll.set_vexpand(true);

    stack.add_titled(&main_scroll, Some(MAIN_PAGE), "main");
    stack.add_named(&values_page.root(), Some(VALUES_PAGE));

    toolbar.set_content(Some(&stack));

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
        let (state, name_row, groups) = (state.clone(), name_row, groups);
        save.connect_clicked(move |_| {
            let name = name_row.text().trim().to_string();
            if name.is_empty() {
                return;
            }
            let root = groups.collect_root();
            let db = state.borrow().db.clone();
            match &existing {
                Some(group) => {
                    let db_id = super::auto_groups::to_db_id(group.id);
                    if let Err(e) = ira_db::update_auto_group(&db, db_id, &name, &root) {
                        eprintln!("Failed to update the auto group: {e}");
                        return;
                    }
                    let mut s = state.borrow_mut();
                    if let Some(stored) = s.auto_groups.iter_mut().find(|g| g.id == group.id) {
                        stored.name = name;
                        stored.root = root;
                    }
                }
                None => {
                    let db_id = match ira_db::create_auto_group(&db, &name, &root) {
                        Ok(id) => id,
                        Err(e) => {
                            eprintln!("Failed to create the auto group: {e}");
                            return;
                        }
                    };
                    state.borrow_mut().auto_groups.push(AutoGroup {
                        id: super::auto_groups::to_memory_id(db_id),
                        name,
                        root,
                    });
                }
            }
            dialog.close();
            super::sidebar::rebuild_sidebar(&state);
        });
    }

    dialog.present(Some(&window));
}


/// One value menu per dimension, fetched once per dialog: the pickers
/// offer what the library has on record (plus the console set, which is
/// closed). Consoles search on their hidden aliases too — "ps1" finds
/// "PlayStation 1" — while only the full name is stored or shown.
fn dimension_menus(state: &SharedState) -> ValueMenus {
    let db = state.borrow().db.clone();
    let load = |kind: &str, role: Option<&str>| {
        ira_db::distinct_entity_names(&db, kind, role).unwrap_or_default()
    };
    let mut menus = ValueMenus::new();
    menus.insert(
        AutoDimension::Genre,
        load(ira_db::KIND_GENRE, None)
            .into_iter()
            .map(ValueOption::plain)
            .collect(),
    );
    menus.insert(
        AutoDimension::Family,
        load(ira_db::KIND_FAMILY, None)
            .into_iter()
            .map(ValueOption::plain)
            .collect(),
    );
    menus.insert(
        AutoDimension::Developer,
        load(ira_db::KIND_COMPANY, Some("is_developer"))
            .into_iter()
            .map(ValueOption::plain)
            .collect(),
    );
    menus.insert(
        AutoDimension::Publisher,
        load(ira_db::KIND_COMPANY, Some("is_publisher"))
            .into_iter()
            .map(ValueOption::plain)
            .collect(),
    );
    menus.insert(
        AutoDimension::Console,
        ira_models::all_consoles()
            .map(|c| ValueOption {
                value: c.display_name.to_string(),
                search: c.search_haystack(),
            })
            .collect(),
    );
    menus
}
