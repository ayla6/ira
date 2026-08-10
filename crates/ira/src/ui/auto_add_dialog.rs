use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;

use adw::prelude::*;

use ira_models::{GameKind, GameLaunchConfig, GameVariant, TrophySource, WineConfig, WineProfile};

use super::add_game_db::{add_game_to_db, AddGameToDbParams};
use super::css::*;
use super::edit_game_dialog::show_edit_game_dialog;
use super::state::SharedState;
use super::steam_search_dialog::{
    show_search_results_dialog, SearchResultsDialogParams, SearchSource,
};
use super::wine_profile_picker::{build_wine_profile_picker, selected_profile_id};
use crate::AppMessage;

/// Which API emulator to install (GOG checked first — a GOG game may ship Steam
/// DLLs that must stay default, so only the Galaxy ones get patched).
#[derive(Clone, Copy)]
pub(super) enum EmuKind {
    Nge,
    Gse,
}

/// Events sent from background threads to the wizard (polled on the main thread).
pub(super) enum WizardEvent {
    Status(String),
    AlreadyExists,
    Identified(Box<IdentifiedGame>),
    Failed(String),
    Added(i64),
    EmulatorPrompt {
        db_id: i64,
        game_folder: PathBuf,
        app_id: String,
        emu_kind: EmuKind,
    },
    InstallDone,
    /// Automatic identification found no Steam game; ask the user to search.
    NeedSteamSearch {
        folder: PathBuf,
    },
}

pub(super) struct IdentifiedGame {
    pub app_id: String,
    pub name: String,
    pub is_windows: bool,
    pub game_folder: PathBuf,
    pub exe: String,
    pub variants: Vec<String>,
    pub logo_position: String,
    pub logo_size: i32,
}

/// Wizard state shared between the main-thread poll closure and signal handlers.
pub(super) struct Wizard {
    pub win: adw::Window,
    pub content: gtk4::Box,
    pub state: SharedState,
    pub profiles: Vec<WineProfile>,
    pub identified: Option<IdentifiedGame>,
    pub profile_row: Option<adw::ComboRow>,
    pub last_folder: Option<PathBuf>,
    pub last_is_windows: bool,
}

pub fn show_auto_add_dialog(state: &SharedState) {
    let parent = state.borrow().window.clone();
    let win = adw::Window::new();
    win.set_title(Some("Auto add game"));
    win.set_default_size(480, 420);
    win.set_modal(false);
    win.set_transient_for(Some(&parent));

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    header.add_css_class(CSS_FLAT);
    content.append(&header);
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.append(&page);
    win.set_content(Some(&content));
    win.present();

    let wizard = Rc::new(RefCell::new(Wizard {
        win: win.clone(),
        content: page.clone(),
        state: state.clone(),
        profiles: ira_db::get_all_profiles(&state.borrow().db).unwrap_or_default(),
        identified: None,
        profile_row: None,
        last_folder: None,
        last_is_windows: false,
    }));

    show_pick_page(&wizard);

    let win_close = win.clone();
    win.connect_close_request(move |_| {
        let _ = win_close;
        glib::Propagation::Proceed
    });
}

fn show_pick_page(wizard: &Rc<RefCell<Wizard>>) {
    let (state, win, content) = {
        let w = wizard.borrow();
        (w.state.clone(), w.win.clone(), w.content.clone())
    };
    clear_children(&content);

    let status = adw::StatusPage::new();
    status.set_title("Auto add game");
    status.set_description(Some("Pick the game's install folder. Ira will identify it, download assets and set everything up."));
    status.set_icon_name(Some("folder-open-symbolic"));

    let pick_btn = gtk4::Button::with_label("Pick game folder…");
    pick_btn.add_css_class(CSS_SUGGESTED_ACTION);
    pick_btn.set_halign(gtk4::Align::Center);

    let wizard_c = wizard.clone();
    pick_btn.connect_clicked(move |_| {
        pick_folder_and_start(&win, &state, &wizard_c);
    });
    status.set_child(Some(&pick_btn));
    content.append(&status);
}

fn pick_folder_and_start(win: &adw::Window, state: &SharedState, wizard: &Rc<RefCell<Wizard>>) {
    let default_folder = state.borrow().cfg.default_game_folder.clone();
    let dialog = gtk4::FileDialog::new();
    dialog.set_title("Select game folder");
    super::helpers::set_initial_folder(&dialog, &default_folder);
    let state_c = state.clone();
    let win_c = win.clone();
    let wizard_c = wizard.clone();
    dialog.select_folder(Some(win), None::<&gtk4::gio::Cancellable>, move |result| {
        if let Ok(file) = result {
            if let Some(path) = file.path() {
                on_folder_picked(&path, &state_c, &win_c, &wizard_c);
            }
        }
    });
}

