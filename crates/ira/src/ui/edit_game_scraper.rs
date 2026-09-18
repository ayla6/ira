//! The Identity page's metadata rows: what a match stored, editable in
//! place — companies and genres through the cached pickers, the small
//! facts as entry and spin rows. Editors stage onto a draft; nothing
//! here touches the database until the game settings' Save runs
//! [`apply_scraper_draft`] — and a ScreenScraper pick, refetch or
//! unmatch stages the same way, so half-finished work never masquerades
//! as saved state. The match row itself lives in the Service group: it
//! wires the game to a service, it is not part of the game's record.

use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use super::css::*;
use super::edit_game_entity_dialog::show_entity_dialog;
use super::edit_game_metadata_editors::{
    show_classifications_edit_dialog, show_synopsis_edit_dialog,
};
use super::helpers::poll_channel;
use super::mass_match_ss::{run_pc_matching, PcMatchTarget, RefetchOutcome, SsOutcome};
use super::ss_match_dialog::{entry_title_trusted, show_ss_search_dialog, SsMatchSink};
use super::state::SharedState;
use crate::Game;
use ira_api::screenscraper::ScrapedGame;
use ira_api::ScraperCreds;
use ira_models::{ScraperClassification, ScraperEntity, ScraperMetadata};

/// A staged service change waiting for Save. The picked answer is kept
/// whole (boxed — the enum sits in the slot, not on the heap) so an
/// external refresh of the rows can fold it in again.
#[derive(Clone)]
pub(super) enum SsPending {
    None,
    Match(Box<ScrapedGame>),
    Unmatch,
}

/// Handle on the metadata rows living inside the Identity group, plus
/// the staged draft they edit against. Editors mutate the draft and
/// repaint the rows — nothing touches the database until the game
/// settings' Save runs [`apply_scraper_draft`], so half-finished edits
/// never masquerade as saved state.
#[derive(Clone)]
pub(crate) struct ScraperSlot {
    /// The Identity group: every metadata row lands here.
    group: adw::PreferencesGroup,
    /// The Service group: where the match row hangs — matching is a
    /// service wiring, not part of the game's own record.
    match_parent: adw::PreferencesGroup,
    rows: Rc<RefCell<Vec<gtk4::Widget>>>,
    match_row: Rc<RefCell<Option<adw::ActionRow>>>,
    pub(super) draft: Rc<RefCell<ScraperMetadata>>,
    pub(super) link: Rc<RefCell<String>>,
    pub(super) pending: Rc<RefCell<SsPending>>,
    /// The draft as it stood before a match was staged this session —
    /// what the revert buttons put back: the Steam garnish, the hand
    /// edits, whatever was there before the answer folded in.
    pub(super) pre_match: Rc<RefCell<Option<ScraperMetadata>>>,
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
    match_parent: &adw::PreferencesGroup,
) -> Option<ScraperSlot> {
    let metadata = stored_metadata(state, game.db_id);
    // Every game the SS pass can touch gets the rows: consoles on mapped
    // platforms, and PC games whose diff search fills them too. PS3/PS4
    // games carry a product code here, not a console.
    let console = ira_models::scraper_console_id(game.kind, &game.platform_id);
    let eligible =
        game.kind.is_pc() || ira_models::screenscraper_system_id(&console).is_some();
    if metadata.is_none() && !eligible {
        return None;
    }
    let slot = ScraperSlot {
        group: group.clone(),
        match_parent: match_parent.clone(),
        rows: Rc::new(RefCell::new(Vec::new())),
        match_row: Rc::new(RefCell::new(None)),
        draft: Rc::new(RefCell::new(metadata.clone().unwrap_or_default())),
        link: Rc::new(RefCell::new(
            game.steam_link_id.clone(),
        )),
        pending: Rc::new(RefCell::new(SsPending::None)),
        pre_match: Rc::new(RefCell::new(None)),
    };
    rebuild_rows(state, game, win, &slot);
    Some(slot)
}

