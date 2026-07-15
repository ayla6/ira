use gtk4::prelude::*;
use adw::prelude::*;
use ira_api::SteamClient;
use ira_config::Config;
use crate::strings as S;
use std::sync::Arc;
use super::helpers::{dialog_layout, make_browse_button, string_list_from};
use super::profile_dialog::build_profiles_page;
use super::state::SharedState;
use super::wine_config_widget::build_wine_config_pages;

fn settings_page_container() -> gtk4::Box {
    gtk4::Box::new(gtk4::Orientation::Vertical, 16)
}

pub(super) fn settings_sidebar_row(icon: &str, label: &str) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    let icon = gtk4::Image::from_icon_name(icon);
    let text = gtk4::Label::new(Some(label));
    text.set_halign(gtk4::Align::Start);
    hbox.append(&icon);
    hbox.append(&text);
    row.set_child(Some(&hbox));
    row.set_size_request(-1, 36);
    row
}

pub(super) fn sidebar_separator() -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    row.set_child(Some(&sep));
    row.set_selectable(false);
    row.set_sensitive(false);
    row.add_css_class("sidebar-separator-row");
    row
}

fn build_general_settings_page(cfg: &Config) -> (gtk4::Box, adw::SwitchRow, adw::SwitchRow, adw::SwitchRow, gtk4::SpinButton) {
    let page = settings_page_container();

    let notif_group = adw::PreferencesGroup::new();
    notif_group.set_title(S::LIVE_UPDATES);

    let notif_row = adw::SwitchRow::new();
    notif_row.set_title(S::NOTIFY_ON_UNLOCKS);
    notif_row.set_subtitle(S::NOTIFY_SUBTITLE);
    notif_row.set_active(cfg.notifications_enabled);
    notif_group.add(&notif_row);

    let bg_row = adw::SwitchRow::new();
    bg_row.set_title(S::CLOSE_TO_BG_TITLE);
    bg_row.set_subtitle(S::CLOSE_TO_BG_SUBTITLE);
    bg_row.set_active(cfg.close_to_background);
    notif_group.add(&bg_row);
    page.append(&notif_group);

    let hidden_group = adw::PreferencesGroup::new();
    let hidden_row = adw::SwitchRow::new();
    hidden_row.set_title(S::SHOW_HIDDEN_GAMES);
    hidden_row.set_active(cfg.show_hidden_games);
    hidden_group.add(&hidden_row);
    page.append(&hidden_group);

    let grid_group = adw::PreferencesGroup::new();
    let grid_adj = gtk4::Adjustment::new(cfg.grid_cover_width as f64, 120.0, 320.0, 10.0, 20.0, 0.0);
    let grid_spin = gtk4::SpinButton::new(Some(&grid_adj), 1.0, 0);
    let grid_row = adw::ActionRow::new();
    grid_row.set_title(S::COVER_SIZE);
    grid_row.add_suffix(&grid_spin);
    grid_group.add(&grid_row);
    page.append(&grid_group);

    (page, notif_row, bg_row, hidden_row, grid_spin)
}

fn build_lutris_settings_page(cfg: &Config) -> (gtk4::Box, adw::SwitchRow) {
    let page = settings_page_container();

    let enable_group = adw::PreferencesGroup::new();
    let enable_row = adw::SwitchRow::new();
    enable_row.set_title("Enable Lutris integration");
    enable_row.set_subtitle("Load games from the Lutris database");
    enable_row.set_active(cfg.lutris_enabled);
    enable_group.add(&enable_row);
    page.append(&enable_group);

    let info_group = adw::PreferencesGroup::new();
    info_group.set_title("Lutris installation");

    let lutris_dir = std::path::Path::new(&std::env::var("HOME").unwrap_or_default()).join(".local/share/lutris");
    let dir_row = adw::ActionRow::new();
    dir_row.set_title("Lutris data directory");
    if lutris_dir.is_dir() {
        dir_row.set_subtitle(&lutris_dir.display().to_string());
    } else {
        dir_row.set_subtitle("Lutris not found");
        dir_row.set_sensitive(false);
    }
    info_group.add(&dir_row);
    page.append(&info_group);

    (page, enable_row)
}