fn on_folder_picked(
    path: &Path,
    state: &SharedState,
    win: &adw::Window,
    wizard: &Rc<RefCell<Wizard>>,
) {
    let default_game_folder = state.borrow().cfg.default_game_folder.clone();

    let inside_games = !default_game_folder.is_empty() && path.starts_with(&default_game_folder);
    let basename = path.file_name().and_then(|n| n.to_str()).unwrap_or("game");

    if inside_games {
        start_identify(path.to_path_buf(), None, wizard);
        return;
    }

    if default_game_folder.is_empty() {
        start_identify(path.to_path_buf(), None, wizard);
        return;
    }

    let dest = Path::new(&default_game_folder).join(basename);
    let picked = path.to_path_buf();
    let alert = adw::AlertDialog::new(
        Some("Move to games folder?"),
        Some("This folder is outside your PC games folder. Move it there now?"),
    );
    alert.add_response("no", "No");
    alert.add_response("yes", "Yes");
    alert.set_response_appearance("yes", adw::ResponseAppearance::Suggested);
    alert.set_default_response(Some("yes"));
    alert.set_close_response("no");

    let wizard_c = wizard.clone();
    alert.choose(
        Some(win),
        None::<&gtk4::gio::Cancellable>,
        move |response| {
            let move_to = if response == "yes" {
                Some(dest.clone())
            } else {
                None
            };
            start_identify(picked.clone(), move_to, &wizard_c);
        },
    );
}

pub(super) fn start_identify(
    path: PathBuf,
    move_to: Option<PathBuf>,
    wizard: &Rc<RefCell<Wizard>>,
) {
    let (db, steam) = {
        let w = wizard.borrow();
        let s = w.state.borrow();
        (s.db.clone(), s.steam.clone())
    };

    set_status(wizard, "Identifying game…");

    let (tx, rx) = mpsc::channel::<WizardEvent>();
    let rx = Rc::new(RefCell::new(rx));
    spawn_identify_thread(tx, path, move_to, db, steam);

    poll_events(wizard, rx);
}