/// Rebuild the rows in place after an external change (a batch match or
/// a refetch job wrote the database): the database wins and the draft
/// resyncs to it — except a match staged here this session, which folds
/// into the resynced record again so the user's unsaved pick survives.
/// `true` when the open settings window actually shows this game's rows.
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
    let Some(game) = state.borrow().find_game(db_id, None) else {
        return false;
    };
    let pending = slot.pending.borrow().clone();
    *slot.pre_match.borrow_mut() = None;
    *slot.draft.borrow_mut() = stored_metadata(state, db_id).unwrap_or_default();
    *slot.link.borrow_mut() = game.steam_link_id.clone();
    if let SsPending::Match(picked) = &pending {
        fold_match(state, &mut slot.draft.borrow_mut(), picked);
    }
    rebuild_rows(state, &game, &sd.window, &slot);
    true
}

/// Flush the staged draft — and any staged match or unmatch — to the
/// database at Save time. `true` when the metadata write succeeded.
pub(super) fn apply_scraper_draft(state: &SharedState, db_id: i64) -> bool {
    let Some(slot) = slot_for(state, db_id) else {
        return false;
    };
    let draft = slot.draft.borrow().clone();
    let link = slot.link.borrow().clone();
    let stored = ira_db::store_scraper_metadata(&state.borrow().db, db_id, &draft)
        .map_err(|e| eprintln!("Failed to store the edited metadata: {e}"))
        .is_ok();
    if ira_db::set_steam_link_id(&state.borrow().db, db_id, &link).is_err() {
        eprintln!("Failed to store the Steam link");
    }
    match slot.pending.borrow().clone() {
        SsPending::None => {}
        // The draft carried the picked id into the store above; what is
        // left is the miss marker and the in-memory copy of the game.
        SsPending::Match(_) => {
            if let Err(e) = ira_db::clear_scraper_miss(&state.borrow().db, db_id) {
                eprintln!("Failed to clear the ScreenScraper miss marker: {e}");
            }
            if let Some(g) = state
                .borrow_mut()
                .games
                .iter_mut()
                .find(|g| g.db_id == db_id)
            {
                g.screenscraper_id = draft.ss_id;
            }
        }
        SsPending::Unmatch => {
            if let Err(e) = ira_db::clear_screenscraper_match(&state.borrow().db, db_id) {
                eprintln!("Failed to unmatch: {e}");
            }
            // The user said no: no automatic pass may pick this game
            // again — the same promise the RA and SGDB unmatches make.
            if let Err(e) = ira_db::set_manual_unmatch(&state.borrow().db, db_id, true) {
                eprintln!("Failed to mark the game manually unmatched: {e}");
            }
            if let Some(g) = state
                .borrow_mut()
                .games
                .iter_mut()
                .find(|g| g.db_id == db_id)
            {
                g.screenscraper_id.clear();
                g.manual_unmatch = true;
            }
        }
    }
    stored
}

/// The open settings window's slot for this game, when it shows one.
fn slot_for(state: &SharedState, db_id: i64) -> Option<ScraperSlot> {
    let s = state.borrow();
    s.settings_data
        .as_ref()
        .filter(|sd| sd.db_id == db_id)
        .and_then(|sd| sd.scraper_slot.clone())
}

/// Rebuild the open settings window's rows for this game from the draft
/// — the staged truth — without consulting the database. `false` when
/// the window is gone, showing another game, or hidden.
fn repaint_slot(state: &SharedState, db_id: i64) -> bool {
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
    let Some(game) = state.borrow().find_game(db_id, None) else {
        return false;
    };
    rebuild_rows(state, &game, &sd.window, &slot);
    true
}

/// Fold a picked answer into a draft as *the match*: the source's merge
/// rules (`ScraperMetadata::merge_match`) — scalars won where the answer
/// has them, companies/genres/age ratings mixed in, never replaced. The
/// answer resolves through the alias table first, so a name the user
/// mapped to a canonical entry folds as that entry and the draft never
/// shows the alias as a second row.
fn fold_match(state: &SharedState, draft: &mut ScraperMetadata, picked: &ScrapedGame) {
    let db = state.borrow().db.clone();
    let timestamp = ira_db::scraper_release_timestamp(&picked.release_date);
    let mut fresh = picked.metadata(timestamp);
    ira_db::resolve_metadata_aliases(&db, &mut fresh);
    draft.merge_match(&fresh);
}

