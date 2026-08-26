use super::css::*;
use super::helpers::clear_children;
use super::matching::match_game_to_steam;
use super::ra_match_dialog::show_ra_search_dialog;
use super::settings_console::build_emulator_dropdown;
use super::state::{PendingImage, SharedState};
use crate::Game;
use adw::prelude::*;
use glib::clone::Downgrade;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

type PendingCell = Rc<RefCell<Option<String>>>;

type GameGeneralPageResult = (
    gtk4::Box,
    adw::EntryRow,
    adw::EntryRow,
    PendingCell,
    Option<adw::EntryRow>,
    Option<adw::ComboRow>,
    PendingCell,
    PendingCell,
    Option<gtk4::Box>,
    Option<adw::EntryRow>,
    Option<gtk4::Button>,
    Option<adw::ComboRow>,
);

fn build_ra_section(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    pending_copies: &Rc<RefCell<HashMap<String, PendingImage>>>,
) -> adw::PreferencesGroup {
    let ra_group = adw::PreferencesGroup::new();
    ra_group.set_title(&crate::tr!("RetroAchievements"));

    if game.trophy_source == ira_models::TrophySource::Ra && !game.app_id.is_empty() {
        let status_row = adw::ActionRow::new();
        status_row.set_title(&crate::tr!("Linked to RetroAchievements"));
        status_row.set_subtitle(&crate::tr!("Game ID: {}").replacen("{}", &game.app_id, 1));
        let pending_key = format!("__ra_unmatch_{}", game.db_id);
        let is_pending_unmatch = pending_copies.borrow().contains_key(&pending_key);
        if is_pending_unmatch {
            status_row.set_subtitle(&crate::tr!("Will be unmatched on Save\u{2026}"));
        }
        ra_group.add(&status_row);

        let unmatch_btn = gtk4::Button::with_label(&crate::tr!("Unmatch"));
        unmatch_btn.add_css_class(CSS_DESTRUCTIVE_ACTION);
        unmatch_btn.set_valign(gtk4::Align::Center);
        if is_pending_unmatch {
            unmatch_btn.set_sensitive(false);
        }
        let pc = pending_copies.clone();
        let pkey = pending_key;
        let sc = state.clone();
        let game_clone = game.clone();
        unmatch_btn.connect_clicked(move |_| {
            pc.borrow_mut()
                .insert(pkey.clone(), PendingImage::Path(String::new()));
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
        let match_btn = gtk4::Button::with_label(&crate::tr!("Match\u{2026}"));
        match_btn.add_css_class(CSS_SUGGESTED_ACTION);
        match_btn.set_valign(gtk4::Align::Center);
        let sc = state.clone();
        let db_id = game.db_id;
        let gn = game.name.clone();
        let pid = game.platform_id.clone();
        let pw = Downgrade::downgrade(win);
        match_btn.connect_clicked(move |_| {
            let Some(pw) = pw.upgrade() else {
                return;
            };
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
    let game = match state
        .borrow()
        .games
        .iter()
        .find(|g| g.db_id == db_id)
        .cloned()
    {
        Some(g) => g,
        None => return,
    };
    super::helpers::clear_children(&ra_container);
    let ra_group = build_ra_section(state, &game, &sd.window, &sd.pending_copies);
    ra_container.append(&ra_group);
}

fn build_title_and_sort_inputs(
    group: &adw::PreferencesGroup,
    game: &Game,
) -> (adw::EntryRow, adw::EntryRow) {
    let title_entry = adw::EntryRow::new();
    title_entry.set_title(&crate::tr!("Game title"));
    title_entry.set_text(&game.name);
    group.add(&title_entry);

    let sort_entry = adw::EntryRow::new();
    sort_entry.set_title(&crate::tr!("Sort title"));
    sort_entry.set_text(&game.sort_title);
    group.add(&sort_entry);

    (title_entry, sort_entry)
}

fn add_game_path_if_needed(group: &adw::PreferencesGroup, game: &Game) {
    if game.game_path.is_empty() || game.kind == ira_models::GameKind::Steam {
        return;
    }
    let path_row = adw::ActionRow::new();
    path_row.set_title(&crate::tr!("Game file"));
    let display_path = game_file_path_for_display(game);
    let escaped = glib::markup_escape_text(&display_path).to_string();
    path_row.set_subtitle(&escaped);
    path_row.set_sensitive(false);
    group.add(&path_row);
}

fn game_file_path_for_display(game: &Game) -> String {
    if game.kind == ira_models::GameKind::Retro && !game.platform_id.is_empty() {
        let path = std::path::Path::new(&game.game_path);
        if !path.is_absolute()
            && path
                .components()
                .next()
                .and_then(|component| component.as_os_str().to_str())
                != Some(game.platform_id.as_str())
        {
            return format!("{}/{}", game.platform_id, game.game_path);
        }
    }
    game.game_path.clone()
}

fn build_game_folder_row(
    parent: &adw::PreferencesGroup,
    game: &Game,
    win: &adw::Window,
) -> Option<adw::EntryRow> {
    if game.kind != ira_models::GameKind::Wine && game.kind != ira_models::GameKind::Linux {
        return None;
    }
    let row = adw::EntryRow::new();
    row.set_title(&crate::tr!("Install directory"));
    row.set_text(&game.game_folder);
    let browse = super::helpers::make_browse_button(
        Some(win),
        "Select game folder",
        true,
        None,
        super::helpers::entry_path_closure(&row),
        {
            let row_c = Downgrade::downgrade(&row);
            move |path| {
                if let Some(row_c) = row_c.upgrade() {
                    row_c.set_text(&path.to_string_lossy());
                }
            }
        },
    );
    row.add_suffix(&browse);
    parent.add(&row);
    Some(row)
}

fn build_runtime_row(parent: &adw::PreferencesGroup, game: &Game) -> Option<adw::ComboRow> {
    if !game.kind.is_managed_pc() {
        return None;
    }
    let row = adw::ComboRow::new();
    row.set_title(&crate::tr!("Runtime"));
    row.set_subtitle(&crate::tr!("Choose native Linux or Windows through Wine"));
    let runtime_model =
        super::helpers::string_list_from(&[crate::tr!("Wine (Windows)"), crate::tr!("Linux")]);
    row.set_model(Some(&runtime_model));
    row.set_selected(if game.kind == ira_models::GameKind::Linux {
        1
    } else {
        0
    });
    parent.add(&row);
    Some(row)
}

fn build_shadps4_version_section(page: &gtk4::Box, game: &Game) -> Rc<RefCell<Option<String>>> {
    let pending_version: Rc<RefCell<Option<String>>> = Default::default();
    if game.kind == ira_models::GameKind::Ps4 {
        let shadps4_versions = ira_platforms::ps4::read_shadps4_launch_options();
        if !shadps4_versions.is_empty() {
            let version_group = adw::PreferencesGroup::new();
            version_group.set_title(&crate::tr!("shadPS4 Version"));

            let version_dropdown = build_emulator_dropdown(
                &game.shadps4_version,
                true,
                "Follow global",
                &shadps4_versions,
            );

            let pending_version_c = pending_version.clone();
            version_dropdown.connect_selected_notify(move |dd| {
                let idx = dd.selected();
                let path = if idx == 0 {
                    String::new()
                } else {
                    match ira_platforms::ps4::read_shadps4_launch_options()
                        .into_iter()
                        .nth((idx - 1) as usize)
                    {
                        Some(v) => v.launch_command,
                        None => return,
                    }
                };
                *pending_version_c.borrow_mut() = Some(path);
            });

            version_dropdown
                .set_subtitle(&crate::tr!("Override the emulator version for this game"));
            version_group.add(&version_dropdown);
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
) -> Option<adw::ComboRow> {
    if cores.is_empty() {
        return None;
    }

    let mut core_names: Vec<String> = vec![crate::tr!("Follow global")];
    core_names.extend(cores.iter().map(|c| c.display_name.clone()));
    let core_dropdown = adw::ComboRow::new();
    core_dropdown.set_title(&crate::tr!("RetroArch core"));
    core_dropdown.set_subtitle(&crate::tr!("Override the RetroArch core for this game"));
    core_dropdown.set_model(Some(&super::helpers::string_list_from(&core_names)));

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

    let is_ra = !game.emulator_override.is_empty()
        && ira_platforms::emulator_detect::is_retroarch(&game.emulator_override);
    core_dropdown.set_visible(is_ra);

    emu_group.add(&core_dropdown);
    Some(core_dropdown)
}

fn add_emulator_dropdown_section(
    page: &gtk4::Box,
    game: &Game,
    pending_ra_core: &Rc<RefCell<Option<String>>>,
    pending_emulator: &Rc<RefCell<Option<String>>>,
) {
    // Retro games key detection off their console platform; Azahar and
    // Cemu games carry a title id as platform, so map the kind instead.
    let console_id: &str = match game.kind {
        ira_models::GameKind::ThreeDS => "3ds",
        ira_models::GameKind::WiiU => "wiiu",
        _ => &game.platform_id,
    };
    let emulators = ira_platforms::emulator_detect::detect_emulators(console_id);
    let cores = ira_platforms::emulator_detect::detect_ra_cores_for_console(console_id);
    if emulators.is_empty() {
        return;
    }

    let emu_group = adw::PreferencesGroup::new();
    emu_group.set_title(&crate::tr!("Emulator"));

    let mut emu_names: Vec<String> = vec![crate::tr!("Follow global")];
    emu_names.extend(emulators.iter().map(|e| e.display_name.clone()));
    let emu_dropdown = adw::ComboRow::new();
    emu_dropdown.set_title(&crate::tr!("Emulator"));
    emu_dropdown.set_subtitle(&crate::tr!("Override the emulator for this game"));
    emu_dropdown.set_model(Some(&super::helpers::string_list_from(&emu_names)));

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

    emu_group.add(&emu_dropdown);

    let core_row = build_core_row(game, &cores, pending_ra_core, &emu_group);

    let pending_emu_c = pending_emulator.clone();
    let emus_clone = emulators.clone();
    let core_row_clone = core_row;
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

    // Retro games pick emulator + RA core per game; PS3, Cemu and Azahar
    // games pick the emulator install per game — only these launch paths
    // consume the generic emulator override (PS4 has its own version
    // selector, Vita3K has a single launcher). Both store the choice as a
    // generic emulator override.
    if game.kind == ira_models::GameKind::Retro {
        add_emulator_dropdown_section(page, game, &pending_ra_core, &pending_emulator);
        let container = build_ra_container(page, state, game, win, pending_copies);
        ra_container = Some(container);
    } else if matches!(
        game.kind,
        ira_models::GameKind::ThreeDS | ira_models::GameKind::WiiU | ira_models::GameKind::Ps3
    ) {
        add_emulator_dropdown_section(page, game, &pending_ra_core, &pending_emulator);
    }

    (pending_ra_core, pending_emulator, ra_container)
}

fn build_service_ids_section(
    parent: &adw::PreferencesGroup,
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

    if game.kind.is_trophy_console() {
        let row = adw::ActionRow::new();
        row.set_title(&crate::tr!("NPWR code"));
        row.set_subtitle(&game.app_id);
        row.set_sensitive(false);
        parent.add(&row);
        let serial_row = adw::ActionRow::new();
        serial_row.set_title(&crate::tr!("Game serial"));
        serial_row.set_subtitle(&game.platform_id);
        serial_row.set_sensitive(false);
        parent.add(&serial_row);
    } else if game.trophy_source == ira_models::TrophySource::Gse {
        let row = adw::EntryRow::new();
        row.set_title(&crate::tr!("Steam app ID"));
        row.set_text(&game.app_id);
        let search_btn = gtk4::Button::from_icon_name("system-search-symbolic");
        search_btn.set_valign(gtk4::Align::Center);
        search_btn.set_tooltip_text(Some(&crate::tr!("Search Steam store")));
        search_btn.add_css_class(CSS_FLAT);
        let sc = state.clone();
        let game_name = game.name.clone();
        let db_id = game.db_id;
        let win_c = Downgrade::downgrade(win);
        let row_c = Downgrade::downgrade(&row);
        search_btn.connect_clicked(move |_| {
            let Some(win) = win_c.upgrade() else {
                return;
            };
            let Some(row_c) = row_c.upgrade() else {
                return;
            };
            let on_select = {
                let sc = sc.clone();
                Rc::new(move |sid: &str, matched_name: &str| {
                    match_game_to_steam(&sc, db_id, sid.to_string(), matched_name.to_string());
                })
            };
            super::steam_search::show_steam_id_search_popup(
                &sc, &game_name, &win, &row_c, "Match", on_select,
            );
        });
        row.add_suffix(&search_btn);
        parent.add(&row);
        app_id_entry = Some(row);
    } else if game.trophy_source == ira_models::TrophySource::Nge {
        let row = adw::EntryRow::new();
        row.set_title(&crate::tr!("GOG product ID"));
        row.set_text(&game.app_id);
        parent.add(&row);
        app_id_entry = Some(row);
    }

    app_id_entry
}

fn build_language_section(
    parent: &adw::PreferencesGroup,
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

    let display_names: Vec<String> = languages
        .iter()
        .map(|code| ira_models::steam_language_name(code).to_string())
        .collect();
    let model = super::helpers::string_list_from(&display_names);
    let row = adw::ComboRow::new();
    row.set_title(&crate::tr!("Game language"));
    row.set_subtitle(&crate::tr!(
        "Language reported to the game by the API emulator"
    ));
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
        .or_else(|| {
            let prefs = &state.borrow().cfg.language_preferences;
            prefs
                .iter()
                .find_map(|p| languages.iter().position(|l| l == p))
        })
        .or_else(|| languages.iter().position(|l| l == "english"))
        .unwrap_or(0);
    row.set_selected(selected as u32);

    parent.add(&row);
    Some(row)
}

/// Build the save migration section. Shows a "Migrate saves" button for
/// games that have UFS save data. Hidden for games without UFS data and for
/// games whose saves are already centralized.
fn build_save_migration_section(
    parent: &adw::PreferencesGroup,
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

    let (wine_prefix, is_wine) = {
        let s = state.borrow();
        let cfg = ira_db::get_game_config(&s.db, game.db_id)
            .ok()
            .flatten()
            .map(|(_, w, _)| w)
            .unwrap_or_default();
        (
            ira_launcher::wine_launch::wine_prefix(&cfg),
            game.kind == ira_models::GameKind::Wine && cfg.enabled,
        )
    };
    let pfx = if is_wine {
        Some(wine_prefix.as_str())
    } else {
        None
    };

    let db = state.borrow().db.clone();
    let cached = ira_db::get_saves_centralized(&db, game.db_id).unwrap_or(false);
    let already_centralized = cached
        || ira_launcher::game_saves::saves_are_centralized(
            &details.ufs_savefiles,
            &details.ufs_rootoverrides,
            &game.app_id,
            &save_dir,
            pfx,
        );
    if already_centralized {
        if !cached {
            if let Err(e) = ira_db::set_saves_centralized(&db, game.db_id, true) {
                eprintln!("Failed to cache saves centralized: {}", e);
            }
        }
        return None;
    }

    let row = adw::ActionRow::new();
    row.set_title(&crate::tr!("Centralize save data"));
    row.set_subtitle(&crate::tr!(
        "Move saves to a persistent location and create symlinks"
    ));

    let btn = gtk4::Button::with_label(&crate::tr!("Migrate"));
    btn.add_css_class(CSS_SUGGESTED_ACTION);
    btn.set_valign(gtk4::Align::Center);
    row.add_suffix(&btn);
    parent.add(&row);

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

    let identity_group = adw::PreferencesGroup::new();
    identity_group.set_title(&crate::tr!("Identity"));
    let (title_entry, sort_entry) = build_title_and_sort_inputs(&identity_group, game);
    add_game_path_if_needed(&identity_group, game);
    let runtime_row = build_runtime_row(&identity_group, game);
    let game_folder_entry = build_game_folder_row(&identity_group, game, win);
    general_page.append(&identity_group);

    let pending_version = build_shadps4_version_section(&general_page, game);
    let (pending_ra_core, pending_emulator, ra_container) =
        build_retro_emulator_and_ra(&general_page, state, game, win, pending_copies);

    let service_group = adw::PreferencesGroup::new();
    service_group.set_title(&crate::tr!("Service"));
    let app_id_entry = build_service_ids_section(&service_group, game, state, win);
    let language_row = build_language_section(&service_group, state, game, languages);
    let migrate_btn = build_save_migration_section(&service_group, state, game);
    if app_id_entry.is_some() || language_row.is_some() || migrate_btn.is_some() {
        general_page.append(&service_group);
    }

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
        runtime_row,
    )
}

#[cfg(test)]
mod tests {
    use super::game_file_path_for_display;
    use crate::Game;

    #[test]
    fn test_game_file_path_for_display_prefixes_retro_console() {
        let game = Game {
            kind: ira_models::GameKind::Retro,
            platform_id: "saturn".to_string(),
            game_path: "Soul Hackers/disc1.chd".to_string(),
            ..Default::default()
        };

        assert_eq!(
            game_file_path_for_display(&game),
            "saturn/Soul Hackers/disc1.chd"
        );
    }

    #[test]
    fn test_game_file_path_for_display_keeps_absolute_path() {
        let game = Game {
            kind: ira_models::GameKind::Retro,
            platform_id: "saturn".to_string(),
            game_path: "/roms/saturn/disc1.chd".to_string(),
            ..Default::default()
        };

        assert_eq!(game_file_path_for_display(&game), "/roms/saturn/disc1.chd");
    }
}
