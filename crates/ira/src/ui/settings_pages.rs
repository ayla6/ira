use super::css::*;
use super::helpers::string_list_from;
use super::settings_dialog::settings_page_container;
use adw::prelude::*;
use ira_config::Config;
use ira_overlay_ipc::{
    clamp_replay_buffer_seconds, MAX_REPLAY_BUFFER_SECONDS, MIN_REPLAY_BUFFER_SECONDS,
};
use std::cell::RefCell;
use std::rc::Rc;

pub(super) fn settings_sidebar_row(icon: &str, label: &str, page_id: &str) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    row.set_widget_name(page_id);
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
    row.add_css_class(CSS_SIDEBAR_SEPARATOR_ROW);
    row
}

pub(super) fn sidebar_section_title(title: &str) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    hbox.set_margin_top(6);
    hbox.set_margin_bottom(2);
    hbox.set_margin_start(10);
    hbox.set_margin_end(10);

    let label = gtk4::Label::new(Some(title));
    label.set_halign(gtk4::Align::Start);
    label.set_valign(gtk4::Align::Center);
    hbox.append(&label);

    let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    sep.set_halign(gtk4::Align::Fill);
    sep.set_hexpand(true);
    sep.set_valign(gtk4::Align::Center);
    hbox.append(&sep);

    row.set_child(Some(&hbox));
    row.set_selectable(false);
    row.set_sensitive(false);
    row.add_css_class(super::css::CSS_SIDEBAR_SECTION_TITLE);
    row
}

pub(super) struct AutoReloadWidgets {
    pub steam: adw::SwitchRow,
    pub roms: adw::SwitchRow,
    pub unpack_roms: adw::SwitchRow,
    pub shadps4: adw::SwitchRow,
    pub rpcs3: adw::SwitchRow,
    pub vita3k: adw::SwitchRow,
    pub cemu: adw::SwitchRow,
    pub azahar: adw::SwitchRow,
}

pub(super) fn build_general_settings_page(
    cfg: &Config,
) -> (
    gtk4::Box,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::PasswordEntryRow,
    adw::PasswordEntryRow,
    gtk4::ListBox,
    adw::SwitchRow,
    AutoReloadWidgets,
) {
    let page = settings_page_container();

    let notif_group = adw::PreferencesGroup::new();
    notif_group.set_title(&crate::tr!("Live updates"));

    let notif_row = adw::SwitchRow::new();
    notif_row.set_title(&crate::tr!("Notify on new unlocks"));
    notif_row.set_subtitle(&crate::tr!(
        "Show a desktop notification the moment a trophy unlocks"
    ));
    notif_row.set_active(cfg.notifications_enabled);
    notif_group.add(&notif_row);

    let bg_row = adw::SwitchRow::new();
    bg_row.set_title(&crate::tr!("Close to background"));
    bg_row.set_subtitle(&crate::tr!(
        "Closing the window keeps the watcher running silently in the background"
    ));
    bg_row.set_active(cfg.close_to_background);
    notif_group.add(&bg_row);
    page.append(&notif_group);

    let hidden_group = adw::PreferencesGroup::new();
    let hidden_row = adw::SwitchRow::new();
    hidden_row.set_title(&crate::tr!("Show hidden games"));
    hidden_row.set_active(cfg.show_hidden_games);
    hidden_group.add(&hidden_row);

    let saves_row = adw::SwitchRow::new();
    saves_row.set_title(&crate::tr!("Centralize game saves"));
    saves_row.set_subtitle(&crate::tr!(
        "Symlink save data to a central location so it persists across Wine prefix resets"
    ));
    saves_row.set_active(cfg.centralize_game_saves);
    hidden_group.add(&saves_row);
    page.append(&hidden_group);

    let reload_group = adw::PreferencesGroup::new();
    reload_group.set_title(&crate::tr!("Automatic library reloads"));
    reload_group.set_description(Some(&crate::tr!(
        "Choose which sources are scanned when Ira starts. Manual rescans always check every enabled source."
    )));
    let auto_reload_steam = auto_reload_row(
        &crate::tr!("Steam"),
        &crate::tr!("Scan installed Steam games when Ira starts"),
        cfg.auto_reload_steam,
    );
    let auto_reload_roms = auto_reload_row(
        &crate::tr!("ROM library"),
        &crate::tr!("Scan ROM folders and refresh RetroAchievements games when Ira starts"),
        cfg.auto_reload_roms,
    );
    let unpack_roms = auto_reload_row(
        &crate::tr!("Read compressed ROMs"),
        &crate::tr!(
            "Unpack .zip/.7z/.zst ROMs in memory to extract DS icons and hashes — slower scans"
        ),
        cfg.unpack_roms,
    );
    let auto_reload_shadps4 = auto_reload_row(
        &crate::tr!("shadPS4"),
        &crate::tr!("Scan shadPS4 games when Ira starts"),
        cfg.auto_reload_shadps4,
    );
    let auto_reload_rpcs3 = auto_reload_row(
        &crate::tr!("RPCS3"),
        &crate::tr!("Scan RPCS3 games when Ira starts"),
        cfg.auto_reload_rpcs3,
    );
    let auto_reload_vita3k = auto_reload_row(
        &crate::tr!("Vita3K"),
        &crate::tr!("Scan Vita3K games when Ira starts"),
        cfg.auto_reload_vita3k,
    );
    let auto_reload_cemu = auto_reload_row(
        &crate::tr!("Cemu"),
        &crate::tr!("Scan Cemu games when Ira starts"),
        cfg.auto_reload_cemu,
    );
    let auto_reload_azahar = auto_reload_row(
        &crate::tr!("Azahar"),
        &crate::tr!("Scan Azahar games when Ira starts"),
        cfg.auto_reload_azahar,
    );
    for row in [
        &auto_reload_steam,
        &auto_reload_roms,
        &unpack_roms,
        &auto_reload_shadps4,
        &auto_reload_rpcs3,
        &auto_reload_vita3k,
        &auto_reload_cemu,
        &auto_reload_azahar,
    ] {
        reload_group.add(row);
    }
    page.append(&reload_group);

    let key_group = adw::PreferencesGroup::new();
    key_group.set_title(&crate::tr!("API keys"));

    let steam_entry = adw::PasswordEntryRow::new();
    steam_entry.set_title(&crate::tr!("Steam web API key"));
    steam_entry.set_text(&cfg.steam_api_key);
    key_group.add(&steam_entry);

    let sgdb_entry = adw::PasswordEntryRow::new();
    sgdb_entry.set_title(&crate::tr!("SteamGridDB API key"));
    sgdb_entry.set_text(&cfg.steam_griddb_api_key);
    key_group.add(&sgdb_entry);
    page.append(&key_group);

    let lang_list = build_language_preferences_list(&cfg.language_preferences);
    let lang_section = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    let lang_title = gtk4::Label::new(Some(&crate::tr!("Language preferences")));
    lang_title.set_halign(gtk4::Align::Start);
    lang_title.add_css_class("heading");
    let lang_desc = gtk4::Label::new(Some(&crate::tr!(
        "When a game is added, the first supported language from this list is used for the emulator config"
    )));
    lang_desc.set_halign(gtk4::Align::Start);
    lang_desc.set_wrap(true);
    lang_desc.add_css_class("dim-label");
    lang_section.append(&lang_title);
    lang_section.append(&lang_desc);
    lang_section.append(&lang_list);
    page.append(&lang_section);

    (
        page,
        notif_row,
        bg_row,
        hidden_row,
        steam_entry,
        sgdb_entry,
        lang_list,
        saves_row,
        AutoReloadWidgets {
            steam: auto_reload_steam,
            roms: auto_reload_roms,
            unpack_roms,
            shadps4: auto_reload_shadps4,
            rpcs3: auto_reload_rpcs3,
            vita3k: auto_reload_vita3k,
            cemu: auto_reload_cemu,
            azahar: auto_reload_azahar,
        },
    )
}

