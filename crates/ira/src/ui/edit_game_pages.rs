use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use super::css::*;
use super::helpers;
use super::settings_dialog;
use super::state::SharedState;
use adw::prelude::*;
use ira_db::DbConn;
use ira_models::{AppDetails, GameLaunchConfig, WineConfig, WineProfile};

/// Converts a single Lutris game to a managed game by reading its Lutris config
/// and saving a GameLaunchConfig + WineConfig to the database.
/// Returns Ok(()) on success, Err(message) on failure.
pub fn convert_lutris_to_managed(
    db: &DbConn,
    db_id: i64,
    lutris_id: i64,
    game_name: &str,
) -> Result<(), String> {
    let (_runner, _directory, config) =
        ira_platforms::lutris_config::read_lutris_game_config(lutris_id)?;

    let launch = GameLaunchConfig {
        exe: config.game.exe.clone(),
        args: config.game.args.clone(),
        working_dir: config.game.working_dir.clone(),
        env_vars: config
            .system
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        ..Default::default()
    };
    let wine = WineConfig {
        enabled: true,
        prefix: config.game.prefix.clone(),
        version: if config.wine.version.is_empty() {
            "system".to_string()
        } else {
            config.wine.version.clone()
        },
        arch: if config.game.arch.is_empty() {
            "auto".to_string()
        } else {
            config.game.arch.clone()
        },
        esync: config.wine.esync,
        fsync: config.wine.fsync,
        dxvk: config.wine.dxvk,
        vkd3d: config.wine.vkd3d,
        d3d_extras: config.wine.d3d_extras,
        dxvk_nvapi: config.wine.dxvk_nvapi,
        fsr: config.wine.fsr,
        battleye: config.wine.battleye,
        eac: config.wine.eac,
        show_debug: if config.wine.show_debug.is_empty() {
            "-all".to_string()
        } else {
            config.wine.show_debug.clone()
        },
        dll_overrides: config
            .wine
            .overrides
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        ..Default::default()
    };
    let profile_id = {
        let profiles = ira_db::get_all_profiles(db).unwrap_or_default();
        let existing = profiles.iter().find(|p| p.prefix == wine.prefix);
        if let Some(p) = existing {
            Some(p.id)
        } else {
            let profile_name = format!("{} ({})", game_name, wine.version);
            let new_profile = WineProfile {
                id: 0,
                name: profile_name,
                wine_version: wine.version.clone(),
                custom_wine_path: wine.custom_wine_path.clone(),
                prefix: wine.prefix.clone(),
                arch: wine.arch.clone(),
                umu_enabled: wine.umu_enabled,
            };
            ira_db::add_profile(db, &new_profile).ok()
        }
    };
    ira_db::save_game_config(db, db_id, &launch, &wine, profile_id).map_err(|e| e.to_string())?;
    Ok(())
}

pub(super) fn build_dlc_page(
    app_details: &Option<AppDetails>,
    sidebar: &gtk4::ListBox,
    stack: &gtk4::Stack,
) -> Vec<adw::SwitchRow> {
    if let Some(ref details) = app_details {
        if !details.dlcs.is_empty() {
            let dlc_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
            let dlc_group = adw::PreferencesGroup::new();
            dlc_group.set_title(&crate::tr!("DLCs  ·  {}").replacen(
                "{}",
                &details.dlcs.len().to_string(),
                1,
            ));

            let mut dlc_list: Vec<(String, ira_models::DlcInfo)> = details
                .dlcs
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            dlc_list.sort_by_key(|(_, d)| d.app_id);

            let mut switches: Vec<adw::SwitchRow> = Vec::new();
            for (_, dlc) in &dlc_list {
                let row = adw::SwitchRow::new();
                row.set_title(&helpers::esc(&dlc.name));
                row.set_subtitle(&crate::tr!("App ID: {}").replacen(
                    "{}",
                    &dlc.app_id.to_string(),
                    1,
                ));
                row.set_active(dlc.enabled);
                dlc_group.add(&row);
                switches.push(row);
            }
            dlc_page.append(&dlc_group);

            let dlc_scroll = gtk4::ScrolledWindow::new();
            dlc_scroll.set_child(Some(&dlc_page));
            dlc_scroll.set_vexpand(true);
            dlc_scroll.set_hexpand(true);

            sidebar.append(&settings_dialog::settings_sidebar_row(
                "package-x-generic-symbolic",
                &crate::tr!("DLC"),
                "dlc",
            ));
            stack.add_named(&dlc_scroll, Some("dlc"));
            switches
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    }
}

pub(super) struct ApiEmuPageParams<'a> {
    pub emu_exe: &'a str,
    pub emu_game_folder: &'a str,
    pub emu_db_id: i64,
    pub emu_trophy_source: ira_models::TrophySource,
    pub emu_app_id: &'a str,
    pub save_dir: &'a str,
    pub win: &'a adw::Window,
    /// Shared pending-uninstall flag. Reused across page rebuilds so the
    /// dialog's save handler keeps observing the same cell.
    pub emu_pending_uninstall: Option<Rc<RefCell<bool>>>,
}

