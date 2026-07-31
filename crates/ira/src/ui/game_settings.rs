use gtk4::prelude::*;
use adw::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use crate::strings as S;
use crate::Game;
use super::matching::match_game_to_steam;
use super::settings_dialog::build_shadps4_version_dropdown;
use super::state::{PendingImage, SharedState};
use super::helpers::clear_children;
use super::ra_match_dialog::show_ra_search_dialog;
use super::css::*;

type PendingCell = Rc<RefCell<Option<String>>>;

type GameGeneralPageResult = (gtk4::Box, adw::EntryRow, adw::EntryRow, PendingCell, Option<adw::EntryRow>, Option<adw::ComboRow>, PendingCell, PendingCell, Option<gtk4::Box>, Option<adw::EntryRow>, Option<gtk4::Button>);

fn build_ra_section(state: &SharedState, game: &Game, win: &adw::Window, pending_copies: &Rc<RefCell<HashMap<String, PendingImage>>>) -> adw::PreferencesGroup {
    let ra_group = adw::PreferencesGroup::new();
    ra_group.set_title("RetroAchievements");

    if game.trophy_source == ira_models::TrophySource::Ra && !game.app_id.is_empty() {
        let status_row = adw::ActionRow::new();
        status_row.set_title("Linked to RetroAchievements");
        status_row.set_subtitle(&format!("Game ID: {}", game.app_id));
        let pending_key = format!("__ra_unmatch_{}", game.db_id);
        let is_pending_unmatch = pending_copies.borrow().contains_key(&pending_key);
        if is_pending_unmatch {
            status_row.set_subtitle("Will be unmatched on Save\u{2026}");
        }
        ra_group.add(&status_row);

        let unmatch_btn = gtk4::Button::with_label("Unmatch");
        unmatch_btn.add_css_class(CSS_DESTRUCTIVE_ACTION);
        unmatch_btn.set_valign(gtk4::Align::Center);
        if is_pending_unmatch {
            unmatch_btn.set_sensitive(false);
        }
        let pc = pending_copies.clone();
        let pkey = pending_key.clone();
        let sc = state.clone();
        let game_clone = game.clone();
        unmatch_btn.connect_clicked(move |_| {
            pc.borrow_mut().insert(pkey.clone(), PendingImage::Path(String::new()));
            let sd = match sc.borrow().settings_data.clone() {
                Some(d) => d,
                None => return,
            };
            if sd.db_id != game_clone.db_id || !sd.window.is_visible() {
                return;
            }
            if let Some(ref ra_container) = sd.ra_container {
                clear_children(ra_container);
                let mut g2 = game_clone.clone();
                g2.trophy_source = ira_models::TrophySource::Empty;
                g2.app_id.clear();
                let ra_group = build_ra_section(&sc, &g2, &sd.window, &sd.pending_copies);
                ra_container.append(&ra_group);
            }
        });
        status_row.add_suffix(&unmatch_btn);
    } else if game.trophy_source == ira_models::TrophySource::Empty {
        let match_btn = gtk4::Button::with_label("Match\u{2026}");
        match_btn.add_css_class(CSS_SUGGESTED_ACTION);
        match_btn.set_valign(gtk4::Align::Center);
        let sc = state.clone();
        let db_id = game.db_id;
        let gn = game.name.clone();
        let pid = game.platform_id.clone();
        let pw = win.clone();
        match_btn.connect_clicked(move |_| {
            show_ra_search_dialog(&sc, db_id, &gn, &pid, &pw, None);
        });
        let match_row = adw::ActionRow::new();
        match_row.add_suffix(&match_btn);
        ra_group.add(&match_row);
    }

    ra_group
}