fn auto_reload_row(title: &str, subtitle: &str, active: bool) -> adw::SwitchRow {
    let row = adw::SwitchRow::new();
    row.set_title(title);
    row.set_subtitle(subtitle);
    row.set_active(active);
    row
}

pub(super) struct SystemDefaultsWidgets {
    pub gamemode: adw::SwitchRow,
    pub mangohud: adw::SwitchRow,
    pub gamescope: gtk4::Switch,
    pub gamescope_flags: adw::EntryRow,
    pub gamescope_w: adw::SpinRow,
    pub gamescope_h: adw::SpinRow,
    pub gamescope_fps: adw::SpinRow,
    pub gamescope_upscaling_row: adw::ComboRow,
    pub gpu_row: Rc<RefCell<Option<adw::ComboRow>>>,
    pub gpu_options: Rc<RefCell<Vec<String>>>,
    pub gpu_default: Rc<RefCell<String>>,
    pub env_vars_box: gtk4::ListBox,
    pub ld_preload: adw::EntryRow,
    pub ld_library_path: adw::EntryRow,
}

pub(super) fn build_system_defaults_page(cfg: &Config) -> (gtk4::Box, SystemDefaultsWidgets) {
    let page = settings_page_container();
    let s = &cfg.default_system;

    let perf_group = adw::PreferencesGroup::new();
    perf_group.set_title(&crate::tr!("Performance"));

    let gamemode = adw::SwitchRow::new();
    gamemode.set_title(&crate::tr!("Gamemode"));
    gamemode.set_subtitle(&crate::tr!("Feral Interactive GameMode"));
    gamemode.set_active(s.gamemode);
    perf_group.add(&gamemode);

    let mangohud = adw::SwitchRow::new();
    mangohud.set_title(&crate::tr!("MangoHud"));
    mangohud.set_subtitle(&crate::tr!("Performance overlay"));
    mangohud.set_active(s.mangohud);
    perf_group.add(&mangohud);

    let gamescope = adw::ExpanderRow::new();
    gamescope.set_title(&crate::tr!("Gamescope"));
    gamescope.set_subtitle(&crate::tr!("Valve Gamescope compositor"));
    let gs_switch = gtk4::Switch::new();
    gs_switch.set_active(s.gamescope);
    gs_switch.set_valign(gtk4::Align::Center);
    gamescope.add_suffix(&gs_switch);
    gamescope.set_expanded(s.gamescope);
    perf_group.add(&gamescope);
    {
        let gse = gamescope.clone();
        gs_switch.connect_active_notify(move |sw| {
            if sw.is_active() {
                gse.set_expanded(true);
            }
        });
    }

    let gs_widgets = super::system_settings::add_gamescope_rows(
        &gamescope,
        &super::system_settings::GamescopeDefaults {
            flags: s.gamescope_flags.clone(),
            w: s.gamescope_w,
            h: s.gamescope_h,
            fps: s.gamescope_fps,
            upscaling: s.gamescope_upscaling.clone(),
        },
        None,
    );
    let gamescope_flags = gs_widgets.flags;
    let gamescope_w = gs_widgets.w;
    let gamescope_h = gs_widgets.h;
    let gamescope_fps = gs_widgets.fps;
    let gamescope_upscaling_row = gs_widgets.upscaling;
    page.append(&perf_group);

    // GPU enumeration invokes filesystem reads and lspci; populate this optional
    // section after the rest of the settings page has been constructed.
    let gpu_area = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    page.append(&gpu_area);
    let gpu_row = Rc::new(RefCell::new(None));
    let gpu_options = Rc::new(RefCell::new(Vec::new()));
    let gpu_default = Rc::new(RefCell::new(s.gpu.clone()));
    start_gpu_detection(
        &gpu_area,
        &s.gpu,
        gpu_row.clone(),
        gpu_options.clone(),
        gpu_default.clone(),
    );

    let (env_group, env_vars_box) = super::system_settings::build_env_vars_group(&s.env_vars);
    page.append(&env_group);

    let (ld_group, ld_preload, ld_library_path) =
        super::system_settings::build_ld_paths_group(&s.ld_preload, &s.ld_library_path);
    page.append(&ld_group);

    (
        page,
        SystemDefaultsWidgets {
            gamemode,
            mangohud,
            gamescope: gs_switch,
            gamescope_flags,
            gamescope_w,
            gamescope_h,
            gamescope_fps,
            gamescope_upscaling_row,
            gpu_row,
            gpu_options,
            gpu_default,
            env_vars_box,
            ld_preload,
            ld_library_path,
        },
    )
}

