//! The Identity page's metadata rows: what a match stored, editable in
//! place — companies and genres through the cached pickers, the small
//! facts as entry and spin rows. Edits persist immediately, like the RA
//! section's do — a pick reads the stored metadata back, mutates one
//! field, and writes it, so the lookup tables and the games row never
//! disagree. Unmatched games get the same editors, so metadata can be
//! filled by hand before any match exists.

use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use super::css::*;
use super::helpers::status_row;
use super::helpers::{clear_children, poll_channel};
use super::mass_match_ss::{RefetchOutcome, spawn_refetch_worker};
use super::steam_search_dialog::match_result_row;
use super::ss_match_dialog::{persist_ss_match, show_ss_search_dialog};
use super::state::SharedState;
use crate::Game;
use ira_api::ScraperCreds;
use ira_models::{ScraperClassification, ScraperEntity, ScraperMetadata};

/// Handle on the metadata rows living inside the Identity group: a
/// refresh removes exactly these rows and rebuilds them from the
/// database, leaving the group's own title/sort/path rows alone.
#[derive(Clone)]
pub(crate) struct ScraperSlot {
    group: adw::PreferencesGroup,
    rows: Rc<RefCell<Vec<gtk4::Widget>>>,
    match_row: Rc<RefCell<Option<adw::ActionRow>>>,
}

impl ScraperSlot {
    fn add(&self, widget: gtk4::Widget) {
        self.group.add(&widget);
        self.rows.borrow_mut().push(widget);
    }

    /// A one-line outcome under the match row's title ("Filled the
    /// missing pieces", a fetch error…) — empty clears it.
    pub(crate) fn set_match_status(&self, text: &str) {
        if let Some(row) = self.match_row.borrow().as_ref() {
            row.set_subtitle(text);
        }
    }
}

/// The Identity group's metadata rows: `None` when they would only be
/// noise — no stored metadata and no ScreenScraper coverage for the
/// platform.
pub(super) fn build_scraper_section(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    group: &adw::PreferencesGroup,
) -> Option<ScraperSlot> {
    let metadata = stored_metadata(state, game.db_id);
    // Every game the SS pass can touch gets the rows: consoles on mapped
    // platforms, and PC games whose diff search fills them too.
    let eligible = game.kind.is_pc()
        || ira_models::screenscraper_system_id(&game.platform_id).is_some();
    if metadata.is_none() && !eligible {
        return None;
    }
    let slot = ScraperSlot {
        group: group.clone(),
        rows: Rc::new(RefCell::new(Vec::new())),
        match_row: Rc::new(RefCell::new(None)),
    };
    rebuild_rows(state, game, win, &slot);
    Some(slot)
}

/// Rebuild the rows in place after an edit or a fresh match. `true` when
/// the open settings window actually shows this game's rows.
pub(super) fn refresh_scraper_section(state: &SharedState, db_id: i64) -> bool {
    let sd = match state.borrow().settings_data.clone() {
        Some(d) => d,
        None => return false,
    };
    if sd.db_id != db_id || !sd.window.is_visible() {
        return false;
    }
    let Some(slot) = sd.scraper_slot.clone() else {
        return false;
    };
    let Some(game) = state
        .borrow()
        .games
        .iter()
        .find(|g| g.db_id == db_id)
        .cloned()
    else {
        return false;
    };
    rebuild_rows(state, &game, &sd.window, &slot);
    true
}

fn stored_metadata(state: &SharedState, db_id: i64) -> Option<ScraperMetadata> {
    ira_db::scraper_metadata_for_game(&state.borrow().db, db_id)
        .ok()
        .flatten()
}

/// Remove the slot's previous rows, then rebuild them from the database:
/// the entity fields, the editable facts, the synopsis, and the match
/// row last.
fn rebuild_rows(state: &SharedState, game: &Game, win: &adw::Window, slot: &ScraperSlot) {
    for widget in slot.rows.borrow().iter() {
        slot.group.remove(widget);
    }
    slot.rows.borrow_mut().clear();
    slot.match_row.borrow_mut().take();

    let metadata = stored_metadata(state, game.db_id).unwrap_or_default();

    for field in entity_fields() {
        let row = field_row(state, game, win, &field, &metadata);
        slot.add(row.upcast());
    }
    fact_rows(state, game, win, &metadata, slot);
    synopsis_row(state, game, win, &metadata.synopses, slot);
    let match_row = search_row(state, game, win);
    slot.add(match_row.clone().upcast());
    *slot.match_row.borrow_mut() = Some(match_row);
}/// Which kind of entity a metadata field collects — decides the search
/// source and the custom-entry factory.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EntityKind {
    Developer,
    Publisher,
    Genre,
}

/// Which metadata field a row edits: its display names (one entry reads
/// singular), the picker it opens, and the read/add/remove accessors.
#[derive(Clone)]
struct EntityField {
    label: String,
    singular: String,
    kind: EntityKind,
    get: fn(&ScraperMetadata) -> &Vec<ScraperEntity>,
    remove: fn(&mut ScraperMetadata, &ScraperEntity),
    push: fn(&mut ScraperMetadata, ScraperEntity),
}

