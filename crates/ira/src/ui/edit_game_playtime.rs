//! The game settings' playtime page: import hours earned outside Ira — a
//! previous launcher, or the Switch version of a game also here — and link
//! entries of the same game so their time and sessions count together.

use super::helpers::{clear_children, display_playtime, refresh_playtime_links};
use super::state::SharedState;
use crate::Game;
use adw::prelude::*;
use ira_models::GameEntry;
use std::cell::RefCell;
use std::rc::Rc;

/// A picker's `(db_id, check)` pairs to read answers from.
type GameChecks = Rc<RefCell<Vec<(i64, gtk4::CheckButton)>>>;

/// The sidebar page: the total (combined when linked), manual imports, and
/// the entry's playtime links. Rebuilds in place after each action.
pub(super) fn build_playtime_page(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    sidebar: &gtk4::ListBox,
    stack: &gtk4::Stack,
) {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    refill_playtime_page(&page, state, game, win);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&page));
    scroll.set_vexpand(true);
    scroll.set_hexpand(true);

    sidebar.append(&super::settings_pages::settings_sidebar_row(
        "document-open-recent-symbolic",
        &crate::tr!("Playtime"),
        "playtime",
    ));
    stack.add_named(&scroll, Some("playtime"));
}

fn refill_playtime_page(page: &gtk4::Box, state: &SharedState, game: &Game, win: &adw::Window) {
    clear_children(page);
    let db = state.borrow().db.clone();
    let members = ira_db::link_members(&db, game.db_id).unwrap_or_default();
    let others: Vec<i64> = members
        .iter()
        .filter(|id| **id != game.db_id)
        .copied()
        .collect();

    let total_group = adw::PreferencesGroup::new();
    total_group.set_title(&crate::tr!("Playtime"));
    let total_row = adw::ActionRow::new();
    total_row.set_title(&crate::tr!("Total playtime"));
    total_row.set_subtitle(&super::game_display::format_playtime(display_playtime(
        state, game,
    )));
    total_group.add(&total_row);
    total_group.add(&import_row(page, state, game, win));
    page.append(&total_group);

    let link_group = adw::PreferencesGroup::new();
    link_group.set_title(&crate::tr!("Linked games"));
    link_group.set_description(Some(&crate::tr!(
        "Entries of the same game from different sources share their totals and session history"
    )));
    link_group.add(&link_more_row(page, state, game, win, &members));
    for member in others {
        if let Ok(Some(entry)) = ira_db::find_by_db_id(&db, member) {
            link_group.add(&member_row(page, state, game, win, &entry));
        }
    }
    if !members.is_empty() {
        link_group.add(&leave_row(page, state, game, win));
    }
    page.append(&link_group);
}

/// The manual import row: hours typed, added to the total on the spot.
/// Unlike session time the import has no session row behind it, so it
/// never shows up in the play history chart.
fn import_row(
    page: &gtk4::Box,
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&crate::tr!("Add playtime"));
    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some(&crate::tr!("Hours, like 2:30 or 12.5")));
    entry.set_width_chars(12);
    entry.set_valign(gtk4::Align::Center);
    let add = gtk4::Button::with_label(&crate::tr!("Add"));
    add.set_valign(gtk4::Align::Center);
    let sync = {
        let entry = entry.clone();
        let add = add.clone();
        move || add.set_sensitive(parse_playtime_input(&entry.text()).is_some())
    };
    entry.connect_changed({
        let sync = sync.clone();
        move |_| sync()
    });
    sync();
    let page_weak = page.downgrade();
    let state = state.clone();
    let win_weak = win.downgrade();
    let db_id = game.db_id;
    let apply = {
        let entry = entry.clone();
        move || {
            apply_import(&state, db_id, &entry.text(), &page_weak, &win_weak);
        }
    };
    add.connect_clicked({
        let apply = apply.clone();
        move |_| apply()
    });
    entry.connect_activate(move |_| apply());
    row.add_suffix(&entry);
    row.add_suffix(&add);
    row
}

