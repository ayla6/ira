use gtk4::prelude::*;
use adw::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use crate::strings as S;
use crate::Game;
use super::matching::match_game_to_steam;
use super::settings_dialog::build_shadps4_version_dropdown;
use super::state::SharedState;
use super::ra_match_dialog::show_ra_search_dialog;

type GameGeneralPageResult = (gtk4::Box, adw::EntryRow, adw::EntryRow, Rc<RefCell<Option<String>>>, Option<adw::EntryRow>, Option<adw::ComboRow>, Rc<RefCell<Option<String>>>, Rc<RefCell<Option<String>>>);

pub(super) fn build_game_general_page(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    languages: &[String],
    pending_copies: &Rc<RefCell<HashMap<String, String>>>,
) -> GameGeneralPageResult {
    let general_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    let title_entry = adw::EntryRow::new();
    title_entry.set_title(S::GAME_TITLE);
    title_entry.set_text(&game.name);
    let general_group = adw::PreferencesGroup::new();
    general_group.set_title("Identity");
    general_group.add(&title_entry);
    general_page.append(&general_group);

    let sort_entry = adw::EntryRow::new();
    sort_entry.set_title("Sort title");
    sort_entry.set_text(&game.sort_title);
    let sort_group = adw::PreferencesGroup::new();
    sort_group.add(&sort_entry);
    general_page.append(&sort_group);

    if !game.game_path.is_empty() && game.kind != ira_models::GameKind::Steam {
        let path_group = adw::PreferencesGroup::new();
        let path_row = adw::ActionRow::new();
        path_row.set_title("Game file");
        let escaped = glib::markup_escape_text(&game.game_path).to_string();
        path_row.set_subtitle(&escaped);
        path_row.set_sensitive(false);
        path_group.add(&path_row);
        general_page.append(&path_group);
    }

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
            general_page.append(&version_group);
        }
    }

    let pending_ra_core: Rc<RefCell<Option<String>>> = Default::default();
    let pending_emulator: Rc<RefCell<Option<String>>> = Default::default();
    if game.kind == ira_models::GameKind::Retro {
        let emulators = ira_platforms::emulator_detect::detect_emulators(&game.platform_id);
        let cores = ira_platforms::emulator_detect::detect_ra_cores();
        if !emulators.is_empty() {
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

            let core_row = if !cores.is_empty() {
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
                let cores_clone = cores.clone();
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
            } else {
                None
            };

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

            general_page.append(&emu_group);
        }

        let ra_group = adw::PreferencesGroup::new();
        ra_group.set_title("RetroAchievements");
        let mut has_ra_content = false;
        if game.trophy_source == ira_models::TrophySource::Ra && !game.app_id.is_empty() {
            has_ra_content = true;
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
            unmatch_btn.add_css_class("destructive-action");
            unmatch_btn.set_valign(gtk4::Align::Center);
            if is_pending_unmatch {
                unmatch_btn.set_sensitive(false);
            }
            let pc = pending_copies.clone();
            let pkey = pending_key.clone();
            let unmatch_btn_c = unmatch_btn.clone();
            unmatch_btn.connect_clicked(move |_| {
                pc.borrow_mut().insert(pkey.clone(), String::new());
                unmatch_btn_c.set_sensitive(false);
                status_row.set_subtitle("Will be unmatched on Save\u{2026}");
            });
            let unmatch_row = adw::ActionRow::new();
            unmatch_row.add_suffix(&unmatch_btn);
            ra_group.add(&unmatch_row);
        } else if game.trophy_source == ira_models::TrophySource::Empty {
            has_ra_content = true;
            let match_btn = gtk4::Button::with_label("Match\u{2026}");
            match_btn.add_css_class("suggested-action");
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
        if has_ra_content {
            general_page.append(&ra_group);
        }
    }

    let mut app_id_entry: Option<adw::EntryRow> = None;

    if game.trophy_source == ira_models::TrophySource::Gse || game.trophy_source == ira_models::TrophySource::Nge || game.kind == ira_models::GameKind::Ps4 {
        let ids_group = adw::PreferencesGroup::new();
        ids_group.set_title("Service IDs");
    if game.kind == ira_models::GameKind::Ps4 {
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
            search_btn.add_css_class("flat");
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
                super::steam_search::show_steam_id_search_popup(&sc, &game_name, &win_c, &row_c, "Match", on_select);
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
        general_page.append(&ids_group);
    }

    let language_row = if !languages.is_empty() && (game.trophy_source == ira_models::TrophySource::Gse || game.trophy_source == ira_models::TrophySource::Nge) {
        let lang_group = adw::PreferencesGroup::new();
        lang_group.set_title("Language");
        let model = super::helpers::string_list_from(languages);
        let row = adw::ComboRow::new();
        row.set_title("Game language");
        row.set_subtitle("Language reported to the game by the API emulator");
        row.set_model(Some(&model));

        let save_dir = state.borrow().save_dir.clone();
        let game_exe = {
            let config = ira_db::get_game_config(&state.borrow().db, game.db_id).ok().flatten();
            config.map(|(l, _, _)| l.exe).unwrap_or_default()
        };
        let current_lang = ira_platforms::api_emulators::read_current_language(
            game.trophy_source, &game_exe, &save_dir, &game.app_id,
        );
        let selected = current_lang
            .as_ref()
            .and_then(|lang| languages.iter().position(|l| l == lang))
            .map(|i| i as u32)
            .unwrap_or(0);
        row.set_selected(selected);

        lang_group.add(&row);
        general_page.append(&lang_group);
        Some(row)
    } else {
        None
    };

    (general_page, title_entry, sort_entry, pending_version, app_id_entry, language_row, pending_ra_core, pending_emulator)
}
