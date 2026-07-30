use gtk4::prelude::*;
use adw::prelude::*;
use ira_config::Config;
use crate::strings as S;
use super::helpers::string_list_from;
use super::settings_dialog::settings_page_container;
use super::css::*;

fn detect_system_font() -> Option<String> {
    let output = std::process::Command::new("fc-match")
        .args(["-f", "%{family}", "sans-serif"])
        .output()
        .ok()?;
    if !output.status.success() { return None; }
    let family = String::from_utf8_lossy(&output.stdout)
        .split(',')
        .next()?
        .trim()
        .to_string();
    if family.is_empty() { None } else { Some(family) }
}

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

pub(super) fn build_general_settings_page(cfg: &Config) -> (gtk4::Box, adw::SwitchRow, adw::SwitchRow, adw::SwitchRow, adw::PasswordEntryRow, adw::PasswordEntryRow) {
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

    (page, notif_row, bg_row, hidden_row, steam_entry, sgdb_entry)
}

pub(super) struct SystemDefaultsWidgets {
    pub gamemode: adw::SwitchRow,
    pub mangohud: adw::SwitchRow,
    pub gamescope: gtk4::Switch,
    pub gamescope_flags: adw::EntryRow,
    pub gamescope_w: gtk4::SpinButton,
    pub gamescope_h: gtk4::SpinButton,
    pub gamescope_fps: gtk4::SpinButton,
    pub gamescope_upscaling_row: adw::ComboRow,
    pub env_vars_box: gtk4::ListBox,
    pub ld_preload: adw::EntryRow,
    pub ld_library_path: adw::EntryRow,
}

