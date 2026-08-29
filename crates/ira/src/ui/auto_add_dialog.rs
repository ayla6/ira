use std::cell::{Cell, RefCell};
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
use super::wizard_window::WizardWindow;
use crate::{AppMessage, AppSender, Game};

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

/// Guess whether a folder holds a Windows game and pick its most likely
/// executable. Walks two levels deep, prefers names that match the folder,
/// and ignores installers/redistributables. Returns `(is_windows, exe)`.
fn detect_game_exe(folder: &Path) -> (bool, String) {
    let basename = folder
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mut windows: Vec<(i32, String)> = Vec::new();
    let mut native: Vec<(i32, String)> = Vec::new();

    let mut stack = vec![(folder.to_path_buf(), 0i32)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if path.is_dir() {
                if depth < 2 {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            let lower = name.to_lowercase();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();
            if ext == "exe" {
                if is_installer_exe(&lower) {
                    continue;
                }
                windows.push((score_candidate(&basename, &lower, depth), name));
            } else if ext.is_empty() || matches!(ext.as_str(), "x86_64" | "AppRun") {
                if name.starts_with('.') {
                    continue;
                }
                if ext.is_empty() && !is_elf(&path) {
                    continue;
                }
                native.push((score_candidate(&basename, &lower, depth), name));
            }
        }
    }

    if let Some((_, exe)) = windows.into_iter().max_by_key(|(score, _)| *score) {
        (true, exe)
    } else if let Some((_, exe)) = native.into_iter().max_by_key(|(score, _)| *score) {
        (false, exe)
    } else {
        (false, String::new())
    }
}

fn is_installer_exe(lower: &str) -> bool {
    const MARKERS: &[&str] = &[
        "setup",
        "install",
        "unins",
        "uninstall",
        "vcredist",
        "vc_redist",
        "dxsetup",
        "dxwebsetup",
        "oalinst",
        "redist",
        "dotnet",
        "directx",
    ];
    MARKERS.iter().any(|m| lower.contains(m))
}

fn score_candidate(basename: &str, lower_name: &str, depth: i32) -> i32 {
    let stem = lower_name.strip_suffix(".exe").unwrap_or(lower_name);
    let depth_penalty = depth * 2;
    if stem == basename {
        100 - depth_penalty
    } else if stem.contains(basename) || basename.contains(stem) {
        60 - depth_penalty
    } else {
        5 - depth_penalty
    }
}

fn is_elf(path: &Path) -> bool {
    use std::io::Read;
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic)
        .map(|_| &magic == b"\x7fELF")
        .unwrap_or(false)
}

/// Wizard state shared between the main-thread poll closure and signal handlers.
pub(super) struct Wizard {
    pub win: WizardWindow,
    pub content: gtk4::Box,
    pub state: SharedState,
    pub profiles: Vec<WineProfile>,
    pub identified: Option<IdentifiedGame>,
    pub profile_row: Option<adw::ComboRow>,
    pub kind_row: Option<adw::ComboRow>,
    pub exe_entry: Option<adw::EntryRow>,
    pub last_folder: Option<PathBuf>,
    pub last_is_windows: bool,
}

pub fn show_auto_add_dialog(state: &SharedState) {
    let parent = state.borrow().window.clone();
    let win = adw::Dialog::new();
    win.set_title(&crate::tr!("Auto add game"));
    win.set_content_width(480);
    win.set_content_height(420);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    header.add_css_class(CSS_FLAT);
    content.append(&header);
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.append(&page);
    win.set_child(Some(&content));
    win.present(Some(&parent));

    let wizard = Rc::new(RefCell::new(Wizard {
        win: WizardWindow::Dialog(win.clone()),
        content: page,
        state: state.clone(),
        profiles: ira_db::get_all_profiles(&state.borrow().db).unwrap_or_default(),
        identified: None,
        profile_row: None,
        kind_row: None,
        exe_entry: None,
        last_folder: None,
        last_is_windows: false,
    }));

    show_pick_page(&wizard);
}

