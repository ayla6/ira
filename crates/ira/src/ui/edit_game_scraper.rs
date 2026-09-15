//! The General page's ScreenScraper block: what a match stored, displayed
//! read-only, with the company and genre lists editable through the
//! cached pickers. Edits persist immediately, like the RA section's do —
//! a pick reads the stored metadata back, mutates one field, and writes
//! it, so the lookup tables and the games row never disagree.

use adw::prelude::*;
use std::rc::Rc;

use super::css::*;
use super::helpers::clear_children;
use super::ss_entity_dialog::{show_entity_picker, EntityKind};
use super::ss_match_dialog::show_ss_search_dialog;
use super::state::SharedState;
use crate::Game;
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
    if metadata.is_none() && ira_models::screenscraper_system_id(&game.platform_id).is_none() {
        return None;
    }
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("ScreenScraper"));
    match metadata {
        Some(metadata) => fill_matched_section(&group, state, game, win, &metadata),
        None => group.add(&plain_row(&crate::tr!("Not matched yet"))),
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

fn fill_matched_section(
    group: &adw::PreferencesGroup,
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    metadata: &ScraperMetadata,
) {
    for (field, entities) in [
        (
            EntityField {
                label: crate::tr!("Developer"),
                kind: EntityKind::Developer,
                get: |m| &mut m.developers,
            },
            &metadata.developers,
        ),
        (
            EntityField {
                label: crate::tr!("Publisher"),
                kind: EntityKind::Publisher,
                get: |m| &mut m.publishers,
            },
            &metadata.publishers,
        ),
        (
            EntityField {
                label: crate::tr!("Genre"),
                kind: EntityKind::Genre,
                get: |m| &mut m.genres,
            },
            &metadata.genres,
        ),
    ] {
        entity_rows(group, state, game, win, &field, entities);
    }

    if !metadata.players.is_empty() {
        group.add(&plain_row(&format!("{}: {}", crate::tr!("Players"), metadata.players)));
    }
    if !metadata.release_date.is_empty() {
        group.add(&plain_row(&format!(
            "{}: {}",
            crate::tr!("Released"),
            metadata.release_date
        )));
    }
    if metadata.rating > 0.0 {
        group.add(&plain_row(&format!(
            "{}: {}/20",
            crate::tr!("Rating"),
            metadata.rating
        )));
    }
    if let Some((_, synopsis)) = metadata
        .synopses
        .iter()
        .find(|(langue, _)| langue == "en")
        .or_else(|| metadata.synopses.first())
    {
        let row = plain_row(&format!("{}: {}", crate::tr!("Synopsis"), synopsis));
        row.set_title_lines(4);
        group.add(&row);
    }
}

/// Which metadata field a set of rows edits: its display name, the picker
/// it opens, and the accessor to the id list.
struct EntityField {
    label: String,
    kind: EntityKind,
    get: fn(&mut ScraperMetadata) -> &mut Vec<ScraperEntity>,
}

/// One row per stored entity plus an add row, so a several-companies
/// credit reads as several rows and each piece can go away on its own.
fn entity_rows(
    group: &adw::PreferencesGroup,
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    field: &EntityField,
    entities: &[ScraperEntity],
) {
    let label = &field.label;
    let kind = field.kind;
    let get = field.get;
    for entity in entities {
        let row = adw::ActionRow::new();
        row.set_title(&entity.name);
        row.set_subtitle(label);
        let remove = gtk4::Button::with_label(&crate::tr!("Remove"));
        remove.add_css_class(CSS_DESTRUCTIVE_ACTION);
        remove.set_valign(gtk4::Align::Center);
        {
            let state = state.clone();
            let game = game.clone();
            let entity = entity.clone();
            remove.connect_clicked(move |_| {
                edit_field(&state, game.db_id, |metadata| {
                    get(metadata).retain(|e| e.id != entity.id);
                });
                refresh_scraper_section(&state, game.db_id);
            });
        }
        row.add_suffix(&remove);
        group.add(&row);
    }

    let add = adw::ActionRow::new();
    add.set_title(&crate::tr!("Add {}…").replacen("{}", label, 1));
    let btn = gtk4::Button::with_label(&crate::tr!("Add"));
    btn.add_css_class(CSS_SUGGESTED_ACTION);
    btn.set_valign(gtk4::Align::Center);
    {
        let state = state.clone();
        let game = game.clone();
        let win = win.clone();
        btn.connect_clicked(move |_| {
            let pick_state = state.clone();
            let game = game.clone();
            let on_pick = Rc::new(move |entity: ScraperEntity| {
                edit_field(&pick_state, game.db_id, |metadata| {
                    let list = get(metadata);
                    // The same company twice is noise, not data.
                    if !list.iter().any(|e| e.id == entity.id) {
                        list.push(entity.clone());
                    }
                });
                refresh_scraper_section(&pick_state, game.db_id);
            });
            show_entity_picker(&state, kind, &win, on_pick);
        });
    }
    add.add_suffix(&btn);
    group.add(&add);
}

fn search_row(state: &SharedState, game: &Game, win: &adw::Window) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    let matched = ira_db::scraper_metadata_for_game(&state.borrow().db, game.db_id)
        .map(|m| m.is_some())
        .unwrap_or(false);
    if matched {
        // Searching an already-matched game would silently replace it;
        // removing the match is the explicit first step.
        row.set_title(&crate::tr!("Matched — unmatch to search again"));
        let btn = gtk4::Button::with_label(&crate::tr!("Unmatch"));
        btn.add_css_class(CSS_DESTRUCTIVE_ACTION);
        btn.set_valign(gtk4::Align::Center);
        {
            let state = state.clone();
            let db_id = game.db_id;
            btn.connect_clicked(move |_| {
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
        row.add_suffix(&btn);
        return row;
    }
    row.set_title(&crate::tr!("Search ScreenScraper…"));
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

fn plain_row(text: &str) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(text);
    row.set_use_markup(false);
    row
}

/// Read the stored metadata, mutate one field, write it back. A store
/// failure leaves the dialog as-is; the error is on stderr.
fn edit_field(state: &SharedState, db_id: i64, mutate: impl FnOnce(&mut ScraperMetadata)) {
    let mut metadata = match ira_db::scraper_metadata_for_game(&state.borrow().db, db_id) {
        Ok(Some(metadata)) => metadata,
        Ok(None) => return,
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