impl EntityField {
    /// The picker's empty-state copy per kind.
    fn empty_text(&self) -> String {
        match self.kind {
            EntityKind::Developer | EntityKind::Publisher => {
                crate::tr!("Companies appear here as games get matched")
            }
            EntityKind::Genre => crate::tr!("No results found"),
        }
    }
}

fn entity_fields() -> [EntityField; 3] {
    [
        EntityField {
            label: crate::tr!("Developers"),
            singular: crate::tr!("Developer"),
            kind: EntityKind::Developer,
            get: |m| &m.developers,
            remove: |m, gone| m.developers.retain(|e| e.id != gone.id),
            push: |m, entity| m.developers.push(entity),
        },
        EntityField {
            label: crate::tr!("Publishers"),
            singular: crate::tr!("Publisher"),
            kind: EntityKind::Publisher,
            get: |m| &m.publishers,
            remove: |m, gone| m.publishers.retain(|e| e.id != gone.id),
            push: |m, entity| m.publishers.push(entity),
        },
        EntityField {
            label: crate::tr!("Genres"),
            singular: crate::tr!("Genre"),
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
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    field: &EntityField,
    metadata: &ScraperMetadata,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_use_markup(false);
    let names: Vec<String> = (field.get)(metadata)
        .iter()
        .map(|entity| entity.name.clone())
        .collect();
    row.set_title(if names.len() == 1 {
        &field.singular
    } else {
        &field.label
    });
    if names.is_empty() {
        row.set_subtitle(&crate::tr!("None"));
    } else {
        row.set_subtitle(&names.join(", "));
    }

    {
        let state = state.clone();
        let game = game.clone();
        let win = win.clone();
        let label = field.label.clone();
        row.add_suffix(&edit_button(move |_| {
            let Some(field) = entity_fields().into_iter().find(|f| f.label == label) else {
                return;
            };
            show_entity_edit_dialog(&state, &game, &win, field);
        }));
    }
    row
}

/// The flat pen button opening a row's editor.
fn edit_button(on_click: impl Fn(&gtk4::Button) + 'static) -> gtk4::Button {
    let button = gtk4::Button::from_icon_name("document-edit-symbolic");
    button.add_css_class(CSS_FLAT);
    button.set_valign(gtk4::Align::Center);
    button.set_tooltip_text(Some(&crate::tr!("Edit")));
    button.connect_clicked(on_click);
    button
}

/// The editable facts: release date, player count, rating and the age
/// boards, one row each, persisted the moment an edit applies. No
/// refresh afterwards — the rows already show what was typed, and a
/// rebuild would drop focus mid-edit.
fn fact_rows(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    metadata: &ScraperMetadata,
    slot: &ScraperSlot,
) {
    let db_id = game.db_id;
    release_date_row(state, game, &metadata.release_date, slot);
    text_row(
        slot,
        state,
        &crate::tr!("Players"),
        &metadata.players,
        "",
        move |state, text| {
            edit_field(state, db_id, |m| m.players = text.to_string());
        },
    );
    classifications_row(state, game, win, &metadata.classifications, slot);
    rating_row(state, slot, db_id, metadata.rating.max(0.0));
}

/// The stored release date as the row's subtitle, a calendar button
/// opening the picker — dates are picked, not typed.
fn release_date_row(
    state: &SharedState,
    game: &Game,
    release_date: &str,
    slot: &ScraperSlot,
) {
    let row = adw::ActionRow::new();
    row.set_title(&crate::tr!("Released"));
    if release_date.is_empty() {
        row.set_subtitle(&crate::tr!("Unknown"));
    } else {
        row.set_subtitle(release_date);
    }

    // libadwaita has no date picker of its own; the GNOME-apps pattern
    // is a GtkCalendar in a popover off the row's calendar button, which
    // also sizes itself to the content — no dead space.
    let pick = gtk4::MenuButton::new();
    pick.set_icon_name("x-office-calendar-symbolic");
    pick.add_css_class(CSS_FLAT);
    pick.set_valign(gtk4::Align::Center);
    pick.set_tooltip_text(Some(&crate::tr!("Pick a date")));

    let calendar = gtk4::Calendar::new();
    let timestamp = ira_db::scraper_release_timestamp(release_date);
    if timestamp > 0 {
        if let Some(date) = chrono::DateTime::from_timestamp(timestamp, 0) {
            use chrono::Datelike;
            if let Ok(preset) = glib::DateTime::from_utc(
                date.year(),
                date.month() as i32,
                date.day() as i32,
                0,
                0,
                0.0,
            ) {
                calendar.select_day(&preset);
            }
        }
    }
    let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let clear = gtk4::Button::with_label(&crate::tr!("Clear"));
    clear.add_css_class(CSS_FLAT);
    clear.set_halign(gtk4::Align::Start);
    clear.set_hexpand(true);
    let apply = gtk4::Button::with_label(&crate::tr!("Apply"));
    apply.add_css_class(CSS_SUGGESTED_ACTION);
    apply.set_halign(gtk4::Align::End);
    buttons.append(&clear);
    buttons.append(&apply);
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    content.set_margin_top(8);
    content.set_margin_bottom(8);
    content.set_margin_start(8);
    content.set_margin_end(8);
    content.append(&calendar);
    content.append(&buttons);
    let popover = gtk4::Popover::new();
    popover.set_child(Some(&content));
    pick.set_popover(Some(&popover));

    {
        let state = state.clone();
        let game = game.clone();
        let calendar = calendar.clone();
        let popover = popover.clone();
        apply.connect_clicked(move |_| {
            let Some(iso) = calendar
                .date()
                .format("%Y-%m-%d")
                .ok()
                .map(|s| s.to_string())
            else {
                return;
            };
            edit_field(&state, game.db_id, |m| {
                m.release_date = iso.clone();
                m.release_timestamp = ira_db::scraper_release_timestamp(&iso);
            });
            popover.popdown();
            refresh_scraper_section(&state, game.db_id);
        });
    }
    {
        let state = state.clone();
        let game = game.clone();
        let popover = popover.clone();
        clear.connect_clicked(move |_| {
            popover.popdown();
            edit_field(&state, game.db_id, |m| {
                m.release_date = String::new();
                m.release_timestamp = 0;
            });
            refresh_scraper_section(&state, game.db_id);
        });
    }
    row.add_suffix(&pick);
    slot.add(row.upcast());
}

/// The Age ratings entry: the title with its Edit… button on one line
/// and the official marks underneath, in stored order — the marks are
/// the point, so no subtitle text. Pairs without a bundled mark keep a
/// small text chip so they don't silently vanish from the summary.
fn classifications_row(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    classifications: &[ScraperClassification],
    slot: &ScraperSlot,
) {
    let outer = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    outer.set_margin_top(12);
    outer.set_margin_bottom(12);
    outer.set_margin_start(12);
    outer.set_margin_end(12);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    let title_text = if classifications.len() == 1 {
        crate::tr!("Age rating")
    } else {
        crate::tr!("Age ratings")
    };
    let title = gtk4::Label::new(Some(&title_text));
    title.set_halign(gtk4::Align::Start);
    title.set_hexpand(true);
    content.append(&title);

    let strip = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    for classification in classifications {
        match ira_images::rating_texture(&classification.kind, &classification.value, 28) {
            Some(texture) => {
                let mark = gtk4::Image::from_paintable(Some(&texture));
                mark.set_pixel_size(28);
                strip.append(&mark);
            }
            None => {
                let text = ira_models::ratings::display(
                    &classification.kind,
                    &classification.value,
                )
                .unwrap_or_else(|| {
                    format!("{} {}", classification.kind, classification.value)
                });
                let chip = gtk4::Label::new(Some(&text));
                chip.set_use_markup(false);
                chip.add_css_class(CSS_CAPTION);
                chip.add_css_class(CSS_DIM_LABEL);
                chip.set_valign(gtk4::Align::Center);
                strip.append(&chip);
            }
        }
    }
    content.append(&strip);
    outer.append(&content);

    {
        let state = state.clone();
        let game = game.clone();
        let win = win.clone();
        outer.append(&edit_button(move |_| {
            show_classifications_edit_dialog(&state, &game, &win);
        }));
    }

    box_row(slot, &outer);
}

/// Wrap custom row content in a real preferences row, so the group's
/// boxed-list styling (card, separators, radius) applies exactly like
/// it does to every other row here — flush with its neighbours.
fn box_row(slot: &ScraperSlot, content: &impl glib::object::IsA<gtk4::Widget>) {
    let row = adw::PreferencesRow::new();
    row.set_child(Some(content));
    row.set_focusable(false);
    slot.add(row.upcast());
}

/// Insert or replace a board: the junction's primary key is
/// (game, kind), so a second entry for the same board is an update, not
/// a duplicate. Kind folds to uppercase, the way the sources spell
/// them; empty halves store nothing.
fn upsert_classification(metadata: &mut ScraperMetadata, kind: &str, value: &str) {
    let kind = kind.trim().to_uppercase();
    let value = value.trim().to_string();
    if kind.is_empty() || value.is_empty() {
        return;
    }
    match metadata
        .classifications
        .iter_mut()
        .find(|c| c.kind.eq_ignore_ascii_case(&kind))
    {
        Some(existing) => existing.value = value,
        None => metadata
            .classifications
            .push(ScraperClassification { kind, value }),
    }
}

/// The nearest 0.5 inside 0..=10 — the rating editor's half-point snap.
fn snap_half_step(value: f64) -> f64 {
    ((value / 0.5).round() * 0.5).clamp(0.0, 10.0)
}

/// A text fact row: an entry seeded with the stored value; apply (enter
/// or the check button) persists through `store`.
fn text_row(
    slot: &ScraperSlot,
    state: &SharedState,
    label: &str,
    value: &str,
    hint: &str,
    store: impl Fn(&SharedState, &str) + 'static,
) -> adw::EntryRow {
    let row = adw::EntryRow::new();
    row.set_title(label);
    row.set_text(value);
    if !hint.is_empty() {
        // No subtitle on an entry row — the format hint rides on a
        // tooltip.
        row.set_tooltip_text(Some(hint));
    }
    let weak = row.downgrade();
    let state = state.clone();
    row.connect_apply(move |_| {
        let Some(row) = weak.upgrade() else {
            return;
        };
        let text = row.text().trim().to_string();
        store(&state, &text);
    });
    slot.add(row.clone().upcast());
    row
}

/// The rating, edited on a 0–10 scale with half-point steps — the
/// stored scale is 0–20, so the display halves and storing doubles.
/// Typed values snap to half points; the adjustment clamps the range.
/// The adjustment is seeded before the handler connects, so rebuilding
/// the rows never writes back what it just read.
fn rating_row(state: &SharedState, slot: &ScraperSlot, db_id: i64, stored: f64) -> adw::SpinRow {
    let adjustment = gtk4::Adjustment::new(stored / 2.0, 0.0, 10.0, 0.5, 1.0, 0.0);
    let row = adw::SpinRow::new(Some(&adjustment), 0.5, 1);
    row.set_title(&crate::tr!("Rating"));
    let weak_adjustment = adjustment.downgrade();
    let state = state.clone();
    adjustment.connect_value_changed(move |_| {
        let Some(adjustment) = weak_adjustment.upgrade() else {
            return;
        };
        let snapped = snap_half_step(adjustment.value());
        // A typed 7.3 re-renders as 7.5; the follow-up pass sees the
        // snapped value and stops here.
        if (adjustment.value() - snapped).abs() > f64::EPSILON {
            adjustment.set_value(snapped);
        }
        edit_field(&state, db_id, |m| m.rating = snapped * 2.0);
    });
    slot.add(row.clone().upcast());
    row
}

/// The per-field editor: a two-page dialog. The root page lists the
/// stored entities with remove buttons; the header's add button slides a
/// search page in from the left — where the add button sits — over the
/// same window: cached ScreenScraper companies/genres to pick, and a
/// custom row that mints the name as a local entry for studios and
/// categories the sources will never list.
fn show_entity_edit_dialog(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    field: EntityField,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&field.label);
    dialog.set_content_width(460);
    dialog.set_content_height(440);

    // Root page: the stored entities.
    let root_toolbar = adw::ToolbarView::new();
    let root_header = adw::HeaderBar::new();
    let root_title = gtk4::Label::new(Some(&field.label));
    root_title.add_css_class("heading");
    root_header.set_title_widget(Some(&root_title));
    let add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_btn.add_css_class(CSS_FLAT);
    add_btn.set_tooltip_text(Some(&crate::tr!("Add")));
    root_header.pack_start(&add_btn);
    root_toolbar.add_top_bar(&root_header);
    let (root_scrolled, root_list) = super::helpers::clamped_boxed_list(460);
    root_toolbar.set_content(Some(&root_scrolled));

    // Search page: the cache query plus a custom-add row, with an
    // explicit back button — a gtk Stack has none of NavigationView's.
    let search_toolbar = adw::ToolbarView::new();
    let search_header = adw::HeaderBar::new();
    let search_title = gtk4::Label::new(Some(&crate::tr!("Search")));
    search_title.add_css_class("heading");
    search_header.set_title_widget(Some(&search_title));
    let back_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
    back_btn.add_css_class(CSS_FLAT);
    back_btn.set_tooltip_text(Some(&crate::tr!("Back")));
    search_header.pack_start(&back_btn);
    search_toolbar.add_top_bar(&search_header);
    let search_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    search_box.set_margin_top(12);
    search_box.set_margin_bottom(12);
    search_box.set_margin_start(12);
    search_box.set_margin_end(12);
    let entry = gtk4::SearchEntry::new();
    entry.set_placeholder_text(Some(&crate::tr!("Search…")));
    search_box.append(&entry);
    let (results_scrolled, results_list) = super::helpers::clamped_boxed_list(460);
    results_scrolled.set_vexpand(true);
    search_box.append(&results_scrolled);
    search_toolbar.set_content(Some(&search_box));

    let stack = gtk4::Stack::new();
    stack.set_vhomogeneous(false);
    stack.add_named(&root_toolbar, Some("list"));
    stack.add_named(&search_toolbar, Some("search"));
    dialog.set_child(Some(&stack));

    fill_entity_list(&root_list, state, game, field.clone());

    {
        let state = state.clone();
        let game = game.clone();
        let field = field.clone();
        let root_list = root_list.clone();
        let results_list = results_list.clone();
        let entry = entry.clone();
        let stack = stack.clone();
        add_btn.connect_clicked(move |_| {
            populate_search_results(&results_list, &state, &game, &field, "", &root_list);
            stack.set_transition_type(gtk4::StackTransitionType::SlideRight);
            stack.set_visible_child(&search_toolbar);
            let entry = entry.clone();
            glib::idle_add_local_once(move || {
                entry.grab_focus();
            });
        });
    }
    {
        let state = state.clone();
        let game = game.clone();
        let field = field.clone();
        let results_list = results_list.clone();
        entry.connect_search_changed(move |entry| {
            populate_search_results(
                &results_list,
                &state,
                &game,
                &field,
                &entry.text(),
                &root_list,
            );
        });
    }
    {
        let stack = stack.clone();
        back_btn.connect_clicked(move |_| {
            stack.set_transition_type(gtk4::StackTransitionType::SlideLeft);
            stack.set_visible_child(&root_toolbar);
        });
    }

    let parent = win.clone();
    dialog.present(Some(&parent));
}

/// The age-rating editor: a two-page dialog like the entity pickers, but
/// with nothing to search — the root page lists the stored ratings with
/// remove buttons (their marks as prefixes), and the add page picks a
/// board and then one of its known values from two combo rows. Adding
/// keeps the page up so several ratings can go in.
fn show_classifications_edit_dialog(state: &SharedState, game: &Game, win: &adw::Window) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&crate::tr!("Age ratings"));
    dialog.set_content_width(460);
    dialog.set_content_height(320);

    // List page: every stored rating with its remove button.
    let list_toolbar = adw::ToolbarView::new();
    let list_header = adw::HeaderBar::new();
    let list_title = gtk4::Label::new(Some(&crate::tr!("Age ratings")));
    list_title.add_css_class("heading");
    list_header.set_title_widget(Some(&list_title));
    let add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_btn.add_css_class(CSS_FLAT);
    add_btn.set_tooltip_text(Some(&crate::tr!("Add")));
    list_header.pack_start(&add_btn);
    list_toolbar.add_top_bar(&list_header);
    let (list_scrolled, list) = super::helpers::clamped_boxed_list(460);
    list_toolbar.set_content(Some(&list_scrolled));

    // Add page: board and rating as two pickers over the hardcoded
    // value sets. There is no source to search, so no query entry.
    let boards: Vec<&ira_models::ratings::RatingBoard> =
        ira_models::ratings::BOARDS.iter().collect();
    let add_toolbar = adw::ToolbarView::new();
    let add_header = adw::HeaderBar::new();
    let add_title = gtk4::Label::new(Some(&crate::tr!("Add a rating")));
    add_title.add_css_class("heading");
    add_header.set_title_widget(Some(&add_title));
    let back_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
    back_btn.add_css_class(CSS_FLAT);
    back_btn.set_tooltip_text(Some(&crate::tr!("Back")));
    add_header.pack_start(&back_btn);
    add_toolbar.add_top_bar(&add_header);
    let add_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    add_box.set_margin_top(12);
    add_box.set_margin_bottom(12);
    add_box.set_margin_start(12);
    add_box.set_margin_end(12);
    let pickers = gtk4::ListBox::new();
    pickers.set_selection_mode(gtk4::SelectionMode::None);
    pickers.add_css_class(CSS_BOXED_LIST);
    let board_combo = adw::ComboRow::new();
    board_combo.set_title(&crate::tr!("Board"));
    board_combo.set_model(Some(&super::helpers::string_list_from(
        &boards
            .iter()
            .map(|board| format!("{} · {}", board.name, board.region))
            .collect::<Vec<_>>(),
    )));
    let value_combo = adw::ComboRow::new();
    value_combo.set_title(&crate::tr!("Rating"));
    fill_value_combo(&value_combo, boards[0]);
    {
        let value_combo = value_combo.clone();
        let boards = boards.clone();
        board_combo.connect_selected_notify(move |combo| {
            let board = boards[combo.selected() as usize];
            fill_value_combo(&value_combo, board);
        });
    }
    pickers.append(&board_combo);
    pickers.append(&value_combo);
    add_box.append(&pickers);
    let confirm = gtk4::Button::with_label(&crate::tr!("Add"));
    confirm.add_css_class(CSS_SUGGESTED_ACTION);
    confirm.set_halign(gtk4::Align::End);
    add_box.append(&confirm);
    add_toolbar.set_content(Some(&add_box));

    let stack = gtk4::Stack::new();
    stack.set_vhomogeneous(false);
    stack.add_named(&list_toolbar, Some("list"));
    stack.add_named(&add_toolbar, Some("add"));
    dialog.set_child(Some(&stack));

    fill_classification_list(&list, state, game);

    {
        let stack = stack.clone();
        let board_combo = board_combo.clone();
        add_btn.connect_clicked(move |_| {
            stack.set_transition_type(gtk4::StackTransitionType::SlideRight);
            stack.set_visible_child(&add_toolbar);
            let board_combo = board_combo.clone();
            glib::idle_add_local_once(move || {
                board_combo.grab_focus();
            });
        });
    }
    {
        let stack = stack.clone();
        back_btn.connect_clicked(move |_| {
            stack.set_transition_type(gtk4::StackTransitionType::SlideLeft);
            stack.set_visible_child(&list_toolbar);
        });
    }
    {
        let state = state.clone();
        let game = game.clone();
        let list = list.clone();
        let board_combo = board_combo.clone();
        let value_combo = value_combo.clone();
        confirm.connect_clicked(move |_| {
            let board = boards[board_combo.selected() as usize];
            let value_index = value_combo.selected() as usize;
            let Some(value) = board.values.get(value_index) else {
                return;
            };
            edit_field(&state, game.db_id, |metadata| {
                upsert_classification(metadata, board.id, value);
            });
            refresh_scraper_section(&state, game.db_id);
            fill_classification_list(&list, &state, &game);
        });
    }

    let parent = win.clone();
    dialog.present(Some(&parent));
}