/// Stage a picked ScreenScraper match onto the dialog's draft: the
/// revert snapshot is taken once and the answer folds into the draft;
/// the title entry picks up the entry's name when the stored title came
/// from a dump and the user hasn't retyped it. Nothing is written until
/// Save.
fn stage_ss_match(state: &SharedState, db_id: i64, picked: &ScrapedGame, slot: &ScraperSlot) {
    let mut pre = slot.pre_match.borrow_mut();
    if pre.is_none() {
        *pre = Some(slot.draft.borrow().clone());
    }
    drop(pre);
    fold_match(state, &mut slot.draft.borrow_mut(), picked);
    *slot.pending.borrow_mut() = SsPending::Match(Box::new(picked.clone()));
    fill_title_entry(state, db_id, picked);
    repaint_slot(state, db_id);
    slot.set_match_status("");
}

/// Stage an unmatch. The id leaves the draft at once — the rows flip to
/// "not matched" — while the database keeps it until Save. Unmatching a
/// match that was only staged this session (the database never saw it)
/// undoes the staging whole, draft and all.
fn stage_ss_unmatch(state: &SharedState, db_id: i64, slot: &ScraperSlot) {
    let db_matched = ira_db::find_by_db_id(&state.borrow().db, db_id)
        .map(|entry| entry.is_some_and(|e| !e.screenscraper_id.is_empty()))
        .unwrap_or(false);
    if db_matched {
        slot.draft.borrow_mut().ss_id = String::new();
        *slot.pending.borrow_mut() = SsPending::Unmatch;
    } else {
        if let Some(pre) = slot.pre_match.borrow_mut().take() {
            *slot.draft.borrow_mut() = pre;
        }
        slot.draft.borrow_mut().ss_id = String::new();
        *slot.pending.borrow_mut() = SsPending::None;
    }
    repaint_slot(state, db_id);
    slot.set_match_status("");
}

/// Fold a refetched answer into the draft, gaps only — a refetch never
/// overwrites, staged or not. Reports whether anything was filled.
fn stage_refetch_fill(state: &SharedState, db_id: i64, picked: &ScrapedGame) -> bool {
    let Some(slot) = slot_for(state, db_id) else {
        return false;
    };
    let db = state.borrow().db.clone();
    let mut draft = slot.draft.borrow_mut();
    let timestamp = ira_db::scraper_release_timestamp(&picked.release_date);
    let mut fresh = picked.metadata(timestamp);
    ira_db::resolve_metadata_aliases(&db, &mut fresh);
    draft.fill_gaps(&fresh)
}

/// The match's name fills the title entry — visible, and still editable,
/// before Save — but only when the stored title came from a dump (the
/// same rule a persisted match renames under) and the entry still shows
/// it untouched.
fn fill_title_entry(state: &SharedState, db_id: i64, picked: &ScrapedGame) {
    if picked.name.is_empty() || entry_title_trusted(state, db_id).unwrap_or(true) {
        return;
    }
    let entry = {
        let s = state.borrow();
        let Some(sd) = s.settings_data.as_ref().filter(|sd| sd.db_id == db_id) else {
            return;
        };
        let current = s
            .find_game(db_id, None)
            .map(|g| g.name.clone())
            .unwrap_or_default();
        let entry = sd.title_entry.clone();
        if entry.text().trim() == current {
            Some(entry)
        } else {
            None
        }
    };
    if let Some(entry) = entry {
        entry.set_text(&picked.name);
    }
}

fn stored_metadata(state: &SharedState, db_id: i64) -> Option<ScraperMetadata> {
    ira_db::scraper_metadata_for_game(&state.borrow().db, db_id)
        .ok()
        .flatten()
}

/// Remove the slot's previous rows, then rebuild them from the draft:
/// the entity fields, the editable facts, the synopsis, and the match
/// row last — in the Service group, not the Identity one.
fn rebuild_rows(state: &SharedState, game: &Game, win: &adw::Window, slot: &ScraperSlot) {
    for widget in slot.rows.borrow().iter() {
        slot.group.remove(widget);
    }
    slot.rows.borrow_mut().clear();
    if let Some(old) = slot.match_row.borrow_mut().take() {
        slot.match_parent.remove(&old);
    }

    let metadata = slot.draft.borrow().clone();

    for field in entity_fields() {
        let row = field_row(state, game, win, slot, &field, &metadata);
        slot.add(row.upcast());
    }
    fact_rows(state, game, win, &metadata, slot);
    synopsis_row(state, game, win, slot, &metadata.synopses);
    let match_row = search_row(state, game, win, slot);
    slot.match_parent.add(&match_row.clone().upcast::<gtk4::Widget>());
    *slot.match_row.borrow_mut() = Some(match_row);
}