fn build_api_keys_page(cfg: &Config) -> (gtk4::Box, adw::EntryRow, adw::EntryRow) {
    let page = settings_page_container();

    let key_group = adw::PreferencesGroup::new();
    key_group.set_title(S::API_KEYS);

    let steam_entry = adw::PasswordEntryRow::new();
    steam_entry.set_title(S::STEAM_WEB_API_KEY);
    steam_entry.set_text(&cfg.steam_api_key);
    key_group.add(&steam_entry);

    let sgdb_entry = adw::PasswordEntryRow::new();
    sgdb_entry.set_title(S::STEAMGRIDDB_KEY);
    sgdb_entry.set_text(&cfg.steam_griddb_api_key);
    key_group.add(&sgdb_entry);

    page.append(&key_group);

    (page, steam_entry.upcast(), sgdb_entry.upcast())
}

pub(super) fn build_shadps4_version_dropdown(current_path: &str, include_global: bool) -> gtk4::DropDown {
    let shadps4_versions = ira_platforms::ps4::read_shadps4_versions();
    let trunc = |s: &str, max: usize| -> String {
        if s.len() > max {
            format!("{}…", &s[..max.saturating_sub(1)])
        } else {
            s.to_string()
        }
    };
    let mut version_strings: Vec<String> = Vec::new();
    if include_global {
        version_strings.push("Follow global".to_string());
    }
    for v in &shadps4_versions {
        let extra = if !v.date.is_empty() { v.date.clone() } else { v.codename.clone() };
        version_strings.push(format!("{}  ({})", v.name, trunc(&extra, 14)));
    }
    let version_model = string_list_from(&version_strings);
    let version_dropdown = gtk4::DropDown::new(Some(version_model), None::<&gtk4::PropertyExpression>);

    let mut selected_idx: u32 = 0;
    if !current_path.is_empty() {
        for (i, v) in shadps4_versions.iter().enumerate() {
            let v_path = v.path.trim_matches('"');
            if v_path == current_path {
                selected_idx = if include_global { (i + 1) as u32 } else { i as u32 };
                break;
            }
        }
    }
    version_dropdown.set_selected(selected_idx);
    version_dropdown
}

fn build_shadps4_settings_page(cfg: &Config, win: &adw::Window) -> (gtk4::Box, adw::SwitchRow, adw::EntryRow) {
    let page = settings_page_container();

    let ps4_enable_group = adw::PreferencesGroup::new();
    let ps4_enable_row = adw::SwitchRow::new();
    ps4_enable_row.set_title("Enable PS4 integration");
    ps4_enable_row.set_subtitle("Scan shadPS4 install directories for PS4 games");
    ps4_enable_row.set_active(cfg.shadps4_enabled);
    ps4_enable_group.add(&ps4_enable_row);
    page.append(&ps4_enable_group);

    let ps4_exe_group = adw::PreferencesGroup::new();
    ps4_exe_group.set_title("Emulator");

    let ps4_exe_row = adw::EntryRow::new();
    ps4_exe_row.set_title("shadPS4 executable path");
    ps4_exe_row.set_text(&cfg.shadps4_executable);

    let shadps4_versions = ira_platforms::ps4::read_shadps4_versions();
    let detected_path = ira_platforms::ps4::detect_shadps4_version_path();

    if !shadps4_versions.is_empty() {
        let current_exe = if cfg.shadps4_executable.is_empty() {
            detected_path.clone().unwrap_or_default()
        } else {
            cfg.shadps4_executable.clone()
        };
        let version_dropdown = build_shadps4_version_dropdown(&current_exe, false);

        let ps4_exe_row_c = ps4_exe_row.clone();
        version_dropdown.connect_selected_notify(move |dd| {
            let idx = dd.selected();
            if let Some(versions) = ira_platforms::ps4::read_shadps4_versions().into_iter().nth(idx as usize) {
                let path = versions.path.trim_matches('"').to_string();
                ps4_exe_row_c.set_text(&path);
            }
        });

        let version_row = adw::ActionRow::new();
        version_row.set_title("Version");
        version_row.set_subtitle("Select a shadPS4 version");
        version_dropdown.set_valign(gtk4::Align::Center);
        version_row.add_suffix(&version_dropdown);
        ps4_exe_group.add(&version_row);
    }

    if let Some(ref detected) = detected_path {
        let auto_btn = gtk4::Button::with_label("Auto-detect");
        auto_btn.add_css_class("flat");
        auto_btn.set_valign(gtk4::Align::Center);
        let exe_row = ps4_exe_row.clone();
        let detected_path = detected.clone();
        auto_btn.connect_clicked(move |_| {
            exe_row.set_text(&detected_path);
        });
        ps4_exe_row.add_suffix(&auto_btn);
    }

    let ps4_exe_browse = make_browse_button(
        Some(win),
        "Select shadPS4 executable",
        false,
        Some(("Executable", &["application/x-executable"])),
        {
            let row = ps4_exe_row.clone();
            move |path| row.set_text(&path.to_string_lossy())
        },
    );
    ps4_exe_row.add_suffix(&ps4_exe_browse);
    ps4_exe_group.add(&ps4_exe_row);
    page.append(&ps4_exe_group);

    let ps4_dirs_group = adw::PreferencesGroup::new();
    ps4_dirs_group.set_title("Install directories");
    ps4_dirs_group.set_description(Some("Managed by shadPS4"));
    let install_dirs = ira_platforms::ps4::read_install_dirs();
    if install_dirs.is_empty() {
        let empty_row = adw::ActionRow::new();
        empty_row.set_title("No install directories configured");
        empty_row.set_sensitive(false);
        ps4_dirs_group.add(&empty_row);
    } else {
        for dir in &install_dirs {
            let dir_row = adw::ActionRow::new();
            dir_row.set_title(&dir.display().to_string());
            dir_row.set_sensitive(false);
            ps4_dirs_group.add(&dir_row);
        }
    }
    page.append(&ps4_dirs_group);

    (page, ps4_enable_row, ps4_exe_row)
}