/// The rating picker's second row: the chosen board's known values.
fn fill_value_combo(value_combo: &adw::ComboRow, board: &ira_models::ratings::RatingBoard) {
    value_combo.set_model(Some(&super::helpers::string_list_from(
        &board.values.iter().map(|v| v.to_string()).collect::<Vec<_>>(),
    )));
    value_combo.set_selected(0);
}

/// (Re)fill the editor's list page: every stored rating with a remove
/// button. Edits persist at once and refresh the Identity page's rows.
fn fill_classification_list(list: &gtk4::ListBox, state: &SharedState, game: &Game) {
    clear_children(list);
    let metadata = ira_db::scraper_metadata_for_game(&state.borrow().db, game.db_id)
        .ok()
        .flatten()
        .unwrap_or_default();
    if metadata.classifications.is_empty() {
        list.append(&status_row(&crate::tr!("No age ratings stored")));
        return;
    }
    for classification in &metadata.classifications {
        let row = adw::ActionRow::new();
        row.set_use_markup(false);
        // "STEAM_GERMANY 12" is really "USK 12" — display the board's
        // name and the canonical value spelling, raw text otherwise.
        let name = ira_models::ratings::display(
            &classification.kind,
            &classification.value,
        )
        .unwrap_or_else(|| {
            format!("{} {}", classification.kind, classification.value)
        });
        row.set_title(&name);
        if let Some(texture) =
            ira_images::rating_texture(&classification.kind, &classification.value, 24)
        {
            let mark = gtk4::Image::from_paintable(Some(&texture));
            mark.set_pixel_size(24);
            row.add_prefix(&mark);
        }
        let remove = gtk4::Button::from_icon_name("user-trash-symbolic");
        remove.add_css_class(CSS_FLAT);
        remove.set_valign(gtk4::Align::Center);
        {
            let state = state.clone();
            let game = game.clone();
            let list = list.clone();
            let kind = classification.kind.clone();
            remove.connect_clicked(move |_| {
                edit_field(&state, game.db_id, |metadata| {
                    metadata
                        .classifications
                        .retain(|c| !c.kind.eq_ignore_ascii_case(&kind));
                });
                refresh_scraper_section(&state, game.db_id);
                fill_classification_list(&list, &state, &game);
            });
        }
        row.add_suffix(&remove);
        list.append(&row);
    }
}

