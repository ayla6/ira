use std::cell::RefCell;
use std::rc::Rc;
use adw::prelude::*;
use ira_models::{AppDetails, GameLaunchConfig, GameVariant, WineConfig, WineProfile};
use super::helpers;
use super::settings_dialog;
use super::state::SharedState;
use ira_db::DbConn;

/// Converts a single Lutris game to a managed game by reading its Lutris config
/// and saving a GameLaunchConfig + WineConfig to the database.
/// Returns Ok(()) on success, Err(message) on failure.
pub fn convert_lutris_to_managed(
    db: &DbConn,
    db_id: i64,
    lutris_id: i64,
    game_name: &str,
) -> Result<(), String> {
    let (_runner, _directory, config) = ira_platforms::lutris_config::read_lutris_game_config(lutris_id)?;

    let launch = GameLaunchConfig {
        exe: config.game.exe.clone(),
        args: config.game.args.clone(),
        working_dir: config.game.working_dir.clone(),
        env_vars: config.system.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        ..Default::default()
    };
    let wine = WineConfig {
        enabled: true,
        prefix: config.game.prefix.clone(),
        version: if config.wine.version.is_empty() { "system".to_string() } else { config.wine.version.clone() },
        arch: if config.game.arch.is_empty() { "auto".to_string() } else { config.game.arch.clone() },
        esync: config.wine.esync,
        fsync: config.wine.fsync,
        dxvk: config.wine.dxvk,
        vkd3d: config.wine.vkd3d,
        d3d_extras: config.wine.d3d_extras,
        dxvk_nvapi: config.wine.dxvk_nvapi,
        fsr: config.wine.fsr,
        battleye: config.wine.battleye,
        eac: config.wine.eac,
        show_debug: if config.wine.show_debug.is_empty() { "-all".to_string() } else { config.wine.show_debug.clone() },
        dll_overrides: config.wine.overrides.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        gamemode: config.system.gamemode,
        mangohud: config.system.mangohud,
        gamescope: config.system.gamescope,
        gamescope_flags: config.system.gamescope_flags.clone(),
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
            dlc_group.set_title(&format!("DLCs  ·  {}", details.dlcs.len()));

            let mut dlc_list: Vec<(String, ira_models::DlcInfo)> = details.dlcs.iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            dlc_list.sort_by_key(|(_, d)| d.app_id);

            let mut switches: Vec<adw::SwitchRow> = Vec::new();
            for (_, dlc) in &dlc_list {
                let row = adw::SwitchRow::new();
                row.set_title(&helpers::esc(&dlc.name));
                row.set_subtitle(&format!("App ID: {}", dlc.app_id));
                row.set_active(dlc.enabled);
                dlc_group.add(&row);
                switches.push(row);
            }
            dlc_page.append(&dlc_group);

            let dlc_scroll = gtk4::ScrolledWindow::new();
            dlc_scroll.set_child(Some(&dlc_page));
            dlc_scroll.set_vexpand(true);
            dlc_scroll.set_hexpand(true);

            sidebar.append(&settings_dialog::settings_sidebar_row("package-x-generic-symbolic", "DLC"));
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
    pub emu_trophy_source: ira_models::TrophySource,
    pub emu_app_id: &'a str,
    pub save_dir: &'a str,
}

pub(super) fn build_api_emulator_page(
    params: ApiEmuPageParams,
    state: &SharedState,
    languages: &[String],
    sidebar: &gtk4::ListBox,
    stack: &gtk4::Stack,
) {
    let (emu_exe, emu_trophy_source, emu_app_id, save_dir) = 
        (params.emu_exe, params.emu_trophy_source, params.emu_app_id, params.save_dir);
    if (emu_trophy_source != ira_models::TrophySource::Gse && emu_trophy_source != ira_models::TrophySource::Nge) || emu_exe.is_empty() {
        return;
    }

    let emu_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    emu_page.set_margin_start(12);
    emu_page.set_margin_end(12);
    emu_page.set_margin_top(12);
    emu_page.set_margin_bottom(12);
    let status_group = adw::PreferencesGroup::new();
    status_group.set_title("Status");

    let status_row = adw::ActionRow::new();
    let is_installed = if emu_trophy_source == ira_models::TrophySource::Gse {
        ira_platforms::api_emulators::is_gse_installed(emu_exe)
    } else {
        ira_platforms::api_emulators::is_nge_installed(emu_exe)
    };
    status_row.set_title(if is_installed { "API emulator installed" } else { "API emulator not installed" });
    status_row.set_sensitive(false);
    status_group.add(&status_row);

    emu_page.append(&status_group);

    let action_group = adw::PreferencesGroup::new();
    action_group.set_title("Actions");

    if is_installed {
        let uninstall_btn = gtk4::Button::with_label("Uninstall API emulator");
        uninstall_btn.add_css_class("destructive-action");
        let exe_c = emu_exe.to_string();
        let status_c = status_row.clone();
        let ts_c = emu_trophy_source;
        uninstall_btn.connect_clicked(move |_| {
            let result = if ts_c == ira_models::TrophySource::Gse {
                ira_platforms::api_emulators::uninstall_gse(&exe_c)
            } else {
                ira_platforms::api_emulators::uninstall_nge(&exe_c)
            };
            match result {
                Ok(()) => {
                    status_c.set_title("API emulator not installed");
                }
                Err(e) => eprintln!("Uninstall failed: {}", e),
            }
        });
        action_group.add(&uninstall_btn);
    } else {
        let versions = if emu_trophy_source == ira_models::TrophySource::Gse {
            ira_platforms::api_emulators::list_gse_versions(save_dir)
        } else {
            ira_platforms::api_emulators::list_gog_versions(save_dir)
        };
        let has_dlls = if emu_trophy_source == ira_models::TrophySource::Gse {
            ira_platforms::api_emulators::has_original_steam_dlls(emu_exe)
        } else {
            ira_platforms::api_emulators::has_original_gog_dlls(emu_exe)
        };

        if !has_dlls {
            let missing_row = adw::ActionRow::new();
            missing_row.set_title("No original Steam/GOG DLLs detected in game folder");
            missing_row.set_subtitle("Install the game first and make sure it has the original API DLLs");
            missing_row.set_sensitive(false);
            action_group.add(&missing_row);
        }

        let version_row = if !versions.is_empty() {
            let vr = adw::ComboRow::new();
            vr.set_title("Emulator version");
            vr.set_subtitle("Version directory to use for installation");
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
            no_ver_row.set_title("No emulator versions available");
            no_ver_row.set_subtitle("Place version directories in api_emulators/");
            no_ver_row.set_sensitive(false);
            action_group.add(&no_ver_row);
            None
        };

        let install_btn = gtk4::Button::with_label("Install API emulator");
        install_btn.add_css_class("suggested-action");
        install_btn.set_sensitive(has_dlls);
        let exe_c = emu_exe.to_string();
        let app_id_c = emu_app_id.to_string();
        let save_dir_c = save_dir.to_string();
        let status_c = status_row.clone();
        let langs_c = languages.to_vec();
        let ts_c = emu_trophy_source;
        install_btn.connect_clicked(move |_| {
            let ver = version_row.as_ref().map(|vr| {
                let idx = vr.selected() as usize;
                if idx < versions.len() { versions[idx].clone() } else { String::new() }
            }).unwrap_or_default();
            let result = if ts_c == ira_models::TrophySource::Gse {
                ira_platforms::api_emulators::install_gse(&save_dir_c, &exe_c, &app_id_c, &langs_c, &ver)
            } else {
                ira_platforms::api_emulators::install_nge(&save_dir_c, &exe_c, &app_id_c, &ver)
            };
            match result {
                Ok(()) => {
                    status_c.set_title("API emulator installed");
                }
                Err(e) => eprintln!("Install failed: {}", e),
            }
        });
        action_group.add(&install_btn);
    }

    if emu_trophy_source == ira_models::TrophySource::Gse && is_installed {
        let gen_btn = gtk4::Button::with_label("Generate steam_interfaces.txt");
        gen_btn.add_css_class("flat");
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
        action_group.add(&gen_btn);
    }

    emu_page.append(&action_group);

    let emu_scroll = gtk4::ScrolledWindow::new();
    emu_scroll.set_child(Some(&emu_page));
    emu_scroll.set_vexpand(true);
    emu_scroll.set_hexpand(true);
    sidebar.append(&settings_dialog::sidebar_separator());
    sidebar.append(&settings_dialog::settings_sidebar_row("applications-engineering-symbolic", "API Emulator"));
    stack.add_named(&emu_scroll, Some("api_emulator"));
}

pub(super) struct VarW {
    pub(super) id: Option<i64>,
    pub(super) name: adw::EntryRow,
    pub(super) exe: adw::EntryRow,
    pub(super) wd: adw::EntryRow,
    pub(super) args: adw::EntryRow,
    pub(super) pre_launch: adw::EntryRow,
    pub(super) custom_images: adw::SwitchRow,
    pub(super) show_as_entry: adw::SwitchRow,
    pub(super) group: adw::PreferencesGroup,
}

fn move_variant_card(
    group: &adw::PreferencesGroup,
    delta: isize,
    var_widgets: &Rc<RefCell<Vec<VarW>>>,
    container: &gtk4::Box,
) {
    let widgets = var_widgets.borrow();
    let idx = widgets.iter().position(|w| w.group == *group);
    if let Some(i) = idx {
        let new_i = i as isize + delta;
        if new_i < 0 || new_i >= widgets.len() as isize { return; }
        let new_i = new_i as usize;
        drop(widgets);
        let mut widgets = var_widgets.borrow_mut();
        widgets.swap(i, new_i);
        let sibling = if new_i == 0 { None } else { Some(widgets[new_i - 1].group.clone()) };
        drop(widgets);
        match &sibling {
            None => container.reorder_child_after(group, None::<&gtk4::Widget>),
            Some(sib) => container.reorder_child_after(group, Some(sib)),
        }
    }
}

pub(super) fn build_variants_page(
    state: &SharedState,
    db_id: i64,
    game_kind: ira_models::GameKind,
    _has_config: bool,
    sidebar: &gtk4::ListBox,
    stack: &gtk4::Stack,
    win: &adw::Window,
) -> Rc<RefCell<Vec<VarW>>> {
    let variants: Vec<GameVariant> = ira_db::get_variants(&state.borrow().db, db_id).unwrap_or_default();
    let variant_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    header.set_margin_start(12);
    header.set_margin_end(12);
    let title = gtk4::Label::new(Some("Variants"));
    title.set_halign(gtk4::Align::Start);
    title.set_hexpand(true);
    title.add_css_class("heading");
    let add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_btn.set_tooltip_text(Some("Add variant"));
    add_btn.set_valign(gtk4::Align::Center);
    add_btn.add_css_class("flat");
    header.append(&title);
    header.append(&add_btn);
    variant_page.append(&header);

    let variant_container = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    variant_container.set_margin_start(12);
    variant_container.set_margin_end(12);
    variant_page.append(&variant_container);

    let var_widgets: Rc<RefCell<Vec<VarW>>> = Rc::new(RefCell::new(Vec::new()));

    let add_variant_fn = {
        let var_widgets = var_widgets.clone();
        let container = variant_container.clone();
        let state_for_fn = state.clone();
        let db_id_for_fn = db_id;
        let win_for_fn = win.clone();
        move |v: GameVariant| {
            let group = adw::PreferencesGroup::new();

            let name_entry = adw::EntryRow::new();
            name_entry.set_title("Variant name");
            name_entry.set_text(&v.name);

            let up_btn = gtk4::Button::from_icon_name("go-up-symbolic");
            up_btn.set_tooltip_text(Some("Move up"));
            up_btn.set_valign(gtk4::Align::Center);
            let group_up = group.clone();
            let vw_up = var_widgets.clone();
            let container_up = container.clone();
            up_btn.connect_clicked(move |_| {
                move_variant_card(&group_up, -1, &vw_up, &container_up);
            });
            name_entry.add_suffix(&up_btn);

            let down_btn = gtk4::Button::from_icon_name("go-down-symbolic");
            down_btn.set_tooltip_text(Some("Move down"));
            down_btn.set_valign(gtk4::Align::Center);
            let group_down = group.clone();
            let vw_down = var_widgets.clone();
            let container_down = container.clone();
            down_btn.connect_clicked(move |_| {
                move_variant_card(&group_down, 1, &vw_down, &container_down);
            });
            name_entry.add_suffix(&down_btn);

            let del_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
            del_btn.set_tooltip_text(Some("Delete variant"));
            del_btn.set_valign(gtk4::Align::Center);
            del_btn.add_css_class("error");
            let container_c = container.clone();
            let group_c = group.clone();
            del_btn.connect_clicked(move |_| {
                container_c.remove(&group_c);
            });
            name_entry.add_suffix(&del_btn);

            group.add(&name_entry);

            let exe_entry = adw::EntryRow::new();
            exe_entry.set_title("Executable");
            exe_entry.set_text(&v.exe);
            let browse = helpers::make_browse_button(
                None,
                "Select variant executable",
                false,
                Some(("Executable", &["application/x-executable"])),
                {
                    let entry = exe_entry.clone();
                    move |path| entry.set_text(&path.to_string_lossy())
                },
            );
            exe_entry.add_suffix(&browse);
            group.add(&exe_entry);

            let args_entry = adw::EntryRow::new();
            args_entry.set_title("Arguments");
            args_entry.set_text(&v.args);
            group.add(&args_entry);

            let wd_entry = adw::EntryRow::new();
            wd_entry.set_title("Working directory");
            wd_entry.set_text(&v.working_dir);
            let wd_browse = helpers::make_browse_button(
                None,
                "Select working directory",
                true,
                None,
                {
                    let entry = wd_entry.clone();
                    move |path| entry.set_text(&path.to_string_lossy())
                },
            );
            wd_entry.add_suffix(&wd_browse);
            group.add(&wd_entry);

            let pre_launch_entry = adw::EntryRow::new();
            pre_launch_entry.set_title("Run before game");
            pre_launch_entry.set_text(&v.pre_launch);
            pre_launch_entry.set_tooltip_text(Some("Shell command to run before launching. If it fails, the game will not launch."));
            group.add(&pre_launch_entry);

            let custom_images_row = adw::SwitchRow::new();
            custom_images_row.set_title("Custom images");
            custom_images_row.set_subtitle("Use custom hero and logo for this variant");
            custom_images_row.set_active(v.custom_images || v.show_as_entry);
            group.add(&custom_images_row);

            let show_as_entry_row = adw::SwitchRow::new();
            show_as_entry_row.set_title("Show as separate entry");
            show_as_entry_row.set_subtitle("Appears in the grid as its own game entry");
            show_as_entry_row.set_active(v.show_as_entry);
            group.add(&show_as_entry_row);

            {
                let custom_images_c = custom_images_row.clone();
                show_as_entry_row.connect_notify_local(Some("active"), move |row, _| {
                    if row.is_active() {
                        custom_images_c.set_active(true);
                        custom_images_c.set_sensitive(false);
                    } else {
                        custom_images_c.set_sensitive(true);
                    }
                });
            }

            if v.show_as_entry {
                custom_images_row.set_sensitive(false);
            }

            let images_expander = adw::ExpanderRow::new();
            images_expander.set_title("Manage images");
            images_expander.set_enable_expansion(custom_images_row.is_active() && v.id > 0);
            if v.id == 0 {
                images_expander.set_subtitle("Save the variant first");
            }
            group.add(&images_expander);
            {
                let images_expander_c = images_expander.clone();
                custom_images_row.connect_notify_local(Some("active"), move |row, _| {
                    images_expander_c.set_enable_expansion(row.is_active());
                });
            }
            if v.id > 0 {
                let state_c = state_for_fn.clone();
                let db_id_c = db_id_for_fn;
                let variant_id = v.id;
                let parent_win = win_for_fn.clone();
                let save_dir = state_for_fn.borrow().save_dir.clone();
                let db = state_for_fn.borrow().db.clone();
                if let Ok(Some(entry)) = ira_db::find_by_db_id(&db, db_id_c) {
                    let image_dir = ira_parser::entry_data_dir(&save_dir, &entry);
                    let var_dir = image_dir.join(format!("variant-{}", variant_id));
                    let _ = std::fs::create_dir_all(&var_dir);
                    for &(label, file_base, asset_type, thumb_h) in &[
                        ("Icon", "icon", "icon", 48i32),
                        ("Hero", "hero", "hero", 64),
                        ("Capsule", "vertical", "grid", 64),
                        ("Header", "header", "header", 48),
                        ("Logo", "logo", "logo", 64),
                    ] {
                        let row = super::image_manager::build_image_section_for_dir(
                            super::image_manager::VariantImageSectionParams {
                                target_dir: &var_dir, label, file_base, asset_type, max_h: thumb_h,
                                state: &state_c, entry: &entry, parent_win: &parent_win,
                            },
                        );
                        images_expander.add_row(&row);
                    }
                }
            }

            container.append(&group);

            var_widgets.borrow_mut().push(VarW {
                id: if v.id > 0 { Some(v.id) } else { None },
                name: name_entry,
                exe: exe_entry,
                wd: wd_entry,
                args: args_entry,
                pre_launch: pre_launch_entry,
                custom_images: custom_images_row,
                show_as_entry: show_as_entry_row,
                group,
            });
        }
    };

    for v in &variants {
        add_variant_fn(v.clone());
    }

    {
        let add_variant_fn = add_variant_fn;
        let new_v = GameVariant { game_id: db_id, ..Default::default() };
        add_btn.connect_clicked(move |_| add_variant_fn(new_v.clone()));
    }

    let variant_scroll = gtk4::ScrolledWindow::new();
    variant_scroll.set_child(Some(&variant_page));
    variant_scroll.set_vexpand(true);
    variant_scroll.set_hexpand(true);
    if game_kind != ira_models::GameKind::Steam && game_kind != ira_models::GameKind::Ps4 && game_kind != ira_models::GameKind::Retro {
        sidebar.append(&settings_dialog::sidebar_separator());
        sidebar.append(&settings_dialog::settings_sidebar_row("application-x-executable-symbolic", "Variants"));
        stack.add_named(&variant_scroll, Some("variants"));
    }

    var_widgets
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
    let profile_row: Option<adw::ComboRow> = if params.has_config && params.saved_wine_enabled {
        let profile_labels: Vec<String> = std::iter::once("New prefix".to_string())
            .chain(params.profiles.iter().map(|p| p.name.clone()))
            .collect();
        let profile_model = helpers::string_list_from(&profile_labels);
        let pr = adw::ComboRow::new();
        pr.set_title("Wine Profile");
        pr.set_subtitle("Links wine version + prefix together");
        pr.set_model(Some(&profile_model));
        if let Some(pid) = params.saved_profile_id {
            for (i, p) in params.profiles.iter().enumerate() {
                if p.id == pid {
                    pr.set_selected((i + 1) as u32);
                    break;
                }
            }
        }

        let edit_btn = gtk4::Button::new();
        edit_btn.set_icon_name("document-edit-symbolic");
        edit_btn.set_tooltip_text(Some("Edit profile"));
        edit_btn.set_valign(gtk4::Align::Center);
        edit_btn.add_css_class("flat");
        let profiles_c: Vec<WineProfile> = params.profiles.to_vec();
        let pr_c = pr.clone();
        let state_c = params.state.clone();
        let win_c = params.win.clone();
        let slug_c = params.game_slug.to_string();
        edit_btn.connect_clicked(move |_| {
            let idx = pr_c.selected() as usize;
            let parent = state_c.borrow().window.clone();
            let db = state_c.borrow().db.clone();
            let slug_arg: Option<&str> = if idx == 0 { Some(&slug_c) } else { None };
            if idx == 0 {
                super::profile_dialog::show_profile_dialog(&parent, &db, None, &state_c, &win_c, None, slug_arg);
            } else if let Some(p) = profiles_c.get(idx - 1) {
                super::profile_dialog::show_profile_dialog(&parent, &db, Some(p.clone()), &state_c, &win_c, None, slug_arg);
            }
        });

        pr.add_suffix(&edit_btn);

        let profile_group = adw::PreferencesGroup::new();
        profile_group.add(&pr);
        params.page.prepend(&profile_group);
        Some(pr)
    } else {
        None
    };

    profile_row
}
