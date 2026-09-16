//! The General page's ScreenScraper block: what a match stored, with the
//! company and genre lists editable through the cached pickers. Edits
//! persist immediately, like the RA section's do — a pick reads the
//! stored metadata back, mutates one field, and writes it, so the lookup
//! tables and the games row never disagree. Unmatched games get the same
//! editors, so metadata can be filled by hand before any match exists.

use adw::prelude::*;
use std::rc::Rc;

use super::css::*;
use super::helpers::{clear_children, poll_channel};
use super::ss_entity_dialog::{show_entity_picker, EntityKind};
use super::ss_match_dialog::{persist_ss_match, show_ss_search_dialog};
use super::state::SharedState;
use crate::Game;
use ira_api::ScraperCreds;
use ira_models::{ScraperEntity, ScraperMetadata};

/// The block for the General page: `None` when it would only be noise —
/// no stored metadata and no ScreenScraper coverage for the platform.
pub(super) fn build_scraper_section(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
) -> Option<adw::PreferencesGroup> {
    let metadata = ira_db::scraper_metadata_for_game(&state.borrow().db, game.db_id)
        .ok()
        .flatten();
    // Every game the SS pass can touch gets the block: consoles on mapped
    // platforms, and PC games whose diff search fills it too.
    let eligible = game.kind.is_pc()
        || ira_models::screenscraper_system_id(&game.platform_id).is_some();
    if metadata.is_none() && !eligible {
        return None;
    }
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("ScreenScraper"));

    let metadata = metadata.unwrap_or_default();
    for field in entity_fields() {
        field_row(&group, state, game, win, &field, &metadata);
    }

    // The small facts share one row; empty pieces drop out.
    let facts = [
        (!metadata.release_date.is_empty()).then(|| {
            format!("{} {}", crate::tr!("Released"), metadata.release_date)
        }),
        (!metadata.players.is_empty())
            .then(|| format!("{} {}", crate::tr!("Players"), metadata.players)),
        (metadata.rating > 0.0)
            .then(|| format!("{} {}/20", crate::tr!("Rating"), metadata.rating)),
        (!metadata.classifications.is_empty()).then(|| {
            metadata
                .classifications
                .iter()
                .map(|c| format!("{} {}", c.kind, c.value))
                .collect::<Vec<_>>()
                .join(" · ")
        }),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    if !facts.is_empty() {
        group.add(&plain_row(&facts));
    }
    if let Some((_, synopsis)) = metadata
        .synopses
        .iter()
        .find(|(langue, _)| langue == "en")
        .or_else(|| metadata.synopses.first())
    {
        let row = plain_row(synopsis);
        row.set_title_lines(4);
        row.add_css_class(CSS_DIM_LABEL);
        group.add(&row);
    }

    group.add(&search_row(state, game, win));
    Some(group)
}

/// Rebuild the block in place after an edit or a fresh match.
pub(super) fn refresh_scraper_section(state: &SharedState, db_id: i64) {
    let sd = match state.borrow().settings_data.clone() {
        Some(d) => d,
        None => return,
    };
    if sd.db_id != db_id || !sd.window.is_visible() {
        return;
    }
    let Some(container) = sd.scraper_container.clone() else {
        return;
    };
    let Some(game) = state
        .borrow()
        .games
        .iter()
        .find(|g| g.db_id == db_id)
        .cloned()
    else {
        return;
    };
    clear_children(&container);
    if let Some(group) = build_scraper_section(state, &game, &sd.window) {
        container.append(&group);
    }
}

/// Which metadata field a row edits: its display name, the picker it
/// opens, and the read/add/remove accessors.
#[derive(Clone)]
struct EntityField {
    label: String,
    kind: EntityKind,
    get: fn(&ScraperMetadata) -> &Vec<ScraperEntity>,
    remove: fn(&mut ScraperMetadata, &ScraperEntity),
    push: fn(&mut ScraperMetadata, ScraperEntity),
}

fn entity_fields() -> [EntityField; 3] {
    [
        EntityField {
            label: crate::tr!("Developer"),
            kind: EntityKind::Developer,
            get: |m| &m.developers,
            remove: |m, gone| m.developers.retain(|e| e.id != gone.id),
            push: |m, entity| m.developers.push(entity),
        },
        EntityField {
            label: crate::tr!("Publisher"),
            kind: EntityKind::Publisher,
            get: |m| &m.publishers,
            remove: |m, gone| m.publishers.retain(|e| e.id != gone.id),
            push: |m, entity| m.publishers.push(entity),
        },
        EntityField {
            label: crate::tr!("Genre"),
            kind: EntityKind::Genre,
            get: |m| &m.genres,
            remove: |m, gone| m.genres.retain(|e| e.id != gone.id),
            push: |m, entity| m.genres.push(entity),
        },
    ]
}

