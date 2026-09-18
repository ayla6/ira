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

/// The dialog's pages, in flow order — the slide direction between two
/// pages follows their depth difference.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Root,
    Search,
    Manage,
    Entity,
    Merge,
}

impl Page {
    fn name(self) -> &'static str {
        match self {
            Page::Root => "root",
            Page::Search => "search",
            Page::Manage => "manage",
            Page::Entity => "entity",
            Page::Merge => "merge",
        }
    }

    fn depth(self) -> u8 {
        match self {
            Page::Root => 0,
            Page::Search | Page::Manage => 1,
            Page::Entity => 2,
            Page::Merge => 3,
        }
    }
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

    // ——— One header bar for every view: back at the start, then the
    // root page's add and manage buttons; the title follows the
    // visible page. The dialog's own close stays at the end. ———
    let header = adw::HeaderBar::new();
    let back_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
    back_btn.add_css_class(CSS_FLAT);
    back_btn.set_tooltip_text(Some(&crate::tr!("Back")));
    back_btn.set_visible(false);
    let add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_btn.add_css_class(CSS_FLAT);
    add_btn.set_tooltip_text(Some(&crate::tr!("Add")));
    let manage_btn = gtk4::Button::from_icon_name("document-edit-symbolic");
    manage_btn.add_css_class(CSS_FLAT);
    manage_btn.set_tooltip_text(Some(&crate::tr!(
        "Rename entries, teach aliases, merge duplicates"
    )));
    header.pack_start(&back_btn);
    header.pack_start(&add_btn);
    header.pack_start(&manage_btn);
    let title = gtk4::Label::new(Some(&field.label));
    title.add_css_class("heading");
    header.set_title_widget(Some(&title));
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);

    // ——— Root page: the game's own entities. ———
    let (root_scrolled, root_list) = super::helpers::clamped_boxed_list(520);
    root_scrolled.set_vexpand(true);

    // ——— Add-search page: the cache query plus a custom-add row. ———
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

    // ——— Manage list page: every cached entity, with its usage. ———
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

    // ——— Entity page: staged rename and aliases, Save to commit. ———
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
    alias_entry.set_show_apply_button(true);
    alias_group.add(&alias_entry);
    entity_content.append(&alias_group);

    let merge_group = adw::PreferencesGroup::new();
    let merge_row = adw::ActionRow::new();
    merge_row.set_title(&crate::tr!("Merge into…"));
    merge_row.set_subtitle(&crate::tr!("Combine this entry with another one"));
    merge_row.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
    merge_group.add(&merge_row);
    entity_content.append(&merge_group);

    let button_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    button_row.set_halign(gtk4::Align::End);
    let discard_btn = gtk4::Button::with_label(&crate::tr!("Discard"));
    discard_btn.add_css_class(CSS_FLAT);
    let save_btn = gtk4::Button::with_label(&crate::tr!("Save"));
    save_btn.add_css_class(CSS_SUGGESTED_ACTION);
    button_row.append(&discard_btn);
    button_row.append(&save_btn);
    entity_content.append(&button_row);
    let entity_scrolled = gtk4::ScrolledWindow::new();
    entity_scrolled.set_vexpand(true);
    entity_scrolled.set_child(Some(&entity_content));

    // ——— Merge page: pick the entity to merge into. ———
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

    let stack = gtk4::Stack::new();
    stack.set_vhomogeneous(false);
    stack.add_named(&root_scrolled, Some(Page::Root.name()));
    stack.add_named(&search_box, Some(Page::Search.name()));
    stack.add_named(&manage_scrolled, Some(Page::Manage.name()));
    stack.add_named(&entity_scrolled, Some(Page::Entity.name()));
    stack.add_named(&merge_box, Some(Page::Merge.name()));
    toolbar.set_content(Some(&stack));
    dialog.set_child(Some(&toolbar));

    let state = Rc::new(state.clone());
    let selected: Rc<RefCell<Option<i64>>> = Default::default();
    let edits: Rc<RefCell<EntityEdits>> = Default::default();
    let page: Cell<Page> = Cell::new(Page::Root);

    // Navigation: pages deeper in the flow slide in from the right,
    // back slides left; the header's buttons and title follow. The
    // entity page carries its subject's name as the title.
    let navigate = {
        let root_title = field.label.clone();
        let stack = stack.clone();
        let page = page.clone();
        let back_btn = back_btn.clone();
        let add_btn = add_btn.clone();
        let manage_btn = manage_btn.clone();
        let title = title.clone();
        Rc::new(move |to: Page, entity_name: &str| {
            let forward = to.depth() > page.get().depth();
            stack.set_transition_type(if forward {
                gtk4::StackTransitionType::SlideRight
            } else {
                gtk4::StackTransitionType::SlideLeft
            });
            stack.set_visible_child_name(to.name());
            back_btn.set_visible(to != Page::Root);
            let on_root = to == Page::Root;
            add_btn.set_visible(on_root);
            manage_btn.set_visible(on_root);
            let search_title = crate::tr!("Search");
            let manage_title =
                crate::tr!("Manage {}").replacen("{}", &root_title, 1);
            let merge_title = crate::tr!("Merge into…");
            title.set_text(match to {
                Page::Root => &root_title,
                Page::Search => &search_title,
                Page::Manage => &manage_title,
                Page::Entity => entity_name,
                Page::Merge => &merge_title,
            });
            page.set(to);
        })
    };

    // Open an entity's management page: its stored state loads into
    // the staging area first.
    let open_entity = {
        let state = state.clone();
        let navigate = navigate.clone();
        let selected = selected.clone();
        let edits = edits.clone();
        let name_entry = name_entry.clone();
        let alias_list = alias_list.clone();
        move |id: i64, name: &str| {
            let db = state.borrow().db.clone();
            let loaded = load_edits(&db, kind, id);
            name_entry.set_text(&loaded.name);
            name_entry.remove_css_class(CSS_ERROR);
            repaint_alias_list(&alias_list, &edits);
            *edits.borrow_mut() = loaded;
            selected.replace(Some(id));
            navigate(Page::Entity, name);
        }
    };
    let open_entity: OpenEntity = Rc::new(open_entity);

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
                    crate::tr!("Nothing in the cache yet — it fills as games get matched")
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

    // The merge page: every other entity, the pick's Merge button
    // folding this one into it. The db keeps the ScreenScraper id when
    // exactly one of the two has one, and the absorbed spelling stays
    // behind as an alias of the survivor.
    let on_merge_pick = {
        let state = state.clone();
        let selected = selected.clone();
        let navigate = navigate.clone();
        let game = game.clone();
        let win = win.clone();
        let slot = slot.clone();
        move |target: i64| {
            let Some(id) = *selected.borrow() else {
                return;
            };
            let db = state.borrow().db.clone();
            if let Err(e) = ira_db::merge_entities(&db, kind, target, id) {
                eprintln!("Failed to merge the entities: {e}");
                return;
            }
            refresh_rows(&state, &game, &win, &slot);
            navigate(Page::Root, "");
        }
    };
    let refresh_merge = {
        let state = state.clone();
        let merge_list = merge_list.clone();
        let selected = selected.clone();
        let on_merge_pick = on_merge_pick.clone();
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
        let navigate = navigate.clone();
        add_btn.connect_clicked(move |_| {
            populate_search("");
            navigate(Page::Search, "");
        });
    }
    {
        let refresh_manage = refresh_manage.clone();
        let navigate = navigate.clone();
        manage_btn.connect_clicked(move |_| {
            refresh_manage("");
            navigate(Page::Manage, "");
        });
    }
    {
        // Back follows the flow: the entity page hands the merge page's
        // state back too, since it was entered from there.
        let state = state.clone();
        let page = page.clone();
        let manage_search = manage_search.clone();
        let refresh_manage = refresh_manage.clone();
        let selected = selected.clone();
        let navigate = navigate.clone();
        back_btn.connect_clicked(move |_| {
            match page.get() {
                Page::Search => navigate(Page::Root, ""),
                Page::Manage => navigate(Page::Root, ""),
                Page::Entity => {
                    refresh_manage(&manage_search.text());
                    navigate(Page::Manage, "");
                }
                Page::Merge => {
                    let name = selected
                        .borrow()
                        .and_then(|id| {
                            ira_db::list_entities(&state.borrow().db, kind, "")
                                .ok()
                                .map(|rows| {
                                    rows.into_iter().find(|(row_id, _)| *row_id == id)
                                })
                                .and_then(|found| found.map(|(_, name)| name))
                        })
                        .unwrap_or_default();
                    navigate(Page::Entity, &name);
                }
                Page::Root => {}
            }
        });
    }
    {
        // Typing stages; nothing is written until Save. An empty name
        // refuses to save (and flags the entry) rather than erasing
        // the entity.
        let edits = edits.clone();
        name_entry.connect_changed(move |entry| {
            edits.borrow_mut().name = entry.text().trim().to_string();
        });
    }
    {
        // An alias typed on apply joins the staged list; Save is what
        // teaches it to the database. A spelling already staged is
        // ignored.
        let edits = edits.clone();
        let alias_list = alias_list.clone();
        alias_entry.connect_apply(move |entry| {
            let text = entry.text().trim().to_string();
            if text.is_empty() {
                return;
            }
            let mut staged = edits.borrow_mut();
            if staged
                .aliases
                .iter()
                .any(|draft| draft.text.eq_ignore_ascii_case(&text))
            {
                entry.set_text("");
                return;
            }
            staged.aliases.push(AliasDraft {
                rowid: None,
                text: text.clone(),
            });
            drop(staged);
            entry.set_text("");
            repaint_alias_list(&alias_list, &edits);
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
        let refresh_manage = refresh_manage.clone();
        let manage_search = manage_search.clone();
        let title = title.clone();
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
            let reloaded = load_edits(&db, kind, id);
            name_entry.set_text(&reloaded.name);
            title.set_text(&reloaded.name);
            repaint_alias_list(&alias_list, &edits);
            *edits.borrow_mut() = reloaded;
            refresh_manage(&manage_search.text());
        });
    }
    {
        // Discard throws the staged edits away and re-reads the entity.
        let state = state.clone();
        let selected = selected.clone();
        let edits = edits.clone();
        let name_entry = name_entry.clone();
        let alias_list = alias_list.clone();
        let title = title.clone();
        discard_btn.connect_clicked(move |_| {
            let Some(id) = *selected.borrow() else {
                return;
            };
            let db = state.borrow().db.clone();
            let reloaded = load_edits(&db, kind, id);
            name_entry.set_text(&reloaded.name);
            title.set_text(&reloaded.name);
            name_entry.remove_css_class(CSS_ERROR);
            repaint_alias_list(&alias_list, &edits);
            *edits.borrow_mut() = reloaded;
        });
    }
    {
        let merge_search_for_row = merge_search.clone();
        let selected = selected.clone();
        let navigate = navigate.clone();
        let refresh_merge = refresh_merge.clone();
        merge_row.connect_activated(move |_| {
            if selected.borrow().is_none() {
                return;
            }
            merge_search_for_row.set_text("");
            refresh_merge("");
            navigate(Page::Merge, "");
        });
    }
    {
        let refresh_merge = refresh_merge.clone();
        merge_search.connect_changed(move |entry| {
            refresh_merge(&entry.text());
        });
    }

    dialog.present(Some(win));
}

/// Repaint the staged alias rows — each with its remove button, which
/// edits the staging area only.
fn repaint_alias_list(alias_list: &gtk4::Box, edits: &Rc<RefCell<EntityEdits>>) {
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
