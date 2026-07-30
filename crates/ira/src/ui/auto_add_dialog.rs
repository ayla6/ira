use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;

use adw::prelude::*;

use ira_models::{GameKind, GameLaunchConfig, GameVariant, TrophySource, WineConfig, WineProfile};

use crate::AppMessage;
use super::add_game_db::{add_game_to_db, AddGameToDbParams};
use super::css::*;
use super::edit_game_dialog::show_edit_game_dialog;
use super::state::SharedState;
use super::wine_profile_picker::{build_wine_profile_picker, selected_profile_id};

/// Events sent from background threads to the wizard (polled on the main thread).
enum WizardEvent {
    Status(String),
    AlreadyExists,
    Identified(Box<IdentifiedGame>),
    Failed(String),
    Added(i64),
}

struct IdentifiedGame {
    app_id: String,
    name: String,
    is_windows: bool,
    game_folder: PathBuf,
    exe: String,
    variants: Vec<String>,
}

/// Wizard state shared between the main-thread poll closure and signal handlers.
struct Wizard {
    win: adw::Window,
    content: gtk4::Box,
    state: SharedState,
    profiles: Vec<WineProfile>,
    identified: Option<IdentifiedGame>,
    profile_row: Option<adw::ComboRow>,
}

pub fn show_auto_add_dialog(state: &SharedState) {
    let parent = state.borrow().window.clone();
    let win = adw::Window::new();
    win.set_title(Some("Auto Add Game"));
    win.set_default_size(480, 420);
    win.set_modal(true);
    win.set_transient_for(Some(&parent));

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    win.set_content(Some(&content));
    win.present();

    let wizard = Rc::new(RefCell::new(Wizard {
        win: win.clone(),
        content: content.clone(),
        state: state.clone(),
        profiles: ira_db::get_all_profiles(&state.borrow().db).unwrap_or_default(),
        identified: None,
        profile_row: None,
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
    status.set_title("Auto Add Game");
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
    let dialog = gtk4::FileDialog::new();
    dialog.set_title("Select game folder");
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

fn on_folder_picked(path: &Path, state: &SharedState, win: &adw::Window, wizard: &Rc<RefCell<Wizard>>) {
    let default_game_folder = state.borrow().cfg.default_game_folder.clone();

    let inside_games = !default_game_folder.is_empty()
        && path.starts_with(&default_game_folder);
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
    alert.choose(Some(win), None::<&gtk4::gio::Cancellable>, move |response| {
        let move_to = if response == "yes" { Some(dest.clone()) } else { None };
        start_identify(picked.clone(), move_to, &wizard_c);
    });
}

fn start_identify(path: PathBuf, move_to: Option<PathBuf>, wizard: &Rc<RefCell<Wizard>>) {
    let (db, steam) = {
        let w = wizard.borrow();
        let s = w.state.borrow();
        (s.db.clone(), s.steam.clone())
    };

    set_status(wizard, "Identifying game…");

    let (tx, rx) = mpsc::channel::<WizardEvent>();
    let rx = Rc::new(RefCell::new(rx));
    spawn_identify_thread(tx, path, move_to, db, steam);

    let wizard_c = wizard.clone();
    glib::source::idle_add_local_full(glib::Priority::LOW, move || {
        match rx.borrow_mut().try_recv() {
            Ok(ev) => {
                handle_identify_event(&wizard_c, ev);
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

fn spawn_identify_thread(
    tx: mpsc::Sender<WizardEvent>, path: PathBuf, move_to: Option<PathBuf>,
    db: ira_db::DbConn, steam: std::sync::Arc<ira_api::SteamDataClient>,
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
                let _ = tx.send(WizardEvent::Failed(
                    "Could not identify this game on Steam. Auto-add only works for Steam games.".to_string(),
                ));
                return;
            }
        };

        let info = match steam.fetch_steamcmd_info(&app_id) {
            Some(i) => i,
            None => {
                let _ = tx.send(WizardEvent::Failed(format!(
                    "Failed to fetch Steam data for app {}.", app_id
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
        let variants: Vec<String> = launches.iter().skip(1).map(|l| l.executable.clone()).collect();

        let _ = tx.send(WizardEvent::Identified(Box::new(IdentifiedGame {
            app_id,
            name: info.name,
            is_windows,
            game_folder: final_folder,
            exe,
            variants,
        })));
    });
}

fn resolve_final_folder(
    path: &Path, move_to: Option<PathBuf>, tx: &mpsc::Sender<WizardEvent>,
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

fn identify_app_id(folder: &Path, steam: &ira_api::SteamDataClient) -> Option<String> {
    if let Some(steamapps) = ira_platforms::steam::steamapps_in_path(folder) {
        let installdir = folder.file_name()?.to_string_lossy().to_string();
        if let Some((appid, _name)) = ira_platforms::steam::find_appid_for_installdir(&steamapps, &installdir) {
            return Some(appid);
        }
    }
    let basename = folder.file_name()?.to_string_lossy().to_string();
    let results = steam.search_steam_store(&basename);
    results.into_iter().next().map(|(id, _)| id)
}

fn handle_identify_event(wizard: &Rc<RefCell<Wizard>>, ev: WizardEvent) {
    match ev {
        WizardEvent::Status(msg) => set_status(wizard, &msg),
        WizardEvent::AlreadyExists => {
            show_error(wizard, "This folder is already in your library. Pick another one.");
            show_pick_page(wizard);
        }
        WizardEvent::Failed(e) => {
            show_error(wizard, &e);
            show_pick_page(wizard);
        }
        WizardEvent::Identified(game) => show_identified_form(wizard, *game),
        WizardEvent::Added(_) => {}
    }
}

fn show_identified_form(wizard: &Rc<RefCell<Wizard>>, game: IdentifiedGame) {
    let (content, win, state, profiles) = {
        let w = wizard.borrow();
        (w.content.clone(), w.win.clone(), w.state.clone(), w.profiles.clone())
    };
    clear_children(&content);

    let is_windows = game.is_windows;
    let group = adw::PreferencesGroup::new();
    group.set_title("Confirm game");

    let name_entry = adw::EntryRow::new();
    name_entry.set_title("Name");
    name_entry.set_text(&game.name);
    group.add(&name_entry);

    let appid_row = adw::EntryRow::new();
    appid_row.set_title("Steam App ID");
    appid_row.set_text(&game.app_id);
    appid_row.set_sensitive(false);
    group.add(&appid_row);

    let profile_row = if is_windows {
        let row = build_wine_profile_picker(&profiles, None, None, &state, &win);
        group.add(&row);
        Some(row)
    } else {
        None
    };

    content.append(&group);

    let info_label = gtk4::Label::new(Some(if is_windows {
        "Detected: Windows game — a Wine profile is recommended."
    } else {
        "Detected: Linux native game."
    }));
    info_label.set_halign(gtk4::Align::Start);
    info_label.add_css_class("dim-label");
    content.append(&info_label);

    let add_btn = gtk4::Button::with_label("Add Game");
    add_btn.add_css_class(CSS_SUGGESTED_ACTION);
    add_btn.set_halign(gtk4::Align::Center);

    {
        let mut w = wizard.borrow_mut();
        w.identified = Some(game);
        w.profile_row = profile_row.clone();
    }
    let name_c = name_entry.clone();
    let appid_c = appid_row.clone();
    let wizard_c = wizard.clone();
    add_btn.connect_clicked(move |_| {
        let mut w = wizard_c.borrow_mut();
        if let Some(game) = w.identified.take() {
            let name = name_c.text().to_string();
            let app_id = appid_c.text().to_string();
            let profile_id = w.profile_row.as_ref()
                .and_then(|r| selected_profile_id(r, &w.profiles));
            start_add(wizard_c.clone(), game, name, app_id, profile_id);
        }
    });
    content.append(&add_btn);
}

fn start_add(
    wizard: Rc<RefCell<Wizard>>, game: IdentifiedGame, name: String, app_id: String,
    profile_id: Option<i64>,
) {
    let (db, steam, save_dir, sender, profiles) = {
        let w = wizard.borrow();
        let s = w.state.borrow();
        (s.db.clone(), s.steam.clone(), s.save_dir.clone(), s.sender.clone(), w.profiles.clone())
    };
    set_status(&wizard, "Adding game and downloading assets…");

    let (tx, rx) = mpsc::channel::<WizardEvent>();
    let rx = Rc::new(RefCell::new(rx));
    spawn_add_thread(tx, AddParams { db, steam, save_dir, sender, game, name, app_id, profile_id, profiles });

    let wizard_c = wizard.clone();
    glib::source::idle_add_local_full(glib::Priority::LOW, move || {
        match rx.borrow_mut().try_recv() {
            Ok(ev) => {
                handle_add_event(&wizard_c, ev);
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

struct AddParams {
    db: ira_db::DbConn,
    steam: std::sync::Arc<ira_api::SteamDataClient>,
    save_dir: String,
    sender: crate::AppSender,
    game: IdentifiedGame,
    name: String,
    app_id: String,
    profile_id: Option<i64>,
    profiles: Vec<WineProfile>,
}

fn spawn_add_thread(tx: mpsc::Sender<WizardEvent>, params: AddParams) {
    std::thread::spawn(move || {
        let AddParams { db, steam, save_dir, sender, game, name, app_id, profile_id, profiles } = params;
        let kind = if game.is_windows { GameKind::Wine } else { GameKind::Linux };
        let exe_path = if game.exe.is_empty() {
            String::new()
        } else {
            game.game_folder.join(&game.exe).to_string_lossy().into_owned()
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
            db: &db, name: &name, kind, trophy_source: TrophySource::Gse,
            app_id: &app_id, platform_id: &app_id, game_folder: &game.game_folder.to_string_lossy(),
            launch_config: &launch_config, wine_config: &wine_config,
            profile_id, steam: &steam, save_dir: &save_dir,
        });

        let db_id = match result {
            Ok(id) => id,
            Err(e) => {
                let _ = tx.send(WizardEvent::Failed(e));
                return;
            }
        };

        for (i, variant_exe) in game.variants.iter().enumerate() {
            let exe_path = game.game_folder.join(variant_exe).to_string_lossy().into_owned();
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
                let _ = tx.send(WizardEvent::Failed("Failed to reload game after add.".to_string()));
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

        crate::ui::enrichment::enrich_game_async(crate::ui::enrichment::EnrichGameParams {
            app_id: game_obj.app_id.clone(),
            trophy_source: game_obj.trophy_source,
            platform_id: game_obj.platform_id.clone(),
            db_id: game_obj.db_id,
            title: name.clone(),
            steam, sender, db, save_dir,
            game: None,
            ra_username: String::new(),
            ra_token: String::new(),
            ra_password: String::new(),
        });

        let _ = tx.send(WizardEvent::Added(db_id));
    });
}

fn handle_add_event(wizard: &Rc<RefCell<Wizard>>, ev: WizardEvent) {
    match ev {
        WizardEvent::Added(db_id) => {
            let (state, win) = {
                let w = wizard.borrow();
                (w.state.clone(), w.win.clone())
            };
            win.close();
            show_edit_game_dialog(&state, db_id);
        }
        WizardEvent::Failed(e) => {
            show_error(wizard, &e);
            show_pick_page(wizard);
        }
        WizardEvent::Status(msg) => set_status(wizard, &msg),
        _ => {}
    }
}

fn resolve_wine_config(profiles: &[WineProfile], profile_id: Option<i64>) -> WineConfig {
    let mut wine = WineConfig { enabled: true, ..Default::default() };
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

fn set_status(wizard: &Rc<RefCell<Wizard>>, msg: &str) {
    let content = wizard.borrow().content.clone();
    clear_children(&content);
    let status = adw::StatusPage::new();
    status.set_title("Auto Add Game");
    status.set_description(Some(msg));
    status.set_icon_name(Some("folder-open-symbolic"));
    let spinner = gtk4::Spinner::new();
    spinner.start();
    status.set_child(Some(&spinner));
    content.append(&status);
}

fn show_error(wizard: &Rc<RefCell<Wizard>>, msg: &str) {
    let win = wizard.borrow().win.clone();
    let alert = adw::AlertDialog::new(Some("Auto-add failed"), Some(msg));
    alert.add_response("ok", "OK");
    alert.set_default_response(Some("ok"));
    alert.set_close_response("ok");
    alert.present(Some(&win));
}

fn clear_children(container: &gtk4::Box) {
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
    copy_dir_recursive(src, dst)?;
    std::fs::remove_dir_all(src).map_err(|e| format!("remove source after copy: {}", e))?;
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    let entries = std::fs::read_dir(src).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let path = entry.path();
        let dest = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &dest)?;
        } else {
            std::fs::copy(&path, &dest).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