/// Repaint the Identity page's rows from the draft after a staged edit.
/// The pickers and editors only mutate the draft — without this the
/// group's field rows keep their old subtitles until the dialog reopens.
/// Never call from inside a fact row's own handler (the players entry,
/// the rating spin): those rows show their edit already, and a rebuild
/// would pull the widget out from under the user.
pub(super) fn refresh_rows(state: &SharedState, game: &Game, win: &adw::Window, slot: &ScraperSlot) {
    rebuild_rows(state, game, win, slot);
}

/// Rebuild a slot's rows once the settings page has finished assembling
/// the Service group: the match row re-appends to the group's end on
/// every rebuild, so attaching it before the group's own rows exist
/// would make its first repaint jump it from the top to the bottom
/// mid-session. Calling this puts the row where every rebuild keeps it.
pub(super) fn rebuild_scraper_rows(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    slot: &ScraperSlot,
) {
    rebuild_rows(state, game, win, slot);
}
/// Which kind of entity a metadata field collects — decides the search
/// source and the custom-entry factory.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EntityKind {
    Developer,
    Publisher,
    Genre,
    Family,
}

/// Which metadata field a row edits: its display names (one entry reads
/// singular), the picker it opens, and the read/add/remove/restore
/// accessors.
#[derive(Clone)]
pub(super) struct EntityField {
    pub(super) label: String,
    pub(super) singular: String,
    pub(super) kind: EntityKind,
    pub(super) get: fn(&ScraperMetadata) -> &Vec<ScraperEntity>,
    pub(super) remove: fn(&mut ScraperMetadata, &ScraperEntity),
    pub(super) push: fn(&mut ScraperMetadata, ScraperEntity),
    /// The revert button's restore: the whole list goes back to the
    /// pre-match snapshot's.
    pub(super) set: fn(&mut ScraperMetadata, Vec<ScraperEntity>),
}

impl EntityField {
    /// The picker's empty-state copy per kind.
    pub(super) fn empty_text(&self) -> String {
        match self.kind {
            EntityKind::Developer | EntityKind::Publisher => {
                crate::tr!("Companies appear here as games get matched")
            }
            EntityKind::Family => crate::tr!("Families appear here as games get matched"),
            EntityKind::Genre => crate::tr!("No results found"),
        }
    }
}

fn entity_fields() -> [EntityField; 4] {
    [
        EntityField {
            label: crate::tr!("Developers"),
            singular: crate::tr!("Developer"),
            kind: EntityKind::Developer,
            get: |m| &m.developers,
            remove: |m, gone| m.developers.retain(|e| e.id != gone.id),
            push: |m, entity| m.developers.push(entity),
            set: |m, list| m.developers = list,
        },
        EntityField {
            label: crate::tr!("Publishers"),
            singular: crate::tr!("Publisher"),
            kind: EntityKind::Publisher,
            get: |m| &m.publishers,
            remove: |m, gone| m.publishers.retain(|e| e.id != gone.id),
            push: |m, entity| m.publishers.push(entity),
            set: |m, list| m.publishers = list,
        },
        EntityField {
            label: crate::tr!("Genres"),
            singular: crate::tr!("Genre"),
            kind: EntityKind::Genre,
            get: |m| &m.genres,
            remove: |m, gone| m.genres.retain(|e| e.id != gone.id),
            push: |m, entity| m.genres.push(entity),
            set: |m, list| m.genres = list,
        },
        EntityField {
            label: crate::tr!("Families"),
            singular: crate::tr!("Family"),
            kind: EntityKind::Family,
            get: |m| &m.families,
            remove: |m, gone| m.families.retain(|e| e.id != gone.id),
            push: |m, entity| m.families.push(entity),
            set: |m, list| m.families = list,
        },
    ]
}

/// Whether two entity lists hold different content — the revert button's
/// changed test for the company and genre rows.
fn entities_differ(a: &[ScraperEntity], b: &[ScraperEntity]) -> bool {
    a.len() != b.len()
        || a.iter()
            .any(|e| !b.iter().any(|o| o.id == e.id && o.name == e.name))
}