fn build_steam_settings_page(cfg: &Config) -> (gtk4::Box, adw::SwitchRow) {
    let page = settings_page_container();

    let enable_group = adw::PreferencesGroup::new();
    let enable_row = adw::SwitchRow::new();
    enable_row.set_title("Enable Steam integration");
    enable_row.set_subtitle("Scan your Steam library for installed games and achievements");
    enable_row.set_active(cfg.steam_enabled);
    enable_group.add(&enable_row);
    page.append(&enable_group);

    let info_group = adw::PreferencesGroup::new();
    info_group.set_title("Steam installation");

    let steam_dir = ira_platforms::steam::steam_install_dir();
    let dir_row = adw::ActionRow::new();
    dir_row.set_title("Steam directory");
    match &steam_dir {
        Some(path) => dir_row.set_subtitle(&path.display().to_string()),
        None => {
            dir_row.set_subtitle("Steam not found");
            dir_row.set_sensitive(false);
        }
    }
    info_group.add(&dir_row);

    let user_ids = ira_platforms::steam::get_steam_user_ids();
    let user_row = adw::ActionRow::new();
    user_row.set_title("Steam user IDs");
    if user_ids.is_empty() {
        user_row.set_subtitle("None found");
        user_row.set_sensitive(false);
    } else {
        user_row.set_subtitle(&user_ids.join(", "));
    }
    info_group.add(&user_row);
    page.append(&info_group);

    (page, enable_row)
}

fn build_ra_settings_page(cfg: &Config) -> (gtk4::Box, adw::SwitchRow, adw::EntryRow, adw::EntryRow, adw::EntryRow) {
    let page = settings_page_container();

    let enable_group = adw::PreferencesGroup::new();
    let enable_row = adw::SwitchRow::new();
    enable_row.set_title("Enable RetroAchievements");
    enable_row.set_subtitle("Fetch achievements for matched retro games from retroachievements.org");
    enable_row.set_active(cfg.ra_enabled);
    enable_group.add(&enable_row);
    page.append(&enable_group);

    let creds_group = adw::PreferencesGroup::new();
    creds_group.set_title("Account");

    let username_row = adw::EntryRow::new();
    username_row.set_title("Username");
    username_row.set_text(&cfg.ra_username);
    creds_group.add(&username_row);

    let password_row = adw::PasswordEntryRow::new();
    password_row.set_title("Password");
    password_row.set_text(&cfg.ra_password);
    creds_group.add(&password_row);

    let token_row = adw::PasswordEntryRow::new();
    token_row.set_title("API Token (optional — auto-fetched from password)");
    token_row.set_text(&cfg.ra_token);
    creds_group.add(&token_row);
    page.append(&creds_group);

    (page, enable_row, username_row, password_row.upcast(), token_row.upcast())
}

