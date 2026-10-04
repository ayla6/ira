//! The entity dialog behind the Identity page's company, genre and
//! family rows: one window, five views. The root page lists the game's
//! own entities; the header's + slides in the add-search, its pencil
//! slides in the cache manager — rename, aliases, merge — and every
//! row opens the entity it belongs to directly. Following the HIG: a
//! single header bar whose controls follow the visible page, boxed
//! list rows with at most two controls, link rows carrying a
//! go-next arrow, and a visible apply button on the alias entry.
//!
//! The game's own list stages onto the metadata draft as everywhere
//! else on these pages; the manager's rename and alias edits stage
//! behind Save, so a stray click never writes the shared cache.

use adw::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The merge-list repainter, shared out so rows can refresh the list
/// after they mutate it.
type MergeRepaint = Rc<RefCell<Option<Rc<dyn Fn(&str)>>>>;

use super::css::*;
use super::edit_game_scraper::{edit_field, refresh_rows, EntityField, EntityKind, ScraperSlot};
use super::helpers::{clear_children, status_row};
use super::state::SharedState;
use crate::Game;
use ira_models::ScraperEntity;

/// One alias as the entity page stages it: `rowid` is `Some` for an
/// alias already in the database, `None` for one added since.
#[derive(Clone)]
struct AliasDraft {
    rowid: Option<i64>,
    text: String,
}

/// The staged edits of the entity page — what Save commits and Discard
/// throws away.
#[derive(Default, Clone)]
struct EntityEdits {
    /// The name as typed. Empty counts as "nothing typed" and refuses
    /// to save.
    name: String,
    /// The name the database had when the page loaded.
    original_name: String,
    aliases: Vec<AliasDraft>,
    /// The alias rowids the database had when the page loaded — a
    /// staged list missing one of these means a removal to commit.
    original_rowids: Vec<i64>,
}

/// The cache a field browses: both studio roles share the company
/// table.
fn managed_kind(field: &EntityField) -> &'static str {
    match field.kind {
        EntityKind::Developer | EntityKind::Publisher => ira_db::KIND_COMPANY,
        EntityKind::Genre => ira_db::KIND_GENRE,
        EntityKind::Family => ira_db::KIND_FAMILY,
    }
}

