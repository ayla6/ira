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

    // Stacked navigation: the editor is the root, the value picker
    // pushes on top of it. The navigation view owns the back button,
    // the swipe-back gesture and Escape — the picker can never be
    // swiped into unopened.
    let nav = adw::NavigationView::new();
    dialog.set_child(Some(&nav));

    let main_toolbar = adw::ToolbarView::new();
    let main_header = adw::HeaderBar::new();
    let main_title = gtk4::Label::new(Some(&crate::tr!("Auto group")));
    main_title.add_css_class("heading");
    main_header.set_title_widget(Some(&main_title));
    main_toolbar.add_top_bar(&main_header);

    let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    buttons.set_halign(gtk4::Align::End);
    buttons.set_margin_top(6);
    buttons.set_margin_bottom(6);
    buttons.set_margin_start(12);
    buttons.set_margin_end(12);

    let values_page = Rc::new(ValuesPage::new());
    let values_toolbar = adw::ToolbarView::new();
    let values_header = adw::HeaderBar::new();
    let values_title = gtk4::Label::new(Some(&crate::tr!("Pick values")));
    values_title.add_css_class("heading");
    values_header.set_title_widget(Some(&values_title));
    values_toolbar.add_top_bar(&values_header);
    values_toolbar.set_content(Some(&values_page.root()));
    let values_nav_page = adw::NavigationPage::new(&values_toolbar, "values");
    {
        let nav = nav.clone();
        let values_nav_page = values_nav_page.clone();
        // Cancel/Save belong to the editor page alone now, so the
        // switch only pushes and pops — pushing an already-open
        // picker would stack it twice, so a repeated open is a no-op.
        values_page.set_navigation(Rc::new(move |on_values: bool| {
            if on_values {
                if values_nav_page.parent().is_none() {
                    nav.push(&values_nav_page);
                }
            } else {
                nav.pop_to_tag("main");
            }
        }));
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
    let groups = GroupsUi::new(&menus, &values_page, existing.as_ref(), state);
    main_page.append(&groups.root);

    let main_scroll = gtk4::ScrolledWindow::new();
    main_scroll.set_child(Some(&main_page));
    main_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    main_scroll.set_vexpand(true);

    main_toolbar.set_content(Some(&main_scroll));
    main_toolbar.add_bottom_bar(&buttons);
    nav.add(&adw::NavigationPage::new(&main_toolbar, "main"));

    let cancel = gtk4::Button::with_label(&crate::tr!("Cancel"));
    cancel.add_css_class(super::css::CSS_FLAT);
    let save = gtk4::Button::with_label(&crate::tr!("Save"));
    save.add_css_class(super::css::CSS_SUGGESTED_ACTION);
    buttons.append(&cancel);
    buttons.append(&save);
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
/// closed, plus the PC platforms, which are not consoles but group like
/// them). Consoles search on their hidden aliases too — "ps1" finds
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
            // PC platforms are not consoles — the models console table
            // drives ROM scanning and emulator settings and must stay
            // console-only — but the Console matcher falls back to the
            // friendly platform name, so offering these values just
            // works. The match folds case, hence the display
            // capitalisation; "Windows" (not "Wine") because the
            // platform id `wine` shows as Windows. Steam (Emulated) is
            // Goldberg games, matched on the GSE trophy source instead
            // of any platform.
            .chain(
                [
                    "Linux",
                    "Windows",
                    "Steam",
                    "GOG",
                    ira_models::STEAM_EMULATED_VALUE,
                ]
                .into_iter()
                .map(|name| ValueOption::plain(name.to_string())),
            )
            .collect(),
    );
    menus
}