/// Add the typed hours, then refresh the page and the app's views.
fn apply_import(
    state: &SharedState,
    db_id: i64,
    text: &str,
    page: &glib::WeakRef<gtk4::Box>,
    win: &glib::WeakRef<adw::Window>,
) {
    let Some(hours) = parse_playtime_input(text) else {
        return;
    };
    let db = state.borrow().db.clone();
    if let Err(e) = ira_db::add_playtime(&db, db_id, hours) {
        eprintln!("Failed to add playtime: {e}");
        return;
    }
    after_playtime_change(state, &[db_id]);
    if let (Some(page), Some(win)) = (page.upgrade(), win.upgrade()) {
        refill_playtime_page(&page, state, &reload_self(state, db_id), &win);
    }
}

/// The row that opens the picker: "Link with other games" while unlinked,
/// "Link more games" once the entry is in a link.
fn link_more_row(
    page: &gtk4::Box,
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    members: &[i64],
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    if members.is_empty() {
        row.set_title(&crate::tr!("Link with other games"));
    } else {
        row.set_title(&crate::tr!("Link more games"));
    }
    let btn = gtk4::Button::with_label(&crate::tr!("Link"));
    btn.set_valign(gtk4::Align::Center);
    let page_weak = page.downgrade();
    let state = state.clone();
    let win_weak = win.downgrade();
    let members = members.to_vec();
    let db_id = game.db_id;
    btn.connect_clicked(move |_| {
        if let Some(win) = win_weak.upgrade() {
            show_link_games_dialog(
                &state,
                db_id,
                win.downcast_ref::<adw::Window>().expect("parent window"),
                &members,
                &page_weak,
            );
        }
    });
    row.add_suffix(&btn);
    row
}

/// One linked entry with its identity, own time, and an unlink button.
fn member_row(
    page: &gtk4::Box,
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    entry: &GameEntry,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_use_markup(false);
    row.set_title(&entry.title);
    row.set_subtitle(&format!(
        "{}  ·  {}",
        identity_line(entry),
        crate::tr!("{} on its own")
            .replacen("{}", &super::game_display::format_playtime(entry.playtime), 1)
    ));
    let remove = gtk4::Button::from_icon_name("edit-delete-symbolic");
    remove.add_css_class("flat");
    remove.set_valign(gtk4::Align::Center);
    remove.set_tooltip_text(Some(&crate::tr!("Remove from link")));
    let page_weak = page.downgrade();
    let state = state.clone();
    let win_weak = win.downgrade();
    let member = entry.id;
    let db_id = game.db_id;
    remove.connect_clicked(move |_| {
        let db = state.borrow().db.clone();
        if let Err(e) = ira_db::unlink_game(&db, member) {
            eprintln!("Failed to unlink game: {e}");
            return;
        }
        after_playtime_change(&state, &[member, db_id]);
        if let (Some(page), Some(win)) = (page_weak.upgrade(), win_weak.upgrade()) {
            refill_playtime_page(&page, &state, &reload_self(&state, db_id), &win);
        }
    });
    row.add_suffix(&remove);
    row
}

/// Stop counting this entry together with its link.
fn leave_row(page: &gtk4::Box, state: &SharedState, game: &Game, win: &adw::Window) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&crate::tr!("Leave link"));
    row.set_subtitle(&crate::tr!("Count this entry's time on its own again"));
    let btn = gtk4::Button::with_label(&crate::tr!("Leave"));
    btn.set_valign(gtk4::Align::Center);
    let page_weak = page.downgrade();
    let state = state.clone();
    let win_weak = win.downgrade();
    let db_id = game.db_id;
    btn.connect_clicked(move |_| {
        let db = state.borrow().db.clone();
        if let Err(e) = ira_db::unlink_game(&db, db_id) {
            eprintln!("Failed to unlink game: {e}");
            return;
        }
        after_playtime_change(&state, &[db_id]);
        if let (Some(page), Some(win)) = (page_weak.upgrade(), win_weak.upgrade()) {
            refill_playtime_page(&page, &state, &reload_self(&state, db_id), &win);
        }
    });
    row.add_suffix(&btn);
    row
}