/// One row per metadata field: the stored names as the subtitle, an Edit
/// button opening the compact remove/add dialog. Works on empty metadata
/// too — edits create the record, so an unmatched game can be filled by
/// hand before any match exists.
fn field_row(
    group: &adw::PreferencesGroup,
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    field: &EntityField,
    metadata: &ScraperMetadata,
) {
    let row = adw::ActionRow::new();
    row.set_use_markup(false);
    row.set_title(&field.label);
    let names: Vec<String> = (field.get)(metadata)
        .iter()
        .map(|entity| entity.name.clone())
        .collect();
    if names.is_empty() {
        row.set_subtitle(&crate::tr!("None"));
    } else {
        row.set_subtitle(&names.join(", "));
    }

    let edit = gtk4::Button::with_label(&crate::tr!("Edit…"));
    edit.add_css_class(CSS_FLAT);
    edit.set_valign(gtk4::Align::Center);
    {
        let state = state.clone();
        let game = game.clone();
        let win = win.clone();
        let label = field.label.clone();
        edit.connect_clicked(move |_| {
            let Some(field) = entity_fields().into_iter().find(|f| f.label == label) else {
                return;
            };
            show_entity_edit_dialog(&state, &game, &win, field);
        });
    }
    row.add_suffix(&edit);
    group.add(&row);
}

/// The compact per-field editor: every stored entity with a remove
/// button, plus an add row opening the cached picker. Edits persist at
/// once and rebuild the dialog's list.
fn show_entity_edit_dialog(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    field: EntityField,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&field.label);
    dialog.set_content_width(420);
    dialog.set_content_height(320);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    let (scrolled, list) = super::helpers::clamped_boxed_list(420);
    toolbar.set_content(Some(&scrolled));
    dialog.set_child(Some(&toolbar));

    fill_entity_list(&list, state, game, field, dialog.clone());

    let parent = win.clone();
    dialog.present(Some(&parent));
}

/// (Re)build the edit dialog's list: every stored entity with a remove
/// button, then the add row opening the cached picker.
fn fill_entity_list(
    list: &gtk4::ListBox,
    state: &SharedState,
    game: &Game,
    field: EntityField,
    dialog: adw::Dialog,
) {
    clear_children(list);
    let metadata = ira_db::scraper_metadata_for_game(&state.borrow().db, game.db_id)
        .ok()
        .flatten()
        .unwrap_or_default();
    for entity in (field.get)(&metadata) {
        let row = adw::ActionRow::new();
        row.set_use_markup(false);
        row.set_title(&entity.name);
        let remove = gtk4::Button::from_icon_name("user-trash-symbolic");
        remove.add_css_class(CSS_FLAT);
        remove.set_valign(gtk4::Align::Center);
        {
            let state = state.clone();
            let game = game.clone();
            let entity = entity.clone();
            let list = list.clone();
            let dialog = dialog.clone();
            let field = field.clone();
            remove.connect_clicked(move |_| {
                let field = field.clone();
                edit_field(&state, game.db_id, |metadata| {
                    (field.remove)(metadata, &entity);
                });
                refresh_scraper_section(&state, game.db_id);
                fill_entity_list(&list, &state, &game, field, dialog.clone());
            });
        }
        row.add_suffix(&remove);
        list.append(&row);
    }

    let add = adw::ActionRow::new();
    add.set_title(&crate::tr!("Add from ScreenScraper…"));
    add.set_activatable(true);
    {
        let state = state.clone();
        let game = game.clone();
        let dialog = dialog.clone();
        let list = list.clone();
        let pick_field = std::rc::Rc::new(field);
        let kind = pick_field.kind;
        add.connect_activated(move |_| {
            let pick_state = state.clone();
            let pick_game = game.clone();
            let pick_list = list.clone();
            let pick_dialog = dialog.clone();
            let pick_field = pick_field.clone();
            let on_pick = Rc::new(move |entity: ScraperEntity| {
                let field = pick_field.clone();
                // The same company twice is noise, not data.
                edit_field(&pick_state, pick_game.db_id, |metadata| {
                    let entities = (field.get)(metadata);
                    if !entities.iter().any(|e| e.id == entity.id) {
                        (field.push)(metadata, entity.clone());
                    }
                });
                refresh_scraper_section(&pick_state, pick_game.db_id);
                fill_entity_list(&pick_list, &pick_state, &pick_game, (*field).clone(), pick_dialog.clone());
            });
            show_entity_picker(&state, kind, &dialog, on_pick);
        });
    }
    list.append(&add);
}

