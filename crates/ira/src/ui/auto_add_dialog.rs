use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc;

use adw::prelude::*;

use ira_api::ScraperCreds;
use ira_models::{GameKind, GameLaunchConfig, GameVariant, TrophySource, WineConfig, WineProfile};

use super::add_game_db::{add_game_to_db, AddGameToDbParams};
use super::css::*;
use super::helpers::clear_children;
use super::ss_match_dialog::persist_ss_match;
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
    /// `name` is the best display name the manifests provided.
    NeedSteamSearch {
        folder: PathBuf,
        name: String,
    },
    /// The add's ScreenScraper pass landed a match; persist it on the main
    /// loop — `persist_ss_match` touches the shared game list.
    SsMatched {
        db_id: i64,
        game: Box<ira_api::screenscraper::ScrapedGame>,
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
/// and ignores installers/redistributables. A `start.sh` (GOG Linux games)
/// beats everything, native ELF binaries beat Windows exes — the Windows
/// exe is only used when the install has no native build at all.
/// Returns `(is_windows, exe)`.
fn detect_game_exe(folder: &Path) -> (bool, String) {
    let basename = folder
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mut windows: Vec<(i32, String)> = Vec::new();
    let mut native: Vec<(i32, String)> = Vec::new();
    let mut start_sh: Option<String> = None;

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
            if lower == "start.sh" {
                // GOG Linux games launch through start.sh; it wins over
                // everything, shallowest match first.
                if start_sh.is_none() {
                    start_sh = Some(name);
                }
                continue;
            }
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

    if let Some(exe) = start_sh {
        (false, exe)
    } else if let Some((_, exe)) = native.into_iter().max_by_key(|(score, _)| *score) {
        (false, exe)
    } else if let Some((_, exe)) = windows.into_iter().max_by_key(|(score, _)| *score) {
        (true, exe)
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

    // Already living in a managed games folder, or no root to move into:
    // add it where it is. Otherwise ask where it belongs.
    if !needs_move_choice(&folders, path) {
        start_identify(path.to_path_buf(), None, wizard);
        return;
    }

    show_move_target_chooser(path, &folders, win, wizard);
}

/// Whether picking `path` should open the move-target chooser: only when it
/// sits outside every configured games root and at least one root exists.
/// A single root still asks — moving the game there silently would be a
/// surprise; the chooser doubles as the yes/no prompt.
fn needs_move_choice(folders: &[std::path::PathBuf], path: &Path) -> bool {
    !folders.is_empty() && !folders.iter().any(|folder| path.starts_with(folder))
}

/// The picked folder is outside every configured games root: offer to move
/// it into one of them, listing each with its free space. A standard alert
/// dialog: the destinations sit in a boxed list, and "Keep where it is" is
/// the suggested response — dismissing the dialog keeps it in place too.
fn show_move_target_chooser(
    path: &Path,
    folders: &[std::path::PathBuf],
    win: &gtk4::Widget,
    wizard: &Rc<RefCell<Wizard>>,
) {
    let basename = path.file_name().and_then(|n| n.to_str()).unwrap_or("game");
    let picked = path.to_path_buf();

    let dialog = adw::AlertDialog::new(
        Some(&crate::tr!("Move to games folder?")),
        Some(&crate::tr!(
            "Pick where to move it, or keep it where it is."
        )),
    );

    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);
    list.add_css_class(CSS_BOXED_LIST);
    for folder in folders {
        list.append(&move_destination_row(folder, basename, &picked, &dialog, wizard));
    }
    dialog.set_extra_child(Some(&list));

    dialog.add_response("keep", &crate::tr!("Keep where it is"));
    dialog.set_response_appearance("keep", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("keep"));
    dialog.set_close_response("keep");
    {
        let keep_wizard = wizard.clone();
        let source = picked.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "keep" {
                start_identify(source.clone(), None, &keep_wizard);
            }
        });
    }

    match super::helpers::hosting_window(win) {
        Some(host) => dialog.present(Some(&host)),
        None => eprintln!("Cannot present move-target chooser without a parent window"),
    }
}

