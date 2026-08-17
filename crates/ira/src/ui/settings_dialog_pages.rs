use super::input_profile_settings::{
    build_input_settings_page, ConsoleProfileWidgets, ControllerDefaultWidgets,
};
use super::profile_dialog::build_profiles_page;
use super::settings_pages::{
    build_api_emulators_page, build_computer_games_page, build_general_settings_page,
    build_lutris_settings_page, build_overlay_settings_page, build_ra_settings_page,
    build_rom_settings_page, build_steam_settings_page, build_system_defaults_page,
    AutoReloadWidgets, OverlayPageWidgets, SystemDefaultsWidgets,
};
use super::state::SharedState;
use super::wine_config_widget::{build_wine_config_pages, WineConfigWidgets, WinePage};
use adw::prelude::*;
use ira_config::Config;
use std::cell::RefCell;
use std::rc::Rc;

pub(super) struct SettingsPageWidgets {
    pub(super) general_page: gtk4::Box,
    pub(super) overlay_page: gtk4::Box,
    pub(super) input_page: gtk4::Box,
    pub(super) system_page: gtk4::Box,
    pub(super) computer_games_page: gtk4::Box,
    pub(super) steam_page: gtk4::Box,
    pub(super) emu_page: gtk4::Box,
    pub(super) lutris_page: gtk4::Box,
    pub(super) wine_pages: Vec<WinePage>,
    pub(super) profiles_page: gtk4::ScrolledWindow,
    pub(super) ra_page: gtk4::Box,
    pub(super) rom_page: gtk4::Box,
    pub(super) notif_row: adw::SwitchRow,
    pub(super) bg_row: adw::SwitchRow,
    pub(super) hidden_row: adw::SwitchRow,
    pub(super) steam_entry: adw::PasswordEntryRow,
    pub(super) sgdb_entry: adw::PasswordEntryRow,
    pub(super) lang_list: gtk4::ListBox,
    pub(super) saves_row: adw::SwitchRow,
    pub(super) auto_reload_widgets: AutoReloadWidgets,
    pub(super) controller_default_widgets: Rc<RefCell<Vec<ControllerDefaultWidgets>>>,
    pub(super) default_game_folder_row: adw::EntryRow,
    pub(super) steam_enable_row: adw::SwitchRow,
    pub(super) emu_version_row: adw::ComboRow,
    pub(super) emu_version_model: gtk4::StringList,
    pub(super) wine_widgets: WineConfigWidgets,
    pub(super) prefix_base_row: adw::EntryRow,
    pub(super) ra_enable_row: adw::SwitchRow,
    pub(super) ra_username_row: adw::EntryRow,
    pub(super) ra_web_api_key_row: adw::EntryRow,
    pub(super) roms_folder_row: adw::EntryRow,
    pub(super) overlay_widgets: OverlayPageWidgets,
    pub(super) system_defaults_widgets: SystemDefaultsWidgets,
    pub(super) linux_controller_profile: ConsoleProfileWidgets,
    pub(super) wine_controller_profile: ConsoleProfileWidgets,
}

pub(super) fn build_settings_pages(
    cfg: &Config,
    win: &adw::Window,
    state: &SharedState,
) -> SettingsPageWidgets {
    let (
        general_page,
        notif_row,
        bg_row,
        hidden_row,
        steam_entry,
        sgdb_entry,
        lang_list,
        saves_row,
        auto_reload_widgets,
    ) = build_general_settings_page(cfg);
    let (overlay_page, overlay_widgets) = build_overlay_settings_page(cfg);
    let registry = state.borrow().controller_registry.clone();
    let (input_page, input_widgets) = build_input_settings_page(win, &cfg.save_dir, cfg, registry);
    let controller_default_widgets = input_widgets.controller_defaults.clone();
    let (system_page, system_defaults_widgets) = build_system_defaults_page(cfg);
    let (computer_games_page, default_game_folder_row) = build_computer_games_page(win, cfg);
    let (linux_controller_profile, wine_controller_profile) =
        build_pc_controller_profiles(&computer_games_page, cfg, win, state);
    let (steam_page, steam_enable_row) = build_steam_settings_page(cfg);
    let (emu_page, emu_version_row, emu_version_model) = build_api_emulators_page(cfg);
    let lutris_page = build_lutris_settings_page(state, win);
    let (wine_pages, wine_widgets) =
        build_wine_config_pages(&cfg.default_wine_config, None, &cfg.save_dir);
    let (profiles_page, prefix_base_row) = build_profiles_page(state, win);
    let (ra_page, ra_enable_row, ra_username_row, ra_web_api_key_row) =
        build_ra_settings_page(cfg);
    let (rom_page, roms_folder_row) = build_rom_settings_page(win, cfg);
    SettingsPageWidgets {
        general_page,
        overlay_page,
        input_page,
        system_page,
        computer_games_page,
        steam_page,
        emu_page,
        lutris_page,
        wine_pages,
        profiles_page,
        ra_page,
        rom_page,
        notif_row,
        bg_row,
        hidden_row,
        steam_entry,
        sgdb_entry,
        lang_list,
        saves_row,
        auto_reload_widgets,
        controller_default_widgets,
        default_game_folder_row,
        steam_enable_row,
        emu_version_row,
        emu_version_model,
        wine_widgets,
        prefix_base_row,
        ra_enable_row,
        ra_username_row,
        ra_web_api_key_row,
        roms_folder_row,
        overlay_widgets,
        system_defaults_widgets,
        linux_controller_profile,
        wine_controller_profile,
    }
}