pub(super) fn refresh_ra_section(state: &SharedState, db_id: i64) {
    let sd = match state.borrow().settings_data.clone() {
        Some(d) => d,
        None => return,
    };
    if sd.db_id != db_id || !sd.window.is_visible() {
        return;
    }
    let ra_container = match &sd.ra_container {
        Some(c) => c.clone(),
        None => return,
    };
    let game = match state.borrow().games.iter().find(|g| g.db_id == db_id).cloned() {
        Some(g) => g,
        None => return,
    };
    super::helpers::clear_children(&ra_container);
    let ra_group = build_ra_section(state, &game, &sd.window, &sd.pending_copies);
    ra_container.append(&ra_group);
}

fn build_title_and_sort_inputs(
    page: &gtk4::Box,
    game: &Game,
) -> (adw::EntryRow, adw::EntryRow) {
    let title_entry = adw::EntryRow::new();
    title_entry.set_title(S::GAME_TITLE);
    title_entry.set_text(&game.name);
    let general_group = adw::PreferencesGroup::new();
    general_group.set_title("Identity");
    general_group.add(&title_entry);
    page.append(&general_group);

    let sort_entry = adw::EntryRow::new();
    sort_entry.set_title("Sort title");
    sort_entry.set_text(&game.sort_title);
    let sort_group = adw::PreferencesGroup::new();
    sort_group.add(&sort_entry);
    page.append(&sort_group);

    (title_entry, sort_entry)
}

fn add_game_path_if_needed(page: &gtk4::Box, game: &Game) {
    if game.game_path.is_empty() || game.kind == ira_models::GameKind::Steam {
        return;
    }
    let path_group = adw::PreferencesGroup::new();
    let path_row = adw::ActionRow::new();
    path_row.set_title("Game file");
    let escaped = glib::markup_escape_text(&game.game_path).to_string();
    path_row.set_subtitle(&escaped);
    path_row.set_sensitive(false);
    path_group.add(&path_row);
    page.append(&path_group);
}

fn build_game_folder_row(page: &gtk4::Box, game: &Game, win: &adw::Window) -> Option<adw::EntryRow> {
    if game.kind != ira_models::GameKind::Wine && game.kind != ira_models::GameKind::Linux {
        return None;
    }
    let group = adw::PreferencesGroup::new();
    group.set_title("Game folder");
    let row = adw::EntryRow::new();
    row.set_title("Install directory");
    row.set_text(&game.game_folder);
    let browse = super::helpers::make_browse_button(
        Some(win),
        "Select game folder",
        true,
        None,
        super::helpers::entry_path_closure(&row),
        {
            let row_c = row.clone();
            move |path| row_c.set_text(&path.to_string_lossy())
        },
    );
    row.add_suffix(&browse);
    group.add(&row);
    page.append(&group);
    Some(row)
}

fn build_shadps4_version_section(
    page: &gtk4::Box,
    game: &Game,
) -> Rc<RefCell<Option<String>>> {
    let pending_version: Rc<RefCell<Option<String>>> = Default::default();
    if game.kind == ira_models::GameKind::Ps4 {
        let shadps4_versions = ira_platforms::ps4::read_shadps4_versions();
        if !shadps4_versions.is_empty() {
            let version_group = adw::PreferencesGroup::new();
            version_group.set_title("shadPS4 Version");

            let version_dropdown = build_shadps4_version_dropdown(&game.shadps4_version, true);

            let pending_version_c = pending_version.clone();
            version_dropdown.connect_selected_notify(move |dd| {
                let idx = dd.selected();
                let path = if idx == 0 {
                    String::new()
                } else {
                    match ira_platforms::ps4::read_shadps4_versions().into_iter().nth((idx - 1) as usize) {
                        Some(v) => v.path.trim_matches('"').to_string(),
                        None => return,
                    }
                };
                *pending_version_c.borrow_mut() = Some(path);
            });

            let version_row = adw::ActionRow::new();
            version_row.set_title("Version");
            version_row.set_subtitle("Override the emulator version for this game");
            version_dropdown.set_valign(gtk4::Align::Center);
            version_row.add_suffix(&version_dropdown);
            version_group.add(&version_row);
            page.append(&version_group);
        }
    }
    pending_version
}