/// One row per metadata field: the stored names as the subtitle, an Edit
/// button opening the compact remove/add dialog and — when a staged
/// match changed the field — an undo button putting the pre-match list
/// back. Works on empty metadata too — edits create the record, so an
/// unmatched game can be filled by hand before any match exists.
fn field_row(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    slot: &ScraperSlot,
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
        let slot = slot.clone();
        let slot_for_dialog = slot.clone();
        let label = field.label.clone();
        row.add_suffix(&edit_button(move |_| {
            let Some(field) = entity_fields().into_iter().find(|f| f.label == label) else {
                return;
            };
            show_entity_dialog(&state, &game, &win, &slot_for_dialog, field);
        }));
    }
    if let Some(revert) = revert_button(
        state,
        game,
        win,
        slot,
        {
            let field = field.clone();
            move |pre, cur| entities_differ((field.get)(pre), (field.get)(cur))
        },
        {
            let field = field.clone();
            move |draft, pre| (field.set)(draft, (field.get)(pre).clone())
        },
    ) {
        row.add_suffix(&revert);
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

/// The undo button a staged match adds to the rows it changed: one click
/// puts that field back to the pre-match snapshot — the Steam synopsis,
/// the hand-picked genres — while the match itself stays staged. `None`
/// when nothing is staged or the field matches the snapshot already.
fn revert_button(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    slot: &ScraperSlot,
    changed: impl Fn(&ScraperMetadata, &ScraperMetadata) -> bool + 'static,
    restore: impl Fn(&mut ScraperMetadata, &ScraperMetadata) + 'static,
) -> Option<gtk4::Button> {
    let pre = slot.pre_match.borrow().clone()?;
    if !changed(&slot.draft.borrow(), &pre) {
        return None;
    }
    let button = gtk4::Button::from_icon_name("edit-undo-symbolic");
    button.add_css_class(CSS_FLAT);
    button.set_valign(gtk4::Align::Center);
    button.set_tooltip_text(Some(&crate::tr!("Revert to the value before the match")));
    let (state, game, win, slot) = (state.clone(), game.clone(), win.clone(), slot.clone());
    button.connect_clicked(move |_| {
        let Some(pre) = slot.pre_match.borrow().clone() else {
            return;
        };
        edit_field(&slot, |draft| restore(draft, &pre));
        refresh_rows(&state, &game, &win, &slot);
    });
    Some(button)
}

/// The editable facts: release date, player count, rating and the age
/// boards, one row each, staged onto the draft like every other edit
/// here. No refresh afterwards — the rows already show what was typed,
/// and a rebuild would drop focus mid-edit.
fn fact_rows(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    metadata: &ScraperMetadata,
    slot: &ScraperSlot,
) {
    release_date_row(state, game, win, &metadata.release_date, slot);
    text_row(
        slot,
        &crate::tr!("Players"),
        &metadata.players,
        "",
        move |text, m| m.players = text.to_string(),
    );
    classifications_row(state, game, win, &metadata.classifications, slot);
    rating_row(slot, metadata.rating.max(0.0));
}

/// The stored release date as the row's subtitle, a calendar button
/// opening the picker — dates are picked, not typed.
fn release_date_row(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
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
        let slot = slot.clone();
        let calendar = calendar.clone();
        let popover = popover.clone();
        let (state, game, win) = (state.clone(), game.clone(), win.clone());
        apply.connect_clicked(move |_| {
            let Some(iso) = calendar
                .date()
                .format("%Y-%m-%d")
                .ok()
                .map(|s| s.to_string())
            else {
                return;
            };
            edit_field(&slot, |m| {
                m.release_date = iso.clone();
                m.release_timestamp = ira_db::scraper_release_timestamp(&iso);
            });
            popover.popdown();
            refresh_rows(&state, &game, &win, &slot);
        });
    }
    {
        let slot = slot.clone();
        let popover = popover.clone();
        let (state, game, win) = (state.clone(), game.clone(), win.clone());
        clear.connect_clicked(move |_| {
            popover.popdown();
            edit_field(&slot, |m| {
                m.release_date = String::new();
                m.release_timestamp = 0;
            });
            refresh_rows(&state, &game, &win, &slot);
        });
    }
    row.add_suffix(&pick);
    slot.add(row.upcast());
}