fn show_pick_page(wizard: &Rc<RefCell<Wizard>>) {
    let (state, win, content) = {
        let w = wizard.borrow();
        (w.state.clone(), w.win.clone(), w.content.clone())
    };
    clear_children(&content);

    let status = adw::StatusPage::new();
    status.set_title(&crate::tr!("Auto add game"));
    status.set_description(Some(&crate::tr!(
        "Pick the game's install folder. Ira will identify it, download assets and set everything up."
    )));
    status.set_icon_name(Some("folder-open-symbolic"));
    status.add_css_class(CSS_STATUS_NO_SCROLL);

    let pick_btn = gtk4::Button::with_label(&crate::tr!("Pick game folder…"));
    pick_btn.add_css_class(CSS_SUGGESTED_ACTION);
    pick_btn.set_halign(gtk4::Align::Center);

    let wizard_c = wizard.clone();
    pick_btn.connect_clicked(move |_| {
        pick_folder_and_start(win.as_widget(), &state, &wizard_c);
    });
    status.set_child(Some(&pick_btn));
    content.append(&status);
}

fn pick_folder_and_start(win: &gtk4::Widget, state: &SharedState, wizard: &Rc<RefCell<Wizard>>) {
    let default_folder = state.borrow().cfg.default_game_folder.clone();
    let dialog = gtk4::FileDialog::new();
    dialog.set_title(&crate::tr!("Select game folder"));
    super::helpers::set_initial_folder(&dialog, &default_folder);
    let state_c = state.clone();
    let win_c = win.clone();
    let wizard_c = wizard.clone();
    let Some(host) = super::helpers::hosting_window(win) else {
        return;
    };
    dialog.select_folder(
        Some(&host),
        None::<&gtk4::gio::Cancellable>,
        move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    on_folder_picked(&path, &state_c, &win_c, &wizard_c);
                }
            }
        },
    );
}

fn on_folder_picked(
    path: &Path,
    state: &SharedState,
    win: &gtk4::Widget,
    wizard: &Rc<RefCell<Wizard>>,
) {
    let folders = state.borrow().cfg.all_game_folders();

    // Already living in a managed games folder (or none configured): no move.
    if folders.is_empty() || folders.iter().any(|folder| path.starts_with(folder)) {
        start_identify(path.to_path_buf(), None, wizard);
        return;
    }

    show_move_target_chooser(path, &folders, win, wizard);
}