pub(super) fn poll_events(
    wizard: &Rc<RefCell<Wizard>>,
    rx: Rc<RefCell<mpsc::Receiver<WizardEvent>>>,
) {
    let wizard_c = wizard.clone();
    glib::source::idle_add_local_full(glib::Priority::LOW, move || {
        match rx.borrow_mut().try_recv() {
            Ok(ev) => {
                let terminal = !matches!(ev, WizardEvent::Status(_));
                handle_identify_event(&wizard_c, ev);
                if terminal {
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

/// Resume identification from a user-chosen Steam app ID, used when
/// automatic identification found nothing and the user picked a game
/// via the Steam search fallback.
pub(super) fn continue_identify(folder: PathBuf, app_id: String, wizard: &Rc<RefCell<Wizard>>) {
    let steam = wizard.borrow().state.borrow().steam.clone();
    set_status(wizard, "Identifying game…");

    let (tx, rx) = mpsc::channel::<WizardEvent>();
    let rx = Rc::new(RefCell::new(rx));
    std::thread::spawn(move || finish_identify(tx, folder, app_id, steam));

    poll_events(wizard, rx);
}

pub(super) fn spawn_identify_thread(
    tx: mpsc::Sender<WizardEvent>,
    path: PathBuf,
    move_to: Option<PathBuf>,
    db: ira_db::DbConn,
    steam: std::sync::Arc<ira_api::SteamDataClient>,
) {
    std::thread::spawn(move || {
        let final_folder = match resolve_final_folder(&path, move_to, &tx) {
            Some(f) => f,
            None => return,
        };

        if let Ok(Some(_)) = ira_db::find_by_game_folder(&db, &final_folder.to_string_lossy()) {
            let _ = tx.send(WizardEvent::AlreadyExists);
            return;
        }

        let app_id = match identify_app_id(&final_folder, &steam) {
            Some(id) => id,
            None => {
                let _ = tx.send(WizardEvent::NeedSteamSearch {
                    folder: final_folder,
                });
                return;
            }
        };

        finish_identify(tx, final_folder, app_id, steam);
    });
}

fn finish_identify(
    tx: mpsc::Sender<WizardEvent>,
    folder: PathBuf,
    app_id: String,
    steam: std::sync::Arc<ira_api::SteamDataClient>,
) {
    let info = match steam.fetch_steamcmd_info(&app_id) {
        Some(i) => i,
        None => {
            let _ = tx.send(WizardEvent::Failed(format!(
                "Failed to fetch Steam data for app {}.",
                app_id
            )));
            return;
        }
    };

    let launches = info.launches;
    let default = launches.first();
    let exe = default.map(|l| l.executable.clone()).unwrap_or_default();
    let is_windows = default
        .map(|l| l.oslist.contains("windows") || l.oslist.is_empty())
        .unwrap_or(info.oslist.contains("windows") || info.oslist.is_empty());

    let target_os = if is_windows { "windows" } else { "linux" };
    let variants: Vec<String> = launches
        .iter()
        .skip(1)
        .filter(|l| l.oslist.contains(target_os) || l.oslist.is_empty())
        .filter(|l| !l.oslist.contains("macos"))
        .map(|l| l.executable.clone())
        .collect();

    let _ = tx.send(WizardEvent::Identified(Box::new(IdentifiedGame {
        app_id,
        name: info.name,
        is_windows,
        game_folder: folder,
        exe,
        variants,
        logo_position: info.logo_position,
        logo_size: info.logo_size,
    })));
}

fn resolve_final_folder(
    path: &Path,
    move_to: Option<PathBuf>,
    tx: &mpsc::Sender<WizardEvent>,
) -> Option<PathBuf> {
    let Some(dest) = move_to else {
        return Some(path.to_path_buf());
    };
    let _ = tx.send(WizardEvent::Status("Moving folder…".to_string()));
    match move_dir(path, &dest) {
        Ok(()) => Some(dest),
        Err(e) => {
            let _ = tx.send(WizardEvent::Failed(format!("Failed to move folder: {}", e)));
            None
        }
    }
}

pub(super) fn identify_app_id(folder: &Path, steam: &ira_api::SteamDataClient) -> Option<String> {
    if let Some(steamapps) = ira_platforms::steam::steamapps_in_path(folder) {
        let installdir = folder.file_name()?.to_string_lossy().to_string();
        if let Some((appid, _name)) =
            ira_platforms::steam::find_appid_for_installdir(&steamapps, &installdir)
        {
            return Some(appid);
        }
    }
    let basename = folder.file_name()?.to_string_lossy().to_string();
    let results = steam.search_steam_store(&basename);
    results.into_iter().next().map(|(id, _)| id)
}

pub(super) fn handle_identify_event(wizard: &Rc<RefCell<Wizard>>, ev: WizardEvent) {
    match ev {
        WizardEvent::Status(msg) => set_status(wizard, &msg),
        WizardEvent::AlreadyExists => {
            show_error(
                wizard,
                "This folder is already in your library. Pick another one.",
            );
            show_pick_page(wizard);
        }
        WizardEvent::Failed(e) => {
            show_error(wizard, &e);
            show_pick_page(wizard);
        }
        WizardEvent::Identified(game) => show_identified_form(wizard, *game, None, false),
        WizardEvent::NeedSteamSearch { folder } => show_steam_search_page(wizard, folder),
        // Add-phase events are handled by handle_add_event.
        WizardEvent::Added(_) | WizardEvent::EmulatorPrompt { .. } | WizardEvent::InstallDone => {}
    }
}

/// Fallback shown when automatic identification finds no Steam game: let the
/// user search Steam manually, or fall back to setting the game up by hand.
fn show_steam_search_page(wizard: &Rc<RefCell<Wizard>>, folder: PathBuf) {
    let (content, win, state, steam) = {
        let w = wizard.borrow();
        let s = w.state.borrow();
        (
            w.content.clone(),
            w.win.clone(),
            w.state.clone(),
            s.steam.clone(),
        )
    };
    clear_children(&content);

    let title = gtk4::Label::new(Some("Couldn't identify this game."));
    title.add_css_class(CSS_TITLE_1);
    title.set_halign(gtk4::Align::Center);
    content.append(&title);

    let hint = gtk4::Label::new(Some("Search Steam for the game, or set it up manually."));
    hint.add_css_class(CSS_DIM_LABEL);
    hint.set_halign(gtk4::Align::Center);
    content.append(&hint);

    let name = folder
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("game")
        .to_string();

    let search_btn = gtk4::Button::with_label("Search Steam…");
    search_btn.add_css_class(CSS_SUGGESTED_ACTION);
    search_btn.set_halign(gtk4::Align::Center);
    let state_c = state.clone();
    let steam_c = steam.clone();
    let win_c = win.clone();
    let wizard_c = wizard.clone();
    let folder_c = folder.clone();
    let name_c = name.clone();
    search_btn.connect_clicked(move |_| {
        show_search_results_dialog(SearchResultsDialogParams {
            state: &state_c,
            steam: steam_c.clone(),
            source_name: "Steam",
            game_name: &name_c,
            db_id: 0,
            source: SearchSource::Steam,
            on_match: {
                let wizard_c = wizard_c.clone();
                let folder_c = folder_c.clone();
                Rc::new(move |app_id: &str, _matched_name: &str| {
                    continue_identify(folder_c.clone(), app_id.to_string(), &wizard_c);
                })
            },
            parent: win_c.upcast_ref(),
            match_in_db: false,
        });
    });
    content.append(&search_btn);

    let manual_btn = gtk4::Button::with_label("Set up manually");
    manual_btn.set_halign(gtk4::Align::Center);
    let wizard_c = wizard.clone();
    let folder_c = folder.clone();
    let name_c = name.clone();
    manual_btn.connect_clicked(move |_| {
        show_identified_form(
            &wizard_c,
            IdentifiedGame {
                app_id: String::new(),
                name: name_c.clone(),
                is_windows: true,
                game_folder: folder_c.clone(),
                exe: String::new(),
                variants: Vec::new(),
                logo_position: String::new(),
                logo_size: 0,
            },
            None,
            false,
        );
    });
    content.append(&manual_btn);
}

pub(super) fn show_identified_form(
    wizard: &Rc<RefCell<Wizard>>,
    game: IdentifiedGame,
    preselected_profile_id: Option<i64>,
    skip_emu_prompt: bool,
) {
    let (content, win, state, profiles) = {
        let w = wizard.borrow();
        (
            w.content.clone(),
            w.win.clone(),
            w.state.clone(),
            w.profiles.clone(),
        )
    };
    clear_children(&content);

    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    body.set_margin_start(16);
    body.set_margin_end(16);
    body.set_margin_top(8);
    body.set_margin_bottom(16);

    let is_windows = game.is_windows;
    let group = adw::PreferencesGroup::new();
    group.set_title("Confirm game");

    let name_entry = adw::EntryRow::new();
    name_entry.set_title("Name");
    name_entry.set_text(&game.name);
    group.add(&name_entry);

    let appid_row = adw::EntryRow::new();
    appid_row.set_title("Steam app ID");
    appid_row.set_text(&game.app_id);
    appid_row.set_sensitive(false);
    group.add(&appid_row);

    let profile_row = if is_windows {
        let row = build_wine_profile_picker(&profiles, preselected_profile_id, None, &state, &win);
        group.add(&row);
        Some(row)
    } else {
        None
    };

    body.append(&group);

    let info_label = gtk4::Label::new(Some(if is_windows {
        "Detected: Windows game — a Wine profile is recommended."
    } else {
        "Detected: Linux native game."
    }));
    info_label.set_halign(gtk4::Align::Start);
    info_label.add_css_class(CSS_DIM_LABEL);
    body.append(&info_label);

    let add_btn = gtk4::Button::with_label("Add game");
    add_btn.add_css_class(CSS_SUGGESTED_ACTION);
    add_btn.set_halign(gtk4::Align::Center);

    {
        let folder = game.game_folder.clone();
        let is_windows = game.is_windows;
        let mut w = wizard.borrow_mut();
        w.identified = Some(game);
        w.profile_row = profile_row.clone();
        w.last_folder = Some(folder);
        w.last_is_windows = is_windows;
    }
    let name_c = name_entry.clone();
    let appid_c = appid_row.clone();
    let wizard_c = wizard.clone();
    add_btn.connect_clicked(move |_| {
        let extracted = {
            let mut w = wizard_c.borrow_mut();
            w.identified.take().map(|game| {
                let name = name_c.text().to_string();
                let app_id = appid_c.text().to_string();
                let profile_id = w
                    .profile_row
                    .as_ref()
                    .and_then(|r| selected_profile_id(r, &w.profiles));
                (game, name, app_id, profile_id)
            })
        };
        if let Some((game, name, app_id, profile_id)) = extracted {
            start_add(
                wizard_c.clone(),
                game,
                name,
                app_id,
                profile_id,
                skip_emu_prompt,
            );
        }
    });
    body.append(&add_btn);
    content.append(&body);
}

pub(super) fn start_add(
    wizard: Rc<RefCell<Wizard>>,
    game: IdentifiedGame,
    name: String,
    app_id: String,
    profile_id: Option<i64>,
    skip_emu_prompt: bool,
) {
    let (db, steam, save_dir, sender, profiles, language_preferences) = {
        let w = wizard.borrow();
        let s = w.state.borrow();
        (
            s.db.clone(),
            s.steam.clone(),
            s.save_dir.clone(),
            s.sender.clone(),
            w.profiles.clone(),
            s.cfg.language_preferences.clone(),
        )
    };
    set_status(&wizard, "Adding game and downloading assets…");

    let (tx, rx) = mpsc::channel::<WizardEvent>();
    let rx = Rc::new(RefCell::new(rx));
    spawn_add_thread(
        tx,
        AddParams {
            db,
            steam,
            save_dir,
            sender,
            game,
            name,
            app_id,
            profile_id,
            profiles,
            skip_emu_prompt,
            language_preferences,
        },
    );

    let wizard_c = wizard.clone();
    glib::source::idle_add_local_full(glib::Priority::LOW, move || {
        match rx.borrow_mut().try_recv() {
            Ok(ev) => {
                let terminal = !matches!(ev, WizardEvent::Status(_));
                handle_add_event(&wizard_c, ev);
                if terminal {
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

pub(super) struct AddParams {
    pub db: ira_db::DbConn,
    pub steam: std::sync::Arc<ira_api::SteamDataClient>,
    pub save_dir: String,
    pub sender: crate::AppSender,
    pub game: IdentifiedGame,
    pub name: String,
    pub app_id: String,
    pub profile_id: Option<i64>,
    pub profiles: Vec<WineProfile>,
    pub skip_emu_prompt: bool,
    pub language_preferences: Vec<String>,
}

pub(super) fn spawn_add_thread(tx: mpsc::Sender<WizardEvent>, params: AddParams) {
    std::thread::spawn(move || {
        let AddParams {
            db,
            steam,
            save_dir,
            sender,
            game,
            name,
            app_id,
            profile_id,
            profiles,
            skip_emu_prompt,
            language_preferences,
        } = params;
        let kind = if game.is_windows {
            GameKind::Wine
        } else {
            GameKind::Linux
        };
        let exe_path = if game.exe.is_empty() {
            String::new()
        } else {
            game.game_folder
                .join(&game.exe)
                .to_string_lossy()
                .into_owned()
        };
        let launch_config = GameLaunchConfig {
            exe: exe_path,
            working_dir: game.game_folder.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let wine_config = if game.is_windows {
            resolve_wine_config(&profiles, profile_id)
        } else {
            WineConfig::default()
        };

        let result = add_game_to_db(AddGameToDbParams {
            db: &db,
            name: &name,
            kind,
            trophy_source: TrophySource::Gse,
            app_id: &app_id,
            platform_id: &app_id,
            game_folder: &game.game_folder.to_string_lossy(),
            launch_config: &launch_config,
            wine_config: &wine_config,
            profile_id,
            steam: &steam,
            save_dir: &save_dir,
        });

        let db_id = match result {
            Ok(id) => id,
            Err(e) => {
                let _ = tx.send(WizardEvent::Failed(e));
                return;
            }
        };

        if !game.logo_position.is_empty() {
            let _ = ira_db::set_logo_settings(&db, db_id, &game.logo_position, game.logo_size);
        }

        for (i, variant_exe) in game.variants.iter().enumerate() {
            let exe_path = game
                .game_folder
                .join(variant_exe)
                .to_string_lossy()
                .into_owned();
            let variant = GameVariant {
                game_id: db_id,
                name: format!("Launch {}", i + 2),
                exe: exe_path,
                working_dir: game.game_folder.to_string_lossy().into_owned(),
                show_as_entry: false,
                ..Default::default()
            };
            if let Err(e) = ira_db::add_variant(&db, &variant) {
                eprintln!("Failed to add variant: {}", e);
            }
        }

        let entry = match ira_db::find_by_db_id(&db, db_id).ok().flatten() {
            Some(e) => e,
            None => {
                let _ = tx.send(WizardEvent::Failed(
                    "Failed to reload game after add.".to_string(),
                ));
                return;
            }
        };
        let mut game_obj = match crate::game_loader::load_game(&entry, &save_dir) {
            Ok(g) => g,
            Err(e) => {
                let _ = tx.send(WizardEvent::Failed(e));
                return;
            }
        };
        game_obj.set_name(&name);
        game_obj.game_path = launch_config.exe.clone();
        let _ = ira_db::update_game_title(&db, game_obj.db_id, &name);
        let _ = sender.send(AppMessage::NewGame(game_obj.clone()));

        let save_dir_for_lang = save_dir.clone();
        let db_for_cache = db.clone();
        crate::ui::enrichment::enrich_game_blocking(crate::ui::enrichment::EnrichGameParams {
            app_id: game_obj.app_id.clone(),
            trophy_source: game_obj.trophy_source,
            platform_id: game_obj.platform_id.clone(),
            db_id: game_obj.db_id,
            title: name.clone(),
            steam,
            sender,
            db,
            save_dir,
            game: None,
            ra_username: String::new(),
            ra_token: String::new(),
            ra_password: String::new(),
        });

        // After enrichment, pick a default language from the user's preferences.
        let game_exe = game.game_folder.join(&game.exe);
        let game_exe_str = game_exe.to_string_lossy().to_string();
        if !language_preferences.is_empty() {
            let appdetails_path =
                ira_parser::data_dir(&save_dir_for_lang, &app_id).join("appdetails.json");
            if let Ok(content) = std::fs::read_to_string(&appdetails_path) {
                if let Ok(details) = serde_json::from_str::<ira_models::AppDetails>(&content) {
                    let chosen = language_preferences
                        .iter()
                        .find(|pref| details.languages.iter().any(|l| l == *pref))
                        .or_else(|| details.languages.iter().find(|l| **l == "english"))
                        .or_else(|| details.languages.first());
                    if let Some(lang) = chosen {
                        ira_platforms::api_emulators::write_language_configs(
                            game_obj.trophy_source,
                            &game_exe_str,
                            &save_dir_for_lang,
                            &app_id,
                            lang,
                        );
                    }
                }
            }
        }

        if skip_emu_prompt {
            let _ = tx.send(WizardEvent::Added(db_id));
            return;
        }

        let game_folder_str = game.game_folder.to_string_lossy().into_owned();

        // Centralize any steam_settings/ or ngalaxye_settings/ found in
        // subdirectories to the game root, with symlinks from DLL dirs.
        if let Err(e) = ira_platforms::api_emulators::centralize_steam_settings(&game_folder_str) {
            eprintln!("Failed to centralize steam_settings: {}", e);
        }
        if let Err(e) = ira_platforms::api_emulators::centralize_galaxy_settings(&game_folder_str) {
            eprintln!("Failed to centralize ngalaxye_settings: {}", e);
        }

        // One-time migration of existing emulator saves to centralized path.
        // Check both GBE and NGE default locations since the auto-add flow
        // may detect GOG DLLs even though trophy_source is Gse.
        let wine_prefix = if game.is_windows {
            Some(ira_launcher::wine_launch::wine_prefix(&wine_config))
        } else {
            None
        };
        ira_platforms::emulator_save_migration::migrate_gbe_saves(
            &save_dir_for_lang,
            &app_id,
            wine_prefix.as_deref(),
        );
        ira_platforms::emulator_save_migration::migrate_nge_saves(
            &save_dir_for_lang,
            wine_prefix.as_deref(),
        );

        // Centralize game saves if UFS data is available
        if let Some(details) = crate::game_loader::read_app_details(&save_dir_for_lang, &app_id) {
            if !details.ufs_savefiles.is_empty() {
                let count = ira_launcher::game_saves::setup_game_saves(
                    &details.ufs_savefiles,
                    &details.ufs_rootoverrides,
                    &app_id,
                    &save_dir_for_lang,
                    wine_prefix.as_deref(),
                );
                if count > 0 {
                    if let Err(e) = ira_db::set_saves_centralized(&db_for_cache, db_id, true) {
                        eprintln!("Failed to cache saves centralized: {}", e);
                    }
                }
            }
        }

        let needs_nge = ira_platforms::api_emulators::find_gog_dlls_recursive(&game_folder_str)
            .iter()
            .any(|d| !ira_platforms::api_emulators::has_gog_emulator_backups(d));
        let needs_gse = !needs_nge
            && ira_platforms::api_emulators::find_steam_dlls_recursive(&game_folder_str)
                .iter()
                .any(|d| {
                    !ira_platforms::api_emulators::has_steam_emulator_backups(d)
                        && !d.join("steam_settings").is_dir()
                });

        if needs_nge {
            let _ = tx.send(WizardEvent::EmulatorPrompt {
                db_id,
                game_folder: game.game_folder.clone(),
                app_id: app_id.clone(),
                emu_kind: EmuKind::Nge,
            });
        } else if needs_gse {
            let _ = tx.send(WizardEvent::EmulatorPrompt {
                db_id,
                game_folder: game.game_folder.clone(),
                app_id: app_id.clone(),
                emu_kind: EmuKind::Gse,
            });
        } else {
            let _ = tx.send(WizardEvent::Added(db_id));
        }
    });
}

pub(super) fn handle_add_event(wizard: &Rc<RefCell<Wizard>>, ev: WizardEvent) {
    match ev {
        WizardEvent::Added(db_id) => finalize(wizard, db_id),
        WizardEvent::EmulatorPrompt {
            db_id,
            game_folder,
            app_id,
            emu_kind,
        } => {
            prompt_install_emulator(wizard, db_id, game_folder, app_id, emu_kind);
        }
        WizardEvent::InstallDone => {}
        WizardEvent::Failed(e) => {
            show_error(wizard, &e);
            show_pick_page(wizard);
        }
        WizardEvent::Status(msg) => set_status(wizard, &msg),
        _ => {}
    }
}

fn prompt_install_emulator(
    wizard: &Rc<RefCell<Wizard>>,
    db_id: i64,
    game_folder: PathBuf,
    app_id: String,
    emu_kind: EmuKind,
) {
    // Honor a remembered choice so the user isn't asked every time.
    let remembered = wizard.borrow().state.borrow().cfg.auto_emu_install;
    match remembered {
        Some(true) => {
            let version = wizard
                .borrow()
                .state
                .borrow()
                .cfg
                .default_api_emu_version
                .clone();
            start_install(
                wizard.clone(),
                game_folder,
                app_id,
                version,
                db_id,
                emu_kind,
            );
            return;
        }
        Some(false) => {
            finalize(wizard, db_id);
            return;
        }
        None => {}
    }

    let (win, default_version) = {
        let w = wizard.borrow();
        let default_version = w.state.borrow().cfg.default_api_emu_version.clone();
        (w.win.clone(), default_version)
    };
    let (title, body) = match emu_kind {
        EmuKind::Nge => (
            "Install Nemirtingas Galaxy emulator?",
            "GOG Galaxy DLLs were found in this game. Install the Nemirtingas Galaxy Emulator to enable achievements? (Steam DLLs, if any, will be left untouched.)",
        ),
        EmuKind::Gse => (
            "Install Goldberg emulator?",
            "Steam API DLLs were found in this game. Install the Goldberg Steam Emulator to enable achievements?",
        ),
    };

    let dialog = adw::Window::new();
    dialog.set_title(Some(title));
    dialog.set_default_size(380, 240);
    dialog.set_modal(true);
    dialog.set_transient_for(Some(&win));

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    outer.set_margin_start(20);
    outer.set_margin_end(20);
    outer.set_margin_top(16);
    outer.set_margin_bottom(16);

    let header = adw::HeaderBar::new();
    header.add_css_class(CSS_FLAT);
    outer.append(&header);

    let msg = gtk4::Label::new(Some(body));
    msg.set_wrap(true);
    msg.set_halign(gtk4::Align::Start);
    outer.append(&msg);

    let remember = adw::SwitchRow::new();
    remember.set_title("Don't ask me again");
    let group = adw::PreferencesGroup::new();
    group.add(&remember);
    outer.append(&group);

    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);
    let no_btn = gtk4::Button::with_label("No");
    let yes_btn = gtk4::Button::with_label("Yes");
    yes_btn.add_css_class(CSS_SUGGESTED_ACTION);
    btn_row.append(&no_btn);
    btn_row.append(&yes_btn);
    outer.append(&btn_row);

    dialog.set_content(Some(&outer));
    dialog.present();

    let wizard_c = wizard.clone();
    let remember_c = remember.clone();
    let dialog_c = dialog.clone();
    yes_btn.connect_clicked(move |_| {
        persist_remember(&wizard_c, remember_c.is_active(), true);
        let version = wizard_c
            .borrow()
            .state
            .borrow()
            .cfg
            .default_api_emu_version
            .clone();
        dialog_c.close();
        start_install(
            wizard_c.clone(),
            game_folder.clone(),
            app_id.clone(),
            version,
            db_id,
            emu_kind,
        );
    });

    let wizard_c2 = wizard.clone();
    let remember_c2 = remember.clone();
    let dialog_c2 = dialog.clone();
    no_btn.connect_clicked(move |_| {
        persist_remember(&wizard_c2, remember_c2.is_active(), false);
        dialog_c2.close();
        finalize(&wizard_c2, db_id);
    });
    let _ = default_version;
}

fn persist_remember(wizard: &Rc<RefCell<Wizard>>, remember: bool, install: bool) {
    if remember {
        let wizard_ref = wizard.borrow_mut();
        let mut state_ref = wizard_ref.state.borrow_mut();
        state_ref.cfg.auto_emu_install = Some(install);
        if let Err(e) = state_ref.cfg.save() {
            eprintln!("Failed to save auto_emu_install preference: {}", e);
        }
    }
}

fn start_install(
    wizard: Rc<RefCell<Wizard>>,
    game_folder: PathBuf,
    app_id: String,
    version: String,
    db_id: i64,
    emu_kind: EmuKind,
) {
    let save_dir = wizard.borrow().state.borrow().save_dir.clone();
    let status = match emu_kind {
        EmuKind::Nge => "Installing Nemirtingas Galaxy emulator…",
        EmuKind::Gse => "Installing Goldberg emulator…",
    };
    set_status(&wizard, status);

    let (tx, rx) = mpsc::channel::<WizardEvent>();
    let tx_c = tx.clone();
    let game_folder_c = game_folder.clone();
    let app_id_c = app_id.clone();
    let version_c = version.clone();
    std::thread::spawn(move || {
        let result = match emu_kind {
            EmuKind::Nge => ira_platforms::api_emulators::install_nge_from_folder(
                &save_dir,
                &game_folder_c.to_string_lossy(),
                &app_id_c,
                &version_c,
            ),
            EmuKind::Gse => ira_platforms::api_emulators::install_gse_from_folder(
                &save_dir,
                &game_folder_c.to_string_lossy(),
                &app_id_c,
                &[],
                &version_c,
            ),
        };
        if let Err(e) = result {
            eprintln!("Emulator install failed: {}", e);
        }
        let _ = tx_c.send(WizardEvent::InstallDone);
    });

    let wizard_c = wizard.clone();
    glib::source::idle_add_local_full(glib::Priority::LOW, move || match rx.try_recv() {
        Ok(_) => {
            finalize(&wizard_c, db_id);
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
    });
}

pub(super) fn finalize(wizard: &Rc<RefCell<Wizard>>, db_id: i64) {
    let (folder, is_windows) = {
        let w = wizard.borrow();
        (w.last_folder.clone(), w.last_is_windows)
    };
    if is_windows {
        if let Some(folder) = &folder {
            if let Some(steamapps) = ira_platforms::steam::steamapps_in_path(folder) {
                let packages = ira_platforms::steam::detect_redists(&steamapps);
                if !packages.is_empty() {
                    prompt_redists(wizard, db_id, packages);
                    return;
                }
            }
            let local = ira_platforms::steam::detect_redists_in_game_folder(folder);
            if !local.is_empty() {
                prompt_redists(wizard, db_id, local);
                return;
            }
        }
    }
    close_and_open_edit(wizard, db_id);
}

pub(super) fn prompt_redists(
    wizard: &Rc<RefCell<Wizard>>,
    db_id: i64,
    packages: Vec<ira_platforms::steam::RedistPackage>,
) {
    let (win, state) = {
        let w = wizard.borrow();
        (w.win.clone(), w.state.clone())
    };
    let body =
        format!(
        "Steamworks redistributables were found:\n{}\n\nInstall the selected ones now via Wine?",
        packages.iter().map(|p| format!("- {}", p.name)).collect::<Vec<_>>().join("\n")
    );
    let alert = adw::AlertDialog::new(Some("Install redistributables?"), Some(&body));
    alert.add_response("skip", "Skip");
    alert.add_response("install", "Install");
    alert.set_response_appearance("install", adw::ResponseAppearance::Suggested);
    alert.set_default_response(Some("install"));
    alert.set_close_response("skip");

    let wizard_c = wizard.clone();
    alert.choose(
        Some(&win),
        None::<&gtk4::gio::Cancellable>,
        move |response| {
            if response == "install" {
                start_redist_install(wizard_c.clone(), db_id, packages);
            } else {
                close_and_open_edit(&wizard_c, db_id);
            }
            let _ = state;
        },
    );
}

pub(super) fn start_redist_install(
    wizard: Rc<RefCell<Wizard>>,
    db_id: i64,
    packages: Vec<ira_platforms::steam::RedistPackage>,
) {
    let (db, save_dir, game_folder) = {
        let w = wizard.borrow();
        let s = w.state.borrow();
        (s.db.clone(), s.save_dir.clone(), w.last_folder.clone())
    };
    set_status(&wizard, "Installing redistributables via Wine…");

    // Copy _CommonRedist into the game folder so redists persist across
    // prefix changes. Installer paths are remapped to the local copy.
    let packages = match game_folder.as_deref() {
        Some(folder) => ira_platforms::steam::localize_redists(folder, packages),
        None => packages,
    };

    let (tx, rx) = mpsc::channel::<WizardEvent>();
    let rx = Rc::new(RefCell::new(rx));
    std::thread::spawn(move || {
        let wine_config = ira_db::get_game_config(&db, db_id)
            .ok()
            .flatten()
            .map(|(_, wine, _)| wine)
            .unwrap_or_default();
        let wine_exe = ira_launcher::wine_launch::find_wine_binary(
            &wine_config.version,
            &wine_config.custom_wine_path,
        )
        .unwrap_or_else(|_| "wine".to_string());
        let env = ira_launcher::wine_launch::build_wine_env(&wine_config, &wine_exe);
        for package in &packages {
            for installer in &package.installers {
                eprintln!(
                    "Running redist installer: {} ({})",
                    package.name,
                    installer.display()
                );
                let mut cmd = std::process::Command::new(&wine_exe);
                cmd.arg(installer);
                for (k, v) in &env {
                    cmd.env(k, v);
                }
                match cmd.status() {
                    Ok(s) if !s.success() => eprintln!(
                        "Installer {} exited with {:?}",
                        installer.display(),
                        s.code()
                    ),
                    Err(e) => eprintln!("Failed to run {}: {}", installer.display(), e),
                    _ => {}
                }
            }
        }
        let _ = save_dir;
        let _ = tx.send(WizardEvent::InstallDone);
    });

    let wizard_c = wizard.clone();
    glib::source::idle_add_local_full(glib::Priority::LOW, move || {
        match rx.borrow_mut().try_recv() {
            Ok(_) => {
                close_and_open_edit(&wizard_c, db_id);
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

pub(super) fn close_and_open_edit(wizard: &Rc<RefCell<Wizard>>, db_id: i64) {
    let (state, win) = {
        let w = wizard.borrow();
        (w.state.clone(), w.win.clone())
    };
    win.close();
    show_edit_game_dialog(&state, db_id);
}

pub(super) fn resolve_wine_config(profiles: &[WineProfile], profile_id: Option<i64>) -> WineConfig {
    let mut wine = WineConfig {
        enabled: true,
        ..Default::default()
    };
    if let Some(pid) = profile_id {
        if let Some(profile) = profiles.iter().find(|p| p.id == pid) {
            wine.version = profile.wine_version.clone();
            wine.custom_wine_path = profile.custom_wine_path.clone();
            wine.prefix = profile.prefix.clone();
            wine.arch = profile.arch.clone();
            wine.umu_enabled = profile.umu_enabled;
        }
    }
    wine
}

pub(super) fn set_status(wizard: &Rc<RefCell<Wizard>>, msg: &str) {
    let content = wizard.borrow().content.clone();
    clear_children(&content);
    let status = adw::StatusPage::new();
    status.set_title("Auto add game");
    status.set_description(Some(msg));
    status.set_icon_name(Some("folder-open-symbolic"));
    let spinner = gtk4::Spinner::new();
    spinner.start();
    status.set_child(Some(&spinner));
    content.append(&status);
}

pub(super) fn show_error(wizard: &Rc<RefCell<Wizard>>, msg: &str) {
    let win = wizard.borrow().win.clone();
    let alert = adw::AlertDialog::new(Some("Auto-add failed"), Some(msg));
    alert.add_response("ok", "OK");
    alert.set_default_response(Some("ok"));
    alert.set_close_response("ok");
    alert.present(Some(&win));
}

pub(super) fn clear_children(container: &gtk4::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn move_dir(src: &Path, dst: &Path) -> Result<(), String> {
    if dst.exists() {
        return Err(format!("destination already exists: {}", dst.display()));
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    let output = std::process::Command::new("mv")
        .arg(src)
        .arg(dst)
        .output()
        .map_err(|e| format!("run mv: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "mv failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}