/// Remove the previously-built "api_emulator" page (sidebar row, its separator,
/// and stack child) so the page can be rebuilt with fresh install state.
fn remove_api_emulator_page(sidebar: &gtk4::ListBox, stack: &gtk4::Stack) {
    if let Some(child) = stack.child_by_name("api_emulator") {
        stack.remove(&child);
    }
    let mut iter = sidebar.first_child();
    let mut prev: Option<gtk4::ListBoxRow> = None;
    while let Some(c) = iter {
        if c.widget_name() == "api_emulator" {
            if let Some(p) = prev {
                if p.widget_name().is_empty() {
                    sidebar.remove(&p);
                }
            }
            sidebar.remove(&c.downcast::<gtk4::ListBoxRow>().unwrap());
            break;
        }
        prev = c.clone().downcast::<gtk4::ListBoxRow>().ok();
        iter = c.next_sibling();
    }
}

fn select_api_emulator_row(sidebar: &gtk4::ListBox) {
    let mut iter = sidebar.first_child();
    while let Some(c) = iter {
        if c.widget_name() == "api_emulator" {
            sidebar.select_row(c.downcast_ref::<gtk4::ListBoxRow>());
            break;
        }
        iter = c.next_sibling();
    }
}

/// Resolve the game's API-emulator DLL folder, preferring the per-game DB
/// cache and falling back to a scan of the install folder (written back to
/// the cache on a miss).
fn resolve_emu_dll_folder(
    db: &DbConn,
    db_id: i64,
    emu_exe: &str,
    emu_game_folder: &str,
    ts: ira_models::TrophySource,
) -> Option<PathBuf> {
    let cached = ira_db::get_api_dll_folder(db, db_id).unwrap_or_default();
    if !cached.is_empty() {
        let folder = PathBuf::from(&cached);
        if folder.is_dir() {
            return Some(folder);
        }
    }
    let found = if ts == ira_models::TrophySource::Gse {
        ira_platforms::api_emulators::find_steam_dll_folder(emu_exe, emu_game_folder)
    } else {
        ira_platforms::api_emulators::find_gog_dll_folder(emu_exe, emu_game_folder)
    };
    if let Some(f) = &found {
        if let Err(e) = ira_db::set_api_dll_folder(db, db_id, &f.to_string_lossy()) {
            eprintln!("Failed to cache API DLL folder: {e}");
        }
    }
    found
}

