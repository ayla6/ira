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
use glib::object::IsA;
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
    pub(super) square_row: adw::SwitchRow,
    pub(super) hidden_row: adw::SwitchRow,
    pub(super) steam_entry: adw::PasswordEntryRow,
    pub(super) sgdb_entry: adw::PasswordEntryRow,
    pub(super) lang_list: gtk4::ListBox,
    pub(super) saves_row: adw::SwitchRow,
    pub(super) auto_reload_widgets: AutoReloadWidgets,
    pub(super) controller_default_widgets: Rc<RefCell<Vec<ControllerDefaultWidgets>>>,
    pub(super) game_folders: super::folder_list::FolderListWidgets,
    pub(super) steam_enable_row: adw::SwitchRow,
    pub(super) emu_version_row: adw::ComboRow,
    pub(super) emu_version_model: gtk4::StringList,
    pub(super) wine_widgets: WineConfigWidgets,
    pub(super) prefix_base_row: adw::EntryRow,
    pub(super) default_version_row: adw::ComboRow,
    pub(super) default_version_values: Vec<(String, String)>,
    pub(super) ra_enable_row: adw::SwitchRow,
    pub(super) ra_username_row: adw::EntryRow,
    pub(super) ra_web_api_key_row: adw::EntryRow,
    pub(super) rom_roots: super::folder_list::FolderListWidgets,
    pub(super) overlay_widgets: OverlayPageWidgets,
    pub(super) system_defaults_widgets: SystemDefaultsWidgets,
    pub(super) linux_controller_profile: ConsoleProfileWidgets,
    pub(super) wine_controller_profile: ConsoleProfileWidgets,
}

pub(super) fn build_settings_pages(
    cfg: &Config,
    win: &impl IsA<gtk4::Widget>,
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
        square_row,
        auto_reload_widgets,
    ) = build_general_settings_page(cfg);

    // One-click maintenance: re-run the SGDB asset ensure for every matched
    // game so missing art (squares included) is fetched again, even for
    // games whose enrichment had been skipped as already complete.
    let fetch_group = adw::PreferencesGroup::new();
    fetch_group.set_title(&crate::tr!("Game images"));
    let fetch_row = adw::ActionRow::new();
    fetch_row.set_title(&crate::tr!("Fetch missing images"));
    fetch_row.set_subtitle(&crate::tr!(
        "Re-downloads any missing SGDB art for matched games, even ones skipped before"
    ));
    let fetch_btn = gtk4::Button::with_label(&crate::tr!("Fetch"));
    fetch_btn.set_valign(gtk4::Align::Center);
    let fetch_state = state.clone();
    fetch_btn.connect_clicked(move |_| {
        super::fetch_images::start_missing_images_fetch(&fetch_state);
    });
    fetch_row.add_suffix(&fetch_btn);
    fetch_group.add(&fetch_row);
    general_page.append(&fetch_group);
    let (overlay_page, overlay_widgets) = build_overlay_settings_page(cfg);
    let registry = state.borrow().controller_registry.clone();
    let (input_page, input_widgets) = build_input_settings_page(
        win,
        &cfg.save_dir,
        cfg,
        state.borrow().steam.clone(),
        registry,
    );
    let controller_default_widgets = input_widgets.controller_defaults;
    let (system_page, system_defaults_widgets) = build_system_defaults_page(cfg);
    let (computer_games_page, game_folders) = build_computer_games_page(cfg);
    let (linux_controller_profile, wine_controller_profile) =
        build_pc_controller_profiles(&computer_games_page, cfg, win, state);
    let (steam_page, steam_enable_row) = build_steam_settings_page(cfg);
    let (emu_page, emu_version_row, emu_version_model) = build_api_emulators_page(cfg);
    let lutris_page = build_lutris_settings_page(state, win);
    let (wine_pages, wine_widgets) =
        build_wine_config_pages(&cfg.default_wine_config, None, &cfg.save_dir);
    let (profiles_page, prefix_base_row, default_version_row, default_version_values) =
        build_profiles_page(state, win);
    let (ra_page, ra_enable_row, ra_username_row, ra_web_api_key_row) = build_ra_settings_page(cfg);
    let (rom_page, rom_roots) = build_rom_settings_page(cfg);
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
        square_row,
        auto_reload_widgets,
        controller_default_widgets,
        game_folders,
        steam_enable_row,
        emu_version_row,
        emu_version_model,
        wine_widgets,
        prefix_base_row,
        default_version_row,
        default_version_values,
        ra_enable_row,
        ra_username_row,
        ra_web_api_key_row,
        rom_roots,
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
        "emblem-system-symbolic",
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
        "emblem-system-symbolic",
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
    let scroll = super::helpers::scrolled_page(page);
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
    win: &impl IsA<gtk4::Widget>,
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
