use gtk4::prelude::*;
use adw::prelude::*;
use ira_config::Config;
use ira_api::SteamClient;
use ira_api::types::SgdbAsset;
use crate::Game;
use crate::strings as S;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use super::state::SharedState;
use super::message_handler::apply_game_update;
use super::helpers::clear_children;
use super::mass_match_dialog::show_sgdb_search_dialog;

type GameGeneralPageResult = (gtk4::Box, adw::EntryRow, adw::EntryRow, Rc<RefCell<Option<String>>>, Option<adw::EntryRow>, Option<adw::ComboRow>, Rc<RefCell<Option<String>>>, Rc<RefCell<Option<String>>>);
type SectionEntry = (&'static str, &'static str, &'static str, i32, i32, &'static [&'static str]);

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

fn build_shadps4_version_dropdown(current_path: &str, include_global: bool) -> gtk4::DropDown {
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
    let version_model = super::helpers::string_list_from(&version_strings);
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

    let ps4_exe_browse = super::helpers::make_browse_button(
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

    let folder_browse = super::helpers::make_browse_button(
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
        let emu_model = super::helpers::string_list_from(&emu_names);
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

    let exe_browse = super::helpers::make_browse_button(
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

    // RetroArch core dropdown — inside the emulator group, shown when emulator is RetroArch
    let is_ra = ira_platforms::emulator_detect::is_retroarch(&cc.executable);
    if is_ra {
        let cores = ira_platforms::emulator_detect::detect_ra_cores();
        if !cores.is_empty() {
            let mut core_names: Vec<String> = vec!["None (auto-detect)".to_string()];
            core_names.extend(cores.iter().map(|c| c.display_name.clone()));
            let core_model = super::helpers::string_list_from(&core_names);
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

    // Default version dropdown
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
        super::helpers::string_list_from(&all_versions)
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
    let layout = super::helpers::dialog_layout(parent);
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

    let (wine_pages, wine_widgets) = super::wine_config_widget::build_wine_config_pages(&cfg.default_wine_config, None);
    sidebar.append(&sidebar_separator());
    for wp in &wine_pages {
        sidebar.append(&settings_sidebar_row(wp.icon, wp.label));
        stack.add_named(&wp.page, Some(wp.label));
    }

    let profiles_page = super::profile_dialog::build_profiles_page(state, &win);
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

pub(super) fn build_game_general_page(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    languages: &[String],
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

    if !game.game_path.is_empty() && game.kind != ira_models::STEAM {
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
    if game.kind == ira_models::PS4 {
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
    if game.kind == ira_models::RETRO {
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
    }

    let mut app_id_entry: Option<adw::EntryRow> = None;

    if game.trophy_source == ira_models::GSE || game.trophy_source == ira_models::NGE || game.kind == ira_models::PS4 {
        let ids_group = adw::PreferencesGroup::new();
        ids_group.set_title("Service IDs");
    if game.kind == ira_models::PS4 {
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
        } else if game.trophy_source == ira_models::GSE {
            let row = adw::EntryRow::new();
            row.set_title("Steam App ID");
            row.set_text(&game.app_id);
            let search_btn = gtk4::Button::from_icon_name("system-search-symbolic");
            search_btn.set_valign(gtk4::Align::Center);
            search_btn.set_tooltip_text(Some("Search Steam Store"));
            search_btn.add_css_class("flat");
            let sc = state.clone();
            let game_name = game.name.clone();
            let lutris_id = game.lutris_id;
            let win_c = win.clone();
            let row_c = row.clone();
            let matched_name = game.name.clone();
            search_btn.connect_clicked(move |_| {
                let on_select = {
                    let sc = sc.clone();
                    let name = matched_name.clone();
                    Rc::new(move |sid: &str| {
                        super::matching::match_game_to_steam(&sc, lutris_id, sid.to_string(), name.clone());
                    })
                };
                show_steam_id_search_popup(&sc, &game_name, &win_c, &row_c, "Match", on_select);
            });
            row.add_suffix(&search_btn);
            ids_group.add(&row);
            app_id_entry = Some(row);
        } else if game.trophy_source == ira_models::NGE {
            let row = adw::EntryRow::new();
            row.set_title("GOG Product ID");
            row.set_text(&game.app_id);
            ids_group.add(&row);
            app_id_entry = Some(row);
        }
        general_page.append(&ids_group);
    }

    let language_row = if !languages.is_empty() && (game.trophy_source == ira_models::GSE || game.trophy_source == ira_models::NGE) {
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
            &game.trophy_source, &game_exe, &save_dir, &game.app_id,
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

pub(super) fn show_steam_id_search_popup(
    state: &SharedState,
    game_name: &str,
    parent: &adw::Window,
    app_id_row: &adw::EntryRow,
    button_label: &str,
    on_select: Rc<dyn Fn(&str)>,
) {
    let dialog = adw::Window::new();
    dialog.set_default_width(500);
    dialog.set_default_height(400);
    dialog.set_modal(true);
    dialog.set_transient_for(Some(parent));
    dialog.set_title(Some("Search Steam Store"));

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    outer.append(&header);

    let search_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    search_box.set_margin_start(12);
    search_box.set_margin_end(12);
    search_box.set_margin_top(8);
    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some("Game name…"));
    entry.set_text(game_name);
    entry.set_hexpand(true);
    let search_btn = gtk4::Button::with_label("Search");
    search_btn.add_css_class("suggested-action");
    search_box.append(&entry);
    search_box.append(&search_btn);
    outer.append(&search_box);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_margin_top(8);
    let list = gtk4::ListBox::new();
    list.set_margin_start(12);
    list.set_margin_end(12);
    list.set_margin_top(8);
    list.set_margin_bottom(12);
    list.set_valign(gtk4::Align::Start);
    list.add_css_class("boxed-list");
    scrolled.set_child(Some(&list));
    outer.append(&scrolled);

    let close_btn = gtk4::Button::with_label("Close");
    close_btn.set_halign(gtk4::Align::End);
    close_btn.set_margin_start(12);
    close_btn.set_margin_end(12);
    close_btn.set_margin_bottom(12);
    let win_c = dialog.clone();
    close_btn.connect_clicked(move |_| win_c.close());
    outer.append(&close_btn);

    dialog.set_content(Some(&outer));

    let state_c = state.clone();
    let list_c = list.clone();
    let dialog_c = dialog.clone();
    let row_c = app_id_row.clone();
    let on_select_c = on_select.clone();

    let entry_s = entry.clone();
    let button_label_s = button_label.to_string();
    let do_search = move || {
        let term = entry_s.text().to_string();
        if term.is_empty() { return; }
        let steam = state_c.borrow().steam.clone();
        let results_shared = Arc::new(std::sync::Mutex::new(None::<Vec<(String, String)>>));
        let results_thread = results_shared.clone();
        std::thread::spawn(move || {
            let r = steam.search_steam_store(&term);
            *results_thread.lock().unwrap() = Some(r);
        });
        let results_poll = results_shared.clone();
        let list_c2 = list_c.clone();
        let dialog_c2 = dialog_c.clone();
        let row_c2 = row_c.clone();
        let on_select_c2 = on_select_c.clone();
        let btn_label = button_label_s.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            if !dialog_c2.is_visible() { return glib::ControlFlow::Break; }
            if let Some(results) = results_poll.lock().unwrap().take() {
                clear_children(&list_c2);
                if results.is_empty() {
                    let row = adw::ActionRow::new();
                    row.set_title("No results found");
                    row.set_sensitive(false);
                    list_c2.append(&row);
                } else {
                    for (app_id, name) in &results {
                        let row = adw::ActionRow::new();
                        row.set_title(name);
                        row.set_subtitle(&format!("App ID: {}", app_id));
                        let match_btn = gtk4::Button::with_label(&btn_label);
                        match_btn.add_css_class("suggested-action");
                        match_btn.set_valign(gtk4::Align::Center);
                        let sid = app_id.clone();
                        let on_select_c3 = on_select_c2.clone();
                        let row_update = row_c2.clone();
                        let dlg = dialog_c2.clone();
                        match_btn.connect_clicked(move |_| {
                            on_select_c3(&sid);
                            row_update.set_text(&sid);
                            dlg.close();
                        });
                        row.add_suffix(&match_btn);
                        list_c2.append(&row);
                    }
                }
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    };

    let ds = do_search.clone();
    entry.connect_activate(move |_| ds());
    let ds2 = do_search.clone();
    search_btn.connect_clicked(move |_| ds2());

    dialog.present();
    do_search();
}

pub(super) fn build_game_logo_page(game: &Game) -> Option<(gtk4::Box, Rc<RefCell<String>>, gtk4::Adjustment)> {
    if game.logo_path.is_empty() {
        return None;
    }

    let logo_page = gtk4::Box::new(gtk4::Orientation::Vertical, 16);

    let selected_pos: Rc<RefCell<String>> = Rc::new(RefCell::new(game.logo_position.clone()));

    let size_pct = game.logo_size.clamp(5, 100);
    let size_adj = gtk4::Adjustment::new(size_pct as f64, 5.0, 100.0, 1.0, 5.0, 0.0);

    let preview_overlay = gtk4::Overlay::new();
    preview_overlay.set_height_request(220);
    preview_overlay.set_overflow(gtk4::Overflow::Hidden);

    let hero_pic = gtk4::Picture::new();
    if let Some(t) = ira_images::texture_for(&game.hero_image_path) {
        hero_pic.set_paintable(Some(&t));
    }
    hero_pic.set_content_fit(gtk4::ContentFit::Cover);
    hero_pic.set_halign(gtk4::Align::Fill);
    hero_pic.set_valign(gtk4::Align::Fill);
    preview_overlay.set_child(Some(&hero_pic));

    let preview_draw = gtk4::DrawingArea::new();
    preview_draw.set_halign(gtk4::Align::Fill);
    preview_draw.set_valign(gtk4::Align::Fill);
    preview_draw.set_hexpand(true);
    preview_draw.set_vexpand(true);

    if let Ok(ref pixbuf) = gtk4::gdk_pixbuf::Pixbuf::from_file(&game.logo_path) {
        let pb_w = pixbuf.width() as f64;
        let pb_h = pixbuf.height() as f64;
        let pixbuf_clone = pixbuf.clone();
        let pos_for_draw = selected_pos.clone();
        let adj_for_draw = size_adj.clone();

        preview_draw.set_draw_func(move |_area, cr, area_w, area_h| {
            let w = area_w as f64;
            let h = area_h as f64;
            if w <= 0.0 || h <= 0.0 { return; }
            let pct = adj_for_draw.value() as i32;
            let (lw, lh) = super::game_display::logo_scaled_dims(w, h, pb_w, pb_h, pct);
            let pos = pos_for_draw.borrow().clone();
            let (halign, valign) = super::game_display::logo_position_align(&pos);
            let x = match halign {
                gtk4::Align::Start => 12.0,
                gtk4::Align::Center => (w - lw) / 2.0,
                gtk4::Align::End => w - lw - 12.0,
                _ => 12.0,
            };
            let y = match valign {
                gtk4::Align::Start => 12.0,
                gtk4::Align::Center => (h - lh) / 2.0,
                gtk4::Align::End => h - lh - 12.0,
                _ => h - lh - 12.0,
            };
            let _ = cr.save();
            cr.translate(x, y);
            cr.scale(lw / pb_w, lh / pb_h);
            cr.set_source_pixbuf(&pixbuf_clone, 0.0, 0.0);
            let _ = cr.paint();
            let _ = cr.restore();
        });
    }

    preview_overlay.add_overlay(&preview_draw);

    let logo_positions = ["top-left", "top-center", "top-right", "center-left", "center", "center-right", "bottom-left", "bottom-center", "bottom-right"];

    let pos_grid = gtk4::Grid::new();
    pos_grid.set_column_spacing(2);
    pos_grid.set_row_spacing(2);
    pos_grid.set_halign(gtk4::Align::Fill);
    pos_grid.set_valign(gtk4::Align::Fill);
    pos_grid.set_hexpand(true);
    pos_grid.set_vexpand(true);

    let mut all_btns: Vec<gtk4::Button> = Vec::new();
    for (i, &pos) in logo_positions.iter().enumerate() {
        let btn = gtk4::Button::new();
        btn.add_css_class("logo-pos-overlay-btn");
        if pos == game.logo_position {
            btn.add_css_class("selected");
        }
        btn.set_hexpand(true);
        btn.set_vexpand(true);
        let row = i / 3;
        let col = i % 3;
        pos_grid.attach(&btn, col as i32, row as i32, 1, 1);
        all_btns.push(btn);
    }

    let btns: Rc<Vec<gtk4::Button>> = Rc::new(all_btns);
    for (i, &pos) in logo_positions.iter().enumerate() {
        let btns_c = btns.clone();
        let selected_pos_c = selected_pos.clone();
        let pos_owned = pos.to_string();
        let preview_clone = preview_draw.clone();
        btns[i].connect_clicked(move |btn| {
            for b in btns_c.iter() {
                b.remove_css_class("selected");
            }
            btn.add_css_class("selected");
            *selected_pos_c.borrow_mut() = pos_owned.clone();
            preview_clone.queue_draw();
        });
    }

    preview_overlay.add_overlay(&pos_grid);

    let preview_frame = gtk4::Frame::new(None::<&str>);
    preview_frame.set_child(Some(&preview_overlay));
    logo_page.append(&preview_frame);

    let size_label = gtk4::Label::new(Some("Size (% of hero height)"));
    size_label.set_halign(gtk4::Align::Start);
    size_label.add_css_class("heading");
    logo_page.append(&size_label);

    let size_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    size_row.set_hexpand(true);

    let size_scale = gtk4::Scale::new(gtk4::Orientation::Horizontal, Some(&size_adj));
    size_scale.set_draw_value(false);
    size_scale.set_hexpand(true);

    let size_spin = gtk4::SpinButton::new(Some(&size_adj), 1.0, 1);
    size_spin.set_numeric(true);
    size_spin.set_digits(1);

    let preview_draw_for_size = preview_draw.clone();
    size_adj.connect_value_changed(move |_| {
        preview_draw_for_size.queue_draw();
    });

    size_row.append(&size_scale);
    size_row.append(&size_spin);
    logo_page.append(&size_row);

    Some((logo_page, selected_pos, size_adj))
}



pub fn build_image_manager_content(state: &SharedState, game: &Game, parent_win: &adw::Window) -> gtk4::Box {
    build_image_manager_content_with_drafts(state, game, parent_win, None)
}

pub fn build_image_manager_content_with_drafts(
    state: &SharedState,
    game: &Game,
    parent_win: &adw::Window,
    pending_copies: Option<Rc<RefCell<HashMap<String, String>>>>,
) -> gtk4::Box {
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.set_margin_top(16);
    content.set_margin_bottom(16);

    let is_steam = ira_models::has_steam_enrichment(&game.trophy_source);

    let sections: [SectionEntry; 5] = [
        ("Icon", "icon.png", "icon", 48, 48, &[]),
        ("Hero", "library_hero.jpg", "hero", 96, 48, &[]),
        ("Capsule", "library_600x900.jpg", "grid", 32, 48, &["600x900"]),
        ("Header", "header.jpg", "header", 96, 48, &["460x215", "920x430"]),
        ("Logo", "logo.png", "logo", 96, 48, &[]),
    ];
    for &(label, file, asset, thumb_w, thumb_h, dimensions) in &sections {
        let section = build_image_section(BuildImageSectionParams {
            label, file_base: file, asset_type: asset,
            thumb_w, thumb_h, dims: dimensions,
            game, state, parent_win,
            pending_copies: pending_copies.clone(),
        });
        content.append(&section);
    }

    {
        let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        btn_box.set_halign(gtk4::Align::Center);
        btn_box.set_margin_top(24);

        if game.sgdb_id.is_empty() && !is_steam {
            let match_btn = gtk4::Button::with_label("Match to SteamGridDB…");
            match_btn.add_css_class("suggested-action");
            let sc = state.clone();
            let gn = game.name.clone();
            let did = game.db_id;
            let pw = parent_win.clone();
            match_btn.connect_clicked(move |_| {
                show_sgdb_search_dialog(&sc, did, &gn, &pw, None);
            });
            btn_box.append(&match_btn);
        }

        if !game.sgdb_id.is_empty() {
            let label = gtk4::Label::new(Some(&format!("Matched (SGDB ID: {})", game.sgdb_id)));
            label.add_css_class("success-label");
            btn_box.append(&label);
            let unmatch_btn = gtk4::Button::with_label("Unmatch SGDB");
            unmatch_btn.add_css_class("destructive-action");
            let pending_pc = pending_copies.clone();
            let sc = state.clone();
            let did = game.db_id;
            unmatch_btn.connect_clicked(move |_| {
                if let Some(ref pc) = pending_pc {
                    pc.borrow_mut().insert("__unmatch__".to_string(), String::new());
                    super::helpers::refresh_settings_images_page(&sc, did, |s, game, win| {
                        let mut g2 = game.clone();
                        g2.sgdb_id.clear();
                        build_image_manager_content_with_drafts(s, &g2, win, Some(pc.clone())).upcast()
                    });
                }
            });
            btn_box.append(&unmatch_btn);
        }

        content.append(&btn_box);
    }

    content
}

fn find_best_image_path(game: &Game, field: &str, file: &str, id: &str, save_dir: &str) -> String {
    let field_path = match field {
        "icon" if !game.icon_path.is_empty() => game.icon_path.clone(),
        "hero" if !game.hero_image_path.is_empty() => game.hero_image_path.clone(),
        "grid" if !game.grid_path.is_empty() => game.grid_path.clone(),
        "header" if !game.header_path.is_empty() => game.header_path.clone(),
        "logo" if !game.logo_path.is_empty() => game.logo_path.clone(),
        _ => String::new(),
    };
    if !field_path.is_empty() && std::path::Path::new(&field_path).is_file() {
        return field_path;
    }
    if !game.sgdb_id.is_empty() {
        let sgdb = format!("{}/{}", ira_parser::sgdb_data_dir(save_dir, &game.sgdb_id).to_string_lossy(), file);
        if std::path::Path::new(&sgdb).is_file() {
            return sgdb;
        }
    }
    let native = if game.kind == ira_models::PS4 {
        format!("{}/{}", ira_parser::ps4_data_dir(save_dir, id).to_string_lossy(), file)
    } else {
        format!("{}/{}", ira_parser::data_dir(save_dir, id).to_string_lossy(), file)
    };
    if std::path::Path::new(&native).is_file() {
        return native;
    }
    if field == "icon" && game.kind == ira_models::PS4 && !game.icon_path.is_empty() && std::path::Path::new(&game.icon_path).is_file() {
        return game.icon_path.clone();
    }
    String::new()
}

fn make_refresh_closure(
    preview_wrapper: &gtk4::Box,
    dest_path: &str,
    state: &SharedState,
    game: &Game,
    pending_copies: Option<Rc<RefCell<HashMap<String, String>>>>,
    asset_type: &str,
) -> Rc<dyn Fn()> {
    let save_dir = state.borrow().save_dir.clone();
    Rc::new({
        let preview_wrapper = preview_wrapper.clone();
        let dest_path = dest_path.to_string();
        let state_clone = state.clone();
        let game_clone = game.clone();
        let pending_copies = pending_copies.clone();
        let asset_c = asset_type.to_string();
        move || {
            clear_children(&preview_wrapper);
            let preview_src = pending_copies.as_ref()
                .and_then(|pc| pc.borrow().get(&asset_c).cloned())
                .filter(|p| std::path::Path::new(p).is_file())
                .or_else(|| {
                    if std::path::Path::new(&dest_path).exists() {
                        Some(dest_path.clone())
                    } else {
                        None
                    }
                });
            if let Some(path) = preview_src {
                let p = gtk4::Picture::for_filename(&path);
                p.set_content_fit(gtk4::ContentFit::ScaleDown);
                preview_wrapper.append(&p);
            } else {
                let ph = gtk4::Label::new(Some("—"));
                ph.add_css_class("dim-label");
                preview_wrapper.append(&ph);
            }
            let s = state_clone.borrow();
            if let Ok(Some(entry)) = ira_db::find_by_lutris_id(&s.db, game_clone.lutris_id) {
                drop(s);
                if let Ok(updated) = crate::game_loader::load_game(&entry, &save_dir) {
                    apply_game_update(&state_clone, updated);
                }
            }
        }
    })
}

struct BuildImageSectionParams<'a> {
    label: &'a str,
    file_base: &'a str,
    asset_type: &'a str,
    thumb_w: i32,
    thumb_h: i32,
    dims: &'a [&'static str],
    game: &'a Game,
    state: &'a SharedState,
    parent_win: &'a adw::Window,
    pending_copies: Option<Rc<RefCell<HashMap<String, String>>>>,
}

fn build_image_section(params: BuildImageSectionParams) -> gtk4::Box {
    let BuildImageSectionParams { label, file_base, asset_type, thumb_w, thumb_h, dims, game, state, parent_win, pending_copies } = params;
    let is_steam = ira_models::has_steam_enrichment(&game.trophy_source);
    let id = game.app_id.clone();
    let save_dir = state.borrow().save_dir.clone();

    let cloud_dir = if !game.sgdb_id.is_empty() {
        ira_parser::sgdb_data_dir(&save_dir, &game.sgdb_id)
    } else if game.kind == ira_models::PS4 {
        ira_parser::ps4_data_dir(&save_dir, &id)
    } else {
        ira_parser::data_dir(&save_dir, &id)
    };
    let cloud_base = cloud_dir.to_string_lossy().into_owned();

    let section = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    let lbl = gtk4::Label::new(Some(label));
    lbl.set_halign(gtk4::Align::Start);
    lbl.add_css_class("heading");
    section.append(&lbl);

    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    row.set_hexpand(true);
    row.set_valign(gtk4::Align::Center);

    let img_path = {
        let draft_path = pending_copies.as_ref()
            .and_then(|pc| pc.borrow().get(asset_type).cloned());
        if let Some(ref src) = draft_path {
            if std::path::Path::new(src).is_file() {
                src.clone()
            } else {
                find_best_image_path(game, asset_type, file_base, &id, &save_dir)
            }
        } else {
            find_best_image_path(game, asset_type, file_base, &id, &save_dir)
        }
    };

    let preview = gtk4::Picture::for_filename(&img_path);
    preview.set_content_fit(gtk4::ContentFit::ScaleDown);
    preview.set_size_request(thumb_w, thumb_h);
    let preview_wrapper = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    preview_wrapper.set_size_request(thumb_w, 48);
    preview_wrapper.set_valign(gtk4::Align::Center);
    if !img_path.is_empty() && std::path::Path::new(&img_path).is_file() {
        preview_wrapper.append(&preview);
    } else {
        let ph = gtk4::Label::new(Some("—"));
        ph.add_css_class("dim-label");
        preview_wrapper.append(&ph);
    }
    row.append(&preview_wrapper);

    let btns = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    btns.set_hexpand(true);
    btns.set_halign(gtk4::Align::End);

    let dest_path = format!("{}/{}", cloud_base, file_base);
    let refresh_images = make_refresh_closure(
        &preview_wrapper, &dest_path, state, game, pending_copies.clone(), asset_type,
    );

    let browse_btn = super::helpers::make_browse_button(
        Some(parent_win),
        "Select image",
        false,
        Some(("Images", &["image/png", "image/jpeg", "image/webp", "image/x-icon"])),
        {
            let pc = pending_copies.clone();
            let refresh = refresh_images.clone();
            let asset_name = asset_type.to_string();
            move |path| {
                if let Some(ref pc_inner) = pc {
                    pc_inner.borrow_mut().insert(asset_name.clone(), path.to_string_lossy().into_owned());
                    refresh();
                }
            }
        },
    );
    btns.append(&browse_btn);

    if is_steam && asset_type != "icon" {
        let btn = gtk4::Button::with_label("Steam");
        let steam = state.borrow().steam.clone();
        let id_c = id.clone();
        let asset_c = asset_type.to_string();
        let refresh = refresh_images.clone();
        btn.connect_clicked(move |_| {
            let _ = steam.force_download_steam(&id_c, &asset_c);
            refresh();
        });
        btns.append(&btn);
    }

    if is_steam && asset_type == "icon" && game.trophy_source == ira_models::STEAM_NATIVE {
        let btn = gtk4::Button::with_label("Steam");
        let steam = state.borrow().steam.clone();
        let id_c = id.clone();
        let save_dir_c = save_dir.clone();
        let refresh = refresh_images.clone();
        btn.connect_clicked(move |_| {
            if let Ok(app_id_num) = id_c.parse::<u32>() {
                if let Some(clienticon) = ira_platforms::steam::get_clienticon(app_id_num) {
                    let dest = ira_parser::data_dir(&save_dir_c, &id_c).join("icon.png");
                    let _ = std::fs::create_dir_all(dest.parent().unwrap());
                    let ico_path = ira_platforms::steam::steam_install_dir()
                        .map(|d| d.join("steam").join("games").join(format!("{}.ico", clienticon)));
                    let got = if let Some(ref p) = ico_path {
                        if p.is_file() {
                            if let Ok(ico_data) = std::fs::read(p) {
                                let tmp = dest.with_extension("ico");
                                if std::fs::write(&tmp, &ico_data).is_ok() {
                                    let r = ira_parser::convert_ico_to_png(&tmp).ok()
                                        .map(|png| { let _ = std::fs::rename(&png, &dest); std::fs::remove_file(&tmp).ok();  });
                                    let _ = std::fs::remove_file(&tmp);
                                    r.is_some()
                                } else { false }
                            } else { false }
                        } else { false }
                    } else { false };
                    if !got {
                        let url = format!("https://cdn.cloudflare.steamstatic.com/steamcommunity/public/images/apps/{}/{}.ico", id_c, clienticon);
                        let tmp = dest.with_extension("ico");
                        if steam.download_file(&url, &tmp).is_ok() {
                            if let Ok(png) = ira_parser::convert_ico_to_png(&tmp) {
                                let _ = std::fs::rename(&png, &dest);
                            }
                            let _ = std::fs::remove_file(&tmp);
                        }
                    }
                }
            }
            refresh();
        });
        btns.append(&btn);
    }

    let sgdb_id_for_picker = if !game.sgdb_id.is_empty() {
        game.sgdb_id.clone()
    } else {
        id.clone()
    };
    let sgdb_is_steam_id = is_steam && game.sgdb_id.is_empty();
    let pending_copies_btn = pending_copies.clone();
    if !sgdb_id_for_picker.is_empty() {
        let btn = gtk4::Button::with_label("SGDB…");
        let steam = state.borrow().steam.clone();
        let asset_c = asset_type.to_string();
        let parent = parent_win.clone();
        let refresh = refresh_images.clone();
        let dims_vec: Vec<&str> = dims.to_vec();
    let sgdb_id_c = sgdb_id_for_picker.clone();
    let save_dir_c = save_dir.clone();
    btn.connect_clicked(move |_| {
        show_sgdb_picker(ShowSgdbPickerParams {
            steam: &steam, id: &sgdb_id_c, asset: &asset_c,
            is_steam_id: sgdb_is_steam_id, dimensions: &dims_vec,
            parent: &parent, on_done: refresh.clone(),
            pending_copies: pending_copies_btn.clone(), save_dir: &save_dir_c,
        });
    });
    btns.append(&btn);
    }

    if asset_type == "icon" && game.kind == ira_models::PS4 {
        let reset_btn = gtk4::Button::with_label("Reset");
        let gc = game.clone();
        let refresh = refresh_images.clone();
        let pending_copies_reset = pending_copies.clone();
        let asset_reset = asset_type.to_string();
        let save_dir_c2 = save_dir.clone();
        reset_btn.connect_clicked(move |_| {
            let app_id = gc.app_id.clone();
            let game_path = gc.game_path.clone();
            let image_dir = std::path::Path::new(&save_dir_c2).join("data").join("ps4").join(&app_id);
            let icon_png = image_dir.join("icon.png");
            let default_path = if icon_png.is_file() {
                Some(icon_png.to_string_lossy().into_owned())
            } else {
                let game_icon = std::path::Path::new(&game_path).join("sce_sys").join("icon0.png");
                if game_icon.is_file() {
                    let _ = std::fs::create_dir_all(&image_dir);
                    let _ = std::fs::copy(&game_icon, &icon_png);
                    Some(icon_png.to_string_lossy().into_owned())
                } else {
                    None
                }
            };
            if let Some(ref pc) = pending_copies_reset {
                pc.borrow_mut().remove(&asset_reset);
                if let Some(path) = default_path {
                    pc.borrow_mut().insert(asset_reset.clone(), path);
                }
            }
            refresh();
        });
        btns.append(&reset_btn);
    }

    row.append(&btns);
    section.append(&row);
    section
}

fn build_sgdb_asset_card(
    a: &SgdbAsset,
    asset_type: &str,
    steam: &Arc<SteamClient>,
    on_download: Rc<dyn Fn()>,
    save_dir: &str,
) -> (gtk4::Widget, gtk4::Widget) {
    let thumb_size = if asset_type == "header" { 138 } else { 90 };

    let mut info = String::new();
    if a.width > 0 && a.height > 0 {
        info = format!("{}\u{d7}{}", a.width, a.height);
    }
    if !a.style.is_empty() {
        if !info.is_empty() { info = format!("{} \u{b7} {}", info, a.style); }
        else { info = a.style.clone(); }
    }
    if !a.author.is_empty() {
        if !info.is_empty() { info = format!("{} \u{b7} by {}", info, a.author); }
        else { info = format!("by {}", a.author); }
    }

    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    card.set_halign(gtk4::Align::Center);
    card.set_valign(gtk4::Align::Start);
    card.set_margin_top(4);
    card.set_margin_bottom(4);

    let grid_pic = gtk4::Picture::new();
    grid_pic.set_content_fit(gtk4::ContentFit::ScaleDown);
    grid_pic.set_size_request(thumb_size, thumb_size);
    card.append(&grid_pic);

    let ilbl = gtk4::Label::new(Some(&info));
    ilbl.set_xalign(0.5);
    ilbl.set_max_width_chars(20);
    ilbl.set_wrap(true);
    ilbl.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    ilbl.add_css_class("dim-label");
    card.append(&ilbl);

    let gdl = gtk4::Button::with_label("Download");
    gdl.add_css_class("suggested-action");
    gdl.set_halign(gtk4::Align::Center);
    card.append(&gdl);

    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    row.set_margin_top(4);
    row.set_margin_bottom(4);

    let list_pic = gtk4::Picture::new();
    list_pic.set_content_fit(gtk4::ContentFit::ScaleDown);
    list_pic.set_size_request(48, 48);
    row.append(&list_pic);

    let rlbl = gtk4::Label::new(Some(&info));
    rlbl.set_xalign(0.0);
    rlbl.set_hexpand(true);
    rlbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    row.append(&rlbl);

    let ldl = gtk4::Button::with_label("Download");
    ldl.add_css_class("suggested-action");
    row.append(&ldl);

    let cb_g = on_download.clone();
    gdl.connect_clicked(move |_| cb_g());
    ldl.connect_clicked(move |_| on_download());

    let url_clone = a.url.clone();
    let steam_thumb = steam.clone();
    let thumb_dir = format!("{}/data/.thumbnails", save_dir);
    let _ = std::fs::create_dir_all(&thumb_dir);
    let thumb_name = format!("{}/{}", thumb_dir, url_clone.rsplit('/').next().unwrap_or("thumb"));
    let tsize = thumb_size;
    let (tx_thumb, rx_thumb) = std::sync::mpsc::channel::<Option<String>>();
    let rx_thumb = std::cell::RefCell::new(rx_thumb);
    std::thread::spawn(move || {
        let final_path = if std::path::Path::new(&thumb_name).exists() {
            Some(thumb_name.clone())
        } else if steam_thumb.download_file(&url_clone, std::path::Path::new(&thumb_name)).is_ok() {
            let mut path = thumb_name.clone();
            if std::path::Path::new(&thumb_name).extension().and_then(|e| e.to_str()) == Some("ico") {
                if let Ok(img) = image::open(&thumb_name) {
                    let png_path = std::path::Path::new(&thumb_name).with_extension("png");
                    if img.save(&png_path).is_ok() {
                        let _ = std::fs::remove_file(&thumb_name);
                        path = png_path.to_string_lossy().into_owned();
                    }
                }
            }
            if let Ok(img) = image::open(&path) {
                let (w, h) = (img.width(), img.height());
                if w > tsize as u32 || h > tsize as u32 {
                    let resized = img.resize(tsize as u32, tsize as u32, image::imageops::FilterType::Lanczos3);
                    let _ = resized.save(&path);
                }
            }
            Some(path)
        } else {
            None
        };
        let _ = tx_thumb.send(final_path);
    });
    let tp_g = grid_pic.clone();
    let tp_l = list_pic.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        if let Ok(path) = rx_thumb.borrow_mut().try_recv() {
            if let Some(p) = path {
                tp_g.set_filename(Some(&p));
                tp_l.set_filename(Some(&p));
            }
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });

    (card.upcast::<gtk4::Widget>(), row.upcast::<gtk4::Widget>())
}

struct ShowSgdbPickerParams<'a> {
    steam: &'a Arc<SteamClient>,
    id: &'a str,
    asset: &'a str,
    is_steam_id: bool,
    dimensions: &'a [&'a str],
    parent: &'a adw::Window,
    on_done: Rc<dyn Fn()>,
    pending_copies: Option<Rc<RefCell<HashMap<String, String>>>>,
    save_dir: &'a str,
}

fn show_sgdb_picker(params: ShowSgdbPickerParams) {
    let ShowSgdbPickerParams { steam, id, asset, is_steam_id, dimensions, parent, on_done, pending_copies, save_dir } = params;
    let picker = adw::Window::new();
    picker.set_default_width(600);
    picker.set_default_height(500);
    picker.set_transient_for(Some(parent));
    picker.set_modal(true);
    let save_dir_owned = save_dir.to_string();

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let header_bar = adw::HeaderBar::new();
    header_bar.set_title_widget(Some(&gtk4::Label::new(Some(&format!("Pick {}", asset)))));

    let toggle_btn = gtk4::ToggleButton::new();
    toggle_btn.set_icon_name("view-list-symbolic");
    toggle_btn.set_tooltip_text(Some("Switch to list view"));
    toggle_btn.add_css_class("flat");
    header_bar.pack_end(&toggle_btn);

    outer.append(&header_bar);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);

    let stack = gtk4::Stack::new();

    let flow = gtk4::FlowBox::new();
    flow.set_selection_mode(gtk4::SelectionMode::None);
    flow.set_homogeneous(true);
    flow.set_min_children_per_line(1);
    flow.set_max_children_per_line(8);
    flow.set_row_spacing(8);
    flow.set_column_spacing(8);
    flow.set_margin_start(12);
    flow.set_margin_end(12);
    flow.set_margin_top(8);
    flow.set_margin_bottom(8);
    flow.set_halign(gtk4::Align::Fill);

    let list_view = gtk4::ListBox::new();
    list_view.set_selection_mode(gtk4::SelectionMode::None);
    list_view.set_margin_start(12);
    list_view.set_margin_end(12);
    list_view.set_margin_top(8);
    list_view.set_margin_bottom(8);

    stack.add_named(&flow, Some("grid"));
    stack.add_named(&list_view, Some("list"));
    stack.set_visible_child_name("grid");

    let loading = gtk4::Label::new(Some("Loading\u{2026}"));
    loading.add_css_class("dim-label");
    flow.append(&loading);
    list_view.append(&gtk4::Label::new(Some("Loading\u{2026}")));

    scrolled.set_child(Some(&stack));
    outer.append(&scrolled);

    let close_btn = gtk4::Button::with_label("Close");
    close_btn.set_halign(gtk4::Align::End);
    close_btn.set_margin_start(12);
    close_btn.set_margin_end(12);
    close_btn.set_margin_bottom(12);
    let win = picker.clone();
    close_btn.connect_clicked(move |_| win.close());
    outer.append(&close_btn);

    picker.set_content(Some(&outer));
    picker.present();

    let (tx, rx) = std::sync::mpsc::channel::<Vec<SgdbAsset>>();
    let rx = std::cell::RefCell::new(rx);
    let steam_c = steam.clone();
    let id_c = id.to_string();
    let asset_c = asset.to_string();
    let dims: Vec<String> = dimensions.iter().map(|s| s.to_string()).collect();
    std::thread::spawn(move || {
        let dims_refs: Vec<&str> = dims.iter().map(|s| s.as_str()).collect();
        let results = steam_c.list_sgdb_assets(&id_c, &asset_c, is_steam_id, &dims_refs);
        let _ = tx.send(results);
    });

    let steam_clone = steam.clone();
    let id_clone = id.to_string();
    let asset_clone = asset.to_string();
    let picker_clone = picker.clone();
    let on_done = on_done.clone();
    let save_dir_clone = save_dir_owned.clone();

    let stack_toggle = stack.clone();
    toggle_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            stack_toggle.set_visible_child_name("list");
            btn.set_icon_name("view-grid-symbolic");
            btn.set_tooltip_text(Some("Switch to grid view"));
        } else {
            stack_toggle.set_visible_child_name("grid");
            btn.set_icon_name("view-list-symbolic");
            btn.set_tooltip_text(Some("Switch to list view"));
        }
    });

    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        if let Ok(assets) = rx.borrow_mut().try_recv() {
            clear_children(&flow);
            clear_children(&list_view);

            if assets.is_empty() {
                let none = gtk4::Label::new(Some("No images found on SteamGridDB"));
                none.add_css_class("dim-label");
                flow.append(&none);
                list_view.append(&gtk4::Label::new(Some("No images found on SteamGridDB")));
                return glib::ControlFlow::Break;
            }

            for a in assets {
                let data_subdir = if is_steam_id { "steam".to_string() } else { "steamgriddb".to_string() };
                let dest_dir = format!("{}/data/{}/{}", save_dir_clone, data_subdir, id_clone);
                let file_name = match asset_clone.as_str() {
                    "icon" => {
                        let ext = if a.mime.contains("icon") || a.mime.contains("x-icon") { "ico" }
                        else if a.mime.contains("png") { "png" }
                        else if a.mime.contains("jpeg") || a.mime.contains("jpg") { "jpg" }
                        else if a.mime.contains("webp") { "webp" }
                        else { std::path::Path::new(&a.url).extension().and_then(|e| e.to_str()).unwrap_or("png") };
                        format!("icon.{}", ext)
                    }
                    "hero" => "library_hero.jpg".to_string(),
                    "grid" => "library_600x900.jpg".to_string(),
                    "header" => "header.jpg".to_string(),
                    "logo" => "logo.png".to_string(),
                    _ => continue,
                };
                let _dest = format!("{}/{}", dest_dir, file_name);
                let dl_url = a.url.clone();
                let steam_dl = steam_clone.clone();
                let picker_dl = picker_clone.clone();
                let on_done_dl = on_done.clone();
                let asset_dl = asset_clone.clone();
                let pending_dl = pending_copies.clone();
                let on_download: Rc<dyn Fn()> = Rc::new(move || {
                    if let Some(ref pc) = pending_dl {
                        let tmp = std::env::temp_dir().join(format!("sgdb_{}", asset_dl));
                        if steam_dl.download_file(&dl_url, &tmp).is_ok() {
                            pc.borrow_mut().insert(asset_dl.clone(), tmp.to_string_lossy().into_owned());
                            on_done_dl();
                            picker_dl.close();
                        } else {
                            eprintln!("Download failed for {}", dl_url);
                        }
                    }
                });

                let (grid_card, list_row) = build_sgdb_asset_card(&a, &asset_clone, &steam_clone, on_download, &save_dir_clone);
                flow.append(&grid_card);
                list_view.append(&list_row);
            }
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}