fn search_row(state: &SharedState, game: &Game, win: &adw::Window) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    let matched = ira_db::scraper_metadata_for_game(&state.borrow().db, game.db_id)
        .map(|m| m.is_some())
        .unwrap_or(false);
    if matched {
        // The entry's id is known, so gaps can be filled without any
        // rematch ambiguity: one exact fetch by id, merged over what's
        // stored. Unmatch stays for genuinely wrong matches.
        row.set_title(&crate::tr!("Matched"));
        let fetch = gtk4::Button::with_label(&crate::tr!("Fetch missing"));
        fetch.add_css_class(CSS_SUGGESTED_ACTION);
        fetch.set_valign(gtk4::Align::Center);
        {
            let state = state.clone();
            let game = game.clone();
            fetch.connect_clicked(move |btn| {
                btn.set_sensitive(false);
                let state = state.clone();
                let game = game.clone();
                run_refetch_missing(&state, &game, move |state, db_id| {
                    refresh_scraper_section(state, db_id);
                });
            });
        }
        row.add_suffix(&fetch);
        let unmatch = gtk4::Button::with_label(&crate::tr!("Unmatch"));
        unmatch.add_css_class(CSS_FLAT);
        unmatch.set_valign(gtk4::Align::Center);
        {
            let state = state.clone();
            let db_id = game.db_id;
            unmatch.connect_clicked(move |_| {
                if let Err(e) = ira_db::clear_screenscraper_match(&state.borrow().db, db_id) {
                    eprintln!("Failed to unmatch: {e}");
                    return;
                }
                if let Some(g) = state
                    .borrow_mut()
                    .games
                    .iter_mut()
                    .find(|g| g.db_id == db_id)
                {
                    g.screenscraper_id = String::new();
                }
                refresh_scraper_section(&state, db_id);
            });
        }
        row.add_suffix(&unmatch);
        return row;
    }

    // A Steam-backed PC game can run the whole automated matching —
    // system search, cross-platform diff, garnish — from here, without
    // waiting for the next mass-matcher opening.
    let auto_matchable = game.kind.is_pc() && game.platform_id.parse::<u32>().is_ok();
    row.set_title(&crate::tr!("Not matched yet"));

    if auto_matchable {
        let btn = gtk4::Button::with_label(&crate::tr!("Auto match"));
        btn.add_css_class(CSS_SUGGESTED_ACTION);
        btn.set_valign(gtk4::Align::Center);
        {
            let state = state.clone();
            let game = game.clone();
            btn.connect_clicked(move |btn| {
                btn.set_sensitive(false);
                let state = state.clone();
                let game = game.clone();
                run_auto_match(&state, &game, |state, db_id, _matched| {
                    refresh_scraper_section(state, db_id);
                });
            });
        }
        row.add_suffix(&btn);
    }

    row.set_tooltip_text(Some(&crate::tr!("Search ScreenScraper…")));
    let btn = gtk4::Button::with_label(&crate::tr!("Search"));
    btn.add_css_class(CSS_SUGGESTED_ACTION);
    btn.set_valign(gtk4::Align::Center);
    {
        let state = state.clone();
        let name = game.name.clone();
        let platform_id = game.platform_id.clone();
        let db_id = game.db_id;
        let win = win.clone();
        btn.connect_clicked(move |_| {
            let dialog_state = state.clone();
            let refresh_state = state.clone();
            let refresh: Rc<dyn Fn()> =
                Rc::new(move || refresh_scraper_section(&refresh_state, db_id));
            show_ss_search_dialog(
                &dialog_state,
                db_id,
                &name,
                &platform_id,
                &win,
                Some(refresh),
            );
        });
    }
    row.add_suffix(&btn);
    row
}