/// Load an entity's stored state into the page's staging area.
fn load_edits(db: &ira_db::DbConn, kind: &'static str, id: i64) -> EntityEdits {
    let name = ira_db::list_entities(db, kind, "")
        .unwrap_or_default()
        .into_iter()
        .find(|(row_id, _)| *row_id == id)
        .map(|(_, name)| name)
        .unwrap_or_default();
    let aliases: Vec<AliasDraft> = ira_db::entity_aliases(db, kind, id)
        .unwrap_or_default()
        .into_iter()
        .map(|(rowid, text)| AliasDraft {
            rowid: Some(rowid),
            text,
        })
        .collect();
    let original_rowids = aliases.iter().filter_map(|draft| draft.rowid).collect();
    EntityEdits {
        name: name.clone(),
        original_name: name,
        aliases,
        original_rowids,
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

/// An entity page opener: loads the entity's stored state, then
/// navigates to its page.
type OpenEntity = Rc<dyn Fn(i64, &str)>;

/// The root list's rebuilder, shared with its own row handlers through
/// a cell — it re-runs itself after every staged change.
type RootRebuild = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

/// Aftermath callbacks for the shared entity editor: the game dialog
/// refreshes its rows and travels; the standalone dialog just closes.
pub(super) struct EntityEditorActions {
    pub saved: Rc<dyn Fn()>,
    pub cancelled: Rc<dyn Fn()>,
    pub merge_open: Rc<dyn Fn()>,
    pub merge_picked: Rc<dyn Fn()>,
}

pub(super) struct EntityEditor {
    /// The Name/Aliases/Merge-it groups.
    pub content: gtk4::Box,
    /// The Cancel/Save row.
    pub actions: gtk4::Box,
    /// The merge-target picker content.
    pub merge: gtk4::Box,
    /// (Re)load an entity into staging, returning its stored name.
    pub show: Rc<dyn Fn(i64) -> String>,
}

/// The entity edit UI shared by the game dialog and the standalone
/// editor: rename, aliases, merge-target picking, Save/Cancel. All
/// writes hit the shared cache directly, so no game context is needed.
/// Hosts wire page travel and refresh through the callbacks.
pub(super) fn build_entity_editor(
    state: &SharedState,
    kind: &'static str,
    actions: EntityEditorActions,
) -> EntityEditor {
    let selected: Rc<RefCell<Option<i64>>> = Default::default();
    let edits: Rc<RefCell<EntityEdits>> = Default::default();

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    let name_group = adw::PreferencesGroup::new();
    name_group.set_title(&crate::tr!("Name"));
    let name_entry = adw::EntryRow::new();
    name_entry.set_title(&crate::tr!("Rename"));
    name_entry.set_tooltip_text(Some(&crate::tr!(
        "Your rename wins over the source's spelling"
    )));
    name_group.add(&name_entry);
    content.append(&name_group);
    // The id rides its own card: fused onto the rename row it reads as
    // one control doing two jobs.
    let id_group = adw::PreferencesGroup::new();
    let id_row = adw::ActionRow::new();
    id_row.set_title(&crate::tr!("ID"));
    id_row.set_sensitive(false);
    id_group.add(&id_row);
    content.append(&id_group);

    let alias_group = adw::PreferencesGroup::new();
    alias_group.set_title(&crate::tr!("Aliases"));
    alias_group.set_description(Some(&crate::tr!(
        "Matching answers that use these names will use this entry"
    )));
    let alias_list = gtk4::ListBox::new();
    alias_list.set_selection_mode(gtk4::SelectionMode::None);
    alias_list.add_css_class(CSS_BOXED_LIST);
    alias_group.add(&alias_list);
    let alias_entry = adw::EntryRow::new();
    alias_entry.set_title(&crate::tr!("Add alias…"));
    let alias_add = gtk4::Button::with_label(&crate::tr!("Add"));
    alias_add.add_css_class(CSS_SUGGESTED_ACTION);
    alias_add.set_valign(gtk4::Align::Center);
    alias_add.set_sensitive(false);
    alias_entry.add_suffix(&alias_add);
    alias_group.add(&alias_entry);
    content.append(&alias_group);

    let merge_group = adw::PreferencesGroup::new();
    let merge_row = adw::ActionRow::new();
    merge_row.set_activatable(true);
    merge_row.set_title(&crate::tr!("Merge into…"));
    merge_row.set_subtitle(&crate::tr!("Combine this entry with another one"));
    merge_row.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
    merge_group.add(&merge_row);
    content.append(&merge_group);

    let merge_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    merge_box.set_margin_top(12);
    merge_box.set_margin_bottom(12);
    merge_box.set_margin_start(12);
    merge_box.set_margin_end(12);
    let merge_search = gtk4::SearchEntry::new();
    merge_search.set_placeholder_text(Some(&crate::tr!("Search…")));
    merge_box.append(&merge_search);
    let (merge_scrolled, merge_list) = super::helpers::clamped_boxed_list(520);
    merge_scrolled.set_vexpand(true);
    merge_box.append(&merge_scrolled);

    let button_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    button_row.set_halign(gtk4::Align::End);
    button_row.set_margin_top(6);
    button_row.set_margin_bottom(12);
    button_row.set_margin_start(12);
    button_row.set_margin_end(12);
    let cancel_btn = gtk4::Button::with_label(&crate::tr!("Cancel"));
    cancel_btn.add_css_class(CSS_FLAT);
    let save_btn = gtk4::Button::with_label(&crate::tr!("Save"));
    save_btn.add_css_class(CSS_SUGGESTED_ACTION);
    button_row.append(&cancel_btn);
    button_row.append(&save_btn);

    let show = {
        let state = state.clone();
        let selected = selected.clone();
        let edits = edits.clone();
        let name_entry = name_entry.clone();
        let id_row = id_row.clone();
        let alias_list = alias_list.clone();
        Rc::new(move |id: i64| {
            let db = state.borrow().db.clone();
            let loaded = load_edits(&db, kind, id);
            name_entry.set_text(&loaded.name);
            id_row.set_subtitle(&id.to_string());
            name_entry.remove_css_class(CSS_ERROR);
            let name = loaded.name.clone();
            *edits.borrow_mut() = loaded;
            repaint_alias_list(&alias_list, &edits);
            selected.replace(Some(id));
            name
        })
    };
    {
        let edits = edits.clone();
        let name_entry = name_entry.clone();
        name_entry.connect_changed(move |entry| {
            edits.borrow_mut().name = entry.text().trim().to_string();
        });
    }
    {
        // An alias joins the staged list from the Add button or the
        // enter key; Save is what teaches it to the database. A blank
        // or already-staged spelling keeps the button dead instead of
        // silently ignoring the click.
        fn stage(
            alias_entry: &adw::EntryRow,
            edits: &Rc<RefCell<EntityEdits>>,
            alias_list: &gtk4::ListBox,
        ) {
            let text = alias_entry.text().trim().to_string();
            if text.is_empty() {
                return;
            }
            let mut staged = edits.borrow_mut();
            if staged
                .aliases
                .iter()
                .any(|draft| draft.text.eq_ignore_ascii_case(&text))
            {
                alias_entry.set_text("");
                return;
            }
            staged.aliases.push(AliasDraft {
                rowid: None,
                text: text.clone(),
            });
            drop(staged);
            alias_entry.set_text("");
            repaint_alias_list(alias_list, edits);
        }
        let stage_alias = Rc::new({
            let alias_entry = alias_entry.clone();
            let edits = edits.clone();
            let alias_list = alias_list.clone();
            move || stage(&alias_entry, &edits, &alias_list)
        });
        alias_add.connect_clicked({
            let stage_alias = stage_alias.clone();
            move |_| stage_alias()
        });
        {
            let stage_alias = stage_alias.clone();
            let alias_entry = alias_entry.clone();
            let edits = edits.clone();
            let alias_add = alias_add.clone();
            alias_entry.connect_changed(move |entry| {
                let text = entry.text().trim().to_string();
                let usable = !text.is_empty()
                    && !edits.borrow().aliases.iter().any(|draft| {
                        draft.text.eq_ignore_ascii_case(&text)
                    });
                alias_add.set_sensitive(usable);
            });
            if let Some(text) = alias_entry
                .delegate()
                .and_downcast::<gtk4::Text>()
            {
                text.connect_activate(move |_| stage_alias());
            }
        }
    }
    {
        // The host pushes the merge page; the search reset re-runs the
        // filter so the list opens complete.
        let merge_search = merge_search.clone();
        let merge_open = actions.merge_open.clone();
        merge_row.connect_activated(move |_| {
            merge_search.set_text("");
            merge_open();
        });
    }
    let merge_refresh: MergeRepaint = Default::default();
    let refresh_merge = {
        let state = state.clone();
        let merge_list = merge_list.clone();
        let selected = selected.clone();
        let merge_search = merge_search.clone();
        let merge_refresh = merge_refresh.clone();
        let on_merge_pick = {
            let state = state.clone();
            let selected = selected.clone();
            let merge_picked = actions.merge_picked.clone();
            Rc::new(move |target: i64| {
                let Some(id) = *selected.borrow() else {
                    return;
                };
                let db = state.borrow().db.clone();
                if let Err(e) = ira_db::merge_entities(&db, kind, target, id) {
                    eprintln!("Failed to merge the entities: {e}");
                    return;
                }
                merge_picked();
            })
        };
        Rc::new(move |filter: &str| {
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
                // The row opens the target's editor; saving there
                // repaints this list. The button merges.
                let state = state.clone();
                let row_c = row.clone();
                let name_c = name.clone();
                let merge_refresh = merge_refresh.clone();
                let merge_search = merge_search.clone();
                row.set_activatable(true);
                row.connect_activated(move |_| {
                    let on_saved: Rc<dyn Fn()> = Rc::new({
                        let merge_refresh = merge_refresh.clone();
                        let merge_search = merge_search.clone();
                        move || {
                            if let Some(repaint) = merge_refresh.borrow().as_ref() {
                                repaint(&merge_search.text());
                            }
                        }
                    });
                    show_entity_editor(&state, &row_c, kind, other, &name_c, on_saved);
                });
                merge_list.append(&row);
            }
        })
    };
    *merge_refresh.borrow_mut() = Some(refresh_merge.clone());
    {
        let refresh_merge = refresh_merge.clone();
        merge_search.connect_changed(move |entry| {
            refresh_merge(&entry.text());
        });
    }
    {
        // Save commits the staged name (when it changed) and the alias
        // diff. A failure flags the name entry red and touches nothing
        // else; success reloads the page from the database.
        let state = state.clone();
        let selected = selected.clone();
        let edits = edits.clone();
        let name_entry = name_entry.clone();
        let alias_list = alias_list.clone();
        let show = show.clone();
        let saved = actions.saved.clone();
        save_btn.connect_clicked(move |_| {
            let Some(id) = *selected.borrow() else {
                return;
            };
            let db = state.borrow().db.clone();
            let staged = edits.borrow().clone();
            if staged.name.is_empty() {
                name_entry.add_css_class(CSS_ERROR);
                return;
            }
            let mut failed = false;
            if staged.name != staged.original_name {
                if let Err(e) = ira_db::rename_entity(&db, kind, id, &staged.name) {
                    eprintln!("Failed to rename the entity: {e}");
                    failed = true;
                }
            }
            if !failed {
                for rowid in &staged.original_rowids {
                    let still_staged =
                        staged.aliases.iter().any(|draft| draft.rowid == Some(*rowid));
                    if !still_staged {
                        if let Err(e) = ira_db::remove_entity_alias(&db, *rowid) {
                            eprintln!("Failed to remove the alias: {e}");
                        }
                    }
                }
                for draft in &staged.aliases {
                    if draft.rowid.is_none() {
                        if let Err(e) = ira_db::add_entity_alias(&db, kind, &draft.text, id) {
                            eprintln!("Failed to add the alias: {e}");
                        }
                    }
                }
            }
            if failed {
                name_entry.add_css_class(CSS_ERROR);
                return;
            }
            name_entry.remove_css_class(CSS_ERROR);
            show(id);
            repaint_alias_list(&alias_list, &edits);
            saved();
        });
    }
    {
        // Discard throws the staged edits away and re-reads the entity.
        let state = state.clone();
        let selected = selected.clone();
        let edits = edits.clone();
        let name_entry = name_entry.clone();
        let alias_list = alias_list.clone();
        let show = show.clone();
        let cancelled = actions.cancelled.clone();
        cancel_btn.connect_clicked(move |_| {
            let Some(id) = *selected.borrow() else {
                cancelled();
                return;
            };
            let db = state.borrow().db.clone();
            let reloaded = load_edits(&db, kind, id);
            *edits.borrow_mut() = reloaded;
            name_entry.remove_css_class(CSS_ERROR);
            repaint_alias_list(&alias_list, &edits);
            show(id);
            cancelled();
        });
    }
    EntityEditor {
        content,
        actions: button_row,
        merge: merge_box,
        show,
    }
}

/// A standalone entity editor: name, aliases, merge, Save/Cancel —
/// no game context. Opened from every entity list outside game
/// settings (organise pickers, auto-group values, merge targets).
/// Save commits, refreshes the opener through `on_saved`, and closes.
pub(super) fn show_entity_editor(
    state: &SharedState,
    parent: &impl glib::prelude::IsA<gtk4::Widget>,
    kind: &'static str,
    id: i64,
    name: &str,
    on_saved: Rc<dyn Fn()>,
) {
    let dialog = adw::Dialog::new();
    dialog.set_content_width(520);
    dialog.set_content_height(560);

    // Stacked navigation: the entity page is the root, the merge
    // picker pushes on top of it. The navigation view owns the back
    // button, the swipe-back gesture and Escape — no shared header to
    // keep in sync, no page that can be swiped into unopened.
    let nav = adw::NavigationView::new();
    dialog.set_child(Some(&nav));

    // The merge page wraps the editor's merge box; it is built after
    // the editor, so the open callback reaches it through this cell.
    let merge_page_cell: Rc<RefCell<Option<adw::NavigationPage>>> = Default::default();
    let editor = build_entity_editor(
        state,
        kind,
        EntityEditorActions {
            saved: {
                let dialog = dialog.clone();
                let on_saved = on_saved.clone();
                Rc::new(move || {
                    on_saved();
                    dialog.close();
                })
            },
            cancelled: {
                let dialog = dialog.clone();
                Rc::new(move || { dialog.close(); })
            },
            merge_open: {
                let nav = nav.clone();
                let merge_page_cell = merge_page_cell.clone();
                Rc::new(move || {
                    if let Some(page) = merge_page_cell.borrow().as_ref() {
                        nav.push(page);
                    }
                })
            },
            merge_picked: {
                let dialog = dialog.clone();
                Rc::new(move || { dialog.close(); })
            },
        },
    );
    let entity_toolbar = adw::ToolbarView::new();
    let entity_header = adw::HeaderBar::new();
    let title = gtk4::Label::new(Some(name));
    title.add_css_class(CSS_HEADING);
    entity_header.set_title_widget(Some(&title));
    entity_toolbar.add_top_bar(&entity_header);
    let entity_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let entity_scrolled = gtk4::ScrolledWindow::new();
    entity_scrolled.set_vexpand(true);
    entity_scrolled.set_child(Some(&editor.content));
    entity_box.append(&entity_scrolled);
    entity_box.append(&editor.actions);
    entity_toolbar.set_content(Some(&entity_box));
    nav.add(&adw::NavigationPage::new(&entity_toolbar, "entity"));

    let merge_toolbar = adw::ToolbarView::new();
    let merge_header = adw::HeaderBar::new();
    let merge_title = gtk4::Label::new(Some(&crate::tr!("Merge into…")));
    merge_title.add_css_class(CSS_HEADING);
    merge_header.set_title_widget(Some(&merge_title));
    merge_toolbar.add_top_bar(&merge_header);
    merge_toolbar.set_content(Some(&editor.merge));
    merge_page_cell
        .borrow_mut()
        .replace(adw::NavigationPage::new(&merge_toolbar, "merge"));

    let shown = (editor.show)(id);
    title.set_text(&shown);
    dialog.present(Some(parent));
}

pub(super) fn show_entity_dialog(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    slot: &ScraperSlot,
    field: EntityField,
) {
    let kind = managed_kind(&field);
    let dialog = adw::Dialog::new();
    dialog.set_title(&field.label);
    dialog.set_content_width(520);
    dialog.set_content_height(520);
    let toast_overlay = adw::ToastOverlay::new();

    // ——— Stacked navigation: one page per level, each with its own
    // header. The root lists the game's own entities; Add and Manage
    // push their pages; every row pushes the entity it belongs to and
    // the merge picker pushes on top of that. The navigation view
    // owns the back button, the swipe-back gesture and Escape — pages
    // only ever exist after an explicit push, so a swipe can never
    // land on an unopened page. ———
    let nav = adw::NavigationView::new();
    toast_overlay.set_child(Some(&nav));
    dialog.set_child(Some(&toast_overlay));

    let root_toolbar = adw::ToolbarView::new();
    let root_header = adw::HeaderBar::new();
    let add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_btn.add_css_class(CSS_FLAT);
    add_btn.set_tooltip_text(Some(&crate::tr!("Add")));
    let manage_btn = gtk4::Button::from_icon_name("document-edit-symbolic");
    manage_btn.add_css_class(CSS_FLAT);
    manage_btn.set_tooltip_text(Some(&crate::tr!(
        "Rename entries, teach aliases, merge duplicates"
    )));
    root_header.pack_start(&add_btn);
    root_header.pack_start(&manage_btn);
    let title = gtk4::Label::new(Some(&field.label));
    title.add_css_class(CSS_HEADING);
    root_header.set_title_widget(Some(&title));
    root_toolbar.add_top_bar(&root_header);

    // ——— Root page: the game's own entities. ———
    let (root_scrolled, root_list) = super::helpers::clamped_boxed_list(520);
    root_scrolled.set_vexpand(true);
    root_toolbar.set_content(Some(&root_scrolled));
    let root_page = adw::NavigationPage::new(&root_toolbar, "root");
    nav.add(&root_page);

    // ——— Add-search page: the cache query plus a custom-add row. ———
    let search_toolbar = adw::ToolbarView::new();
    let search_header = adw::HeaderBar::new();
    let search_title = gtk4::Label::new(Some(&crate::tr!("Search")));
    search_title.add_css_class(CSS_HEADING);
    search_header.set_title_widget(Some(&search_title));
    search_toolbar.add_top_bar(&search_header);
    let search_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    search_box.set_margin_top(12);
    search_box.set_margin_bottom(12);
    search_box.set_margin_start(12);
    search_box.set_margin_end(12);
    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some(&crate::tr!("Search…")));
    search_box.append(&search_entry);
    let (search_scrolled, search_list) = super::helpers::clamped_boxed_list(520);
    search_scrolled.set_vexpand(true);
    search_box.append(&search_scrolled);
    search_toolbar.set_content(Some(&search_box));
    let search_page = adw::NavigationPage::new(&search_toolbar, "search");

    // ——— Manage list page: every cached entity, with its usage. ———
    let manage_title_text =
        crate::tr!("Manage {}").replacen("{}", &field.label, 1);
    let manage_toolbar = adw::ToolbarView::new();
    let manage_header = adw::HeaderBar::new();
    let manage_heading = gtk4::Label::new(Some(&manage_title_text));
    manage_heading.add_css_class(CSS_HEADING);
    manage_header.set_title_widget(Some(&manage_heading));
    manage_toolbar.add_top_bar(&manage_header);
    let manage_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    manage_box.set_margin_top(12);
    manage_box.set_margin_bottom(12);
    manage_box.set_margin_start(12);
    manage_box.set_margin_end(12);
    let manage_search = gtk4::SearchEntry::new();
    manage_search.set_placeholder_text(Some(&crate::tr!("Search…")));
    manage_box.append(&manage_search);
    let (manage_scrolled, manage_list) = super::helpers::clamped_boxed_list(520);
    manage_scrolled.set_vexpand(true);
    manage_box.append(&manage_scrolled);
    manage_toolbar.set_content(Some(&manage_box));
    let manage_page = adw::NavigationPage::new(&manage_toolbar, "manage");

    // open_entity is defined with the editor below; refresh_manage
    // (below it) reaches it through this cell.
    let open_entity_cell: Rc<RefCell<Option<OpenEntity>>> = Default::default();

    // Refresh the manage list; defined up here so the editor (below)
    // can refresh on save. Rows open through the cell, filled in once
    // open_entity exists.
    let refresh_manage = {
        let state = state.clone();
        let manage_list = manage_list.clone();
        let open_entity_cell = open_entity_cell.clone();
        move |filter: &str| {
            clear_children(&manage_list);
            let db = state.borrow().db.clone();
            let usage = ira_db::entity_usage(&db, kind).unwrap_or_default();
            let rows = ira_db::list_entities(&db, kind, filter).unwrap_or_default();
            if rows.is_empty() {
                let note = if filter.is_empty() {
                    crate::tr!("The cache fills as games get matched")
                } else {
                    crate::tr!("No results found")
                };
                manage_list.append(&status_row(&note));
                return;
            }
            for (id, name) in rows {
                let row = adw::ActionRow::new();
                row.set_use_markup(false);
                row.set_title(&name);
                row.set_subtitle(&usage_subtitle(&usage, id));
                row.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
                let open_entity_cell = open_entity_cell.clone();
                let name = name.clone();
                row.set_activatable(true);
                row.connect_activated(move |_| {
                    if let Some(open_entity) = open_entity_cell.borrow().as_ref() {
                        open_entity(id, &name);
                    }
                });
                manage_list.append(&row);
            }
        }
    };


    let state = Rc::new(state.clone());
    // The genre and family caches fill from the source's whole table on
    // first need — companies have no listing endpoint and stay
    // harvest-only. The fetch rides the sidebar strip and its marker
    // lives in the database; a refetch later is a matter of clearing
    // the marker.
    let warming: Rc<Cell<bool>> = Default::default();

    // The merge page wraps the editor's merge box; it is built after
    // the editor, so the open callback reaches it through this cell.
    let merge_page_cell: Rc<RefCell<Option<adw::NavigationPage>>> = Default::default();

    // The shared editor, hosted on the entity and merge pages: save
    // refreshes the manage list with a toast and pops back to the
    // page the row was opened from; cancel just pops.
    let editor = build_entity_editor(
        &state,
        kind,
        EntityEditorActions {
            saved: {
                let manage_search = manage_search.clone();
                let toast_overlay = toast_overlay.clone();
                let nav = nav.clone();
                Rc::new(move || {
                    refresh_manage(&manage_search.text());
                    toast_overlay.add_toast(adw::Toast::new(&crate::tr!("Saved")));
                    nav.pop();
                })
            },
            cancelled: {
                let nav = nav.clone();
                Rc::new(move || {
                    nav.pop();
                })
            },
            merge_open: {
                let nav = nav.clone();
                let merge_page_cell = merge_page_cell.clone();
                Rc::new(move || {
                    if let Some(page) = merge_page_cell.borrow().as_ref() {
                        nav.push(page);
                    }
                })
            },
            merge_picked: {
                let state = state.clone();
                let game = game.clone();
                let win = win.clone();
                let slot = slot.clone();
                let nav = nav.clone();
                Rc::new(move || {
                    refresh_rows(&state, &game, &win, &slot);
                    nav.pop_to_tag("root");
                })
            },
        },
    );
    // The entity page carries its subject's name as the title; the
    // merge page sits on top of it. Both push only on an explicit
    // open, so they never exist as a swipe target beforehand.
    let entity_toolbar = adw::ToolbarView::new();
    let entity_header = adw::HeaderBar::new();
    let entity_heading = gtk4::Label::new(None);
    entity_heading.add_css_class(CSS_HEADING);
    entity_header.set_title_widget(Some(&entity_heading));
    entity_toolbar.add_top_bar(&entity_header);
    let entity_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    {
        let entity_scrolled = gtk4::ScrolledWindow::new();
        entity_scrolled.set_vexpand(true);
        entity_scrolled.set_child(Some(&editor.content));
        entity_box.append(&entity_scrolled);
        entity_box.append(&editor.actions);
    }
    entity_toolbar.set_content(Some(&entity_box));
    let entity_page = adw::NavigationPage::new(&entity_toolbar, "entity");
    let merge_toolbar = adw::ToolbarView::new();
    let merge_header = adw::HeaderBar::new();
    let merge_heading = gtk4::Label::new(Some(&crate::tr!("Merge into…")));
    merge_heading.add_css_class(CSS_HEADING);
    merge_header.set_title_widget(Some(&merge_heading));
    merge_toolbar.add_top_bar(&merge_header);
    merge_toolbar.set_content(Some(&editor.merge));
    merge_page_cell
        .borrow_mut()
        .replace(adw::NavigationPage::new(&merge_toolbar, "merge"));

    // Open an entity's management page: staging loads through the
    // shared editor, then the page pushes on top of whichever list
    // the row lived on — back returns there on its own.
    let open_entity = {
        let editor_show = editor.show.clone();
        let entity_heading = entity_heading.clone();
        let nav = nav.clone();
        let entity_page = entity_page.clone();
        move |id: i64, _name: &str| {
            let fresh = (editor_show)(id);
            entity_heading.set_text(&fresh);
            nav.push(&entity_page);
        }
    };
    let open_entity: OpenEntity = Rc::new(open_entity);
    open_entity_cell.borrow_mut().replace(open_entity.clone());

    // The root list: the game's own entities, each with an edit button
    // straight into its management page and the staged remove. The
    // builder re-runs itself after every staged change, which the cell
    // makes possible from inside the row handlers.
    let build_root: RootRebuild = Default::default();
    let build_root_fn = {
        let state = state.clone();
        let game = game.clone();
        let win = win.clone();
        let slot = slot.clone();
        let field = field.clone();
        let root_list = root_list.clone();
        let open_entity = open_entity.clone();
        let build_root = build_root.clone();
        Rc::new(move || {
            clear_children(&root_list);
            let metadata = slot.draft.borrow().clone();
            let entities: Vec<ScraperEntity> = (field.get)(&metadata).clone();
            if entities.is_empty() {
                root_list.append(&status_row(&crate::tr!("None")));
                return;
            }
            for entity in entities {
                let row = adw::ActionRow::new();
                row.set_use_markup(false);
                let name = entity.name.clone();
                row.set_title(&name);
                // Edit straight into the entity's management page —
                // when the row carries a database id at all.
                if let Ok(id) = entity.id.parse::<i64>() {
                    let open_entity = open_entity.clone();
                    let row_name = name.clone();
                    let pen =
                        gtk4::Button::from_icon_name("document-edit-symbolic");
                    pen.add_css_class(CSS_FLAT);
                    pen.set_valign(gtk4::Align::Center);
                    pen.set_tooltip_text(Some(&crate::tr!("Manage")));
                    pen.connect_clicked(move |_| open_entity(id, &row_name));
                    row.add_suffix(&pen);
                }
                let remove = gtk4::Button::from_icon_name("user-trash-symbolic");
                remove.add_css_class(CSS_FLAT);
                remove.set_valign(gtk4::Align::Center);
                remove.set_tooltip_text(Some(&crate::tr!("Remove")));
                {
                    let state = state.clone();
                    let game = game.clone();
                    let win = win.clone();
                    let slot = slot.clone();
                    let field = field.clone();
                    let entity = entity.clone();
                    let build_root = build_root.clone();
                    remove.connect_clicked(move |_| {
                        let field = field.clone();
                        edit_field(&slot, |metadata| {
                            (field.remove)(metadata, &entity);
                        });
                        refresh_rows(&state, &game, &win, &slot);
                        // Rebuild for the empty state; destroying the
                        // clicked row from inside its own handler is
                        // the same pattern the editors have always
                        // used for their list pages.
                        if let Some(rebuild) = build_root.borrow().as_ref() {
                            rebuild();
                        }
                    });
                }
                row.add_suffix(&remove);
                root_list.append(&row);
            }
        })
    };
    *build_root.borrow_mut() = Some(build_root_fn.clone());
    let refresh_root = move || build_root_fn();

    // The add-search page: the cache matches, then a custom-add row
    // for the term itself. Picking restages the game's list and
    // repaints both it and the Identity page.
    let populate_search = {
        let state = state.clone();
        let game = game.clone();
        let win = win.clone();
        let slot = slot.clone();
        let field = field.clone();
        let search_list = search_list.clone();
        let refresh_root = refresh_root.clone();
        move |term: &str| {
            clear_children(&search_list);
            let term = term.trim();
            let store = {
                let state = state.clone();
                let game = game.clone();
                let win = win.clone();
                let slot = slot.clone();
                let field = field.clone();
                let refresh_root = refresh_root.clone();
                Rc::new(move |entity: ScraperEntity| {
                    let field = field.clone();
                    edit_field(&slot, |metadata| {
                        let entities = (field.get)(metadata);
                        // The same company twice is noise, not data.
                        if !entities.iter().any(|e| e.id == entity.id) {
                            (field.push)(metadata, entity.clone());
                        }
                    });
                    refresh_root();
                    refresh_rows(&state, &game, &win, &slot);
                })
            };
            if term.is_empty() {
                search_list.append(&status_row(&field.empty_text()));
                return;
            }
            let rows = match field.kind {
                EntityKind::Developer | EntityKind::Publisher => {
                    ira_db::scraper_companies_search(&state.borrow().db, term)
                        .unwrap_or_default()
                }
                EntityKind::Genre => {
                    ira_db::search_genres(&state.borrow().db, term).unwrap_or_default()
                }
                EntityKind::Family => {
                    ira_db::search_families(&state.borrow().db, term).unwrap_or_default()
                }
            };
            let draft_holds = |entity: &ScraperEntity| {
                (field.get)(&slot.draft.borrow())
                    .iter()
                    .any(|held| held.id == entity.id)
            };
            for entity in &rows {
                let store = store.clone();
                let row_entity = entity.clone();
                search_list.append(&pick_row(
                    &entity.name,
                    &format!("id {}", entity.id),
                    draft_holds(entity),
                    move || store(row_entity.clone()),
                ));
            }
            // The term itself as a custom entry — minted only on click,
            // never while typing.
            if !term.is_empty() {
                let state = state.clone();
                let field = field.clone();
                let store = store.clone();
                let mint_and_store = Rc::new(move |term: String| {
                    let entity = match field.kind {
                        EntityKind::Developer | EntityKind::Publisher => {
                            ira_db::steam_company_entity(&state.borrow().db, &term)
                        }
                        EntityKind::Genre => {
                            ira_db::local_genre_entity(&state.borrow().db, &term)
                        }
                        EntityKind::Family => {
                            ira_db::local_family_entity(&state.borrow().db, &term)
                        }
                    };
                    if let Some(entity) = entity {
                        store(entity);
                    }
                });
                let already = (field.get)(&slot.draft.borrow())
                    .iter()
                    .any(|held| held.name == term);
                let term_c = term.to_string();
                search_list.append(&pick_row(
                    &crate::tr!("Add \"{}\"").replacen("{}", term, 1),
                    "",
                    already,
                    move || mint_and_store(term_c.clone()),
                ));
            }
        }
    };

    // The manage list: every cached entity with its usage. Rows open
    // the entity's page, the HIG's go-next arrow marking the link.
    let refresh_manage = {
        let state = state.clone();
        let manage_list = manage_list.clone();
        let open_entity = open_entity.clone();
        move |filter: &str| {
            clear_children(&manage_list);
            let db = state.borrow().db.clone();
            let usage = ira_db::entity_usage(&db, kind).unwrap_or_default();
            let rows = ira_db::list_entities(&db, kind, filter).unwrap_or_default();
            if rows.is_empty() {
                let note = if filter.is_empty() {
                    crate::tr!("The cache fills as games get matched")
                } else {
                    crate::tr!("No results found")
                };
                manage_list.append(&status_row(&note));
                return;
            }
            for (id, name) in rows {
                let row = adw::ActionRow::new();
                row.set_use_markup(false);
                row.set_title(&name);
                row.set_subtitle(&usage_subtitle(&usage, id));
                row.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
                let open_entity = open_entity.clone();
                let name = name.clone();
                row.set_activatable(true);
                row.connect_activated(move |_| open_entity(id, &name));
                manage_list.append(&row);
            }
        }
    };


    // Fetch the source's whole table when this kind's cache was never
    // fetched: the pickers search the local cache, and a library new
    // to matching would otherwise present three rows where the source
    // has hundreds. The fetched-at marker makes "never" explicit and
    // gives a future refetch its hook; the run reports on the sidebar
    // strip like every other job.
    let maybe_warm: Rc<dyn Fn()> = Rc::new({
        let state = state.clone();
        let warming = warming.clone();
        let refresh_manage = refresh_manage.clone();
        let manage_search = manage_search.clone();
        move || {
            // Genres only: a small fixed vocabulary worth having whole.
            // Families are library-dependent — every match harvests the
            // series its games actually belong to — so they and
            // companies never bulk-fetch.
            if kind != ira_db::KIND_GENRE || warming.get() {
                return;
            }
            let (steam, cfg, db) = {
                let s = state.borrow();
                (s.steam.clone(), s.cfg.clone(), s.db.clone())
            };
            if !cfg.screenscraper_enabled {
                return;
            }
            if matches!(ira_db::entity_fetched_at(&db, kind), Ok(Some(_))) {
                return;
            }
            let short_label = crate::tr!("Fetching genres…");
            let status_label = crate::tr!("Fetching the genre table…");
            let done_status = crate::tr!("Genre table fetched");
            let Some(job) =
                super::fetch_images::begin_strip_job(&state, &short_label, &status_label)
            else {
                return;
            };
            warming.set(true);
            let creds = ira_api::ScraperCreds::from_account(
                cfg.screenscraper_id.clone(),
                cfg.screenscraper_password.clone(),
            );
            let cancel = job.cancel_flag();
            let (tx, rx) =
                std::sync::mpsc::channel::<Result<usize, String>>();
            std::thread::spawn(move || {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = tx.send(Ok(0));
                    return;
                }
                let _ = tx.send(super::fetch_metadata::fetch_and_warm_genres(
                    &steam, &db, &creds,
                ));
            });
            let refresh_manage = refresh_manage.clone();
            let manage_search = manage_search.clone();
            let warming = warming.clone();
            let state = state.clone();
            let job = job.clone();
            super::helpers::poll_channel(rx, move |fetched| {
                warming.set(false);
                match fetched {
                    // A cancelled run never calls fetch_and_warm, so the
                    // marker stays absent and the next manage entry
                    // retries; finish() itself goes silent on cancel.
                    Ok(count) => {
                        refresh_manage(&manage_search.text());
                        job.finish(
                            &state,
                            &crate::tr!("{} entries cached")
                                .replacen("{}", &count.to_string(), 1),
                            &done_status,
                        );
                    }
                    Err(e) => {
                        job.finish(
                            &state,
                            &crate::tr!("Fetch failed"),
                            &crate::tr!("Fetch failed: {}").replacen("{}", &e, 1),
                        );
                    }
                }
            });
        }
    });

    // ——— Wiring ———
    refresh_root();
    {
        let populate_search = populate_search.clone();
        search_entry.connect_changed(move |entry| {
            populate_search(&entry.text());
        });
    }
    {
        let populate_search = populate_search.clone();
        let nav = nav.clone();
        let search_page = search_page.clone();
        add_btn.connect_clicked(move |_| {
            populate_search("");
            nav.push(&search_page);
        });
    }
    {
        let refresh_manage = refresh_manage.clone();
        let nav = nav.clone();
        let manage_page = manage_page.clone();
        let maybe_warm = maybe_warm.clone();
        manage_btn.connect_clicked(move |_| {
            refresh_manage("");
            nav.push(&manage_page);
            maybe_warm();
        });
    }
    {
        let refresh_manage = refresh_manage.clone();
        manage_search.connect_changed(move |entry| {
            refresh_manage(&entry.text());
        });
    }

    dialog.present(Some(win));
}