fn start_gpu_detection(
    area: &gtk4::Box,
    current_gpu: &str,
    gpu_row: Rc<RefCell<Option<adw::ComboRow>>>,
    gpu_options: Rc<RefCell<Vec<String>>>,
    gpu_default: Rc<RefCell<String>>,
) {
    let current_gpu = current_gpu.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _s = tracing::info_span!("detect_settings_gpus").entered();
        let _ = tx.send(ira_launcher::gpu::detect_gpus());
    });

    let rx = RefCell::new(rx);
    let area = area.clone();
    glib::source::idle_add_local_full(glib::Priority::LOW, move || {
        match rx.borrow_mut().try_recv() {
            Ok(gpus) => {
                if gpus.len() > 1 {
                    let group = adw::PreferencesGroup::new();
                    group.set_title(&crate::tr!("Graphics"));

                    let model = gtk4::StringList::new(&[]);
                    model.append(&crate::tr!("Auto"));
                    for gpu in &gpus {
                        model.append(&gpu.short_name());
                    }
                    let row = adw::ComboRow::new();
                    row.set_title(&crate::tr!("GPU"));
                    row.set_subtitle(&crate::tr!("Graphics card to use for rendering by default"));
                    row.set_model(Some(&model));
                    let options: Vec<String> = gpus.iter().map(|gpu| gpu.card.clone()).collect();
                    let selected = options
                        .iter()
                        .position(|card| card == &current_gpu)
                        .map(|index| index + 1)
                        .unwrap_or(0);
                    row.set_selected(selected as u32);
                    group.add(&row);
                    area.append(&group);
                    *gpu_options.borrow_mut() = options;
                    *gpu_row.borrow_mut() = Some(row);
                } else {
                    gpu_default.borrow_mut().clear();
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

pub(super) fn build_lutris_settings_page(
    state: &super::state::SharedState,
    settings_win: &adw::Window,
) -> gtk4::Box {
    let page = settings_page_container();

    let info_group = adw::PreferencesGroup::new();
    info_group.set_title(&crate::tr!("Lutris installation"));

    let lutris_dir = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
        .join(".local/share/lutris");
    let dir_row = adw::ActionRow::new();
    dir_row.set_title(&crate::tr!("Lutris data directory"));
    if lutris_dir.is_dir() {
        dir_row.set_subtitle(&lutris_dir.display().to_string());
    } else {
        dir_row.set_subtitle(&crate::tr!("Lutris not found"));
        dir_row.set_sensitive(false);
    }
    info_group.add(&dir_row);
    page.append(&info_group);

    let migrate_group = adw::PreferencesGroup::new();
    migrate_group.set_title(&crate::tr!("Migration"));
    let migrate_row = adw::ActionRow::new();
    migrate_row.set_title(&crate::tr!("Import Lutris games"));
    migrate_row.set_subtitle(&crate::tr!(
        "Reads each Lutris game's config and creates a game entry with wine settings"
    ));
    let migrate_btn = gtk4::Button::with_label(&crate::tr!("Import all"));
    migrate_btn.add_css_class(CSS_SUGGESTED_ACTION);
    migrate_btn.set_valign(gtk4::Align::Center);
    migrate_row.add_suffix(&migrate_btn);
    migrate_group.add(&migrate_row);
    page.append(&migrate_group);

    let db = state.borrow().db.clone();
    let settings_win = settings_win.clone();
    let migrate_btn_for_callback = migrate_btn.clone();
    migrate_btn.connect_clicked(move |_| {
        migrate_btn_for_callback.set_sensitive(false);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _s = tracing::info_span!("load_lutris_games").entered();
            let _ = tx.send(ira_platforms::lutris::load_lutris_games());
        });

        let migrate_btn = migrate_btn_for_callback.clone();
        let settings_win = settings_win.clone();
        let db = db.clone();
        let rx = std::cell::RefCell::new(rx);
        glib::source::idle_add_local_full(glib::Priority::LOW, move || {
            match rx.borrow_mut().try_recv() {
                Ok(Ok(lutris_games)) => {
                    migrate_btn.set_sensitive(true);
                    if lutris_games.is_empty() {
                        return glib::ControlFlow::Break;
                    }

                    let alert = adw::AlertDialog::new(
                        Some(&crate::tr!("Import Lutris games")),
                        Some(
                            &crate::tr!("Import {} Lutris game(s) as managed Wine games?")
                                .replacen("{}", &lutris_games.len().to_string(), 1),
                        ),
                    );
                    alert.add_response("cancel", &crate::tr!("Cancel"));
                    alert.add_response("migrate", &crate::tr!("Migrate"));
                    alert.set_response_appearance("migrate", adw::ResponseAppearance::Suggested);
                    alert.set_default_response(Some("cancel"));
                    alert.set_close_response("cancel");

                    let db = db.clone();
                    let lutris_games = std::rc::Rc::new(lutris_games);
                    alert.connect_response(None, move |_, response| {
                        if response == "migrate" {
                            let db = db.clone();
                            let lutris_games = (*lutris_games).clone();
                            std::thread::spawn(move || {
                                let mut ok = 0;
                                let mut errors = 0;
                                for lg in &lutris_games {
                                    match ira_db::add_game(
                                        &db,
                                        ira_models::GameKind::Wine,
                                        ira_models::TrophySource::Empty,
                                        "",
                                        "",
                                        "",
                                        &lg.name,
                                    ) {
                                        Ok(db_id) => {
                                            match super::edit_game_pages::convert_lutris_to_managed(
                                                &db, db_id, lg.id, &lg.name,
                                            ) {
                                                Ok(()) => ok += 1,
                                                Err(e) => {
                                                    errors += 1;
                                                    eprintln!(
                                                        "Failed to import '{}': {}",
                                                        lg.name, e
                                                    );
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            errors += 1;
                                            eprintln!("Failed to add '{}': {}", lg.name, e);
                                        }
                                    }
                                }
                                eprintln!("Imported {} game(s), {} failed", ok, errors);
                            });
                        }
                    });
                    alert.present(Some(&settings_win));
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    migrate_btn.set_sensitive(true);
                    eprintln!("Failed to load Lutris games: {e}");
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    migrate_btn.set_sensitive(true);
                    glib::ControlFlow::Break
                }
            }
        });
    });

    page
}

pub(super) fn build_steam_settings_page(cfg: &Config) -> (gtk4::Box, adw::SwitchRow) {
    let page = settings_page_container();

    let enable_group = adw::PreferencesGroup::new();
    let enable_row = adw::SwitchRow::new();
    enable_row.set_title(&crate::tr!("Enable Steam integration"));
    enable_row.set_subtitle(&crate::tr!(
        "Scan your Steam library for installed games and achievements"
    ));
    enable_row.set_active(cfg.steam_enabled);
    enable_group.add(&enable_row);
    page.append(&enable_group);

    let info_group = adw::PreferencesGroup::new();
    info_group.set_title(&crate::tr!("Steam installation"));

    let steam_dir = ira_platforms::steam::steam_install_dir();
    let dir_row = adw::ActionRow::new();
    dir_row.set_title(&crate::tr!("Steam directory"));
    match &steam_dir {
        Some(path) => dir_row.set_subtitle(&path.display().to_string()),
        None => {
            dir_row.set_subtitle(&crate::tr!("Steam not found"));
            dir_row.set_sensitive(false);
        }
    }
    info_group.add(&dir_row);

    let user_ids = ira_platforms::steam::get_steam_user_ids();
    let user_row = adw::ActionRow::new();
    user_row.set_title(&crate::tr!("Steam user IDs"));
    if user_ids.is_empty() {
        user_row.set_subtitle(&crate::tr!("None found"));
        user_row.set_sensitive(false);
    } else {
        user_row.set_subtitle(&user_ids.join(", "));
    }
    info_group.add(&user_row);
    page.append(&info_group);

    (page, enable_row)
}

pub(super) fn build_computer_games_page(
    win: &adw::Window,
    cfg: &Config,
) -> (gtk4::Box, adw::EntryRow) {
    let page = settings_page_container();

    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Default game folder"));

    let folder_row = adw::EntryRow::new();
    folder_row.set_title(&crate::tr!("Game folder"));
    folder_row.set_text(&cfg.default_game_folder);

    let folder_browse = super::helpers::make_browse_button(
        Some(win),
        &crate::tr!("Select default game folder"),
        true,
        None,
        super::helpers::entry_path_closure(&folder_row),
        {
            let row = folder_row.clone();
            move |path| row.set_text(&path.to_string_lossy())
        },
    );
    folder_row.add_suffix(&folder_browse);
    group.add(&folder_row);
    page.append(&group);

    (page, folder_row)
}

pub(super) fn build_rom_settings_page(
    win: &adw::Window,
    cfg: &Config,
) -> (gtk4::Box, adw::EntryRow) {
    let page = settings_page_container();
    let rom_group = adw::PreferencesGroup::new();
    rom_group.set_title(&crate::tr!("ROM library"));
    let missing_systems: Vec<&str> = ira_models::all_consoles()
        .filter(|def| def.uses_rom_folder())
        .filter(|def| cfg.console(def.id).enabled && !cfg.rom_folder(def.id).is_dir())
        .map(|def| def.id)
        .collect();
    if !cfg.roms_folder.is_empty() && !std::path::Path::new(&cfg.roms_folder).is_dir() {
        rom_group.set_description(Some(&crate::tr!(
            "The ROM root is missing. Choose a new location; game metadata and relative paths will be kept."
        )));
    } else if !missing_systems.is_empty() {
        rom_group.set_description(Some(&crate::tr!(
            "Missing system folders: {}. Choose another base folder if these games were moved; metadata is retained."
        ).replacen("{}", &missing_systems.join(", "), 1)));
    } else {
        rom_group.set_description(Some(&crate::tr!(
            "ROMs are stored in one folder with a subfolder for each system, such as gba, psx, and ps2."
        )));
    }

    let roms_folder_row = adw::EntryRow::new();
    roms_folder_row.set_title(&crate::tr!("Base ROM folder"));
    roms_folder_row.set_text(&cfg.roms_folder);
    let roms_browse = super::helpers::make_browse_button(
        Some(win),
        &crate::tr!("Select base ROM folder"),
        true,
        None,
        super::helpers::entry_path_closure(&roms_folder_row),
        {
            let row = roms_folder_row.clone();
            move |path| row.set_text(&path.to_string_lossy())
        },
    );
    roms_folder_row.add_suffix(&roms_browse);
    rom_group.add(&roms_folder_row);
    page.append(&rom_group);

    (page, roms_folder_row)
}

pub(super) fn build_ra_settings_page(
    cfg: &Config,
) -> (gtk4::Box, adw::SwitchRow, adw::EntryRow, adw::EntryRow) {
    let page = settings_page_container();

    let enable_group = adw::PreferencesGroup::new();
    let enable_row = adw::SwitchRow::new();
    enable_row.set_title(&crate::tr!("Enable RetroAchievements"));
    enable_row.set_subtitle(&crate::tr!(
        "Fetch achievements for matched retro games from retroachievements.org"
    ));
    enable_row.set_active(cfg.ra_enabled);
    enable_group.add(&enable_row);
    page.append(&enable_group);

    let creds_group = adw::PreferencesGroup::new();
    creds_group.set_title(&crate::tr!("Account"));
    creds_group.set_description(Some(&crate::tr!(
        "A Web API key is required: it fetches hardcore unlocks and real earned dates. Get it from retroachievements.org → Settings → Keys."
    )));

    let username_row = adw::EntryRow::new();
    username_row.set_title(&crate::tr!("Username"));
    username_row.set_text(&cfg.ra_username);
    creds_group.add(&username_row);

    let web_api_row = adw::PasswordEntryRow::new();
    web_api_row.set_title(&crate::tr!("Web API key"));
    web_api_row.set_text(&cfg.ra_web_api_key);
    creds_group.add(&web_api_row);
    page.append(&creds_group);

    (page, enable_row, username_row, web_api_row.upcast())
}

pub(super) fn build_api_emulators_page(
    cfg: &Config,
) -> (gtk4::Box, adw::ComboRow, gtk4::StringList) {
    let page = settings_page_container();

    let emu_dir = ira_platforms::api_emulators::api_emulators_dir(&cfg.save_dir);
    let _ = std::fs::create_dir_all(&emu_dir);

    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("API emulator files"));
    group.set_description(Some(&crate::tr!(
        "Drop emulator files into the structure below"
    )));

    let dir_row = adw::ActionRow::new();
    dir_row.set_title(&crate::tr!("Directory"));
    dir_row.set_subtitle(&emu_dir.to_string_lossy());
    dir_row.set_sensitive(false);

    let open_btn = gtk4::Button::with_label(&crate::tr!("Open"));
    open_btn.add_css_class(CSS_FLAT);
    open_btn.set_valign(gtk4::Align::Center);
    {
        let path = emu_dir;
        open_btn.connect_clicked(move |_| {
            let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
        });
    }
    dir_row.add_suffix(&open_btn);
    group.add(&dir_row);
    page.append(&group);

    let version_group = adw::PreferencesGroup::new();
    version_group.set_title(&crate::tr!("Default version"));
    version_group.set_description(Some(&crate::tr!(
        "Version to use when installing API emulators on games"
    )));

    let gse_versions = ira_platforms::api_emulators::list_gse_versions(&cfg.save_dir);
    let gog_versions = ira_platforms::api_emulators::list_gog_versions(&cfg.save_dir);
    let mut all_versions: Vec<String> = gse_versions
        .iter()
        .chain(gog_versions.iter())
        .cloned()
        .collect();
    all_versions.sort();
    all_versions.dedup();

    let version_model = if all_versions.is_empty() {
        let strings = [crate::tr!("(no versions installed)")];
        let refs: Vec<&str> = strings.iter().map(String::as_str).collect();
        gtk4::StringList::new(&refs)
    } else {
        string_list_from(&all_versions)
    };
    let version_row = adw::ComboRow::new();
    version_row.set_title(&crate::tr!("Emulator version"));
    version_row.set_subtitle(&crate::tr!(
        "Default version directory to use when installing"
    ));
    version_row.set_model(Some(&version_model));
    if !cfg.default_api_emu_version.is_empty() {
        if let Some(idx) = all_versions
            .iter()
            .position(|v| v == &cfg.default_api_emu_version)
        {
            version_row.set_selected(idx as u32);
        }
    } else if !all_versions.is_empty() {
        version_row.set_selected(0);
    }
    version_group.add(&version_row);
    page.append(&version_group);

    (page, version_row, version_model)
}

pub(super) struct OverlayPageWidgets {
    pub enable_row: adw::SwitchRow,
    pub replay_buffer_row: adw::SwitchRow,
    pub replay_duration_row: adw::SpinRow,
    pub encoder_row: adw::ComboRow,
    pub quality_row: adw::ComboRow,
    pub toggle_hotkey: super::hotkey_widget::HotkeyWidgets,
    pub screenshot_hotkey: super::hotkey_widget::HotkeyWidgets,
    pub record_hotkey: super::hotkey_widget::HotkeyWidgets,
    pub font_button: gtk4::FontDialogButton,
}

pub(super) fn build_overlay_settings_page(cfg: &Config) -> (gtk4::Box, OverlayPageWidgets) {
    let page = settings_page_container();
    let o = &cfg.overlay;

    let enable_group = adw::PreferencesGroup::new();
    let enable_row = adw::SwitchRow::new();
    enable_row.set_title(&crate::tr!("Enable in-game overlay"));
    enable_row.set_subtitle(&crate::tr!(
        "Shows achievements, screenshots, and recording during gameplay (Vulkan games only)"
    ));
    enable_row.set_active(o.enabled);
    enable_group.add(&enable_row);
    page.append(&enable_group);

    let recording_group = adw::PreferencesGroup::new();
    recording_group.set_title(&crate::tr!("Recording"));

    let encoder_strings = [
        crate::tr!("Auto"),
        crate::tr!("VAAPI (AMD/Intel)"),
        crate::tr!("NVENC (NVIDIA)"),
        crate::tr!("Software (CPU)"),
    ];
    let encoder_refs: Vec<&str> = encoder_strings.iter().map(String::as_str).collect();
    let encoder_model = gtk4::StringList::new(&encoder_refs);
    let encoder_row = adw::ComboRow::new();
    encoder_row.set_title(&crate::tr!("Video encoder"));
    encoder_row.set_subtitle(&crate::tr!(
        "Auto detects the best available encoder. Use Software if your GPU lacks hardware encoding."
    ));
    encoder_row.set_model(Some(&encoder_model));
    encoder_row.set_selected(o.encoder as u32);
    recording_group.add(&encoder_row);

    let quality_strings = [
        crate::tr!("Low (720p 30fps)"),
        crate::tr!("Medium (1080p 30fps)"),
        crate::tr!("High (1080p 60fps)"),
    ];
    let quality_refs: Vec<&str> = quality_strings.iter().map(String::as_str).collect();
    let quality_model = gtk4::StringList::new(&quality_refs);
    let quality_row = adw::ComboRow::new();
    quality_row.set_title(&crate::tr!("Recording quality"));
    quality_row.set_model(Some(&quality_model));
    quality_row.set_selected(o.recording_quality.as_u32());
    recording_group.add(&quality_row);

    let replay_buffer_row = adw::SwitchRow::new();
    replay_buffer_row.set_title(&crate::tr!("Enable replay buffer"));
    replay_buffer_row.set_subtitle(&crate::tr!(
        "Keeps recent gameplay available as a rolling DASH session"
    ));
    replay_buffer_row.set_active(o.replay_buffer_enabled);
    recording_group.add(&replay_buffer_row);

    let replay_minutes = clamp_replay_buffer_seconds(o.replay_buffer_seconds).div_ceil(60);
    let replay_duration_adjustment = gtk4::Adjustment::new(
        replay_minutes as f64,
        (MIN_REPLAY_BUFFER_SECONDS / 60) as f64,
        (MAX_REPLAY_BUFFER_SECONDS / 60) as f64,
        1.0,
        5.0,
        0.0,
    );
    let replay_duration_row = adw::SpinRow::new(Some(&replay_duration_adjustment), 1.0, 0);
    replay_duration_row.set_title(&crate::tr!("Replay buffer duration"));
    replay_duration_row.set_subtitle(&crate::tr!(
        "Minutes of gameplay retained for instant replay"
    ));
    replay_duration_row.set_sensitive(o.replay_buffer_enabled);
    recording_group.add(&replay_duration_row);
    {
        let replay_duration_row = replay_duration_row.clone();
        replay_buffer_row.connect_active_notify(move |row| {
            replay_duration_row.set_sensitive(row.is_active());
        });
    }
    page.append(&recording_group);

    let hotkeys_group = adw::PreferencesGroup::new();
    hotkeys_group.set_title(&crate::tr!("Hotkeys"));
    hotkeys_group.set_description(Some(&crate::tr!(
        "Keyboard and gamepad bindings. Click to set."
    )));

    let toggle_hotkey = super::hotkey_widget::build_hotkey_row(
        &hotkeys_group,
        &crate::tr!("Toggle overlay"),
        &o.toggle_hotkey,
        &o.toggle_hotkey_gamepad,
        "Shift+Tab",
        "Guide",
    );
    let screenshot_hotkey = super::hotkey_widget::build_hotkey_row(
        &hotkeys_group,
        &crate::tr!("Screenshot"),
        &o.screenshot_hotkey,
        &o.screenshot_hotkey_gamepad,
        "F12",
        "Guide+DpadDown",
    );
    let record_hotkey = super::hotkey_widget::build_hotkey_row(
        &hotkeys_group,
        &crate::tr!("Toggle recording"),
        &o.record_hotkey,
        &o.record_hotkey_gamepad,
        "F11",
        "Guide+DpadUp",
    );
    page.append(&hotkeys_group);

    let font_group = adw::PreferencesGroup::new();
    font_group.set_title(&crate::tr!("Appearance"));

    let font_dialog = gtk4::FontDialog::new();
    font_dialog.set_title(&crate::tr!("Select overlay font"));

    let font_button = gtk4::FontDialogButton::new(Some(font_dialog));
    font_button.set_valign(gtk4::Align::Center);
    font_button.set_level(gtk4::FontLevel::Family);
    font_button.set_use_font(false);

    let system_font = "Sans";
    if let Some(ref family) = o.font_family {
        let desc = pango::FontDescription::from_string(family);
        font_button.set_font_desc(&desc);
    } else {
        let desc = pango::FontDescription::from_string(system_font);
        font_button.set_font_desc(&desc);
    }

    let font_reset = gtk4::Button::from_icon_name("edit-undo-symbolic");
    font_reset.set_valign(gtk4::Align::Center);
    font_reset.set_tooltip_text(Some(&crate::tr!("Reset to system default")));
    font_reset.add_css_class("flat");
    font_reset.set_visible(o.font_family.is_some());

    let font_row = adw::ActionRow::new();
    font_row.set_title(&crate::tr!("Font family"));
    font_row.add_suffix(&font_button);
    font_row.add_suffix(&font_reset);
    font_group.add(&font_row);
    page.append(&font_group);

    // Store font_reset in the widgets so the save handler can access it
    font_reset.connect_clicked({
        let fb = font_button.clone();
        let desc = pango::FontDescription::from_string(system_font);
        move |_| {
            fb.set_font_desc(&desc);
        }
    });

    (
        page,
        OverlayPageWidgets {
            enable_row,
            replay_buffer_row,
            replay_duration_row,
            encoder_row,
            quality_row,
            toggle_hotkey,
            screenshot_hotkey,
            record_hotkey,
            font_button,
        },
    )
}

/// Build a reorderable list of preferred languages.
/// Each row shows the English language name with up/down/remove buttons.
/// An "add" row at the bottom uses a dropdown of unused languages.
fn build_language_preferences_list(enabled: &[String]) -> gtk4::ListBox {
    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);
    list.add_css_class("boxed-list");

    for code in enabled {
        add_language_row(&list, code);
    }
    add_add_language_row(&list);
    list
}

fn add_language_row(list: &gtk4::ListBox, code: &str) {
    let row = gtk4::ListBoxRow::new();
    row.set_widget_name(code);

    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    hbox.set_margin_start(12);
    hbox.set_margin_end(12);
    hbox.set_margin_top(8);
    hbox.set_margin_bottom(8);

    let name = gtk4::Label::new(Some(ira_models::steam_language_name(code)));
    name.set_halign(gtk4::Align::Start);
    name.set_hexpand(true);

    let up_btn = gtk4::Button::from_icon_name("go-up-symbolic");
    up_btn.add_css_class(CSS_FLAT);
    up_btn.set_valign(gtk4::Align::Center);
    up_btn.set_tooltip_text(Some(&crate::tr!("Move up")));

    let down_btn = gtk4::Button::from_icon_name("go-down-symbolic");
    down_btn.add_css_class(CSS_FLAT);
    down_btn.set_valign(gtk4::Align::Center);
    down_btn.set_tooltip_text(Some(&crate::tr!("Move down")));

    let remove_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
    remove_btn.add_css_class(CSS_FLAT);
    remove_btn.set_valign(gtk4::Align::Center);
    remove_btn.set_tooltip_text(Some(&crate::tr!("Remove language")));

    hbox.append(&name);
    hbox.append(&up_btn);
    hbox.append(&down_btn);
    hbox.append(&remove_btn);
    row.set_child(Some(&hbox));

    let list_c = list.clone();
    let row_c = row.clone();
    up_btn.connect_clicked(move |_| {
        let pos = row_c.index();
        if pos > 0 {
            list_c.remove(&row_c);
            list_c.insert(&row_c, pos - 1);
        }
    });

    let list_c = list.clone();
    let row_c = row.clone();
    down_btn.connect_clicked(move |_| {
        let pos = row_c.index();
        let total = count_language_rows(&list_c) as i32;
        if pos >= 0 && pos < total - 1 {
            list_c.remove(&row_c);
            list_c.insert(&row_c, pos + 1);
        }
    });

    let list_c = list.clone();
    let row_c = row.clone();
    remove_btn.connect_clicked(move |_| {
        list_c.remove(&row_c);
    });

    list.insert(&row, count_language_rows(list) as i32);
}

/// Count only language rows (not the "add" row, which has no widget name).
fn count_language_rows(list: &gtk4::ListBox) -> u32 {
    let mut count = 0u32;
    let mut child = list.first_child();
    while let Some(c) = child {
        if c.widget_name() != "add_language" && c.is::<gtk4::ListBoxRow>() {
            count += 1;
        }
        child = c.next_sibling();
    }
    count
}

fn add_add_language_row(list: &gtk4::ListBox) {
    let row = gtk4::ListBoxRow::new();
    row.set_widget_name("add_language");
    row.set_selectable(false);

    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    hbox.set_margin_start(12);
    hbox.set_margin_end(12);
    hbox.set_margin_top(8);
    hbox.set_margin_bottom(8);

    let initial_strings = available_language_strings(list);
    let initial_refs: Vec<&str> = initial_strings.iter().map(|s| s.as_str()).collect();
    let dropdown = gtk4::DropDown::from_strings(&initial_refs);
    dropdown.set_hexpand(true);

    let add_btn = gtk4::Button::with_label(&crate::tr!("Add"));
    add_btn.add_css_class(CSS_SUGGESTED_ACTION);
    add_btn.set_valign(gtk4::Align::Center);

    hbox.append(&dropdown);
    hbox.append(&add_btn);
    row.set_child(Some(&hbox));

    let list_c = list.clone();
    let dropdown_c = dropdown;
    add_btn.connect_clicked(move |_| {
        let selected = dropdown_c.selected() as usize;
        let available = available_language_codes(&list_c);
        if let Some(code) = available.get(selected) {
            add_language_row(&list_c, code);
            let strings = available_language_strings(&list_c);
            let refs: Vec<&str> = strings.iter().map(|s| s.as_str()).collect();
            dropdown_c.set_model(Some(&gtk4::StringList::new(&refs)));
            dropdown_c.set_selected(0);
        }
    });

    list.append(&row);
}

fn enabled_language_codes(list: &gtk4::ListBox) -> Vec<String> {
    let mut codes = Vec::new();
    let mut child = list.first_child();
    while let Some(c) = child {
        if let Some(row) = c.downcast_ref::<gtk4::ListBoxRow>() {
            let name = row.widget_name().to_string();
            if name != "add_language" && !name.is_empty() {
                codes.push(name);
            }
        }
        child = c.next_sibling();
    }
    codes
}

fn available_language_codes(list: &gtk4::ListBox) -> Vec<&'static str> {
    let enabled = enabled_language_codes(list);
    ira_models::STEAM_LANGUAGES
        .iter()
        .map(|l| l.code)
        .filter(|code| !enabled.iter().any(|e| e == code))
        .collect()
}

fn available_language_strings(list: &gtk4::ListBox) -> Vec<String> {
    available_language_codes(list)
        .iter()
        .map(|code| ira_models::steam_language_name(code).to_string())
        .collect()
}

/// Read the ordered language codes from the ListBox (for saving).
pub(super) fn read_language_preferences(list: &gtk4::ListBox) -> Vec<String> {
    enabled_language_codes(list)
}