/// The picked folder is outside every configured games root: offer to move
/// it into one of them, listing each root with its free space.
fn show_move_target_chooser(
    path: &Path,
    folders: &[std::path::PathBuf],
    win: &gtk4::Widget,
    wizard: &Rc<RefCell<Wizard>>,
) {
    let basename = path.file_name().and_then(|n| n.to_str()).unwrap_or("game");
    let picked = path.to_path_buf();

    let dialog = adw::Dialog::new();
    dialog.set_title(&crate::tr!("Move to games folder?"));
    dialog.set_content_width(520);

    let header = adw::HeaderBar::new();
    let title = adw::WindowTitle::new(&crate::tr!("Move to games folder?"), "");
    header.set_title_widget(Some(&title));

    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);
    list.add_css_class("boxed-list");

    for folder in folders {
        let row = adw::ActionRow::new();
        row.set_title(&super::helpers::esc(&folder.to_string_lossy()));
        if let Some(free) = super::disk_space::available_bytes(folder) {
            row.set_subtitle(&crate::tr!("{} free").replacen(
                "{}",
                &super::disk_space::format_size(free),
                1,
            ));
        }
        row.set_activatable(true);
        let dest = folder.join(basename);
        let chosen = dialog.clone();
        let move_wizard = wizard.clone();
        let source = picked.clone();
        row.connect_activated(move |_| {
            chosen.close();
            start_identify(source.clone(), Some(dest.clone()), &move_wizard);
        });
        list.append(&row);
    }

    let keep_row = adw::ActionRow::new();
    keep_row.set_title(&crate::tr!("Keep where it is"));
    keep_row.set_activatable(true);
    let keep_wizard = wizard.clone();
    let keep_source = picked.clone();
    keep_row.connect_activated(move |_| {
        start_identify(keep_source.clone(), None, &keep_wizard);
    });
    list.append(&keep_row);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scroll.set_vexpand(true);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&scroll));
    dialog.set_child(Some(&toolbar));

    match super::helpers::hosting_window(win) {
        Some(host) => dialog.present(Some(&host)),
        None => eprintln!("Cannot present move-target chooser without a parent window"),
    }
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

    set_status(wizard, &crate::tr!("Identifying game…"));

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
    set_status(wizard, &crate::tr!("Identifying game…"));

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
    let _ = tx.send(WizardEvent::Status(crate::tr!("Moving folder…")));
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
                &crate::tr!("This folder is already in your library. Pick another one."),
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

    let title = gtk4::Label::new(Some(&crate::tr!("Couldn't identify this game.")));
    title.add_css_class(CSS_TITLE_1);
    title.set_halign(gtk4::Align::Center);
    content.append(&title);

    let hint = gtk4::Label::new(Some(&crate::tr!(
        "Search Steam for the game, or set it up manually."
    )));
    hint.add_css_class(CSS_DIM_LABEL);
    hint.set_halign(gtk4::Align::Center);
    content.append(&hint);

    let name = folder
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("game")
        .to_string();

    let search_btn = gtk4::Button::with_label(&crate::tr!("Search Steam…"));
    search_btn.add_css_class(CSS_SUGGESTED_ACTION);
    search_btn.set_halign(gtk4::Align::Center);
    let state_c = state;
    let steam_c = steam;
    let win_c = win;
    let wizard_c = wizard.clone();
    let folder_c = folder.clone();
    let name_c = name.clone();
    search_btn.connect_clicked(move |_| {
        show_search_results_dialog(SearchResultsDialogParams {
            state: &state_c,
            steam: steam_c.clone(),
            source_name: &crate::tr!("Steam"),
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
            parent: win_c.as_widget(),
            match_in_db: false,
        });
    });
    content.append(&search_btn);

    let manual_btn = gtk4::Button::with_label(&crate::tr!("Set up manually"));
    manual_btn.set_halign(gtk4::Align::Center);
    let wizard_c = wizard.clone();
    let folder_c = folder;
    let name_c = name;
    manual_btn.connect_clicked(move |_| {
        let (is_windows, exe) = detect_game_exe(&folder_c);
        show_identified_form(
            &wizard_c,
            IdentifiedGame {
                app_id: String::new(),
                name: name_c.clone(),
                is_windows,
                game_folder: folder_c.clone(),
                exe,
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
    group.set_title(&crate::tr!("Confirm game"));

    let name_entry = adw::EntryRow::new();
    name_entry.set_title(&crate::tr!("Name"));
    name_entry.set_text(&game.name);
    group.add(&name_entry);

    let appid_row = adw::EntryRow::new();
    appid_row.set_title(&crate::tr!("Steam app ID"));
    appid_row.set_text(&game.app_id);
    let appid_search_btn = gtk4::Button::from_icon_name("system-search-symbolic");
    appid_search_btn.set_valign(gtk4::Align::Center);
    appid_search_btn.set_tooltip_text(Some(&crate::tr!("Search Steam store")));
    appid_search_btn.add_css_class(CSS_FLAT);
    {
        let state_c = state.clone();
        let win_c = win.clone();
        let appid_c = appid_row.clone();
        let name_c = name_entry.clone();
        appid_search_btn.connect_clicked(move |_| {
            let search_text = name_c.text().to_string();
            let name_entry_c = name_c.clone();
            super::steam_search::show_steam_id_search_popup(
                &state_c,
                &search_text,
                win_c.as_widget(),
                &appid_c,
                &crate::tr!("Select"),
                Rc::new(move |_app_id: &str, matched_name: &str| {
                    name_entry_c.set_text(matched_name);
                }),
            );
        });
    }
    appid_row.add_suffix(&appid_search_btn);
    group.add(&appid_row);

    let kind_row = adw::ComboRow::new();
    kind_row.set_title(&crate::tr!("Kind"));
    let kind_model = {
        let labels = [crate::tr!("Native Linux"), crate::tr!("Wine (Windows)")];
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        gtk4::StringList::new(&refs)
    };
    kind_row.set_model(Some(&kind_model));
    kind_row.set_selected(if is_windows { 1 } else { 0 });
    group.add(&kind_row);

    let exe_entry = adw::EntryRow::new();
    exe_entry.set_title(&crate::tr!("Executable"));
    exe_entry.set_text(&game.exe);
    let exe_browse = super::helpers::make_browse_button(
        Some(win.as_widget()),
        &crate::tr!("Select executable"),
        false,
        Some((
            &crate::tr!("Executable"),
            &["application/x-executable", "application/x-msdos-program"],
        )),
        || None,
        {
            let entry = exe_entry.clone();
            move |path| entry.set_text(&path.to_string_lossy())
        },
    );
    exe_entry.add_suffix(&exe_browse);
    group.add(&exe_entry);

    let profile_row =
        build_wine_profile_picker(&profiles, preselected_profile_id, None, &state, win.as_widget());
    profile_row.set_visible(is_windows);
    group.add(&profile_row);
    let profile_row_c = profile_row.clone();
    kind_row.connect_selected_notify(move |row| {
        profile_row_c.set_visible(row.selected() == 1);
    });

    body.append(&group);

    let add_btn = gtk4::Button::with_label(&crate::tr!("Add game"));
    add_btn.add_css_class(CSS_SUGGESTED_ACTION);
    add_btn.set_halign(gtk4::Align::Center);

    {
        let folder = game.game_folder.clone();
        let mut w = wizard.borrow_mut();
        w.identified = Some(game);
        w.profile_row = Some(profile_row);
        w.kind_row = Some(kind_row);
        w.exe_entry = Some(exe_entry);
        w.last_folder = Some(folder);
        w.last_is_windows = is_windows;
    }
    let name_c = name_entry;
    let appid_c = appid_row;
    let wizard_c = wizard.clone();
    add_btn.connect_clicked(move |_| {
        let extracted = {
            let mut w = wizard_c.borrow_mut();
            w.identified.take().map(|mut game| {
                let name = name_c.text().to_string();
                let app_id = appid_c.text().to_string();
                game.is_windows = w
                    .kind_row
                    .as_ref()
                    .map(|r| r.selected() == 1)
                    .unwrap_or(game.is_windows);
                game.exe = w
                    .exe_entry
                    .as_ref()
                    .map(|e| e.text().to_string())
                    .unwrap_or(game.exe);
                let profile_id = if game.is_windows {
                    w.profile_row
                        .as_ref()
                        .and_then(|r| selected_profile_id(r, &w.state.borrow().db))
                } else {
                    None
                };
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
    let (db, steam, save_dir, sender, profiles, language_preferences, cfg) = {
        let w = wizard.borrow();
        let s = w.state.borrow();
        (
            s.db.clone(),
            s.steam.clone(),
            s.save_dir.clone(),
            s.sender.clone(),
            w.profiles.clone(),
            s.cfg.language_preferences.clone(),
            s.cfg.clone(),
        )
    };
    set_status(&wizard, &crate::tr!("Adding game and downloading assets…"));

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
            cfg,
        },
    );

    let wizard_c = wizard;
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
    pub cfg: ira_config::Config,
}

struct AddGameSetup {
    kind: GameKind,
    launch_config: GameLaunchConfig,
    wine_config: WineConfig,
}

struct AddGameRecordParams<'a> {
    db: &'a ira_db::DbConn,
    steam: &'a std::sync::Arc<ira_api::SteamDataClient>,
    save_dir: &'a str,
    game: &'a IdentifiedGame,
    name: &'a str,
    app_id: &'a str,
    profile_id: Option<i64>,
    setup: &'a AddGameSetup,
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
            cfg,
        } = params;
        let setup = build_add_game_setup(&game, &profiles, profile_id);
        let db_id = match add_game_record(AddGameRecordParams {
            db: &db,
            steam: &steam,
            save_dir: &save_dir,
            game: &game,
            name: &name,
            app_id: &app_id,
            profile_id,
            setup: &setup,
        }) {
            Ok(id) => id,
            Err(e) => {
                let _ = tx.send(WizardEvent::Failed(e));
                return;
            }
        };

        let game_obj = match load_and_publish_game(
            &db,
            &save_dir,
            &sender,
            db_id,
            &name,
            &setup.launch_config,
        ) {
            Ok(g) => g,
            Err(e) => {
                let _ = tx.send(WizardEvent::Failed(e));
                return;
            }
        };

        let save_dir_for_lang = save_dir.clone();
        let db_for_cache = db.clone();
        enrich_added_game(db, steam, sender, save_dir, cfg, &game_obj, name);
        apply_language_preference(
            &game,
            &game_obj,
            &save_dir_for_lang,
            &app_id,
            &language_preferences,
        );

        if skip_emu_prompt {
            let _ = tx.send(WizardEvent::Added(db_id));
            return;
        }

        migrate_game_saves(
            &db_for_cache,
            &save_dir_for_lang,
            &app_id,
            db_id,
            &game,
            &setup.wine_config,
        );

        if let Some(emu_kind) = emulator_needed(&game.game_folder.to_string_lossy()) {
            let _ = tx.send(WizardEvent::EmulatorPrompt {
                db_id,
                game_folder: game.game_folder.clone(),
                app_id,
                emu_kind,
            });
        } else {
            let _ = tx.send(WizardEvent::Added(db_id));
        }
    });
}

fn build_add_game_setup(
    game: &IdentifiedGame,
    profiles: &[WineProfile],
    profile_id: Option<i64>,
) -> AddGameSetup {
    let exe_path = if game.exe.is_empty() {
        String::new()
    } else {
        game.game_folder
            .join(&game.exe)
            .to_string_lossy()
            .into_owned()
    };
    AddGameSetup {
        kind: if game.is_windows {
            GameKind::Wine
        } else {
            GameKind::Linux
        },
        launch_config: GameLaunchConfig {
            exe: exe_path,
            working_dir: game.game_folder.to_string_lossy().into_owned(),
            ..Default::default()
        },
        wine_config: if game.is_windows {
            resolve_wine_config(profiles, profile_id)
        } else {
            WineConfig::default()
        },
    }
}

fn add_game_record(params: AddGameRecordParams<'_>) -> Result<i64, String> {
    let AddGameRecordParams {
        db,
        steam,
        save_dir,
        game,
        name,
        app_id,
        profile_id,
        setup,
    } = params;
    let game_folder = game.game_folder.to_string_lossy();
    let db_id = add_game_to_db(AddGameToDbParams {
        db,
        name,
        kind: setup.kind,
        trophy_source: TrophySource::Gse,
        app_id,
        platform_id: app_id,
        game_folder: &game_folder,
        launch_config: &setup.launch_config,
        wine_config: &setup.wine_config,
        profile_id,
        steam,
        save_dir,
    })?;

    if !game.logo_position.is_empty() {
        let _ = ira_db::set_logo_settings(db, db_id, &game.logo_position, game.logo_size);
    }
    add_game_variants(db, db_id, game);
    Ok(db_id)
}

fn add_game_variants(db: &ira_db::DbConn, db_id: i64, game: &IdentifiedGame) {
    let working_dir = game.game_folder.to_string_lossy().into_owned();
    for (i, variant_exe) in game.variants.iter().enumerate() {
        let variant = GameVariant {
            game_id: db_id,
            name: format!("Launch {}", i + 2),
            exe: game
                .game_folder
                .join(variant_exe)
                .to_string_lossy()
                .into_owned(),
            working_dir: working_dir.clone(),
            show_as_entry: false,
            ..Default::default()
        };
        if let Err(e) = ira_db::add_variant(db, &variant) {
            eprintln!("Failed to add variant: {}", e);
        }
    }
}

fn load_and_publish_game(
    db: &ira_db::DbConn,
    save_dir: &str,
    sender: &AppSender,
    db_id: i64,
    name: &str,
    launch_config: &GameLaunchConfig,
) -> Result<Game, String> {
    let entry = ira_db::find_by_db_id(db, db_id)
        .ok()
        .flatten()
        .ok_or_else(|| "Failed to reload game after add.".to_string())?;
    let mut game = crate::game_loader::load_game(&entry, save_dir)?;
    game.set_name(name);
    game.game_path = launch_config.exe.clone();
    let _ = ira_db::update_game_title(db, game.db_id, name);
    let _ = sender.send(AppMessage::NewGame(game.clone()));
    Ok(game)
}

fn enrich_added_game(
    db: ira_db::DbConn,
    steam: std::sync::Arc<ira_api::SteamDataClient>,
    sender: AppSender,
    save_dir: String,
    cfg: ira_config::Config,
    game: &Game,
    title: String,
) {
    crate::ui::enrichment::enrich_game_blocking(crate::ui::enrichment::EnrichGameParams {
        app_id: game.app_id.clone(),
        trophy_source: game.trophy_source,
        platform_id: game.platform_id.clone(),
        db_id: game.db_id,
        title,
        steam,
        sender,
        save_dir,
        db,
        game: None,
        ra_username: String::new(),
        ra_web_api_key: String::new(),
        cfg,
    });
}

fn apply_language_preference(
    identified: &IdentifiedGame,
    game: &Game,
    save_dir: &str,
    app_id: &str,
    language_preferences: &[String],
) {
    if language_preferences.is_empty() {
        return;
    }
    let game_exe = identified.game_folder.join(&identified.exe);
    let game_exe_str = game_exe.to_string_lossy().to_string();
    let appdetails_path = ira_parser::data_dir(save_dir, app_id).join("appdetails.json");
    if let Ok(content) = std::fs::read_to_string(appdetails_path) {
        if let Ok(details) = serde_json::from_str::<ira_models::AppDetails>(&content) {
            let chosen = language_preferences
                .iter()
                .find(|pref| details.languages.iter().any(|language| language == *pref))
                .or_else(|| {
                    details
                        .languages
                        .iter()
                        .find(|language| **language == "english")
                })
                .or_else(|| details.languages.first());
            if let Some(lang) = chosen {
                ira_platforms::api_emulators::write_language_configs(
                    game.trophy_source,
                    &game_exe_str,
                    save_dir,
                    app_id,
                    lang,
                );
            }
        }
    }
}

fn migrate_game_saves(
    db: &ira_db::DbConn,
    save_dir: &str,
    app_id: &str,
    db_id: i64,
    game: &IdentifiedGame,
    wine_config: &WineConfig,
) {
    let game_folder = game.game_folder.to_string_lossy();
    let has_steam_dlls =
        !ira_platforms::api_emulators::find_steam_dlls_recursive(&game_folder).is_empty();
    let has_gog_dlls =
        !ira_platforms::api_emulators::find_gog_dlls_recursive(&game_folder).is_empty();
    if !app_id.is_empty() || has_steam_dlls {
        if let Err(e) = ira_platforms::api_emulators::centralize_steam_settings(&game_folder) {
            eprintln!("Failed to centralize steam_settings: {}", e);
        }
    }
    if has_gog_dlls {
        if let Err(e) = ira_platforms::api_emulators::centralize_galaxy_settings(&game_folder) {
            eprintln!("Failed to centralize ngalaxye_settings: {}", e);
        }
    }

    let steam_related = !app_id.is_empty() || has_steam_dlls;
    let wine_prefix = if game.is_windows {
        Some(ira_launcher::wine_launch::wine_prefix(wine_config))
    } else {
        None
    };
    if steam_related {
        ira_platforms::emulator_save_migration::migrate_gbe_saves(
            save_dir,
            app_id,
            wine_prefix.as_deref(),
        );
    }
    if has_gog_dlls {
        ira_platforms::emulator_save_migration::migrate_nge_saves(save_dir, wine_prefix.as_deref());
    }

    if let Some(details) = crate::game_loader::read_app_details(save_dir, app_id) {
        if !details.ufs_savefiles.is_empty() {
            let count = ira_launcher::game_saves::setup_game_saves(
                &details.ufs_savefiles,
                &details.ufs_rootoverrides,
                app_id,
                save_dir,
                wine_prefix.as_deref(),
            );
            if count > 0 {
                if let Err(e) = ira_db::set_saves_centralized(db, db_id, true) {
                    eprintln!("Failed to cache saves centralized: {}", e);
                }
            }
        }
    }
}

fn emulator_needed(game_folder: &str) -> Option<EmuKind> {
    let needs_nge = ira_platforms::api_emulators::find_gog_dlls_recursive(game_folder)
        .iter()
        .any(|dir| !ira_platforms::api_emulators::has_gog_emulator_backups(dir));
    if needs_nge {
        return Some(EmuKind::Nge);
    }
    let needs_gse = ira_platforms::api_emulators::find_steam_dlls_recursive(game_folder)
        .iter()
        .any(|dir| {
            !ira_platforms::api_emulators::has_steam_emulator_backups(dir)
                && !dir.join("steam_settings").is_dir()
        });
    needs_gse.then_some(EmuKind::Gse)
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

    let (win, default_version, versions) = {
        let w = wizard.borrow();
        let state = w.state.borrow();
        let default_version = state.cfg.default_api_emu_version.clone();
        let versions = match emu_kind {
            EmuKind::Nge => ira_platforms::api_emulators::list_gog_versions(&state.save_dir),
            EmuKind::Gse => ira_platforms::api_emulators::list_gse_versions(&state.save_dir),
        };
        (w.win.clone(), default_version, versions)
    };
    let (title, body) = match emu_kind {
        EmuKind::Nge => (
            crate::tr!("Install Nemirtingas Galaxy emulator?"),
            crate::tr!("GOG Galaxy DLLs were found in this game. Install the Nemirtingas Galaxy Emulator to enable achievements? (Steam DLLs, if any, will be left untouched.)"),
        ),
        EmuKind::Gse => (
            crate::tr!("Install Goldberg emulator?"),
            crate::tr!("Steam API DLLs were found in this game. Install the Goldberg Steam Emulator to enable achievements?"),
        ),
    };

    let dialog = adw::Dialog::new();
    dialog.set_title(&title);
    dialog.set_content_width(380);
    dialog.set_content_height(240);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    outer.set_margin_start(20);
    outer.set_margin_end(20);
    outer.set_margin_top(16);
    outer.set_margin_bottom(16);

    let header = adw::HeaderBar::new();
    header.add_css_class(CSS_FLAT);
    outer.append(&header);

    let msg = gtk4::Label::new(Some(&body));
    msg.set_wrap(true);
    msg.set_halign(gtk4::Align::Start);
    outer.append(&msg);

    let group = adw::PreferencesGroup::new();
    let version_row = if !versions.is_empty() {
        let version_model = {
            let labels: Vec<&str> = versions.iter().map(|s| s.as_str()).collect();
            gtk4::StringList::new(&labels)
        };
        let vr = adw::ComboRow::new();
        vr.set_title(&crate::tr!("Emulator version"));
        vr.set_subtitle(&crate::tr!("Version directory to install"));
        vr.set_model(Some(&version_model));
        if !default_version.is_empty() {
            if let Some(idx) = versions.iter().position(|v| v == &default_version) {
                vr.set_selected(idx as u32);
            }
        }
        group.add(&vr);
        Some(vr)
    } else {
        let no_ver_row = adw::ActionRow::new();
        no_ver_row.set_title(&crate::tr!("No emulator versions available"));
        no_ver_row.set_subtitle(&crate::tr!("Place version directories in api_emulators/"));
        no_ver_row.set_sensitive(false);
        group.add(&no_ver_row);
        None
    };
    let remember = adw::SwitchRow::new();
    remember.set_title(&crate::tr!("Don't ask me again"));
    group.add(&remember);
    outer.append(&group);

    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);
    let no_btn = gtk4::Button::with_label(&crate::tr!("No"));
    let yes_btn = gtk4::Button::with_label(&crate::tr!("Yes"));
    yes_btn.add_css_class(CSS_SUGGESTED_ACTION);
    btn_row.append(&no_btn);
    btn_row.append(&yes_btn);
    outer.append(&btn_row);

    dialog.set_child(Some(&outer));
    dialog.present(Some(win.as_widget()));

    let resolved = Rc::new(Cell::new(false));
    let wizard_c = wizard.clone();
    let remember_c = remember.clone();
    let dialog_c = dialog.clone();
    let versions_for_yes = versions;
    let resolved_for_yes = resolved.clone();
    yes_btn.connect_clicked(move |_| {
        resolved_for_yes.set(true);
        persist_remember(&wizard_c, remember_c.is_active(), true);
        let version = version_row
            .as_ref()
            .map(|vr| {
                let idx = vr.selected() as usize;
                if idx < versions_for_yes.len() {
                    versions_for_yes[idx].clone()
                } else {
                    String::new()
                }
            })
            .unwrap_or(default_version.clone());
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
    let remember_c2 = remember;
    let dialog_c2 = dialog.clone();
    let resolved_for_no = resolved.clone();
    no_btn.connect_clicked(move |_| {
        resolved_for_no.set(true);
        persist_remember(&wizard_c2, remember_c2.is_active(), false);
        dialog_c2.close();
        finalize(&wizard_c2, db_id);
    });
    let wizard_for_close = wizard.clone();
    dialog.connect_closed(move |_| {
        if !resolved.get() {
            finalize(&wizard_for_close, db_id);
        }
    });
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
        EmuKind::Nge => crate::tr!("Installing Nemirtingas Galaxy emulator…"),
        EmuKind::Gse => crate::tr!("Installing Goldberg emulator…"),
    };
    set_status(&wizard, &status);

    let (tx, rx) = mpsc::channel::<WizardEvent>();
    let tx_c = tx;
    let game_folder_c = game_folder;
    let app_id_c = app_id;
    let version_c = version;
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

    let wizard_c = wizard;
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
    let body = crate::tr!(
        "Steamworks redistributables were found:\n{}\n\nInstall the selected ones now via Wine?"
    )
    .replacen(
        "{}",
        &packages
            .iter()
            .map(|p| format!("- {}", p.name))
            .collect::<Vec<_>>()
            .join("\n"),
        1,
    );
    let alert = adw::AlertDialog::new(Some(&crate::tr!("Install redistributables?")), Some(&body));
    alert.add_response("skip", &crate::tr!("Skip"));
    alert.add_response("install", &crate::tr!("Install"));
    alert.set_response_appearance("install", adw::ResponseAppearance::Suggested);
    alert.set_default_response(Some("install"));
    alert.set_close_response("skip");

    let wizard_c = wizard.clone();
    alert.choose(
        Some(win.as_widget()),
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
    set_status(
        &wizard,
        &crate::tr!("Installing redistributables via Wine…"),
    );

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

    let wizard_c = wizard;
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
    status.set_title(&crate::tr!("Auto add game"));
    status.set_description(Some(msg));
    status.set_icon_name(Some("folder-open-symbolic"));
    status.add_css_class(CSS_STATUS_NO_SCROLL);
    let spinner = gtk4::Spinner::new();
    spinner.start();
    status.set_child(Some(&spinner));
    content.append(&status);
}

pub(super) fn show_error(wizard: &Rc<RefCell<Wizard>>, msg: &str) {
    let win = wizard.borrow().win.clone();
    let alert = adw::AlertDialog::new(Some(&crate::tr!("Auto-add failed")), Some(msg));
    alert.add_response("ok", &crate::tr!("OK"));
    alert.set_default_response(Some("ok"));
    alert.set_close_response("ok");
    alert.present(Some(win.as_widget()));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &std::path::Path, name: &str) {
        let p = path.join(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, b"\x7fELF").unwrap();
    }

    #[test]
    fn test_detect_game_exe_picks_matching_windows_exe() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "setup.exe");
        write(tmp.path(), "Fallout3.exe");

        let (is_windows, exe) = detect_game_exe(tmp.path());

        assert!(is_windows);
        assert_eq!(exe, "Fallout3.exe");
    }

    #[test]
    fn test_detect_game_exe_skips_installers_and_prefers_folder_match() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join("HollowKnight");
        write(&game_dir, "vcredist_x64.exe");
        write(&game_dir, "HollowKnight.exe");
        write(&game_dir, "bin/HollowKnight_Data.exe");

        let (is_windows, exe) = detect_game_exe(&game_dir);

        assert!(is_windows);
        assert_eq!(exe, "HollowKnight.exe");
    }

    #[test]
    fn test_detect_game_exe_finds_native_elf_when_no_exe() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "game.x86_64");
        write(tmp.path(), "README.md");

        let (is_windows, exe) = detect_game_exe(tmp.path());

        assert!(!is_windows);
        assert_eq!(exe, "game.x86_64");

        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "game");
        write(tmp.path(), "README.md");

        let (is_windows, exe) = detect_game_exe(tmp.path());

        assert!(!is_windows);
        assert_eq!(exe, "game");
    }

    #[test]
    fn test_detect_game_exe_returns_empty_for_bare_folder() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "readme.txt");

        let (is_windows, exe) = detect_game_exe(tmp.path());

        assert!(!is_windows);
        assert!(exe.is_empty());
    }

    #[test]
    fn test_score_candidate_prefers_exact_folder_match() {
        assert!(score_candidate("doom", "doom.exe", 0) > score_candidate("doom", "game.exe", 0));
        assert!(score_candidate("doom", "doom.exe", 0) > score_candidate("doom", "doom.exe", 1));
    }
}