fn build_core_row(
    game: &Game,
    cores: &[ira_platforms::emulator_detect::RaCore],
    pending_ra_core: &Rc<RefCell<Option<String>>>,
    emu_group: &adw::PreferencesGroup,
) -> Option<adw::ActionRow> {
    if cores.is_empty() {
        return None;
    }

    let mut core_names: Vec<String> = vec!["Follow global".to_string()];
    core_names.extend(cores.iter().map(|c| c.display_name.clone()));
    let core_model = super::helpers::string_list_from(&core_names);
    let core_dropdown = gtk4::DropDown::new(Some(core_model), None::<&gtk4::PropertyExpression>);

    let mut selected_idx: u32 = 0;
    if !game.ra_core.is_empty() {
        for (i, c) in cores.iter().enumerate() {
            if c.path == game.ra_core {
                selected_idx = (i + 1) as u32;
                break;
            }
        }
    }
    core_dropdown.set_selected(selected_idx);

    let pending_ra_core_c = pending_ra_core.clone();
    let cores_clone = cores.to_vec();
    core_dropdown.connect_selected_notify(move |dd| {
        let idx = dd.selected();
        let path = if idx == 0 {
            String::new()
        } else {
            match cores_clone.get((idx - 1) as usize) {
                Some(c) => c.path.clone(),
                None => return,
            }
        };
        *pending_ra_core_c.borrow_mut() = Some(path);
    });

    let cr = adw::ActionRow::new();
    cr.set_title("RetroArch Core");
    cr.set_subtitle("Override the RetroArch core for this game");
    core_dropdown.set_valign(gtk4::Align::Center);
    cr.add_suffix(&core_dropdown);

    let is_ra = !game.emulator_override.is_empty()
        && ira_platforms::emulator_detect::is_retroarch(&game.emulator_override);
    cr.set_visible(is_ra);

    emu_group.add(&cr);
    Some(cr)
}

fn add_emulator_dropdown_section(
    page: &gtk4::Box,
    game: &Game,
    pending_ra_core: &Rc<RefCell<Option<String>>>,
    pending_emulator: &Rc<RefCell<Option<String>>>,
) {
    let emulators = ira_platforms::emulator_detect::detect_emulators(&game.platform_id);
    let cores = ira_platforms::emulator_detect::detect_ra_cores();
    if emulators.is_empty() {
        return;
    }

    let emu_group = adw::PreferencesGroup::new();
    emu_group.set_title("Emulator");

    let mut emu_names: Vec<String> = vec!["Follow global".to_string()];
    emu_names.extend(emulators.iter().map(|e| e.display_name.clone()));
    let emu_model = super::helpers::string_list_from(&emu_names);
    let emu_dropdown = gtk4::DropDown::new(Some(emu_model), None::<&gtk4::PropertyExpression>);

    let mut selected_emu: u32 = 0;
    if !game.emulator_override.is_empty() {
        for (i, e) in emulators.iter().enumerate() {
            if e.launch_command == game.emulator_override {
                selected_emu = (i + 1) as u32;
                break;
            }
        }
    }
    emu_dropdown.set_selected(selected_emu);

    let emu_row = adw::ActionRow::new();
    emu_row.set_title("Emulator");
    emu_row.set_subtitle("Override the emulator for this game");
    emu_dropdown.set_valign(gtk4::Align::Center);
    emu_row.add_suffix(&emu_dropdown);
    emu_group.add(&emu_row);

    let core_row = build_core_row(game, &cores, pending_ra_core, &emu_group);

    let pending_emu_c = pending_emulator.clone();
    let emus_clone = emulators.clone();
    let core_row_clone = core_row.clone();
    emu_dropdown.connect_selected_notify(move |dd| {
        let idx = dd.selected();
        let cmd = if idx == 0 {
            String::new()
        } else {
            match emus_clone.get((idx - 1) as usize) {
                Some(e) => e.launch_command.clone(),
                None => return,
            }
        };
        *pending_emu_c.borrow_mut() = Some(cmd);

        if let Some(ref cr) = core_row_clone {
            let is_ra = if idx == 0 {
                false
            } else {
                match emus_clone.get((idx - 1) as usize) {
                    Some(e) => ira_platforms::emulator_detect::is_retroarch(&e.launch_command),
                    None => false,
                }
            };
            cr.set_visible(is_ra);
        }
    });

    page.append(&emu_group);
}

