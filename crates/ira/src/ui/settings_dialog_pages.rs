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
    pub(super) sgdb_page: gtk4::Box,
    pub(super) sgdb_widgets: super::settings_pages::SgdbSettingsWidgets,
    pub(super) screenscraper_page: gtk4::Box,
    pub(super) screenscraper_enable_row: adw::SwitchRow,
    pub(super) screenscraper_id_row: adw::EntryRow,
    pub(super) screenscraper_password_row: adw::PasswordEntryRow,
    pub(super) hidden_row: adw::SwitchRow,
    pub(super) steam_entry: adw::PasswordEntryRow,
    pub(super) lang_list: gtk4::ListBox,
    pub(super) saves_row: adw::SwitchRow,
    pub(super) default_fullscreen_row: adw::SwitchRow,
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

pub(super) struct GeneralSection {
    pub(super) general_page: gtk4::Box,
    pub(super) notif_row: adw::SwitchRow,
    pub(super) bg_row: adw::SwitchRow,
    pub(super) hidden_row: adw::SwitchRow,
    pub(super) steam_entry: adw::PasswordEntryRow,
    pub(super) lang_list: gtk4::ListBox,
    pub(super) saves_row: adw::SwitchRow,
    pub(super) square_row: adw::SwitchRow,
    pub(super) default_fullscreen_row: adw::SwitchRow,
    pub(super) auto_reload_widgets: AutoReloadWidgets,
}