/// One destination row: the games root by name, with its path and free
/// space as the subtitle. Activating it closes the chooser and moves the
/// game into that folder.
fn move_destination_row(
    folder: &Path,
    basename: &str,
    picked: &Path,
    dialog: &adw::AlertDialog,
    wizard: &Rc<RefCell<Wizard>>,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    let icon = gtk4::Image::from_icon_name("folder-new-symbolic");
    icon.set_valign(gtk4::Align::Center);
    row.add_prefix(&icon);
    row.set_title(&super::helpers::esc(
        &folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| folder.to_string_lossy().into_owned()),
    ));
    let mut subtitle = folder.to_string_lossy().into_owned();
    if let Some(free) = super::disk_space::available_bytes(folder) {
        subtitle.push_str(" · ");
        subtitle.push_str(&crate::tr!("{} free").replacen(
            "{}",
            &super::disk_space::format_size(free),
            1,
        ));
    }
    row.set_subtitle(&super::helpers::esc(&subtitle));
    row.set_activatable(true);
    let dest = folder.join(basename);
    let chosen = dialog.clone();
    let move_wizard = wizard.clone();
    let source = picked.to_path_buf();
    row.connect_activated(move |_| {
        // Handled here, so the dialog must close without also emitting the
        // "keep" response.
        chosen.force_close();
        start_identify(source.clone(), Some(dest.clone()), &move_wizard);
    });
    row
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

        let (app_id, name) = identify_game(&final_folder, &steam);
        let Some(app_id) = app_id else {
            let _ = tx.send(WizardEvent::NeedSteamSearch {
                folder: final_folder,
                name,
            });
            return;
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
    let steam_exe = default.map(|l| l.executable.clone()).unwrap_or_default();
    let steam_is_windows = default
        .map(|l| l.oslist.contains("windows") || l.oslist.is_empty())
        .unwrap_or(info.oslist.contains("windows") || info.oslist.is_empty());

    let target_os = if steam_is_windows { "windows" } else { "linux" };
    let steam_variants: Vec<String> = launches
        .iter()
        .skip(1)
        .filter(|l| l.oslist.contains(target_os) || l.oslist.is_empty())
        .filter(|l| !l.oslist.contains("macos"))
        .map(|l| l.executable.clone())
        .collect();

    let (is_windows, exe, variants) =
        reconcile_steam_exe_with_folder(&folder, steam_is_windows, steam_exe, steam_variants);

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

/// Steam launch configs describe the Steam install layout; a GOG Linux
/// install keeps the real executable directly in the game folder, so Steam's
/// paths don't apply. Local native evidence (start.sh or an ELF) wins; the
/// Steam exe is only kept when it actually exists on disk here.
fn reconcile_steam_exe_with_folder(
    folder: &Path,
    steam_is_windows: bool,
    steam_exe: String,
    steam_variants: Vec<String>,
) -> (bool, String, Vec<String>) {
    let (local_windows, local_exe) = detect_game_exe(folder);
    if !local_windows && !local_exe.is_empty() {
        return (false, local_exe, Vec::new());
    }
    if steam_exe.is_empty() || !folder.join(&steam_exe).is_file() {
        return (local_windows, local_exe, Vec::new());
    }
    (steam_is_windows, steam_exe, steam_variants)
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

/// Best-effort identification of a picked folder, manifests first: Steam's
/// `appmanifest_*.acf` pins the app id exactly when the folder sits in
/// `steamapps/common`, and clean GOG installs carry `goggame-*.info`
/// manifests whose game name searches the store far better than a folder's
/// basename. Returns the identified Steam app id (if any) plus the best
/// display name found, for the search fallback and manual setup.
pub(super) fn identify_game(
    folder: &Path,
    steam: &ira_api::SteamDataClient,
) -> (Option<String>, String) {
    if let Some(steamapps) = ira_platforms::steam::steamapps_in_path(folder) {
        if let Some(installdir) = folder.file_name().and_then(|n| n.to_str()) {
            if let Some((appid, name)) =
                ira_platforms::steam::find_appid_for_installdir(&steamapps, installdir)
            {
                return (Some(appid), name);
            }
        }
    }
    // Goldberg-style releases ship their manifests in a bracketed
    // `[Steam]` folder inside the game directory.
    if let Some((appid, name)) = ira_platforms::steam::find_appid_in_game_folder(folder) {
        return (Some(appid), name);
    }
    let basename = folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let manifest_name = folder_manifest_name(folder);
    if let Some(name) = &manifest_name {
        if let Some((id, _)) = steam.search_steam_store(name).into_iter().next() {
            return (Some(id), name.clone());
        }
    }
    let app_id = steam
        .search_steam_store(&basename)
        .into_iter()
        .next()
        .map(|(id, _)| id);
    (app_id, manifest_name.unwrap_or(basename))
}

/// The game name carried by on-disk manifests for `folder`, if any: clean
/// GOG installs name themselves in `goggame-*.info` files.
fn folder_manifest_name(folder: &Path) -> Option<String> {
    ira_platforms::gog::find_gog_info(&folder.to_string_lossy()).map(|(_, _, name)| name)
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
        WizardEvent::NeedSteamSearch { folder, name } => {
            show_steam_search_page(wizard, folder, name)
        }
        // Add-phase events are handled by handle_add_event.
        WizardEvent::Added(_)
        | WizardEvent::EmulatorPrompt { .. }
        | WizardEvent::InstallDone
        | WizardEvent::SsMatched { .. } => {}
    }
}

/// Fallback shown when automatic identification finds no Steam game: let the
/// user search Steam manually, or fall back to setting the game up by hand.
/// `name` is the best display name identification came up with — a GOG
/// manifest's game name beats the folder's basename for both the search and
/// the manual form.
fn show_steam_search_page(wizard: &Rc<RefCell<Wizard>>, folder: PathBuf, name: String) {
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
    // No window for the download: the wizard goes away here and the game
    // simply appears in the sidebar, the strip carrying the progress.
    // Decision prompts (emulator install, redists) still come back over
    // the main window.
    wizard.borrow().win.close();

    // Skipped too when a matching job owns the quota gate — one search
    // is not worth colliding with a batch pass.
    let ss_auto_match = {
        let w = wizard.borrow();
        let s = w.state.borrow();
        cfg.screenscraper_enabled && !s.ss_job_busy.get()
    };

    let (tx, rx) = mpsc::channel::<WizardEvent>();
    let rx = Rc::new(RefCell::new(rx));

    // The download's phase reports land on the sidebar strip instead of
    // a wizard status line, so they stay visible now that the wizard is
    // closed. The strip is claimed on the first report — an add
    // that fails before enriching never flashes it — and, when another
    // job holds it, the reports are dropped.
    let (progress_tx, progress_rx) =
        super::helpers::ui_channel::<(usize, usize, String)>();
    let strip_name = name.clone();
    let strip_state = wizard.borrow().state.clone();
    glib::spawn_future_local(async move {
        let short = crate::tr!("Adding {}…").replacen("{}", &strip_name, 1);
        let mut job = None;
        while let Ok((done, total, label)) = progress_rx.recv().await {
            if job.is_none() {
                job = super::fetch_images::begin_strip_job(
                    &strip_state,
                    &short,
                    &crate::tr!("Downloading assets…"),
                );
            }
            if let Some(job) = &job {
                job.progress(&strip_state, done, total, &label);
            }
        }
        if let Some(job) = job {
            job.finish(
                &strip_state,
                &crate::tr!("{} added").replacen("{}", &strip_name, 1),
                &crate::tr!("Assets downloaded"),
            );
        }
    });

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
            ss_auto_match,
            progress: Arc::new(move |done, total, label| {
                let _ = progress_tx.try_send((done, total, label.to_string()));
            }),
        },
    );

    let wizard_c = wizard;
    glib::source::idle_add_local_full(glib::Priority::LOW, move || {
        match rx.borrow_mut().try_recv() {
            Ok(ev) => {
                // The ScreenScraper match precedes the terminal event and
                // changes nothing about the wizard's fate.
                let terminal =
                    !matches!(ev, WizardEvent::Status(_) | WizardEvent::SsMatched { .. });
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
    /// Run the one-shot ScreenScraper match after enriching (the config
    /// enables the source and no batch pass owns the quota gate).
    pub ss_auto_match: bool,
    /// Feeds the sidebar strip the download's phase reports.
    pub progress: crate::ui::enrichment::EnrichProgress,
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
            ss_auto_match,
            progress,
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
        enrich_added_game(
            db.clone(),
            steam.clone(),
            sender,
            save_dir.clone(),
            cfg.clone(),
            &game_obj,
            progress,
        );
        apply_language_preference(
            &game,
            &game_obj,
            &save_dir_for_lang,
            &app_id,
            &language_preferences,
        );

        if ss_auto_match {
            auto_match_screenscraper(&steam, &db_for_cache, &cfg, &game_obj, &name, db_id, &tx);
        }

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
    progress: crate::ui::enrichment::EnrichProgress,
) {
    crate::ui::enrichment::enrich_game_blocking(crate::ui::enrichment::EnrichGameParams {
        app_id: game.app_id.clone(),
        trophy_source: game.trophy_source,
        platform_id: game.platform_id.clone(),
        db_id: game.db_id,
        // load_and_publish_game already set this exact name on the game.
        title: game.name.clone(),
        steam,
        sender,
        save_dir,
        db,
        game: None,
        ra_username: String::new(),
        ra_web_api_key: String::new(),
        cfg,
        progress: Some(progress),
    });
}

/// The add's own ScreenScraper pass: one ranked search for the fresh
/// game, the hit sent back for persisting on the main loop. A miss is
/// silent — the game's Identity page keeps the manual search.
fn auto_match_screenscraper(
    steam: &std::sync::Arc<ira_api::SteamDataClient>,
    db: &ira_db::DbConn,
    cfg: &ira_config::Config,
    game: &Game,
    display: &str,
    db_id: i64,
    tx: &mpsc::Sender<WizardEvent>,
) {
    let creds = ScraperCreds::from_account(
        cfg.screenscraper_id.clone(),
        cfg.screenscraper_password.clone(),
    );
    let target = super::mass_match_ss::PcMatchTarget {
        kind: game.kind,
        platform_id: &game.platform_id,
        title: &game.name,
        display,
        db_id,
    };
    if let super::mass_match_ss::SsOutcome::Hit(picked) =
        super::mass_match_ss::run_pc_matching(steam, &creds, db, &target)
    {
        let _ = tx.send(WizardEvent::SsMatched {
            db_id,
            game: picked,
        });
    }
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
        WizardEvent::SsMatched { db_id, game } => {
            persist_ss_match(&wizard.borrow().state, db_id, &game);
        }
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
        WizardEvent::Failed(e) => show_add_error(wizard, &e),
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

    let (main_win, default_version, versions) = {
        let w = wizard.borrow();
        let state = w.state.borrow();
        let default_version = state.cfg.default_api_emu_version.clone();
        let versions = match emu_kind {
            EmuKind::Nge => ira_platforms::api_emulators::list_gog_versions(&state.save_dir),
            EmuKind::Gse => ira_platforms::api_emulators::list_gse_versions(&state.save_dir),
        };
        (state.window.clone(), default_version, versions)
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
    dialog.present(Some(&main_win));

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
            }
        }
    }
    // Nothing opens at the end: the game is in the sidebar and the strip
    // announced the add. The settings screen stays a manual visit.
}

pub(super) fn prompt_redists(
    wizard: &Rc<RefCell<Wizard>>,
    db_id: i64,
    packages: Vec<ira_platforms::steam::RedistPackage>,
) {
    let win = wizard.borrow().state.borrow().window.clone();
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
        Some(&win),
        None::<&gtk4::gio::Cancellable>,
        move |response| {
            if response == "install" {
                start_redist_install(wizard_c.clone(), db_id, packages);
            }
            // Skipping installs nothing and announces nothing — the
            // strip's "added" already ran.
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

    // Copy _CommonRedist into the game folder so redists persist across
    // prefix changes. Installer paths are remapped to the local copy.
    let packages = match game_folder.as_deref() {
        Some(folder) => ira_platforms::steam::localize_redists(folder, packages),
        None => packages,
    };

    let (tx, rx) = mpsc::channel::<WizardEvent>();
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

    // The installers run with no window over them; when they finish the
    // add is simply over — the strip said "added" long since.
    glib::source::idle_add_local_full(glib::Priority::LOW, move || match rx.try_recv() {
        Ok(_) => glib::ControlFlow::Break,
        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
    });
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

/// Add-phase failures surface on the main window: the wizard closed
/// itself when the add began, so it can no longer host alerts.
fn show_add_error(wizard: &Rc<RefCell<Wizard>>, msg: &str) {
    let win = wizard.borrow().state.borrow().window.clone();
    let alert = adw::AlertDialog::new(Some(&crate::tr!("Auto-add failed")), Some(msg));
    alert.add_response("ok", &crate::tr!("OK"));
    alert.set_default_response(Some("ok"));
    alert.set_close_response("ok");
    alert.present(Some(&win));
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
    fn test_detect_game_exe_prefers_native_over_windows_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join("HollowKnight");
        write(&game_dir, "HollowKnight.exe");
        write(&game_dir, "HollowKnight.x86_64");

        let (is_windows, exe) = detect_game_exe(&game_dir);

        assert!(!is_windows);
        assert_eq!(exe, "HollowKnight.x86_64");
    }

    #[test]
    fn test_detect_game_exe_prefers_start_sh_over_everything() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join("SomeGOGGame");
        write(&game_dir, "start.sh");
        write(&game_dir, "SomeGOGGame.exe");
        write(&game_dir, "SomeGOGGame.x86_64");

        let (is_windows, exe) = detect_game_exe(&game_dir);

        assert!(!is_windows);
        assert_eq!(exe, "start.sh");
    }

    #[test]
    fn test_reconcile_native_folder_wins_over_steam_windows_config() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join("GOGGame");
        write(&game_dir, "start.sh");

        let (is_windows, exe, variants) = reconcile_steam_exe_with_folder(
            &game_dir,
            true,
            "Double/Game.exe".to_string(),
            vec!["Other.exe".to_string()],
        );

        assert!(!is_windows);
        assert_eq!(exe, "start.sh");
        assert!(variants.is_empty());
    }

    #[test]
    fn test_reconcile_keeps_steam_exe_when_it_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join("Game");
        write(&game_dir, "Bin/Game.exe");

        let (is_windows, exe, variants) = reconcile_steam_exe_with_folder(
            &game_dir,
            true,
            "Bin/Game.exe".to_string(),
            vec!["Alt.exe".to_string()],
        );

        assert!(is_windows);
        assert_eq!(exe, "Bin/Game.exe");
        assert_eq!(variants, vec!["Alt.exe".to_string()]);
    }

    #[test]
    fn test_reconcile_falls_back_to_local_when_steam_path_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join("GOGGame");
        write(&game_dir, "GOGGame.exe");

        let (is_windows, exe, variants) = reconcile_steam_exe_with_folder(
            &game_dir,
            true,
            "Double/Game.exe".to_string(),
            Vec::new(),
        );

        assert!(is_windows);
        assert_eq!(exe, "GOGGame.exe");
        assert!(variants.is_empty());
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

    #[test]
    fn test_needs_move_choice_asks_for_outside_paths_with_one_root() {
        let folders = vec![PathBuf::from("/games/pc")];
        assert!(!needs_move_choice(&folders, Path::new("/games/pc/Cool Game")));
        // Outside the single root still asks: the chooser is the move yes/no.
        assert!(needs_move_choice(&folders, Path::new("/downloads/Cool Game")));
        // Component-wise prefix: a sibling named /games/pc-games is outside.
        assert!(needs_move_choice(&folders, Path::new("/games/pc-games/Cool Game")));
        // No roots configured: nothing to move into, keep it where it is.
        assert!(!needs_move_choice(&[], Path::new("/downloads/Cool Game")));
    }

    #[test]
    fn test_needs_move_choice_asks_only_for_outside_paths() {
        let folders = vec![PathBuf::from("/games/pc"), PathBuf::from("/mnt/hdd/games")];
        assert!(needs_move_choice(&folders, Path::new("/downloads/Cool Game")));
        assert!(!needs_move_choice(&folders, Path::new("/mnt/hdd/games/Cool Game")));
        // Component-wise prefix: a sibling named /games/pc-games is outside.
        assert!(needs_move_choice(&folders, Path::new("/games/pc-games/Cool Game")));
    }
}