/// (Re)fill the root page's list: every stored entity with a remove
/// button. Edits persist at once and refresh the Identity page's rows.
fn fill_entity_list(
    list: &gtk4::ListBox,
    state: &SharedState,
    game: &Game,
    field: EntityField,
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
            let field = field.clone();
            remove.connect_clicked(move |_| {
                let field = field.clone();
                edit_field(&state, game.db_id, |metadata| {
                    (field.remove)(metadata, &entity);
                });
                refresh_scraper_section(&state, game.db_id);
                fill_entity_list(&list, &state, &game, field);
            });
        }
        row.add_suffix(&remove);
        list.append(&row);
    }
}

/// (Re)fill the search page for a term: the cache matches, then the
/// custom-add row for the term itself. Picking anything stores it and
/// refreshes the Identity page; the search page stays up so several
/// credits can be added in a row.
fn populate_search_results(
    list: &gtk4::ListBox,
    state: &SharedState,
    game: &Game,
    field: &EntityField,
    term: &str,
    root_list: &gtk4::ListBox,
) {
    clear_children(list);
    let term = term.trim();
    let store = {
        let state = state.clone();
        let game = game.clone();
        let field = field.clone();
        let root_list = root_list.clone();
        Rc::new(move |entity: ScraperEntity| {
            edit_field(&state, game.db_id, |metadata| {
                let entities = (field.get)(metadata);
                // The same company twice is noise, not data.
                if !entities.iter().any(|e| e.id == entity.id) {
                    (field.push)(metadata, entity.clone());
                }
            });
            refresh_scraper_section(&state, game.db_id);
            fill_entity_list(&root_list, &state, &game, field.clone());
        })
    };
    let mint_and_store = {
        let state = state.clone();
        let field = field.clone();
        let store = store.clone();
        Rc::new(move |term: &str| {
            // Minting happens only on click, never while typing — the
            // db must not fill up with every prefix the user tried.
            let entity = match field.kind {
                EntityKind::Developer | EntityKind::Publisher => {
                    ira_db::steam_company_entity(&state.borrow().db, term)
                }
                EntityKind::Genre => ira_db::local_genre_entity(&state.borrow().db, term),
            };
            if let Some(entity) = entity {
                store(entity);
            }
        })
    };

    let rows = match field.kind {
        EntityKind::Developer | EntityKind::Publisher => {
            ira_db::scraper_companies_search(&state.borrow().db, term).unwrap_or_default()
        }
        EntityKind::Genre => ira_db::search_genres(&state.borrow().db, term).unwrap_or_default(),
    };
    if rows.is_empty() && term.is_empty() {
        list.append(&status_row(&field.empty_text()));
        return;
    }
    for entity in &rows {
        let store = store.clone();
        let row_entity = entity.clone();
        list.append(&search_result_row(
            entity,
            Rc::new(move || store(row_entity.clone())),
        ));
    }
    // The term itself as a custom entry — a local company or genre when
    // the sources have no row for it, the cache's own when they do.
    // Nothing is minted here: allocation happens on click.
    if !term.is_empty() {
        let row = adw::ActionRow::new();
        row.set_use_markup(false);
        row.set_title(&crate::tr!("Add \"{}\"").replacen("{}", term, 1));
        let add = gtk4::Button::with_label(&crate::tr!("Add"));
        add.add_css_class(CSS_SUGGESTED_ACTION);
        add.set_valign(gtk4::Align::Center);
        {
            let mint_and_store = mint_and_store.clone();
            let term = term.to_string();
            add.connect_clicked(move |_| mint_and_store(&term));
        }
        row.add_suffix(&add);
        list.append(&row);
    }
}