struct ConsolePageWidgets {
    enable_row: adw::SwitchRow,
    folder_row: adw::EntryRow,
    exe_row: adw::EntryRow,
    core_dropdown: Option<gtk4::DropDown>,
    fullscreen_row: adw::SwitchRow,
}

fn build_console_settings_page(
    win: &adw::Window,
    def: &ira_models::ConsoleDef,
    cc: &ira_config::ConsoleConfig,
) -> (gtk4::Box, ConsolePageWidgets) {
    let page = settings_page_container();

    let enable_group = adw::PreferencesGroup::new();
    let enable_row = adw::SwitchRow::new();
    enable_row.set_title(&format!("Enable {} ROM discovery", def.display_name));
    enable_row.set_subtitle(&format!("Scan for {} ROM files in the configured folder", def.display_name));
    enable_row.set_active(cc.enabled);
    enable_group.add(&enable_row);
    page.append(&enable_group);

    let rom_group = adw::PreferencesGroup::new();
    rom_group.set_title("ROMs");

    let folder_row = adw::EntryRow::new();
    folder_row.set_title("ROM folder");
    folder_row.set_text(&cc.folder);

    let folder_browse = make_browse_button(
        Some(win),
        "Select ROM folder",
        true,
        None,
        {
            let row = folder_row.clone();
            move |path| row.set_text(&path.to_string_lossy())
        },
    );
    folder_row.add_suffix(&folder_browse);
    rom_group.add(&folder_row);
    page.append(&rom_group);

    let emu_group = adw::PreferencesGroup::new();
    emu_group.set_title("Emulator");

    let detected_emulators = ira_platforms::emulator_detect::detect_emulators(def.id);

    let exe_row = adw::EntryRow::new();
    exe_row.set_title("Emulator executable");
    exe_row.set_text(&cc.executable);

    let mut core_dropdown: Option<gtk4::DropDown> = None;

    if !detected_emulators.is_empty() {
        let emu_names: Vec<String> = detected_emulators.iter()
            .map(|e| e.display_name.clone())
            .collect();
        let emu_model = string_list_from(&emu_names);
        let emu_dropdown = gtk4::DropDown::new(Some(emu_model), None::<&gtk4::PropertyExpression>);

        let mut selected_idx: u32 = 0;
        if !cc.executable.is_empty() {
            for (i, e) in detected_emulators.iter().enumerate() {
                if e.launch_command == cc.executable {
                    selected_idx = i as u32;
                    break;
                }
            }
        }
        emu_dropdown.set_selected(selected_idx);

        let exe_row_c = exe_row.clone();
        let emus_clone = detected_emulators.clone();
        emu_dropdown.connect_selected_notify(move |dd| {
            let idx = dd.selected() as usize;
            if let Some(e) = emus_clone.get(idx) {
                exe_row_c.set_text(&e.launch_command);
            }
        });

        let emu_select_row = adw::ActionRow::new();
        emu_select_row.set_title("Detected emulators");
        emu_dropdown.set_valign(gtk4::Align::Center);
        emu_select_row.add_suffix(&emu_dropdown);
        emu_group.add(&emu_select_row);
    }

    let auto_btn = gtk4::Button::with_label("Auto-detect");
    auto_btn.add_css_class("flat");
    auto_btn.set_valign(gtk4::Align::Center);
    {
        let exe_row = exe_row.clone();
        let emus_clone = detected_emulators.clone();
        auto_btn.connect_clicked(move |_| {
            if let Some(e) = emus_clone.first() {
                exe_row.set_text(&e.launch_command);
            }
        });
    }
    exe_row.add_suffix(&auto_btn);

    let exe_browse = make_browse_button(
        Some(win),
        "Select emulator executable",
        false,
        Some(("Executable", &["application/x-executable"])),
        {
            let row = exe_row.clone();
            move |path| row.set_text(&path.to_string_lossy())
        },
    );
    exe_row.add_suffix(&exe_browse);
    emu_group.add(&exe_row);

    let is_ra = ira_platforms::emulator_detect::is_retroarch(&cc.executable);
    if is_ra {
        let cores = ira_platforms::emulator_detect::detect_ra_cores();
        if !cores.is_empty() {
            let mut core_names: Vec<String> = vec!["None (auto-detect)".to_string()];
            core_names.extend(cores.iter().map(|c| c.display_name.clone()));
            let core_model = string_list_from(&core_names);
            let dropdown = gtk4::DropDown::new(Some(core_model), None::<&gtk4::PropertyExpression>);

            let mut selected_idx: u32 = 0;
            if !cc.ra_core.is_empty() {
                for (i, c) in cores.iter().enumerate() {
                    if c.path == cc.ra_core {
                        selected_idx = (i + 1) as u32;
                        break;
                    }
                }
            }
            dropdown.set_selected(selected_idx);

            let core_row = adw::ActionRow::new();
            core_row.set_title("RetroArch core");
            core_row.set_subtitle("Select a core for this console");
            dropdown.set_valign(gtk4::Align::Center);
            core_row.add_suffix(&dropdown);
            emu_group.add(&core_row);

            core_dropdown = Some(dropdown);
        }
    }

    let fullscreen_row = adw::SwitchRow::new();
    fullscreen_row.set_title("Start games in fullscreen");
    fullscreen_row.set_subtitle("Launch the emulator in fullscreen mode");
    fullscreen_row.set_active(cc.fullscreen);
    emu_group.add(&fullscreen_row);

    page.append(&emu_group);
    (page, ConsolePageWidgets { enable_row, folder_row, exe_row, core_dropdown, fullscreen_row })
}