pub(super) fn register_settings_pages(
    pages: &SettingsPageWidgets,
    sidebar: &gtk4::ListBox,
    stack: &gtk4::Stack,
) {
    register_page(
        sidebar,
        stack,
        &pages.general_page,
        "cogged-wheel-big-symbolic",
        &crate::tr!("General"),
        "general",
    );
    register_page(
        sidebar,
        stack,
        &pages.overlay_page,
        "layers-symbolic",
        &crate::tr!("Overlay"),
        "overlay",
    );
    register_page(
        sidebar,
        stack,
        &pages.input_page,
        "games-symbolic",
        &crate::tr!("Controller"),
        "input",
    );
    register_page(
        sidebar,
        stack,
        &pages.system_page,
        "cogged-wheel-big-symbolic",
        &crate::tr!("Game system"),
        "system",
    );
    sidebar.append(&super::settings_pages::sidebar_section_title(&crate::tr!(
        "PC games"
    )));
    register_page(
        sidebar,
        stack,
        &pages.computer_games_page,
        "games-symbolic",
        &crate::tr!("PC games"),
        "computer_games",
    );
    register_page(
        sidebar,
        stack,
        &pages.steam_page,
        "steam-train-symbolic",
        &crate::tr!("Steam"),
        "steam",
    );
    register_page(
        sidebar,
        stack,
        &pages.emu_page,
        "api-symbolic",
        &crate::tr!("API emulators"),
        "api_emulators",
    );
    register_page(
        sidebar,
        stack,
        &pages.lutris_page,
        "system-software-install-symbolic",
        &crate::tr!("Lutris migration"),
        "migration",
    );
    sidebar.append(&super::settings_pages::sidebar_section_title(&crate::tr!(
        "Wine"
    )));
    register_existing_scrolled_page(
        sidebar,
        stack,
        &pages.profiles_page,
        "wine-glass-symbolic",
        &crate::tr!("Profiles"),
        "profiles",
    );
    for page in &pages.wine_pages {
        register_existing_scrolled_page(
            sidebar,
            stack,
            &page.page,
            page.icon,
            &page.label,
            page.page_id,
        );
    }
    sidebar.append(&super::settings_pages::sidebar_section_title(&crate::tr!(
        "Emulation"
    )));
    register_page(
        sidebar,
        stack,
        &pages.ra_page,
        "trophy-symbolic",
        &crate::tr!("RetroAchievements"),
        "ra",
    );
    register_page(
        sidebar,
        stack,
        &pages.rom_page,
        "library-symbolic",
        &crate::tr!("ROM library"),
        "roms",
    );
}

fn register_page(
    sidebar: &gtk4::ListBox,
    stack: &gtk4::Stack,
    page: &gtk4::Box,
    icon: &str,
    label: &str,
    page_id: &str,
) {
    sidebar.append(&super::settings_pages::settings_sidebar_row(
        icon, label, page_id,
    ));
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_hexpand(true);
    scroll.set_vexpand(true);
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scroll.set_child(Some(page));
    stack.add_named(&scroll, Some(page_id));
}

fn register_existing_scrolled_page(
    sidebar: &gtk4::ListBox,
    stack: &gtk4::Stack,
    page: &gtk4::ScrolledWindow,
    icon: &str,
    label: &str,
    page_id: &str,
) {
    sidebar.append(&super::settings_pages::settings_sidebar_row(
        icon, label, page_id,
    ));
    stack.add_named(page, Some(page_id));
}

fn build_pc_controller_profiles(
    page: &gtk4::Box,
    cfg: &Config,
    win: &adw::Window,
    state: &SharedState,
) -> (ConsoleProfileWidgets, ConsoleProfileWidgets) {
    let mut pc_controller_cfg = cfg.clone();
    pc_controller_cfg.consoles.insert(
        "linux".to_string(),
        ira_config::ConsoleConfig {
            controller_mode: cfg.linux_controller_mode,
            controller_profile: cfg.linux_controller_profile.clone(),
            ..Default::default()
        },
    );
    pc_controller_cfg.consoles.insert(
        "wine".to_string(),
        ira_config::ConsoleConfig {
            controller_mode: cfg.wine_controller_mode,
            controller_profile: cfg.wine_controller_profile.clone(),
            ..Default::default()
        },
    );
    let registry = state.borrow().controller_registry.clone();
    let linux = super::input_profile_settings::add_pc_profile_group(
        page,
        &pc_controller_cfg,
        "linux",
        &crate::tr!("Linux controller"),
        win,
        registry.clone(),
    );
    let wine = super::input_profile_settings::add_pc_profile_group(
        page,
        &pc_controller_cfg,
        "wine",
        &crate::tr!("Wine controller"),
        win,
        registry,
    );
    (linux, wine)
}