/// One search answer row: picking it stores the entity.
fn search_result_row(entity: &ScraperEntity, on_store: Rc<dyn Fn()>) -> adw::ActionRow {
    match_result_row(&entity.name, &format!("id {}", entity.id), move || on_store())
}

fn search_row(state: &SharedState, game: &Game, win: &adw::Window) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    // Matched means an entry id is on record — a hand-filled record on
    // an unmatched game still gets the search/auto-match row below.
    let matched = !game.screenscraper_id.is_empty();
    if matched {
        // The entry's id is known, so gaps can be filled without any
        // rematch ambiguity: one exact fetch by id, merged over what's
        // stored. Unmatch stays for genuinely wrong matches.
        row.set_title(&crate::tr!("Matched"));
        let fetch = gtk4::Button::with_label(&crate::tr!("Fetch missing"));
        fetch.add_css_class(CSS_FLAT);
        fetch.set_valign(gtk4::Align::Center);
        {
            let state = state.clone();
            let game = game.clone();
            fetch.connect_clicked(move |btn| {
                btn.set_sensitive(false);
                let state = state.clone();
                let game = game.clone();
                run_refetch_missing(&state, &game, move |state, db_id, outcome| {
                    if refresh_scraper_section(state, db_id) {
                        let slot = state
                            .borrow()
                            .settings_data
                            .as_ref()
                            .and_then(|sd| sd.scraper_slot.clone());
                        if let Some(slot) = slot {
                            let text = match outcome {
                                RefetchOutcome::Filled => {
                                    crate::tr!("Filled the missing pieces")
                                }
                                RefetchOutcome::Unchanged => {
                                    crate::tr!("Everything was already stored")
                                }
                                RefetchOutcome::Failed(e) => {
                                    crate::tr!("Fetch failed: {}").replacen("{}", &e, 1)
                                }
                            };
                            slot.set_match_status(&text);
                        }
                    }
                });
            });
        }
        row.add_suffix(&fetch);
        let unmatch = gtk4::Button::with_label(&crate::tr!("Unmatch"));
        unmatch.add_css_class(CSS_DESTRUCTIVE_ACTION);
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
            let refresh: Rc<dyn Fn()> = Rc::new(move || {
                refresh_scraper_section(&refresh_state, db_id);
            });
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
/// and merge only what's missing into the stored metadata. Takes the
/// quota gate so a mass job and this never request the same entry twice;
/// off-thread, `on_done` runs on the main loop with the outcome.
fn run_refetch_missing(
    state: &SharedState,
    game: &Game,
    on_done: impl Fn(&SharedState, i64, RefetchOutcome) + 'static,
) {
    let db_id = game.db_id;
    if state.borrow().ss_job_busy.get() {
        on_done(
            state,
            db_id,
            RefetchOutcome::Failed(crate::tr!("Another metadata job is running").to_string()),
        );
        return;
    }
    state.borrow().ss_job_busy.set(true);
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
    let rx = spawn_refetch_worker(vec![db_id], steam, creds, db, None);
    let state = state.clone();
    poll_channel(rx, move |progress| {
        state.borrow().ss_job_busy.set(false);
        on_done(&state, db_id, progress.outcome);
    });
}

/// The synopsis: the English one when the sources sent several — every
/// language they sent stays stored, this row only edits the one on
/// display. Clicking toggles between the four-line summary and the full
/// text; the pen opens the text editor.
fn synopsis_row(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    synopses: &[(String, String)],
    slot: &ScraperSlot,
) {
    let Some((lang, text)) = synopses
        .iter()
        .find(|(langue, _)| langue == "en")
        .or_else(|| synopses.first())
        .cloned()
    else {
        return;
    };

    let row = adw::ActionRow::new();
    row.set_use_markup(false);
    row.set_title(&crate::tr!("Synopsis"));
    row.set_subtitle(&text);
    row.set_subtitle_lines(4);
    let expanded = std::cell::Cell::new(false);
    row.set_activatable(true);
    row.connect_activated(move |row| {
        expanded.set(!expanded.get());
        if expanded.get() {
            row.set_subtitle_lines(0);
        } else {
            row.set_subtitle_lines(4);
        }
    });
    {
        let state = state.clone();
        let game = game.clone();
        let win = win.clone();
        row.add_suffix(&edit_button(move |_| {
            show_synopsis_edit_dialog(&state, &game, &win, lang.clone(), text.clone());
        }));
    }
    slot.add(row.upcast());
}

/// The synopsis editor: the displayed language's text in a text view.
/// Applying it emptied removes the entry; the language's siblings stay
/// untouched.
fn show_synopsis_edit_dialog(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    lang: String,
    text: String,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&crate::tr!("Synopsis"));
    dialog.set_content_width(460);
    dialog.set_content_height(340);

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let title = gtk4::Label::new(Some(&crate::tr!("Synopsis")));
    title.add_css_class("heading");
    header.set_title_widget(Some(&title));
    // The apply lives in the header — nothing floating over dead space.
    let apply = gtk4::Button::with_label(&crate::tr!("Apply"));
    apply.add_css_class(CSS_SUGGESTED_ACTION);
    apply.set_valign(gtk4::Align::Center);
    header.pack_end(&apply);
    toolbar.add_top_bar(&header);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    // The framed, scrolling text view fills the dialog — a real editor
    // surface instead of a text field stranded in empty space.
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    let view = gtk4::TextView::new();
    view.set_wrap_mode(gtk4::WrapMode::WordChar);
    view.buffer().set_text(&text);
    scrolled.set_child(Some(&view));
    let frame = gtk4::Frame::new(None);
    frame.set_child(Some(&scrolled));
    content.append(&frame);
    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));

    {
        let state = state.clone();
        let game = game.clone();
        let dialog = dialog.clone();
        let view = view.clone();
        apply.connect_clicked(move |_| {
            let buffer = view.buffer();
            let (start, end) = buffer.bounds();
            let text = buffer.text(&start, &end, false).trim().to_string();
            dialog.close();
            edit_field(&state, game.db_id, |m| {
                if text.is_empty() {
                    m.synopses.retain(|(l, _)| *l != lang);
                } else {
                    match m.synopses.iter_mut().find(|(l, _)| *l == lang) {
                        Some(entry) => entry.1 = text.clone(),
                        None => m.synopses.push((lang.clone(), text.clone())),
                    }
                }
            });
            refresh_scraper_section(&state, game.db_id);
        });
    }

    dialog.present(Some(win));
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