fn build_api_emulators_page(cfg: &Config) -> (gtk4::Box, adw::ComboRow, gtk4::StringList) {
    let page = settings_page_container();

    let emu_dir = ira_platforms::api_emulators::api_emulators_dir(&cfg.save_dir);
    let _ = std::fs::create_dir_all(&emu_dir);

    let group = adw::PreferencesGroup::new();
    group.set_title("API Emulator Files");
    group.set_description(Some("Drop emulator files into the structure below"));

    let dir_row = adw::ActionRow::new();
    dir_row.set_title("Directory");
    dir_row.set_subtitle(&emu_dir.to_string_lossy());
    dir_row.set_sensitive(false);

    let open_btn = gtk4::Button::with_label("Open");
    open_btn.add_css_class("flat");
    open_btn.set_valign(gtk4::Align::Center);
    {
        let path = emu_dir.clone();
        open_btn.connect_clicked(move |_| {
            let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
        });
    }
    dir_row.add_suffix(&open_btn);
    group.add(&dir_row);
    page.append(&group);

    let version_group = adw::PreferencesGroup::new();
    version_group.set_title("Default Version");
    version_group.set_description(Some("Version to use when installing API emulators on games"));

    let gse_versions = ira_platforms::api_emulators::list_gse_versions(&cfg.save_dir);
    let gog_versions = ira_platforms::api_emulators::list_gog_versions(&cfg.save_dir);
    let mut all_versions: Vec<String> = Vec::new();
    for v in &gse_versions {
        if !all_versions.contains(v) {
            all_versions.push(v.clone());
        }
    }
    for v in &gog_versions {
        if !all_versions.contains(v) {
            all_versions.push(v.clone());
        }
    }

    let version_model = if all_versions.is_empty() {
        let strings = vec!["(no versions installed)"];
        gtk4::StringList::new(&strings)
    } else {
        string_list_from(&all_versions)
    };
    let version_row = adw::ComboRow::new();
    version_row.set_title("Emulator version");
    version_row.set_subtitle("Default version directory to use when installing");
    version_row.set_model(Some(&version_model));
    if !cfg.default_api_emu_version.is_empty() {
        if let Some(idx) = all_versions.iter().position(|v| v == &cfg.default_api_emu_version) {
            version_row.set_selected(idx as u32);
        }
    } else if !all_versions.is_empty() {
        version_row.set_selected(0);
    }
    version_group.add(&version_row);
    page.append(&version_group);

    (page, version_row, version_model)
}