/// Pick entries of the same game and fold them into this entry's link.
/// Hidden entries are pickable — hiding a duplicate from the library is
/// exactly why it would need linking in the first place.
fn show_link_games_dialog(
    state: &SharedState,
    db_id: i64,
    win: &adw::Window,
    members: &[i64],
    page: &glib::WeakRef<gtk4::Box>,
) {
    // Straight from the database, so hidden entries are offered too.
    let candidates: Vec<GameEntry> = {
        let s = state.borrow();
        ira_db::load_all_games(&s.db)
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| entry.id != db_id && !members.contains(&entry.id))
            .collect()
    };
    if candidates.is_empty() {
        return;
    }
    let dialog = adw::AlertDialog::new(
        Some(&crate::tr!("Link game playtime")),
        Some(&crate::tr!("Choose the entries whose time counts together")),
    );
    let (picker, checks) = build_game_picker(&candidates);
    dialog.set_extra_child(Some(&picker));
    dialog.add_response("cancel", &crate::tr!("Cancel"));
    dialog.add_response("link", &crate::tr!("Link"));
    dialog.set_response_appearance("link", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("link"));
    dialog.set_close_response("cancel");

    // Linkable as soon as one entry is picked — this entry joins too.
    let sync = {
        let dialog = dialog.downgrade();
        let checks = checks.clone();
        move || {
            if let Some(dialog) = dialog.upgrade() {
                let chosen = checks
                    .borrow()
                    .iter()
                    .filter(|(_, check)| check.is_active())
                    .count();
                dialog.set_response_enabled("link", chosen >= 1);
            }
        }
    };
    for (_, check) in checks.borrow().iter() {
        let sync = sync.clone();
        check.connect_toggled(move |_| sync());
    }
    sync();

    let state = state.clone();
    let win_weak = win.downgrade();
    let page = page.clone();
    let members = members.to_vec();
    dialog.connect_response(None, move |_, resp| {
        if resp != "link" {
            return;
        }
        let picked: Vec<i64> = checks
            .borrow()
            .iter()
            .filter(|(_, check)| check.is_active())
            .map(|(id, _)| *id)
            .collect();
        if picked.is_empty() {
            return;
        }
        // The whole group moves into one fresh link, so adding to an
        // existing link keeps its current members.
        let mut group = members.clone();
        group.push(db_id);
        group.extend(picked.iter().copied());
        let db = state.borrow().db.clone();
        if let Err(e) = ira_db::link_games(&db, &group) {
            eprintln!("Failed to link games: {e}");
            return;
        }
        after_playtime_change(&state, &group);
        if let (Some(page), Some(win)) = (page.upgrade(), win_weak.upgrade()) {
            refill_playtime_page(&page, &state, &reload_self(&state, db_id), &win);
        }
    });
    dialog.present(Some(win));
}

/// A searchable check list of entries, several picks at a time. Each row
/// names the entry plus its source, id, and location, so several copies
/// of one title tell themselves apart. Returns the widget and the checks
/// to read answers from.
fn build_game_picker(games: &[GameEntry]) -> (gtk4::Widget, GameChecks) {
    let column = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    let search = gtk4::SearchEntry::new();
    column.append(&search);
    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);
    list.add_css_class("boxed-list");
    let checks = Rc::new(RefCell::new(Vec::new()));
    let rows: Rc<RefCell<Vec<(String, gtk4::ListBoxRow)>>> = Rc::new(RefCell::new(Vec::new()));
    for game in games {
        let row = adw::ActionRow::new();
        // Game titles are shown as typed — an "&" in "Kirby & The
        // Amazing Mirror" is not markup.
        row.set_use_markup(false);
        row.set_title(&game.title);
        row.set_subtitle(&identity_line(game));
        let check = gtk4::CheckButton::new();
        check.set_valign(gtk4::Align::Center);
        row.add_suffix(&check);
        row.set_activatable_widget(Some(&check));
        let row_widget = row.clone();
        list.append(&row);
        rows.borrow_mut().push((game.title.to_lowercase(), row_widget.into()));
        checks.borrow_mut().push((game.id, check));
    }
    search.connect_search_changed({
        let rows = rows.clone();
        move |entry| {
            let query = entry.text().to_lowercase();
            for (title, row) in rows.borrow().iter() {
                row.set_visible(query.is_empty() || title.contains(&query));
            }
        }
    });
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    scroll.set_min_content_width(420);
    scroll.set_min_content_height(320);
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    column.append(&scroll);
    (column.upcast(), checks)
}