pub(super) fn build_system_defaults_page(cfg: &Config) -> (gtk4::Box, SystemDefaultsWidgets) {
    let page = settings_page_container();
    let s = &cfg.default_system;

    let perf_group = adw::PreferencesGroup::new();
    perf_group.set_title("Performance");

    let gamemode = adw::SwitchRow::new();
    gamemode.set_title("Gamemode");
    gamemode.set_subtitle("Feral Interactive GameMode");
    gamemode.set_active(s.gamemode);
    perf_group.add(&gamemode);

    let mangohud = adw::SwitchRow::new();
    mangohud.set_title("MangoHud");
    mangohud.set_subtitle("Performance overlay");
    mangohud.set_active(s.mangohud);
    perf_group.add(&mangohud);

    let gamescope = adw::ExpanderRow::new();
    gamescope.set_title("Gamescope");
    gamescope.set_subtitle("Valve Gamescope compositor");
    let gs_switch = gtk4::Switch::new();
    gs_switch.set_active(s.gamescope);
    gs_switch.set_valign(gtk4::Align::Center);
    gamescope.add_suffix(&gs_switch);
    gamescope.set_expanded(s.gamescope);
    perf_group.add(&gamescope);
    {
        let gse = gamescope.clone();
        gs_switch.connect_active_notify(move |sw| {
            if sw.is_active() { gse.set_expanded(true); }
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

    let (env_group, env_vars_box) = super::system_settings::build_env_vars_group(&s.env_vars);
    page.append(&env_group);

    let (ld_group, ld_preload, ld_library_path) = super::system_settings::build_ld_paths_group(
        &s.ld_preload, &s.ld_library_path,
    );
    page.append(&ld_group);

    (page, SystemDefaultsWidgets { gamemode, mangohud, gamescope: gs_switch, gamescope_flags, gamescope_w, gamescope_h, gamescope_fps, gamescope_upscaling_row, env_vars_box, ld_preload, ld_library_path })
}

pub(super) fn build_lutris_settings_page(state: &super::state::SharedState, settings_win: &adw::Window) -> gtk4::Box {
    let page = settings_page_container();

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

    let migrate_group = adw::PreferencesGroup::new();
    migrate_group.set_title("Migration");
    let migrate_row = adw::ActionRow::new();
    migrate_row.set_title("Import Lutris games");
    migrate_row.set_subtitle("Reads each Lutris game's config and creates a game entry with wine settings");
    let migrate_btn = gtk4::Button::with_label("Import All");
    migrate_btn.add_css_class(CSS_SUGGESTED_ACTION);
    migrate_btn.set_valign(gtk4::Align::Center);
    migrate_row.add_suffix(&migrate_btn);
    migrate_group.add(&migrate_row);
    page.append(&migrate_group);

    let sc = state.clone();
    let settings_win = settings_win.clone();
    migrate_btn.connect_clicked(move |_| {
        let lutris_games = match ira_platforms::lutris::load_lutris_games() {
            Ok(g) => g,
            Err(e) => {
                eprintln!("Failed to load Lutris games: {}", e);
                return;
            }
        };

        if lutris_games.is_empty() {
            return;
        }

        let alert = adw::AlertDialog::new(
            Some("Import Lutris games"),
            Some(&format!("Import {} Lutris game(s) as managed Wine games?", lutris_games.len())),
        );
        alert.add_response("cancel", "Cancel");
        alert.add_response("migrate", "Migrate");
        alert.set_response_appearance("migrate", adw::ResponseAppearance::Suggested);
        alert.set_default_response(Some("cancel"));
        alert.set_close_response("cancel");

        let db = sc.borrow().db.clone();
        let lutris_games = std::rc::Rc::new(lutris_games);
        alert.connect_response(None, move |_, response| {
            if response == "migrate" {
                let db = db.clone();
                let lutris_games = (*lutris_games).clone();
                std::thread::spawn(move || {
                    let mut ok = 0;
                    let mut errors = 0;
                    for lg in &lutris_games {
                        match ira_db::add_game(&db, ira_models::GameKind::Wine, ira_models::TrophySource::Empty, "", "", "", &lg.name) {
                            Ok(db_id) => {
                                match super::edit_game_pages::convert_lutris_to_managed(&db, db_id, lg.id, &lg.name) {
                                    Ok(()) => ok += 1,
                                    Err(e) => {
                                        errors += 1;
                                        eprintln!("Failed to import '{}': {}", lg.name, e);
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
    });

    page
}

pub(super) fn build_steam_settings_page(cfg: &Config) -> (gtk4::Box, adw::SwitchRow) {
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

pub(super) fn build_computer_games_page(
    win: &adw::Window,
    cfg: &Config,
) -> (gtk4::Box, adw::EntryRow) {
    let page = settings_page_container();

    let group = adw::PreferencesGroup::new();
    group.set_title("Default game folder");

    let folder_row = adw::EntryRow::new();
    folder_row.set_title("Game folder");
    folder_row.set_text(&cfg.default_game_folder);

    let folder_browse = super::helpers::make_browse_button(
        Some(win),
        "Select default game folder",
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

pub(super) fn build_ra_settings_page(cfg: &Config) -> (gtk4::Box, adw::SwitchRow, adw::EntryRow, adw::EntryRow) {
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
    page.append(&creds_group);

    (page, enable_row, username_row, password_row.upcast())
}

pub(super) fn build_api_emulators_page(cfg: &Config) -> (gtk4::Box, adw::ComboRow, gtk4::StringList) {
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
    open_btn.add_css_class(CSS_FLAT);
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
    let mut all_versions: Vec<String> = gse_versions.iter()
        .chain(gog_versions.iter())
        .cloned()
        .collect();
    all_versions.sort();
    all_versions.dedup();

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

pub(super) struct OverlayPageWidgets {
    pub enable_row: adw::SwitchRow,
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
    enable_row.set_title("Enable in-game overlay");
    enable_row.set_subtitle("Shows achievements, screenshots, and recording during gameplay (Vulkan games only)");
    enable_row.set_active(o.enabled);
    enable_group.add(&enable_row);
    page.append(&enable_group);

    let recording_group = adw::PreferencesGroup::new();
    recording_group.set_title("Recording");

    let encoder_model = gtk4::StringList::new(&["Auto", "VAAPI (AMD/Intel)", "NVENC (NVIDIA)", "Software (CPU)"]);
    let encoder_row = adw::ComboRow::new();
    encoder_row.set_title("Video encoder");
    encoder_row.set_subtitle("Auto detects the best available encoder. Use Software if your GPU lacks hardware encoding.");
    encoder_row.set_model(Some(&encoder_model));
    encoder_row.set_selected(o.encoder as u32);
    recording_group.add(&encoder_row);

    let quality_model = gtk4::StringList::new(&["Low (720p 30fps)", "Medium (1080p 30fps)", "High (1080p 60fps)"]);
    let quality_row = adw::ComboRow::new();
    quality_row.set_title("Recording quality");
    quality_row.set_model(Some(&quality_model));
    quality_row.set_selected(o.recording_quality.as_u32());
    recording_group.add(&quality_row);
    page.append(&recording_group);

    let hotkeys_group = adw::PreferencesGroup::new();
    hotkeys_group.set_title("Hotkeys");
    hotkeys_group.set_description(Some("Keyboard and gamepad bindings. Click to set."));

    let toggle_hotkey = super::hotkey_widget::build_hotkey_row(
        &hotkeys_group, "Toggle overlay",
        &o.toggle_hotkey, &o.toggle_hotkey_gamepad,
        "Shift+Tab", "Guide",
    );
    let screenshot_hotkey = super::hotkey_widget::build_hotkey_row(
        &hotkeys_group, "Screenshot",
        &o.screenshot_hotkey, &o.screenshot_hotkey_gamepad,
        "F12", "Guide+DpadDown",
    );
    let record_hotkey = super::hotkey_widget::build_hotkey_row(
        &hotkeys_group, "Toggle recording",
        &o.record_hotkey, &o.record_hotkey_gamepad,
        "F11", "Guide+DpadUp",
    );
    page.append(&hotkeys_group);

    let font_group = adw::PreferencesGroup::new();
    font_group.set_title("Appearance");

    let font_dialog = gtk4::FontDialog::new();
    font_dialog.set_title("Select overlay font");

    let font_button = gtk4::FontDialogButton::new(Some(font_dialog));
    font_button.set_valign(gtk4::Align::Center);
    font_button.set_level(gtk4::FontLevel::Family);
    font_button.set_use_font(false);

    let system_font = detect_system_font().unwrap_or_else(|| "Sans".to_string());
    if let Some(ref family) = o.font_family {
        let desc = pango::FontDescription::from_string(family);
        font_button.set_font_desc(&desc);
    } else {
        let desc = pango::FontDescription::from_string(&system_font);
        font_button.set_font_desc(&desc);
    }

    let font_reset = gtk4::Button::from_icon_name("edit-undo-symbolic");
    font_reset.set_valign(gtk4::Align::Center);
    font_reset.set_tooltip_text(Some("Reset to system default"));
    font_reset.add_css_class("flat");
    font_reset.set_visible(o.font_family.is_some());

    let font_row = adw::ActionRow::new();
    font_row.set_title("Font family");
    font_row.add_suffix(&font_button);
    font_row.add_suffix(&font_reset);
    font_group.add(&font_row);
    page.append(&font_group);

    // Store font_reset in the widgets so the save handler can access it
    font_reset.connect_clicked({
        let fb = font_button.clone();
        let desc = pango::FontDescription::from_string(&system_font);
        move |_| {
            fb.set_font_desc(&desc);
        }
    });

    (
        page,
        OverlayPageWidgets { enable_row, encoder_row, quality_row, toggle_hotkey, screenshot_hotkey, record_hotkey, font_button },
    )
}