/// The Age ratings entry: the title with its Edit… (and, after a staged
/// match, revert) buttons on one line and the official marks underneath,
/// in stored order — the marks are the point, so no subtitle text. Pairs
/// without a bundled mark keep a small text chip so they don't silently
/// vanish from the summary.
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
        let slot = slot.clone();
        outer.append(&edit_button(move |_| {
            show_classifications_edit_dialog(&state, &game, &win, &slot);
        }));
    }
    if let Some(revert) = revert_button(
        state,
        game,
        win,
        slot,
        |pre, cur| pre.classifications_differ(cur),
        |draft, pre| draft.classifications = pre.classifications.clone(),
    ) {
        outer.append(&revert);
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

/// The nearest 0.5 inside 0..=10 — the rating editor's half-point snap.
fn snap_half_step(value: f64) -> f64 {
    ((value / 0.5).round() * 0.5).clamp(0.0, 10.0)
}

/// A text fact row: an entry seeded with the stored value; apply (enter
/// or the check button) stages through the draft.
fn text_row(
    slot: &ScraperSlot,
    label: &str,
    value: &str,
    hint: &str,
    store: impl Fn(&str, &mut ScraperMetadata) + 'static,
) -> adw::EntryRow {
    let row = adw::EntryRow::new();
    row.set_title(label);
    row.set_text(value);
    if !hint.is_empty() {
        // No subtitle on an entry row — the format hint rides on a
        // tooltip.
        row.set_tooltip_text(Some(hint));
    }
    let row_for_add = row.clone();
    let draft = std::rc::Rc::clone(&slot.draft);
    row.connect_apply(move |row| {
        let text = row.text().trim().to_string();
        mutate_draft(&draft, |m| store(&text, m));
    });
    slot.add(row_for_add.upcast());
    row
}

/// The rating, edited on a 0–10 scale with half-point steps — the
/// stored scale is 0–20, so the display halves and storing doubles.
/// Typed values snap to half points; the adjustment clamps the range.
/// The adjustment is seeded before the handler connects, so rebuilding
/// the rows never writes back what it just read.
fn rating_row(slot: &ScraperSlot, stored: f64) -> adw::SpinRow {
    let adjustment = gtk4::Adjustment::new(stored / 2.0, 0.0, 10.0, 0.5, 1.0, 0.0);
    let row = adw::SpinRow::new(Some(&adjustment), 0.5, 1);
    row.set_title(&crate::tr!("Rating"));
    let weak_adjustment = adjustment.downgrade();
    let slot_for_closure = slot.clone();
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
        edit_field(&slot_for_closure, |m| m.rating = snapped * 2.0);
    });
    slot.add(row.clone().upcast());
    row
}

/// The synopsis: the English one when the sources sent several — every
/// language they sent stays stored, this row only edits the one on
/// display. Clicking toggles between the four-line summary and the full
/// text; the pen opens the text editor, and a staged match that brought
/// its own text adds the revert button beside it.
fn synopsis_row(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    slot: &ScraperSlot,
    synopses: &[(String, String)],
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
        let (state, game, win, slot) = (state.clone(), game.clone(), win.clone(), slot.clone());
        row.add_suffix(&edit_button(move |_| {
            show_synopsis_edit_dialog(&state, &game, &win, &slot, lang.clone(), text.clone());
        }));
    }
    if let Some(revert) = revert_button(
        state,
        game,
        win,
        slot,
        |pre, cur| pre.synopses_differ(cur),
        |draft, pre| draft.synopses = pre.synopses.clone(),
    ) {
        row.add_suffix(&revert);
    }
    slot.add(row.upcast());
}

/// Stage a mutation directly on a draft; display is refreshed by the
/// caller. A store failure surfaces at Save, not here.
fn mutate_draft(draft: &RefCell<ScraperMetadata>, mutate: impl FnOnce(&mut ScraperMetadata)) {
    mutate(&mut draft.borrow_mut());
}

/// Stage a metadata change on the slot's draft — the Save button is what
/// writes the database.
pub(super) fn edit_field(slot: &ScraperSlot, mutate: impl FnOnce(&mut ScraperMetadata)) {
    mutate(&mut slot.draft.borrow_mut());
}