/// The landing page, built synchronously at open: the dialog paints with
/// real content while the remaining pages build in bulk just after.
pub(super) fn build_general_section(
    cfg: &Config,
    state: &SharedState,
) -> GeneralSection {
    let (
        general_page,
        notif_row,
        bg_row,
        hidden_row,
        steam_entry,
        lang_list,
        saves_row,
        square_row,
        default_fullscreen_row,
        auto_reload_widgets,
    ) = build_general_settings_page(cfg);

    // One-click maintenance: mass refetches of what matching skips —
    // missing art, and holes in matched games' stored metadata. Both
    // jobs report on the sidebar strip and outlive their dialogs.
    let fetch_group = adw::PreferencesGroup::new();
    fetch_group.set_title(&crate::tr!("Fetch missing"));
    let fetch_row = adw::ActionRow::new();
    fetch_row.set_title(&crate::tr!("Missing images"));
    fetch_row.set_subtitle(&crate::tr!(
        "Re-downloads any missing SGDB art for matched games, even ones skipped before"
    ));
    let fetch_btn = gtk4::Button::with_label(&crate::tr!("Fetch"));
    fetch_btn.set_valign(gtk4::Align::Center);
    let fetch_state = state.clone();
    fetch_btn.connect_clicked(move |btn| {
        if let Err(message) = super::fetch_images::start_missing_images_fetch(&fetch_state) {
            super::helpers::toast_on_root(btn, &message);
        }
    });
    fetch_row.add_suffix(&fetch_btn);
    fetch_group.add(&fetch_row);
    let all_row = adw::ActionRow::new();
    all_row.set_title(&crate::tr!("Missing metadata"));
    all_row.set_subtitle(&crate::tr!(
        "One pass over both sources: Steam fills PC games, ScreenScraper refetches matched entries"
    ));
    let all_btn = gtk4::Button::with_label(&crate::tr!("Fetch"));
    all_btn.set_valign(gtk4::Align::Center);
    let all_state = state.clone();
    all_btn.connect_clicked(move |btn| {
        if !super::fetch_metadata::start_full_refetch(&all_state, false) {
            super::helpers::toast_on_root(btn, &crate::tr!("Nothing to fetch right now").to_string());
        }
    });
    all_row.add_suffix(&all_btn);
    fetch_group.add(&all_row);
    let genres_row = adw::ActionRow::new();
    genres_row.set_title(&crate::tr!("Genre table"));
    genres_row.set_subtitle(&crate::tr!(
        "Fetches ScreenScraper's whole genre list. Run it again to refresh"
    ));
    let genres_btn = gtk4::Button::with_label(&crate::tr!("Fetch"));
    genres_btn.set_valign(gtk4::Align::Center);
    let genres_state = state.clone();
    genres_btn.connect_clicked(move |btn| {
        if !super::fetch_metadata::start_genre_warm(&genres_state) {
            super::helpers::toast_on_root(btn, &crate::tr!("A job is already running").to_string());
        }
    });
    genres_row.add_suffix(&genres_btn);
    fetch_group.add(&genres_row);
    let force_row = adw::ActionRow::new();
    force_row.set_title(&crate::tr!("Everything, even complete games"));
    force_row.set_subtitle(&crate::tr!(
        "Re-reads every matched game and every linked Steam page, picking up anything new"
    ));
    let force_btn = gtk4::Button::with_label(&crate::tr!("Force refetch"));
    force_btn.set_valign(gtk4::Align::Center);
    let force_state = state.clone();
    force_btn.connect_clicked(move |_| {
        super::fetch_metadata::start_full_refetch(&force_state, true);
    });
    force_row.add_suffix(&force_btn);
    fetch_group.add(&force_row);
    let meta_row = adw::ActionRow::new();
    meta_row.set_title(&crate::tr!("Missing Steam data"));
    meta_row.set_subtitle(&crate::tr!(
        "Fills games' release dates, studios, synopses and age boards from Steam"
    ));
    let meta_btn = gtk4::Button::with_label(&crate::tr!("Fetch"));
    meta_btn.set_valign(gtk4::Align::Center);
    let meta_state = state.clone();
    meta_btn.connect_clicked(move |btn| {
        if !super::fetch_metadata::start_steam_refetch(&meta_state, false) {
            super::helpers::toast_on_root(btn, &crate::tr!("Nothing to fetch right now").to_string());
        }
    });
    meta_row.add_suffix(&meta_btn);
    fetch_group.add(&meta_row);
    let ss_row = adw::ActionRow::new();
    ss_row.set_title(&crate::tr!("Missing ScreenScraper data"));
    ss_row.set_subtitle(&crate::tr!(
        "Refetches matched games' entries by id and fills any gaps"
    ));
    let ss_btn = gtk4::Button::with_label(&crate::tr!("Fetch"));
    ss_btn.set_valign(gtk4::Align::Center);
    let ss_state = state.clone();
    ss_btn.connect_clicked(move |btn| {
        if !super::fetch_metadata::start_metadata_refetch(&ss_state, false) {
            super::helpers::toast_on_root(btn, &crate::tr!("Nothing to fetch right now").to_string());
        }
    });
    ss_row.add_suffix(&ss_btn);
    fetch_group.add(&ss_row);
    general_page.append(&fetch_group);
    GeneralSection {
        general_page,
        notif_row,
        bg_row,
        hidden_row,
        steam_entry,
        lang_list,
        saves_row,
        square_row,
        default_fullscreen_row,
        auto_reload_widgets,
    }
}