/// The automated PC matching, off-thread: resolve, then persist and
/// report on the UI loop. `on_done` runs on the main loop with whether a
/// match landed.
fn run_auto_match(
    state: &SharedState,
    game: &Game,
    on_done: impl Fn(&SharedState, i64, bool) + 'static,
) {
    let (steam, creds, db) = {
        let s = state.borrow();
        (
            s.steam.clone(),
            ScraperCreds::from_account(
                s.cfg.screenscraper_id.clone(),
                s.cfg.screenscraper_password.clone(),
            ),
            s.db.clone(),
        )
    };
    let kind = game.kind;
    let platform_id = game.platform_id.clone();
    let title = game.name.clone();
    let display = game.name.clone();
    let db_id = game.db_id;
    let (tx, rx) = std::sync::mpsc::channel::<Option<ira_api::screenscraper::ScrapedGame>>();
    std::thread::spawn(move || {
        let target = super::mass_match_ss::PcMatchTarget {
            kind,
            platform_id: &platform_id,
            title: &title,
            display: &display,
            db_id,
        };
        let matched = super::mass_match_ss::run_pc_matching(&steam, &creds, &db, &target);
        let _ = tx.send(matched);
    });
    let state = state.clone();
    poll_channel(rx, move |matched| {
        if let Some(game) = matched.as_ref() {
            persist_ss_match(&state, db_id, game);
        }
        on_done(&state, db_id, matched.is_some());
    });
}

/// Re-fetch the matched entry by its own id — no search, no ambiguity —
/// and merge only what's missing into the stored metadata. Off-thread;
/// `on_done` runs on the main loop.
fn run_refetch_missing(
    state: &SharedState,
    game: &Game,
    on_done: impl Fn(&SharedState, i64) + 'static,
) {
    let (steam, creds, db) = {
        let s = state.borrow();
        (
            s.steam.clone(),
            ScraperCreds::from_account(
                s.cfg.screenscraper_id.clone(),
                s.cfg.screenscraper_password.clone(),
            ),
            s.db.clone(),
        )
    };
    let Some(ss_id) = ira_db::find_by_db_id(&db, game.db_id)
        .ok()
        .flatten()
        .map(|e| e.screenscraper_id)
        .filter(|id| !id.is_empty())
    else {
        on_done(state, game.db_id);
        return;
    };
    let db_id = game.db_id;
    let (tx, rx) = std::sync::mpsc::channel::<bool>();
    std::thread::spawn(move || {
        let fresh = steam
            .screenscraper_game(&creds, &ss_id)
            .ok()
            .and_then(|games| games.into_iter().next());
        let changed = fresh
            .and_then(|game| {
                let fresh_meta = ira_models::ScraperMetadata {
                    ss_id: ss_id.clone(),
                    release_date: game.release_date.clone(),
                    release_timestamp: ira_db::scraper_release_timestamp(&game.release_date),
                    release_dates: game.release_dates.clone(),
                    developers: game.developers.clone(),
                    publishers: game.publishers.clone(),
                    genres: game.genres.clone(),
                    players: game.players.clone(),
                    rating: game.rating,
                    classifications: game.classifications.clone(),
                    synopses: game
                        .synopses
                        .iter()
                        .find(|(l, _)| l == "en")
                        .or_else(|| game.synopses.first())
                        .cloned()
                        .into_iter()
                        .collect(),
                };
                ira_db::merge_missing_scraper_metadata(&db, db_id, &fresh_meta).ok()
            })
            .unwrap_or(false);
        let _ = tx.send(changed);
    });
    let state = state.clone();
    let db_id = game.db_id;
    poll_channel(rx, move |changed| {
        if changed {
            eprintln!("SS refetch: filled missing metadata for {db_id}");
        }
        on_done(&state, db_id);
    });
}

fn plain_row(text: &str) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(text);
    row.set_use_markup(false);
    row
}

/// Read the stored metadata — an empty record when none exists yet, so
/// unmatched games can be filled by hand — mutate one field, write it
/// back. A store failure leaves the dialog as-is; the error is on
/// stderr.
fn edit_field(state: &SharedState, db_id: i64, mutate: impl FnOnce(&mut ScraperMetadata)) {
    let mut metadata = match ira_db::scraper_metadata_for_game(&state.borrow().db, db_id) {
        Ok(Some(metadata)) => metadata,
        Ok(None) => ScraperMetadata::default(),
        Err(e) => {
            eprintln!("Failed to read ScreenScraper metadata: {e}");
            return;
        }
    };
    mutate(&mut metadata);
    if let Err(e) = ira_db::store_scraper_metadata(&state.borrow().db, db_id, &metadata) {
        eprintln!("Failed to store ScreenScraper metadata: {e}");
    }
}