fn build_ra_container(
    page: &gtk4::Box,
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    pending_copies: &Rc<RefCell<HashMap<String, PendingImage>>>,
) -> gtk4::Box {
    let ra_group = build_ra_section(state, game, win, pending_copies);
    let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    container.append(&ra_group);
    page.append(&container);
    container
}

fn build_retro_emulator_and_ra(
    page: &gtk4::Box,
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    pending_copies: &Rc<RefCell<HashMap<String, PendingImage>>>,
) -> (PendingCell, PendingCell, Option<gtk4::Box>) {
    let pending_ra_core: Rc<RefCell<Option<String>>> = Default::default();
    let pending_emulator: Rc<RefCell<Option<String>>> = Default::default();
    let mut ra_container: Option<gtk4::Box> = None;

    if game.kind == ira_models::GameKind::Retro {
        add_emulator_dropdown_section(page, game, &pending_ra_core, &pending_emulator);
        let container = build_ra_container(page, state, game, win, pending_copies);
        ra_container = Some(container);
    }

    (pending_ra_core, pending_emulator, ra_container)
}

fn build_service_ids_section(
    page: &gtk4::Box,
    game: &Game,
    state: &SharedState,
    win: &adw::Window,
) -> Option<adw::EntryRow> {
    let mut app_id_entry: Option<adw::EntryRow> = None;

    if game.trophy_source != ira_models::TrophySource::Gse
        && game.trophy_source != ira_models::TrophySource::Nge
        && !game.kind.is_trophy_console()
    {
        return None;
    }

    let ids_group = adw::PreferencesGroup::new();
    ids_group.set_title("Service IDs");

    if game.kind.is_trophy_console() {
        let row = adw::ActionRow::new();
        row.set_title("NPWR Code");
        row.set_subtitle(&game.app_id);
        row.set_sensitive(false);
        ids_group.add(&row);
        let serial_row = adw::ActionRow::new();
        serial_row.set_title("Game Serial");
        serial_row.set_subtitle(&game.platform_id);
        serial_row.set_sensitive(false);
        ids_group.add(&serial_row);
    } else if game.trophy_source == ira_models::TrophySource::Gse {
        let row = adw::EntryRow::new();
        row.set_title("Steam App ID");
        row.set_text(&game.app_id);
        let search_btn = gtk4::Button::from_icon_name("system-search-symbolic");
        search_btn.set_valign(gtk4::Align::Center);
        search_btn.set_tooltip_text(Some("Search Steam Store"));
        search_btn.add_css_class(CSS_FLAT);
        let sc = state.clone();
        let game_name = game.name.clone();
        let db_id = game.db_id;
        let win_c = win.clone();
        let row_c = row.clone();
        let matched_name = game.name.clone();
        search_btn.connect_clicked(move |_| {
            let on_select = {
                let sc = sc.clone();
                let name = matched_name.clone();
                Rc::new(move |sid: &str| {
                    match_game_to_steam(&sc, db_id, sid.to_string(), name.clone());
                })
            };
            super::steam_search::show_steam_id_search_popup(
                &sc, &game_name, &win_c, &row_c, "Match", on_select,
            );
        });
        row.add_suffix(&search_btn);
        ids_group.add(&row);
        app_id_entry = Some(row);
    } else if game.trophy_source == ira_models::TrophySource::Nge {
        let row = adw::EntryRow::new();
        row.set_title("GOG Product ID");
        row.set_text(&game.app_id);
        ids_group.add(&row);
        app_id_entry = Some(row);
    }

    page.append(&ids_group);
    app_id_entry
}