#[cfg(test)]
mod tests {
    use super::{snap_half_step, upsert_classification};
    use ira_models::ScraperMetadata;

    #[test]
    fn test_upsert_classification_inserts_then_updates_by_kind() {
        let mut metadata = ScraperMetadata::default();
        upsert_classification(&mut metadata, "PEGI", "18");
        upsert_classification(&mut metadata, "ESRB", "M");
        assert_eq!(metadata.classifications.len(), 2);
        // A second entry for the same board replaces the value — the
        // junction's primary key is (game, kind).
        upsert_classification(&mut metadata, "pegi", "16");
        assert_eq!(metadata.classifications.len(), 2);
        assert_eq!(metadata.classifications[0].kind, "PEGI");
        assert_eq!(metadata.classifications[0].value, "16");
    }

    #[test]
    fn test_upsert_classification_trims_and_needs_both_halves() {
        let mut metadata = ScraperMetadata::default();
        upsert_classification(&mut metadata, "  CERO  ", " C ");
        assert_eq!(
            metadata.classifications.first().map(|c| (c.kind.as_str(), c.value.as_str())),
            Some(("CERO", "C"))
        );
        // Empty boards and empty values store nothing.
        upsert_classification(&mut metadata, "", "18");
        upsert_classification(&mut metadata, "   ", "18");
        upsert_classification(&mut metadata, "PEGI", "");
        upsert_classification(&mut metadata, "PEGI", "   ");
        assert_eq!(metadata.classifications.len(), 1);
    }

    #[test]
    fn test_snap_half_step_snaps_and_clamps() {
        assert_eq!(snap_half_step(7.3), 7.5);
        assert_eq!(snap_half_step(8.0), 8.0);
        assert_eq!(snap_half_step(0.24), 0.0);
        assert_eq!(snap_half_step(0.26), 0.5);
        // Out of range folds back onto the scale.
        assert_eq!(snap_half_step(-0.3), 0.0);
        assert_eq!(snap_half_step(10.4), 10.0);
    }
}