/// Repaint the staged alias rows — each with its remove button, which
/// edits the staging area only.
fn repaint_alias_list(alias_list: &gtk4::ListBox, edits: &Rc<RefCell<EntityEdits>>) {
    clear_children(alias_list);
    let drafts = edits.borrow().aliases.clone();
    if drafts.is_empty() {
        alias_list.append(&status_row(&crate::tr!("No aliases")));
        return;
    }
    for (index, draft) in drafts.iter().enumerate() {
        let row = adw::ActionRow::new();
        row.set_use_markup(false);
        row.set_title(&draft.text);
        let edits = edits.clone();
        let list_for_closure = alias_list.clone();
        let remove = gtk4::Button::from_icon_name("user-trash-symbolic");
        remove.add_css_class(CSS_FLAT);
        remove.set_valign(gtk4::Align::Center);
        remove.set_tooltip_text(Some(&crate::tr!("Remove")));
        remove.connect_clicked(move |_| {
            edits.borrow_mut().aliases.remove(index);
            repaint_alias_list(&list_for_closure, &edits);
        });
        row.add_suffix(&remove);
        alias_list.append(&row);
    }
}

/// One picker answer row: the add button flips to a dead "Added" once
/// the pick lands — and starts that way when the draft already holds
/// the entity — so a second click can neither duplicate nor confuse.
fn pick_row(
    title: &str,
    subtitle: &str,
    already_added: bool,
    on_pick: impl Fn() + 'static,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    // Markup is off, so the text renders literally — no escaping, or a
    // custom name with a quote would show as &quot;.
    row.set_use_markup(false);
    row.set_title(title);
    row.set_subtitle(subtitle);
    let pick = gtk4::Button::new();
    let added = std::cell::Cell::new(already_added);
    mark_added(&pick, already_added);
    pick.set_valign(gtk4::Align::Center);
    pick.connect_clicked(move |pick| {
        if added.get() {
            return;
        }
        on_pick();
        added.set(true);
        mark_added(pick, true);
    });
    row.add_suffix(&pick);
    row
}

/// The button's two states: a suggested "Add", then a flat, insensitive
/// "Added" — impossible to click twice.
fn mark_added(button: &gtk4::Button, added: bool) {
    button.set_label(&if added {
        crate::tr!("Added")
    } else {
        crate::tr!("Add")
    });
    button.set_sensitive(!added);
    button.remove_css_class(CSS_SUGGESTED_ACTION);
    if added {
        button.add_css_class(CSS_FLAT);
    } else {
        button.add_css_class(CSS_SUGGESTED_ACTION);
    }
}