pub(super) fn build_api_emulator_page(
    params: ApiEmuPageParams,
    state: &SharedState,
    languages: &[String],
    sidebar: &gtk4::ListBox,
    stack: &gtk4::Stack,
) -> Option<Rc<RefCell<bool>>> {
    let (emu_exe, emu_game_folder, emu_db_id, emu_trophy_source, emu_app_id, save_dir, win) = (
        params.emu_exe,
        params.emu_game_folder,
        params.emu_db_id,
        params.emu_trophy_source,
        params.emu_app_id,
        params.save_dir,
        params.win,
    );
    let pending_uninstall = params
        .emu_pending_uninstall
        .unwrap_or_else(|| Rc::new(RefCell::new(false)));
    if (emu_trophy_source != ira_models::TrophySource::Gse
        && emu_trophy_source != ira_models::TrophySource::Nge)
        || emu_exe.is_empty()
    {
        return None;
    }

    let db = state.borrow().db.clone();
    let dll_folder =
        resolve_emu_dll_folder(&db, emu_db_id, emu_exe, emu_game_folder, emu_trophy_source);

    let emu_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    emu_page.set_margin_start(12);
    emu_page.set_margin_end(12);
    emu_page.set_margin_top(12);
    emu_page.set_margin_bottom(12);
    let status_group = adw::PreferencesGroup::new();
    status_group.set_title(&crate::tr!("Status"));

    let status_row = adw::ActionRow::new();
    let is_installed = dll_folder
        .as_deref()
        .map(|d| {
            if emu_trophy_source == ira_models::TrophySource::Gse {
                ira_platforms::api_emulators::is_gse_installed(d)
            } else {
                ira_platforms::api_emulators::is_nge_installed(d)
            }
        })
        .unwrap_or(false);
    let status_title = if is_installed {
        crate::tr!("API emulator installed")
    } else {
        crate::tr!("API emulator not installed")
    };
    status_row.set_title(&status_title);
    status_row.set_sensitive(false);
    status_group.add(&status_row);

    emu_page.append(&status_group);

    let action_group = adw::PreferencesGroup::new();
    action_group.set_title(&crate::tr!("Actions"));

    if is_installed {
        let uninstall_btn = gtk4::Button::with_label(&crate::tr!("Uninstall API emulator"));
        uninstall_btn.add_css_class(CSS_DESTRUCTIVE_ACTION);
        uninstall_btn.set_valign(gtk4::Align::Center);
        let status_c = status_row.clone();
        let pu_c = pending_uninstall.clone();
        let win_c = win.clone();
        uninstall_btn.connect_clicked(move |_| {
            let alert = adw::AlertDialog::new(
                Some(&crate::tr!("Uninstall API emulator?")),
                Some(&crate::tr!("This will restore the original Steam/GOG DLLs. The change will be applied when you save.")),
            );
            alert.add_response("cancel", &crate::tr!("Cancel"));
            alert.add_response("uninstall", &crate::tr!("Uninstall"));
            alert.set_response_appearance("uninstall", adw::ResponseAppearance::Destructive);
            alert.set_default_response(Some("cancel"));
            alert.set_close_response("cancel");
            let pu_c = pu_c.clone();
            let status_c = status_c.clone();
            alert.choose(Some(&win_c), None::<&gio::Cancellable>, move |response| {
                if response == "uninstall" {
                    *pu_c.borrow_mut() = true;
                    status_c.set_title(&crate::tr!("API emulator will be uninstalled on save"));
                }
            });
        });
        let uninstall_row = adw::ActionRow::new();
        uninstall_row.set_title(&crate::tr!("Remove emulator"));
        uninstall_row.set_subtitle(&crate::tr!("Restores original API DLLs (applies on save)"));
        uninstall_row.add_suffix(&uninstall_btn);
        action_group.add(&uninstall_row);
    } else {
        let versions = if emu_trophy_source == ira_models::TrophySource::Gse {
            ira_platforms::api_emulators::list_gse_versions(save_dir)
        } else {
            ira_platforms::api_emulators::list_gog_versions(save_dir)
        };
        let has_dlls = dll_folder
            .as_deref()
            .map(|d| {
                if emu_trophy_source == ira_models::TrophySource::Gse {
                    ira_platforms::api_emulators::has_original_steam_dlls(d)
                } else {
                    ira_platforms::api_emulators::has_original_gog_dlls(d)
                }
            })
            .unwrap_or(false);

        if !has_dlls {
            let missing_row = adw::ActionRow::new();
            missing_row.set_title(&crate::tr!(
                "No original Steam/GOG DLLs detected in game folder"
            ));
            missing_row.set_subtitle(&crate::tr!(
                "Install the game first and make sure it has the original API DLLs"
            ));
            missing_row.set_sensitive(false);
            action_group.add(&missing_row);
        }

        let version_row = if !versions.is_empty() {
            let vr = adw::ComboRow::new();
            vr.set_title(&crate::tr!("Emulator version"));
            vr.set_subtitle(&crate::tr!("Version directory to use for installation"));
            let model = helpers::string_list_from(&versions);
            vr.set_model(Some(&model));
            let default_ver = &state.borrow().cfg.default_api_emu_version;
            if !default_ver.is_empty() {
                if let Some(idx) = versions.iter().position(|v| v == default_ver) {
                    vr.set_selected(idx as u32);
                }
            }
            action_group.add(&vr);
            Some(vr)
        } else {
            let no_ver_row = adw::ActionRow::new();
            no_ver_row.set_title(&crate::tr!("No emulator versions available"));
            no_ver_row.set_subtitle(&crate::tr!("Place version directories in api_emulators/"));
            no_ver_row.set_sensitive(false);
            action_group.add(&no_ver_row);
            None
        };

        let install_btn = gtk4::Button::with_label(&crate::tr!("Install API emulator"));
        install_btn.add_css_class(CSS_SUGGESTED_ACTION);
        install_btn.set_sensitive(has_dlls);
        install_btn.set_valign(gtk4::Align::Center);
        let exe_c = emu_exe.to_string();
        let game_folder_c = emu_game_folder.to_string();
        let app_id_c = emu_app_id.to_string();
        let save_dir_c = save_dir.to_string();
        let db_c = db.clone();
        let db_id_c = emu_db_id;
        let status_c = status_row.clone();
        let langs_c = languages.to_vec();
        let ts_c = emu_trophy_source;
        let state_c = state.clone();
        let win_c = win.clone();
        let sidebar_c = sidebar.clone();
        let stack_c = stack.clone();
        let pending_uninstall_c = pending_uninstall.clone();
        install_btn.connect_clicked(move |_| {
            let ver = version_row
                .as_ref()
                .map(|vr| {
                    let idx = vr.selected() as usize;
                    if idx < versions.len() {
                        versions[idx].clone()
                    } else {
                        String::new()
                    }
                })
                .unwrap_or_default();
            let result = if ts_c == ira_models::TrophySource::Gse {
                ira_platforms::api_emulators::install_gse(
                    &save_dir_c,
                    &exe_c,
                    &game_folder_c,
                    &app_id_c,
                    &langs_c,
                    &ver,
                )
            } else {
                ira_platforms::api_emulators::install_nge(
                    &save_dir_c,
                    &exe_c,
                    &game_folder_c,
                    &app_id_c,
                    &ver,
                )
            };
            match result {
                Ok(folder) => {
                    if let Err(e) =
                        ira_db::set_api_dll_folder(&db_c, db_id_c, &folder.to_string_lossy())
                    {
                        eprintln!("Failed to cache API DLL folder: {}", e);
                    }
                    remove_api_emulator_page(&sidebar_c, &stack_c);
                    build_api_emulator_page(
                        ApiEmuPageParams {
                            emu_exe: &exe_c,
                            emu_game_folder: &game_folder_c,
                            emu_db_id: db_id_c,
                            emu_trophy_source: ts_c,
                            emu_app_id: &app_id_c,
                            save_dir: &save_dir_c,
                            win: &win_c,
                            emu_pending_uninstall: Some(pending_uninstall_c.clone()),
                        },
                        &state_c,
                        &langs_c,
                        &sidebar_c,
                        &stack_c,
                    );
                    select_api_emulator_row(&sidebar_c);
                }
                Err(e) => {
                    eprintln!("Install failed: {}", e);
                    status_c.set_title(&crate::tr!("API emulator install failed"));
                }
            }
        });
        let install_row = adw::ActionRow::new();
        install_row.set_title(&crate::tr!("Install emulator"));
        install_row.set_subtitle(&crate::tr!("Patches the game to use the API emulator"));
        install_row.add_suffix(&install_btn);
        action_group.add(&install_row);
    }

    if emu_trophy_source == ira_models::TrophySource::Gse && is_installed {
        let gen_btn = gtk4::Button::with_label(&crate::tr!("Generate steam_interfaces.txt"));
        gen_btn.add_css_class(CSS_FLAT);
        gen_btn.set_valign(gtk4::Align::Center);
        let exe_c = emu_exe.to_string();
        gen_btn.connect_clicked(move |_| {
            let game_dir = std::path::Path::new(&exe_c).parent();
            if let Some(dir) = game_dir {
                let settings_dir = dir.join("steam_settings");
                let gen_path = settings_dir.join("generate_interfaces");
                if gen_path.is_file() {
                    let _ = std::process::Command::new(&gen_path)
                        .current_dir(&settings_dir)
                        .status();
                } else {
                    eprintln!("generate_interfaces not found in steam_settings folder");
                }
            }
        });
        let gen_row = adw::ActionRow::new();
        gen_row.set_title(&crate::tr!("Generate steam_interfaces.txt"));
        gen_row.set_subtitle(&crate::tr!(
            "Run the generate_interfaces tool from steam_settings"
        ));
        gen_row.add_suffix(&gen_btn);
        action_group.add(&gen_row);
    }

    emu_page.append(&action_group);

    let emu_scroll = gtk4::ScrolledWindow::new();
    emu_scroll.set_child(Some(&emu_page));
    emu_scroll.set_vexpand(true);
    emu_scroll.set_hexpand(true);
    sidebar.append(&settings_dialog::sidebar_separator());
    sidebar.append(&settings_dialog::settings_sidebar_row(
        "applications-engineering-symbolic",
        &crate::tr!("API emulator"),
        "api_emulator",
    ));
    stack.add_named(&emu_scroll, Some("api_emulator"));
    Some(pending_uninstall)
}

pub(super) struct ProfileDropdownParams<'a> {
    pub has_config: bool,
    pub saved_wine_enabled: bool,
    pub saved_profile_id: Option<i64>,
    pub profiles: &'a [WineProfile],
    pub page: &'a gtk4::Box,
    pub state: &'a SharedState,
    pub win: &'a adw::Window,
    pub game_slug: &'a str,
}

pub(super) fn build_profile_dropdown(params: ProfileDropdownParams) -> Option<adw::ComboRow> {
    if !params.has_config || !params.saved_wine_enabled {
        return None;
    }
    let row = super::wine_profile_picker::build_wine_profile_picker(
        params.profiles,
        params.saved_profile_id,
        Some(params.game_slug),
        params.state,
        params.win,
    );
    let profile_group = adw::PreferencesGroup::new();
    profile_group.add(&row);
    params.page.prepend(&profile_group);
    Some(row)
}
