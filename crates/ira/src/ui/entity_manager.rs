//! The scraper entity cache manager: browse one entity kind —
//! companies, genres or families — rename rows, teach aliases, and
//! merge duplicates. A rename latches the row against the source's
//! freshening; an alias teaches every future store that some incoming
//! name means a canonical entry; a merge moves all game references to
//! one survivor (the ScreenScraper id wins when there is exactly one)
//! and leaves the absorbed spelling behind as an alias, so the dedupe
//! holds against the next match. Every operation re-syncs the open
//! game settings' draft, whose staged copies would otherwise write the
//! pre-edit spellings right back.

use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use super::css::*;
use super::edit_game_scraper::{EntityField, EntityKind};
use super::helpers::{clear_children, status_row};
use super::state::SharedState;

/// The entity cache a picker's field browses: both studio roles share
/// the company table.
pub(super) fn managed_kind(field: &EntityField) -> &'static str {
    match field.kind {
        EntityKind::Developer | EntityKind::Publisher => ira_db::KIND_COMPANY,
        EntityKind::Genre => ira_db::KIND_GENRE,
        EntityKind::Family => ira_db::KIND_FAMILY,
    }
}

/// One entity's usage count as a subtitle — the number that makes a
/// duplicate worth merging.
fn usage_subtitle(usage: &std::collections::HashMap<i64, i64>, id: i64) -> String {
    match usage.get(&id) {
        Some(count) => crate::tr!("Used by {} games").replacen("{}", &count.to_string(), 1),
        None => crate::tr!("Unused"),
    }
}

/// Re-sync the open settings window's scraper rows from the database,
/// so its staged draft picks up renames, merges and aliases instead of
/// storing the pre-edit spellings back over them.
fn refresh_open_settings(state: &SharedState) {
    let db_id = state.borrow().settings_data.as_ref().map(|sd| sd.db_id);
    if let Some(db_id) = db_id {
        super::edit_game_scraper::refresh_scraper_section(state, db_id);
    }
}

/// Rebuild the alias rows for one entity, each with its remove button.
fn refresh_alias_list(state: &SharedState, kind: &'static str, id: i64, alias_list: &gtk4::Box) {
    clear_children(alias_list);
    let db = state.borrow().db.clone();
    let aliases = ira_db::entity_aliases(&db, kind, id).unwrap_or_default();
    if aliases.is_empty() {
        alias_list.append(&status_row(&crate::tr!("No aliases")));
        return;
    }
    for (alias_id, text) in aliases {
        let row = adw::ActionRow::new();
        row.set_use_markup(false);
        row.set_title(&text);
        let state = state.clone();
        let list_for_closure = alias_list.clone();
        let remove = gtk4::Button::from_icon_name("user-trash-symbolic");
        remove.add_css_class(CSS_FLAT);
        remove.set_valign(gtk4::Align::Center);
        remove.set_tooltip_text(Some(&crate::tr!("Remove")));
        remove.connect_clicked(move |_| {
            let db = state.borrow().db.clone();
            if let Err(e) = ira_db::remove_entity_alias(&db, alias_id) {
                eprintln!("Failed to remove the alias: {e}");
                return;
            }
            refresh_alias_list(&state, kind, id, &list_for_closure);
        });
        row.add_suffix(&remove);
        alias_list.append(&row);
    }
}