pub fn show_settings_dialog(
    parent: &adw::ApplicationWindow,
    cfg: Config,
    steam: Arc<SteamClient>,
    state: &SharedState,
) {
    let layout = dialog_layout(parent);
    layout.window.set_default_size(640, 480);
    layout.window.set_deletable(false);
    layout.sidebar_area.set_size_request(180, -1);

    let win = layout.window;
    let sidebar = layout.sidebar;
    let stack = layout.stack;
    let content_area = layout.content_area;

    let (general_page, notif_row, bg_row, hidden_row, grid_spin) = build_general_settings_page(&cfg);
    sidebar.append(&settings_sidebar_row("preferences-system-symbolic", "General"));
    stack.add_named(&general_page, Some("general"));

    let (api_page, steam_entry, sgdb_entry) = build_api_keys_page(&cfg);
    sidebar.append(&settings_sidebar_row("dialog-password-symbolic", "API Keys"));
    stack.add_named(&api_page, Some("api"));

    sidebar.append(&sidebar_separator());

    let (lutris_page, lutris_enable_row) = build_lutris_settings_page(&cfg);
    sidebar.append(&settings_sidebar_row("application-x-executable-symbolic", "Lutris"));
    stack.add_named(&lutris_page, Some("lutris"));

    let (steam_page, steam_enable_row) = build_steam_settings_page(&cfg);
    sidebar.append(&settings_sidebar_row("application-x-executable-symbolic", "Steam"));
    stack.add_named(&steam_page, Some("steam"));

    let (ra_page, ra_enable_row, ra_username_row, ra_password_row, ra_token_row) = build_ra_settings_page(&cfg);
    sidebar.append(&settings_sidebar_row("applications-science-symbolic", "RetroAchievements"));
    stack.add_named(&ra_page, Some("ra"));

    let mut console_widgets: Vec<(&'static str, ConsolePageWidgets)> = Vec::new();
    let mut ps4_enable_row: Option<adw::SwitchRow> = None;
    let mut ps4_exe_row: Option<adw::EntryRow> = None;
    for def in ira_models::CONSOLES {
        let cc = cfg.console(def.id);
        let (page, widgets) = build_console_settings_page(&win, def, cc);
        sidebar.append(&settings_sidebar_row("applications-games-symbolic", def.display_name));
        stack.add_named(&page, Some(def.display_name.to_lowercase().as_str()));
        console_widgets.push((def.id, widgets));

        if def.id == "ps2" {
            let (ps4_page, ps4_en, ps4_exe) = build_shadps4_settings_page(&cfg, &win);
            sidebar.append(&settings_sidebar_row("applications-games-symbolic", "PS4"));
            stack.add_named(&ps4_page, Some("ps4"));
            ps4_enable_row = Some(ps4_en);
            ps4_exe_row = Some(ps4_exe);
        }
    }

    let (wine_pages, wine_widgets) = build_wine_config_pages(&cfg.default_wine_config, None);
    sidebar.append(&sidebar_separator());
    for wp in &wine_pages {
        sidebar.append(&settings_sidebar_row(wp.icon, wp.label));
        stack.add_named(&wp.page, Some(wp.label));
    }

    let profiles_page = build_profiles_page(state, &win);
    sidebar.append(&settings_sidebar_row("system-users-symbolic", "Wine Profiles"));
    stack.add_named(&profiles_page, Some("profiles"));

    sidebar.append(&sidebar_separator());
    let (emu_page, emu_version_row, emu_version_model) = build_api_emulators_page(&cfg);
    sidebar.append(&settings_sidebar_row("applications-engineering-symbolic", "API Emulators"));
    stack.add_named(&emu_page, Some("api_emulators"));

    let stack_clone = stack.clone();
    sidebar.connect_row_selected(move |_, row| {
        if let Some(row) = row {
            if let Some(child) = row.child() {
                if let Some(hbox) = child.downcast_ref::<gtk4::Box>() {
                    if let Some(sibling) = hbox.last_child() {
                        if let Some(label) = sibling.downcast_ref::<gtk4::Label>() {
                            let page_id = match label.text().as_str() {
                                "General" => "general",
                                "API Keys" => "api",
                                "Lutris" => "lutris",
                                "Steam" => "steam",
                                "RetroAchievements" => "ra",
                                "PS1" => "ps1",
                                "PS2" => "ps2",
                                "PS4" => "ps4",
                                "PSP" => "psp",
                                "Performance" => "Performance",
                                "Graphics" => "Graphics",
                                "Wine Advanced" => "Wine Advanced",
                                "Wine Profiles" => "profiles",
                                "API Emulators" => "api_emulators",
                                _ => "general",
                            };
                            stack_clone.set_visible_child_name(page_id);
                        }
                    }
                }
            }
        }
    });

    if let Some(first) = sidebar.row_at_index(0) {
        sidebar.select_row(Some(&first));
    }

    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);
    btn_row.set_margin_start(16);
    btn_row.set_margin_end(16);
    btn_row.set_margin_top(8);
    btn_row.set_margin_bottom(12);

    let cancel_btn = gtk4::Button::with_label(S::CANCEL);
    let win_c = win.clone();
    cancel_btn.connect_clicked(move |_| win_c.close());

    let save_btn = gtk4::Button::with_label(S::SAVE);
    save_btn.add_css_class("suggested-action");

    let state_clone = state.clone();
    let win_clone = win.clone();
    let steam_clone = steam.clone();
    save_btn.connect_clicked(move |_| {
        let mut s = state_clone.borrow_mut();
        s.cfg.steam_api_key = steam_entry.text().to_string();
        s.cfg.steam_griddb_api_key = sgdb_entry.text().to_string();
        s.cfg.notifications_enabled = notif_row.is_active();
        s.cfg.close_to_background = bg_row.is_active();
        s.cfg.show_hidden_games = hidden_row.is_active();
        s.cfg.grid_cover_width = grid_spin.value() as i32;
        if let Some(row) = &ps4_enable_row {
            s.cfg.shadps4_enabled = row.is_active();
        }
        if let Some(row) = &ps4_exe_row {
            s.cfg.shadps4_executable = row.text().to_string();
        }
        s.cfg.steam_enabled = steam_enable_row.is_active();
        s.cfg.lutris_enabled = lutris_enable_row.is_active();
        s.cfg.ra_enabled = ra_enable_row.is_active();
        s.cfg.ra_username = ra_username_row.text().to_string();
        s.cfg.ra_password = ra_password_row.text().to_string();
        s.cfg.ra_token = ra_token_row.text().to_string();
        for (console_id, widgets) in &console_widgets {
            let cc = s.cfg.console_mut(console_id);
            cc.enabled = widgets.enable_row.is_active();
            cc.folder = widgets.folder_row.text().to_string();
            cc.executable = widgets.exe_row.text().to_string();
            cc.fullscreen = widgets.fullscreen_row.is_active();
            if let Some(ref dd) = widgets.core_dropdown {
                if dd.selected() > 0 {
                    let cores = ira_platforms::emulator_detect::detect_ra_cores();
                    if let Some(c) = cores.get((dd.selected() - 1) as usize) {
                        cc.ra_core = c.path.clone();
                    }
                } else {
                    cc.ra_core = String::new();
                }
            }
        }
        s.cfg.default_wine_config = wine_widgets.to_wine_config();

        let idx = emu_version_row.selected();
        let ver = emu_version_model.string(idx).map(|s| s.to_string()).unwrap_or_default();
        if !ver.is_empty() && !ver.starts_with("(no versions") {
            s.cfg.default_api_emu_version = ver.to_string();
        }

        steam_clone.update_keys(&s.cfg.steam_api_key, &s.cfg.steam_griddb_api_key);

        let cfg = s.cfg.clone();
        drop(s);

        if let Err(e) = cfg.save() {
            eprintln!("Failed to save config: {}", e);
        }
        win_clone.close();
    });

    btn_row.append(&cancel_btn);
    btn_row.append(&save_btn);
    content_area.append(&btn_row);
    win.present();
}