fn search_row(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    slot: &ScraperSlot,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    // Matched means an entry id is on record — in the draft, so a match
    // staged this session reads as matched before Save too. A
    // hand-filled record on an unmatched game still gets the
    // search/auto-match row below.
    let ss_id = slot.draft.borrow().ss_id.clone();
    let matched = !ss_id.is_empty();
    // The row names the service; the state rides in the subtitle, like
    // the Steam row next to it.
    row.set_title(&crate::tr!("ScreenScraper"));
    if matched {
        // The entry's id is known, so gaps can be filled without any
        // rematch ambiguity: one exact fetch by id, merged over what's
        // stored. Unmatch stays for genuinely wrong matches.
        row.set_subtitle(&crate::tr!("Matched · ID {}").replacen("{}", &ss_id, 1));
        // The entry's page on the site — what got matched, one click away.
        let open = gtk4::Button::from_icon_name("adw-external-link-symbolic");
        open.add_css_class(CSS_FLAT);
        open.set_tooltip_text(Some(&crate::tr!("Open the ScreenScraper page")));
        open.set_valign(gtk4::Align::Center);
        {
            let uri = ira_api::screenscraper::game_page_url(&ss_id);
            open.connect_clicked(move |btn| {
                super::helpers::open_uri(btn.upcast_ref(), &uri);
            });
        }
        row.add_suffix(&open);
        let fetch = gtk4::Button::with_label(&crate::tr!("Fetch missing"));
        fetch.add_css_class(CSS_FLAT);
        fetch.set_valign(gtk4::Align::Center);
        {
            let state = state.clone();
            let game = game.clone();
            let slot = slot.clone();
            fetch.connect_clicked(move |btn| {
                if !state.borrow().cfg.screenscraper_enabled {
                    slot.set_match_status(&crate::tr!("ScreenScraper is disabled in Settings"));
                    return;
                }
                btn.set_sensitive(false);
                let state = state.clone();
                let game = game.clone();
                let db_id = game.db_id;
                run_refetch_missing(&state, &game, move |state, outcome| {
                    // Every outcome repaints: a fill shows the merged
                    // rows, and Unchanged/Failed re-enable the button.
                    repaint_slot(state, db_id);
                    let slot = state
                        .borrow()
                        .settings_data
                        .as_ref()
                        .and_then(|sd| sd.scraper_slot.clone());
                    if let Some(slot) = slot {
                        let text = match outcome {
                            RefetchOutcome::Filled => crate::tr!("Filled the missing pieces"),
                            RefetchOutcome::Unchanged => {
                                crate::tr!("Everything was already stored")
                            }
                            RefetchOutcome::Failed(e) => {
                                crate::tr!("Fetch failed: {}").replacen("{}", &e, 1)
                            }
                        };
                        slot.set_match_status(&text);
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
            let slot = slot.clone();
            unmatch.connect_clicked(move |_| {
                stage_ss_unmatch(&state, db_id, &slot);
            });
        }
        row.add_suffix(&unmatch);
        return row;
    }

    // A Steam-backed PC game can run the whole automated matching —
    // system search, cross-platform diff, garnish — from here, without
    // waiting for the next mass-matcher opening.
    let auto_matchable = game.kind.is_pc() && game.platform_id.parse::<u32>().is_ok();
    row.set_subtitle(&crate::tr!("Not matched"));

    if auto_matchable {
        let btn = gtk4::Button::with_label(&crate::tr!("Auto match"));
        btn.add_css_class(CSS_SUGGESTED_ACTION);
        btn.set_valign(gtk4::Align::Center);
        {
            let state = state.clone();
            let game = game.clone();
            let slot = slot.clone();
            btn.connect_clicked(move |btn| {
                if !state.borrow().cfg.screenscraper_enabled {
                    slot.set_match_status(&crate::tr!("ScreenScraper is disabled in Settings"));
                    return;
                }
                btn.set_sensitive(false);
                let state = state.clone();
                let game = game.clone();
                run_auto_match(&state, &game, &slot);
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
        let platform_id = ira_models::scraper_console_id(game.kind, &game.platform_id);
        let db_id = game.db_id;
        let win = win.clone();
        let slot = slot.clone();
        btn.connect_clicked(move |_| {
            let dialog_state = state.clone();
            let stage_slot = slot.clone();
            let sink = SsMatchSink::Stage(Rc::new(
                move |st: &SharedState, id: i64, picked: &ScrapedGame| {
                    stage_ss_match(st, id, picked, &stage_slot);
                },
            ));
            show_ss_search_dialog(
                &dialog_state,
                db_id,
                &name,
                &platform_id,
                &win,
                sink,
                None,
            );
        });
    }
    row.add_suffix(&btn);
    row
}

/// The automated PC matching, off-thread: resolve, then stage the hit on
/// the dialog's draft and report on the UI loop. A miss repaints too —
/// the fresh Auto match button is the way to answer it.
fn run_auto_match(state: &SharedState, game: &Game, slot: &ScraperSlot) {
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
    let (tx, rx) = std::sync::mpsc::channel::<Option<ScrapedGame>>();
    std::thread::spawn(move || {
        let target = PcMatchTarget {
            kind,
            platform_id: &platform_id,
            title: &title,
            display: &display,
            db_id,
        };
        let matched = match run_pc_matching(&steam, &creds, &db, &target) {
            SsOutcome::Hit(game) => Some(*game),
            _ => None,
        };
        let _ = tx.send(matched);
    });
    let state = state.clone();
    let slot = slot.clone();
    poll_channel(rx, move |matched| {
        match matched.as_ref() {
            Some(picked) => stage_ss_match(&state, db_id, picked, &slot),
            None => {
                repaint_slot(&state, db_id);
                slot.set_match_status(&crate::tr!("No ScreenScraper entry found"));
            }
        }
    });
}

/// Re-fetch the matched entry by its own id — no search, no ambiguity —
/// and merge only what's missing into the draft; Save is what writes.
/// Takes the quota gate so a mass job and this never request the same
/// entry twice; off-thread, `on_done` runs on the main loop.
fn run_refetch_missing(
    state: &SharedState,
    game: &Game,
    on_done: impl Fn(&SharedState, RefetchOutcome) + 'static,
) {
    let db_id = game.db_id;
    if state.borrow().ss_job_busy.get() {
        on_done(
            state,
            RefetchOutcome::Failed(crate::tr!("Another metadata job is running").to_string()),
        );
        return;
    }
    // The staged id when a match just landed here, the stored one
    // otherwise — same value unless Save has not run yet.
    let ss_id = {
        let s = state.borrow();
        let staged = s
            .settings_data
            .as_ref()
            .filter(|sd| sd.db_id == db_id)
            .and_then(|sd| sd.scraper_slot.clone())
            .map(|slot| slot.draft.borrow().ss_id.clone())
            .filter(|id| !id.is_empty());
        staged.unwrap_or_else(|| game.screenscraper_id.clone())
    };
    if ss_id.is_empty() {
        on_done(
            state,
            RefetchOutcome::Failed("no ScreenScraper id".to_string()),
        );
        return;
    }
    state.borrow().ss_job_busy.set(true);
    let (steam, creds) = {
        let s = state.borrow();
        (
            s.steam.clone(),
            ScraperCreds::from_account(
                s.cfg.screenscraper_id.clone(),
                s.cfg.screenscraper_password.clone(),
            ),
        )
    };
    let (tx, rx) = std::sync::mpsc::channel::<Result<ScrapedGame, String>>();
    std::thread::spawn(move || {
        let outcome = steam
            .screenscraper_game(&creds, &ss_id)
            .and_then(|games| {
                games
                    .into_iter()
                    .next()
                    .ok_or_else(|| "entry not found".to_string())
            });
        let _ = tx.send(outcome);
    });
    let state = state.clone();
    poll_channel(rx, move |outcome| {
        state.borrow().ss_job_busy.set(false);
        let result = match outcome {
            Ok(picked) => {
                if stage_refetch_fill(&state, db_id, &picked) {
                    RefetchOutcome::Filled
                } else {
                    RefetchOutcome::Unchanged
                }
            }
            Err(e) => RefetchOutcome::Failed(e),
        };
        on_done(&state, result);
    });
}

#[cfg(test)]
mod tests {
    use super::snap_half_step;

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