pub(super) fn show_entity_manager(
    state: &SharedState,
    parent: &impl IsA<gtk4::Widget>,
    kind: &'static str,
    title: &str,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(title);
    dialog.set_content_width(520);
    dialog.set_content_height(520);

    // ——— List page ———
    let list_toolbar = adw::ToolbarView::new();
    let list_header = adw::HeaderBar::new();
    let list_title = gtk4::Label::new(Some(title));
    list_title.add_css_class("heading");
    list_header.set_title_widget(Some(&list_title));
    list_toolbar.add_top_bar(&list_header);
    let search = gtk4::SearchEntry::new();
    search.set_placeholder_text(Some(&crate::tr!("Search…")));
    let (list_scrolled, list) = super::helpers::clamped_boxed_list(520);
    list_scrolled.set_vexpand(true);
    let list_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    list_box.set_margin_top(12);
    list_box.set_margin_bottom(12);
    list_box.set_margin_start(12);
    list_box.set_margin_end(12);
    list_box.append(&search);
    list_box.append(&list_scrolled);
    list_toolbar.set_content(Some(&list_box));

    // ——— Entity page: rename, aliases, merge ———
    let entity_toolbar = adw::ToolbarView::new();
    let entity_header = adw::HeaderBar::new();
    let back_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
    back_btn.add_css_class(CSS_FLAT);
    let entity_title = gtk4::Label::new(None);
    entity_title.add_css_class("heading");
    entity_header.set_title_widget(Some(&entity_title));
    entity_header.pack_start(&back_btn);
    entity_toolbar.add_top_bar(&entity_header);

    let entity_content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    entity_content.set_margin_top(12);
    entity_content.set_margin_bottom(12);
    entity_content.set_margin_start(12);
    entity_content.set_margin_end(12);
    let name_group = adw::PreferencesGroup::new();
    name_group.set_title(&crate::tr!("Name"));
    let name_entry = adw::EntryRow::new();
    name_entry.set_title(&crate::tr!("Rename"));
    name_entry.set_tooltip_text(Some(&crate::tr!(
        "A renamed entry keeps its name when the source answers with the old spelling"
    )));
    name_group.add(&name_entry);
    entity_content.append(&name_group);

    let alias_group = adw::PreferencesGroup::new();
    alias_group.set_title(&crate::tr!("Aliases"));
    alias_group.set_description(Some(&crate::tr!(
        "Incoming names on this list store as the entity above"
    )));
    let alias_list = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    alias_group.add(&alias_list);
    let alias_entry = adw::EntryRow::new();
    alias_entry.set_title(&crate::tr!("Add alias…"));
    alias_group.add(&alias_entry);
    entity_content.append(&alias_group);

    let merge_btn = gtk4::Button::with_label(&crate::tr!("Merge into…"));
    merge_btn.set_halign(gtk4::Align::Start);
    entity_content.append(&merge_btn);
    let entity_scrolled = gtk4::ScrolledWindow::new();
    entity_scrolled.set_vexpand(true);
    entity_scrolled.set_child(Some(&entity_content));
    entity_toolbar.set_content(Some(&entity_scrolled));

    // ——— Merge page: pick the entity to merge into ———
    let merge_toolbar = adw::ToolbarView::new();
    let merge_header = adw::HeaderBar::new();
    let merge_back = gtk4::Button::from_icon_name("go-previous-symbolic");
    merge_back.add_css_class(CSS_FLAT);
    let merge_page_title = gtk4::Label::new(Some(&crate::tr!("Merge into…")));
    merge_page_title.add_css_class("heading");
    merge_header.set_title_widget(Some(&merge_page_title));
    merge_header.pack_start(&merge_back);
    merge_toolbar.add_top_bar(&merge_header);
    let merge_search = gtk4::SearchEntry::new();
    merge_search.set_placeholder_text(Some(&crate::tr!("Search…")));
    let (merge_scrolled, merge_list) = super::helpers::clamped_boxed_list(520);
    merge_scrolled.set_vexpand(true);
    let merge_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    merge_box.set_margin_top(12);
    merge_box.set_margin_bottom(12);
    merge_box.set_margin_start(12);
    merge_box.set_margin_end(12);
    merge_box.append(&merge_search);
    merge_box.append(&merge_scrolled);
    merge_toolbar.set_content(Some(&merge_box));

    let stack = gtk4::Stack::new();
    stack.set_vhomogeneous(false);
    stack.add_named(&list_toolbar, Some("list"));
    stack.add_named(&entity_toolbar, Some("entity"));
    stack.add_named(&merge_toolbar, Some("merge"));
    dialog.set_child(Some(&stack));

    let state = Rc::new(state.clone());
    let selected: Rc<RefCell<Option<i64>>> = Default::default();

    // The list page: every cached entity, alphabetically, with its
    // usage. A row's pen button opens the entity page.
    let refresh_list = {
        let state = state.clone();
        let list = list.clone();
        let stack = stack.clone();
        let selected = selected.clone();
        let entity_title = entity_title.clone();
        let name_entry = name_entry.clone();
        let alias_list = alias_list.clone();
        move |filter: &str| {
            clear_children(&list);
            let db = state.borrow().db.clone();
            let usage = ira_db::entity_usage(&db, kind).unwrap_or_default();
            let rows = ira_db::list_entities(&db, kind, filter).unwrap_or_default();
            if rows.is_empty() {
                let note = if filter.is_empty() {
                    crate::tr!("Nothing in the cache yet — it fills as games get matched")
                } else {
                    crate::tr!("No results found")
                };
                list.append(&status_row(&note));
                return;
            }
            for (id, name) in rows {
                let row = adw::ActionRow::new();
                row.set_use_markup(false);
                row.set_title(&name);
                row.set_subtitle(&usage_subtitle(&usage, id));
                let state = state.clone();
                let stack = stack.clone();
                let selected = selected.clone();
                let entity_title = entity_title.clone();
                let name_entry = name_entry.clone();
                let alias_list = alias_list.clone();
                let title = name.clone();
                row.add_suffix(&pen_button(move |_| {
                    selected.replace(Some(id));
                    entity_title.set_text(&title);
                    name_entry.set_text(&title);
                    refresh_alias_list(&state, kind, id, &alias_list);
                    stack.set_visible_child_name("entity");
                }));
                list.append(&row);
            }
        }
    };

    // A merge target picked: the page's entity folds into the pick —
    // the db's survivor rule may crown the other row when only it has a
    // real id — and the manager returns to the refreshed list. The
    // merge page itself refills when it is entered, so nothing circular
    // is captured here.
    let on_merge_pick = {
        let state = state.clone();
        let selected = selected.clone();
        let stack = stack.clone();
        let refresh_list = refresh_list.clone();
        let search = search.clone();
        move |target: i64| {
            let Some(id) = *selected.borrow() else {
                return;
            };
            let db = state.borrow().db.clone();
            if let Err(e) = ira_db::merge_entities(&db, kind, target, id) {
                eprintln!("Failed to merge the entities: {e}");
                return;
            }
            refresh_open_settings(&state);
            selected.borrow_mut().take();
            search.set_text("");
            refresh_list("");
            stack.set_visible_child_name("list");
        }
    };

    // The merge page: every other entity, the pick's Merge button
    // folding this one into it.
    let refresh_merge_list = {
        let state = state.clone();
        let merge_list = merge_list.clone();
        let selected = selected.clone();
        move |filter: &str| {
            clear_children(&merge_list);
            let Some(id) = *selected.borrow() else {
                return;
            };
            let db = state.borrow().db.clone();
            let usage = ira_db::entity_usage(&db, kind).unwrap_or_default();
            let rows: Vec<_> = ira_db::list_entities(&db, kind, filter)
                .unwrap_or_default()
                .into_iter()
                .filter(|(other, _)| *other != id)
                .collect();
            if rows.is_empty() {
                merge_list.append(&status_row(&crate::tr!("No results found")));
                return;
            }
            for (other, name) in rows {
                let row = adw::ActionRow::new();
                row.set_use_markup(false);
                row.set_title(&name);
                row.set_subtitle(&usage_subtitle(&usage, other));
                let on_merge_pick = on_merge_pick.clone();
                let pick = gtk4::Button::with_label(&crate::tr!("Merge"));
                pick.add_css_class(CSS_SUGGESTED_ACTION);
                pick.set_valign(gtk4::Align::Center);
                pick.connect_clicked(move |_| on_merge_pick(other));
                row.add_suffix(&pick);
                merge_list.append(&row);
            }
        }
    };

    refresh_list("");
    {
        let refresh_list = refresh_list.clone();
        search.connect_search_changed(move |search| {
            refresh_list(&search.text());
        });
    }
    {
        let stack_for_back = stack.clone();
        back_btn.connect_clicked(move |_| stack_for_back.set_visible_child_name("list"));
        let stack_for_merge_back = stack.clone();
        let refresh_merge_list = refresh_merge_list.clone();
        merge_back.connect_clicked(move |_| {
            stack_for_merge_back.set_visible_child_name("entity");
            refresh_merge_list("");
        });
    }
    {
        // Rename on apply; the row latches in the db, and the page and
        // list repaint with the new spelling.
        let state = state.clone();
        let selected = selected.clone();
        let entity_title = entity_title.clone();
        let name_entry = name_entry.clone();
        let refresh_list = refresh_list.clone();
        let search = search.clone();
        name_entry.connect_apply(move |entry| {
            let Some(id) = *selected.borrow() else {
                return;
            };
            let name = entry.text().trim().to_string();
            if name.is_empty() {
                return;
            }
            let db = state.borrow().db.clone();
            if let Err(e) = ira_db::rename_entity(&db, kind, id, &name) {
                eprintln!("Failed to rename the entity: {e}");
                return;
            }
            entity_title.set_text(&name);
            refresh_open_settings(&state);
            refresh_list(&search.text());
        });
    }
    {
        // An alias declared on apply — a repeated alias re-targets, so
        // a mistake is fixed by declaring it again.
        let state = state.clone();
        let selected = selected.clone();
        let alias_entry = alias_entry.clone();
        let alias_list = alias_list.clone();
        alias_entry.connect_apply(move |entry| {
            let Some(id) = *selected.borrow() else {
                return;
            };
            let text = entry.text().trim().to_string();
            if text.is_empty() {
                return;
            }
            let db = state.borrow().db.clone();
            if let Err(e) = ira_db::add_entity_alias(&db, kind, &text, id) {
                eprintln!("Failed to add the alias: {e}");
                return;
            }
            entry.set_text("");
            refresh_alias_list(&state, kind, id, &alias_list);
        });
    }
    {
        let selected = selected.clone();
        let stack = stack.clone();
        let refresh_merge_list = refresh_merge_list.clone();
        merge_btn.connect_clicked(move |_| {
            if selected.borrow().is_none() {
                return;
            }
            merge_search.set_text("");
            refresh_merge_list("");
            stack.set_visible_child_name("merge");
        });
    }

    dialog.present(Some(parent));
}

/// The flat pen button opening an entity's management page.
fn pen_button(on_click: impl Fn(&gtk4::Button) + 'static) -> gtk4::Button {
    let button = gtk4::Button::from_icon_name("document-edit-symbolic");
    button.add_css_class(CSS_FLAT);
    button.set_valign(gtk4::Align::Center);
    button.set_tooltip_text(Some(&crate::tr!("Manage")));
    button.connect_clicked(on_click);
    button
}