fn build_language_section(
    page: &gtk4::Box,
    state: &SharedState,
    game: &Game,
    languages: &[String],
) -> Option<adw::ComboRow> {
    if languages.is_empty()
        || (game.trophy_source != ira_models::TrophySource::Gse
            && game.trophy_source != ira_models::TrophySource::Nge)
    {
        return None;
    }

    let lang_group = adw::PreferencesGroup::new();
    lang_group.set_title("Language");
    let display_names: Vec<String> = languages
        .iter()
        .map(|code| ira_models::steam_language_name(code).to_string())
        .collect();
    let model = super::helpers::string_list_from(&display_names);
    let row = adw::ComboRow::new();
    row.set_title("Game language");
    row.set_subtitle("Language reported to the game by the API emulator");
    row.set_model(Some(&model));

    let save_dir = state.borrow().save_dir.clone();
    let game_exe = {
        let config = ira_db::get_game_config(&state.borrow().db, game.db_id)
            .ok()
            .flatten();
        config.map(|(l, _, _)| l.exe).unwrap_or_default()
    };
    let current_lang = ira_platforms::api_emulators::read_current_language(
        game.trophy_source,
        &game_exe,
        &save_dir,
        &game.app_id,
    );
    let selected = current_lang
        .as_ref()
        .and_then(|lang| languages.iter().position(|l| l == lang))
        .map(|i| i as u32)
        .unwrap_or(0);
    row.set_selected(selected);

    lang_group.add(&row);
    page.append(&lang_group);
    Some(row)
}

/// Build the save migration section. Shows a "Migrate saves" button for
/// games that have UFS save data. Hidden for games without UFS data.
fn build_save_migration_section(
    page: &gtk4::Box,
    state: &SharedState,
    game: &Game,
) -> Option<gtk4::Button> {
    if !game.trophy_source.has_steam_enrichment() || game.app_id.is_empty() {
        return None;
    }

    let save_dir = state.borrow().save_dir.clone();
    let details = crate::game_loader::read_app_details(&save_dir, &game.app_id)?;
    if details.ufs_savefiles.is_empty() {
        return None;
    }

    let group = adw::PreferencesGroup::new();
    group.set_title("Save data");

    let row = adw::ActionRow::new();
    row.set_title("Centralize save data");
    row.set_subtitle("Move saves to a persistent location and create symlinks");

    let btn = gtk4::Button::with_label("Migrate");
    btn.add_css_class(CSS_SUGGESTED_ACTION);
    btn.set_valign(gtk4::Align::Center);
    row.add_suffix(&btn);
    group.add(&row);
    page.append(&group);

    Some(btn)
}

pub(super) fn build_game_general_page(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    languages: &[String],
    pending_copies: &Rc<RefCell<HashMap<String, PendingImage>>>,
) -> GameGeneralPageResult {
    let general_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    let (title_entry, sort_entry) = build_title_and_sort_inputs(&general_page, game);
    add_game_path_if_needed(&general_page, game);
    let game_folder_entry = build_game_folder_row(&general_page, game, win);
    let pending_version = build_shadps4_version_section(&general_page, game);
    let (pending_ra_core, pending_emulator, ra_container) =
        build_retro_emulator_and_ra(&general_page, state, game, win, pending_copies);
    let app_id_entry = build_service_ids_section(&general_page, game, state, win);
    let language_row = build_language_section(&general_page, state, game, languages);
    let migrate_btn = build_save_migration_section(&general_page, state, game);

    (
        general_page,
        title_entry,
        sort_entry,
        pending_version,
        app_id_entry,
        language_row,
        pending_ra_core,
        pending_emulator,
        ra_container,
        game_folder_entry,
        migrate_btn,
    )
}