/// The refilling closures read the entry fresh from the games list so the
/// total row and link membership show the change they just made.
fn reload_self(state: &SharedState, db_id: i64) -> Game {
    state
        .borrow()
        .games
        .iter()
        .find(|g| g.db_id == db_id && g.variant_id.is_none())
        .cloned()
        .unwrap_or_default()
}

/// Re-read the links cache and refresh every touched game's entry, page,
/// and sidebar row.
fn after_playtime_change(state: &SharedState, affected: &[i64]) {
    refresh_playtime_links(state);
    for id in affected {
        let _ = state
            .borrow()
            .sender
            .send(crate::AppMessage::VariantsChanged(*id));
    }
}

/// The identifying line for an entry row: its source, the id that source
/// knows it by, and where it lives — enough to tell apart several
/// entries of one title. Empty pieces drop out.
fn identity_line(entry: &GameEntry) -> String {
    identity_parts(
        entry.kind,
        &entry.steam_id,
        &entry.game_id,
        &entry.platform_id,
        &entry.rom_path,
        &entry.game_folder,
    )
}

/// Pure part of [`identity_line`], over the raw identity fields.
fn identity_parts(
    kind: ira_models::GameKind,
    steam_id: &str,
    game_id: &str,
    platform_id: &str,
    rom_path: &str,
    game_folder: &str,
) -> String {
    let mut parts: Vec<String> = vec![kind.display_name().to_string()];
    if !steam_id.is_empty() {
        parts.push(format!("App {steam_id}"));
    } else if !game_id.is_empty() {
        parts.push(game_id.to_string());
    } else if !platform_id.is_empty() {
        parts.push(platform_id.to_string());
    }
    let locations = [rom_path, game_folder];
    if let Some(location) = locations.iter().find(|l| !l.is_empty()) {
        parts.push((*location).to_string());
    }
    parts.join("  ·  ")
}

/// Parse a manual playtime entry: `"3"` or `"2.5"` hours, or `"2:30"`
/// hours:minutes. `None` when nothing usable was typed.
fn parse_playtime_input(text: &str) -> Option<f64> {
    let text = text.trim();
    let parsed = if let Some((hours, minutes)) = text.split_once(':') {
        let hours: f64 = hours.trim().parse().ok()?;
        let minutes: f64 = minutes.trim().parse().ok()?;
        if !(0.0..60.0).contains(&minutes) {
            return None;
        }
        Some(hours + minutes / 60.0)
    } else {
        text.parse().ok()
    };
    parsed.filter(|hours| hours.is_finite() && *hours > 0.0)
}

#[cfg(test)]
mod tests {
    use super::{identity_parts, parse_playtime_input};
    use ira_models::GameKind;

    #[test]
    fn test_identity_parts_names_source_id_and_location() {
        assert_eq!(
            identity_parts(GameKind::Steam, "12345", "", "", "/games/elden/elden.exe", ""),
            "Steam  ·  App 12345  ·  /games/elden/elden.exe"
        );
    }

    #[test]
    fn test_identity_parts_drops_missing_pieces() {
        // No ids, no location: just the source.
        assert_eq!(identity_parts(GameKind::Switch, "", "", "", "", ""), "Nintendo Switch");
        // A console entry identifies by its serial.
        assert_eq!(
            identity_parts(
                GameKind::Switch,
                "",
                "",
                "0100000000010000",
                "",
                "/roms/switch/elden"
            ),
            "Nintendo Switch  ·  0100000000010000  ·  /roms/switch/elden"
        );
    }

    #[test]
    fn test_parse_playtime_input_hours_and_clock_forms() {
        assert_eq!(parse_playtime_input("2"), Some(2.0));
        assert_eq!(parse_playtime_input(" 12.5 "), Some(12.5));
        assert_eq!(parse_playtime_input("2:30"), Some(2.5));
        assert_eq!(parse_playtime_input("0:45"), Some(0.75));
    }

    #[test]
    fn test_parse_playtime_input_rejects_junk() {
        assert_eq!(parse_playtime_input(""), None);
        assert_eq!(parse_playtime_input("abc"), None);
        assert_eq!(parse_playtime_input("-3"), None);
        assert_eq!(parse_playtime_input("0"), None);
        assert_eq!(parse_playtime_input(":30"), None);
        // Minutes past the hour are a typo, not 75 minutes.
        assert_eq!(parse_playtime_input("2:75"), None);
    }
}