pub(super) struct RemainingPages {
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
    pub(super) sgdb_page: gtk4::Box,
    pub(super) sgdb_widgets: super::settings_pages::SgdbSettingsWidgets,
    pub(super) screenscraper_page: gtk4::Box,
    pub(super) screenscraper_enable_row: adw::SwitchRow,
    pub(super) screenscraper_id_row: adw::EntryRow,
    pub(super) screenscraper_password_row: adw::PasswordEntryRow,
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

/// Every page but the landing one, built in bulk just after open (or on
/// demand when a row or Save beats the idle callback to it).
pub(super) fn build_remaining_pages(
    cfg: &Config,
    win: &impl IsA<gtk4::Widget>,
    state: &SharedState,
) -> RemainingPages {
    let (sgdb_page, sgdb_widgets) = super::settings_pages::build_sgdb_settings_page(cfg);
    let (
        screenscraper_page,
        screenscraper_enable_row,
        screenscraper_id_row,
        screenscraper_password_row,
    ) = super::settings_pages::build_screenscraper_settings_page(cfg);
    let (overlay_page, overlay_widgets) = build_overlay_settings_page(cfg);
    let registry = state.borrow().controller_registry.clone();
    // One profile read for the whole dialog: listing re-parses and
    // re-validates every stored layout, so the input page, the PC groups
    // and the profile rows all share this vec. Lazy console pages and
    // user actions read fresh on their own.
    let profiles = super::input_profile_settings::read_profiles(&cfg.save_dir);
    let (input_page, input_widgets) = build_input_settings_page(
        win,
        &cfg.save_dir,
        cfg,
        state.borrow().steam.clone(),
        registry,
        &profiles,
    );
    let controller_default_widgets = input_widgets.controller_defaults;
    let (system_page, system_defaults_widgets) = build_system_defaults_page(cfg);
    let (computer_games_page, game_folders) = build_computer_games_page(cfg);
    let (linux_controller_profile, wine_controller_profile) =
        build_pc_controller_profiles(&computer_games_page, cfg, win, state, &profiles);
    let (steam_page, steam_enable_row) = build_steam_settings_page(cfg);
    let (emu_page, emu_version_row, emu_version_model) = build_api_emulators_page(cfg);
    let lutris_page = build_lutris_settings_page(state, win);
    let (wine_pages, wine_widgets) =
        build_wine_config_pages(&cfg.default_wine_config, None, &cfg.save_dir);
    let (profiles_page, prefix_base_row, default_version_row, default_version_values) =
        build_profiles_page(state, win);
    let (ra_page, ra_enable_row, ra_username_row, ra_web_api_key_row) = build_ra_settings_page(cfg);
    let (rom_page, rom_roots) = build_rom_settings_page(cfg);
    RemainingPages {
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
        sgdb_page,
        sgdb_widgets,
        screenscraper_page,
        screenscraper_enable_row,
        screenscraper_id_row,
        screenscraper_password_row,
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

/// One dialog's full page set: the eagerly built landing section plus the
/// bulk-built remainder. The save path reads this, so Save forces the bulk
/// build when it beats the idle callback to it.
pub(super) fn assemble_pages(general: GeneralSection, rest: RemainingPages) -> SettingsPageWidgets {
    SettingsPageWidgets {
        overlay_page: rest.overlay_page,
        input_page: rest.input_page,
        system_page: rest.system_page,
        computer_games_page: rest.computer_games_page,
        steam_page: rest.steam_page,
        emu_page: rest.emu_page,
        lutris_page: rest.lutris_page,
        wine_pages: rest.wine_pages,
        profiles_page: rest.profiles_page,
        ra_page: rest.ra_page,
        rom_page: rest.rom_page,
        notif_row: general.notif_row,
        bg_row: general.bg_row,
        hidden_row: general.hidden_row,
        steam_entry: general.steam_entry,
        sgdb_page: rest.sgdb_page,
        sgdb_widgets: rest.sgdb_widgets,
        screenscraper_page: rest.screenscraper_page,
        screenscraper_enable_row: rest.screenscraper_enable_row,
        screenscraper_id_row: rest.screenscraper_id_row,
        screenscraper_password_row: rest.screenscraper_password_row,
        lang_list: general.lang_list,
        saves_row: general.saves_row,
        square_row: general.square_row,
        default_fullscreen_row: general.default_fullscreen_row,
        auto_reload_widgets: general.auto_reload_widgets,
        controller_default_widgets: rest.controller_default_widgets,
        game_folders: rest.game_folders,
        steam_enable_row: rest.steam_enable_row,
        emu_version_row: rest.emu_version_row,
        emu_version_model: rest.emu_version_model,
        wine_widgets: rest.wine_widgets,
        prefix_base_row: rest.prefix_base_row,
        default_version_row: rest.default_version_row,
        default_version_values: rest.default_version_values,
        ra_enable_row: rest.ra_enable_row,
        ra_username_row: rest.ra_username_row,
        ra_web_api_key_row: rest.ra_web_api_key_row,
        rom_roots: rest.rom_roots,
        overlay_widgets: rest.overlay_widgets,
        system_defaults_widgets: rest.system_defaults_widgets,
        linux_controller_profile: rest.linux_controller_profile,
        wine_controller_profile: rest.wine_controller_profile,
    }
}

/// The dialog's sidebar rows with empty placeholders, in sidebar order:
/// the shell paints instantly and the bulk build swaps real content in. Placeholders are plain boxes — unlike the
/// console pages' spinners they are never actually seen (the idle bulk wins
/// the race to every navigation), so there is nothing to animate.
pub(super) fn register_settings_rows(sidebar: &gtk4::ListBox, stack: &gtk4::Stack) {
    enum Row {
        Page(&'static str, String, &'static str),
        Section(String),
    }
    let rows = [
        Row::Page("emblem-system-symbolic", crate::tr!("General"), "general"),
        Row::Page("layers-symbolic", crate::tr!("Overlay"), "overlay"),
        Row::Page("games-symbolic", crate::tr!("Controller"), "input"),
        Row::Page(
            "emblem-system-symbolic",
            crate::tr!("Game system"),
            "system",
        ),
        Row::Section(crate::tr!("Metadata")),
        Row::Page("globe-symbolic", crate::tr!("SteamGridDB"), "sgdb"),
        Row::Page(
            "folder-saved-search-symbolic",
            crate::tr!("ScreenScraper"),
            "screenscraper",
        ),
        Row::Section(crate::tr!("PC games")),
        Row::Page("games-symbolic", crate::tr!("PC games"), "computer_games"),
        Row::Page("steam-train-symbolic", crate::tr!("Steam"), "steam"),
        Row::Page(
            "api-symbolic",
            crate::tr!("API emulators"),
            "api_emulators",
        ),
        Row::Page(
            "system-software-install-symbolic",
            crate::tr!("Lutris migration"),
            "migration",
        ),
        Row::Section(crate::tr!("Wine")),
        Row::Page("wine-glass-symbolic", crate::tr!("Profiles"), "profiles"),
        Row::Page(
            "power-profile-performance-symbolic",
            crate::tr!("Performance"),
            "performance",
        ),
        Row::Page(
            "video-display-symbolic",
            crate::tr!("Graphics"),
            "graphics",
        ),
        Row::Page(
            "preferences-other-symbolic",
            crate::tr!("Advanced"),
            "advanced",
        ),
        Row::Section(crate::tr!("Emulation")),
        Row::Page(
            "trophy-symbolic",
            crate::tr!("RetroAchievements"),
            "ra",
        ),
        Row::Page("library-symbolic", crate::tr!("ROM library"), "roms"),
    ];
    for row in rows {
        match row {
            Row::Section(title) => {
                sidebar.append(&super::settings_pages::sidebar_section_title(&title))
            }
            Row::Page(icon, label, page_id) => {
                sidebar.append(&super::settings_pages::settings_sidebar_row(
                    icon, &label, page_id,
                ));
                stack.add_named(
                    &gtk4::Box::new(gtk4::Orientation::Vertical, 0),
                    Some(page_id),
                );
            }
        }
    }
}

/// Swap a placeholder for the built page. Plain boxes wrap in the scrolled
/// page like `register_page`; already-scrolled pages (profiles, wine)
/// go in as they are.
pub(super) fn fill_page(stack: &gtk4::Stack, page_id: &str, page: &impl IsA<gtk4::Widget>) {
    if let Some(placeholder) = stack.child_by_name(page_id) {
        stack.remove(&placeholder);
    }
    let scroll = super::helpers::scrolled_page(page);
    stack.add_named(&scroll, Some(page_id));
}

/// Swap a placeholder for an already-scrolled page.
pub(super) fn fill_scrolled_page(
    stack: &gtk4::Stack,
    page_id: &str,
    page: &gtk4::ScrolledWindow,
) {
    if let Some(placeholder) = stack.child_by_name(page_id) {
        stack.remove(&placeholder);
    }
    stack.add_named(page, Some(page_id));
}

fn build_pc_controller_profiles(
    page: &gtk4::Box,
    cfg: &Config,
    win: &impl IsA<gtk4::Widget>,
    state: &SharedState,
    profiles: &[super::input_profile_store::StoredProfile],
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
        ira_models::GameKind::Linux.as_str(),
        &crate::tr!("Linux controller"),
        win,
        registry.clone(),
        profiles,
    );
    let wine = super::input_profile_settings::add_pc_profile_group(
        page,
        &pc_controller_cfg,
        ira_models::GameKind::Wine.as_str(),
        &crate::tr!("Wine controller"),
        win,
        registry,
        profiles,
    );
    (linux, wine)
}
