use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use adw::prelude::*;

use ira_platforms::installer::{
    extract_data_zip, extract_inno, game_platform, innoextract_available, installer_type,
    is_gog_makeself, is_inno_setup, is_linuxrulez, split_gog_installer, GamePlatform,
    InstallerType,
};

use super::auto_add_dialog::{
    clear_children, resolve_wine_config, set_status, show_error, show_identified_form,
    spawn_identify_thread, IdentifiedGame, Wizard, WizardEvent,
};
use super::css::*;
use super::helpers::esc;
use super::state::SharedState;
use super::wine_profile_picker::{build_wine_profile_picker, selected_profile_id};
use super::wizard_window::WizardWindow;

struct InstallerState {
    installers: Vec<PathBuf>,
    current_index: usize,
    profile_id: Option<i64>,
    wow64: bool,
    gamescope: bool,
    default_game_folder: PathBuf,
    pre_snapshot: Vec<String>,
    game_platform: GamePlatform,
    detected_folder: Option<PathBuf>,
}

pub fn show_installer_add_dialog(state: &SharedState) {
    let parent = state.borrow().window.clone();
    // A separate window instead of an in-window dialog so the library stays
    // usable while installers run.
    let win = adw::Window::new();
    win.set_title(Some(&crate::tr!("Install from Installer")));
    win.set_default_size(520, 580);
    win.set_transient_for(Some(&parent));
    // Closing mid-install only hides the window; the run keeps going and
    // can present its result prompt on it again.
    win.set_hide_on_close(true);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    header.add_css_class(CSS_FLAT);
    content.append(&header);
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content.append(&page);
    // AdwWindow only accepts set_content; gtk_window_set_child aborts.
    win.set_content(Some(&content));
    win.present();

    let profiles = ira_db::get_all_profiles(&state.borrow().db).unwrap_or_default();
    let wizard = Rc::new(RefCell::new(Wizard {
        win: WizardWindow::Window(win.clone()),
        content: page,
        state: state.clone(),
        profiles,
        identified: None,
        profile_row: None,
        kind_row: None,
        exe_entry: None,
        last_folder: None,
        last_is_windows: false,
    }));

    let default_game_folder = PathBuf::from(&state.borrow().cfg.default_game_folder);
    let ist = Rc::new(RefCell::new(InstallerState {
        installers: Vec::new(),
        current_index: 0,
        profile_id: None,
        wow64: false,
        gamescope: false,
        default_game_folder,
        pre_snapshot: Vec::new(),
        game_platform: GamePlatform::Linux,
        detected_folder: None,
    }));

    show_config_page(&wizard, &ist);

}

fn show_config_page(wizard: &Rc<RefCell<Wizard>>, ist: &Rc<RefCell<InstallerState>>) {
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

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    body.set_margin_start(16);
    body.set_margin_end(16);
    body.set_margin_top(8);
    body.set_margin_bottom(16);
    scrolled.set_child(Some(&body));
    content.append(&scrolled);

    let installer_group = adw::PreferencesGroup::new();
    installer_group.set_title(&crate::tr!("Installers"));
    let list = gtk4::ListBox::new();
    list.add_css_class(CSS_BOXED_LIST);
    list.set_selection_mode(gtk4::SelectionMode::None);
    installer_group.add(&list);
    body.append(&installer_group);

    let add_btn = gtk4::Button::with_label(&crate::tr!("Add installer…"));
    add_btn.set_margin_top(8);
    let ist_c = ist.clone();
    let list_c = list.clone();
    let win_c = win.clone();
    add_btn.connect_clicked(move |_| {
        let dialog = gtk4::FileDialog::new();
        dialog.set_title(&crate::tr!("Select installer files"));
        let ist_c2 = ist_c.clone();
        let list_c2 = list_c.clone();
        let Some(host) = super::helpers::hosting_window(win_c.as_widget()) else {
            return;
        };
        dialog.open_multiple(
            Some(&host),
            None::<&gtk4::gio::Cancellable>,
            move |result| {
                if let Ok(files) = result {
                    let mut ist = ist_c2.borrow_mut();
                    let mut added = false;
                    let mut idx = files.iter::<gtk4::gio::File>();
                    while let Some(Ok(file)) = idx.next() {
                        if let Some(path) = file.path() {
                            ist.installers.push(path);
                            added = true;
                        }
                    }
                    drop(ist);
                    if added {
                        rebuild_installer_list(&list_c2, &ist_c2);
                    }
                }
            },
        );
    });
    body.append(&add_btn);

    let wine_group = adw::PreferencesGroup::new();
    wine_group.set_title(&crate::tr!("Wine"));
    let profile_row = build_wine_profile_picker(&profiles, None, None, &state, win.as_widget());
    wine_group.add(&profile_row);
    let wow64_row = adw::SwitchRow::new();
    wow64_row.set_title(&crate::tr!("Use WOW64"));
    wow64_row.set_subtitle(&crate::tr!(
        "Some installers don't work with it (Proton only)"
    ));
    wine_group.add(&wow64_row);
    let gamescope_row = adw::SwitchRow::new();
    gamescope_row.set_title(&crate::tr!("Run under gamescope"));
    gamescope_row.set_subtitle(&crate::tr!("Useful for installers with flaky windows"));
    wine_group.add(&gamescope_row);
    body.append(&wine_group);

    let guide_group = adw::PreferencesGroup::new();
    let default_folder = ist.borrow().default_game_folder.clone();
    if default_folder.as_os_str().is_empty() {
        let row = adw::ActionRow::new();
        row.set_title(&crate::tr!("Set your PC games folder"));
        row.set_subtitle(&crate::tr!(
            "Set a default games folder in Settings to enable auto-detection of installed games."
        ));
        guide_group.add(&row);
    } else {
        let row = adw::ActionRow::new();
        row.set_title(&crate::tr!("Install to this folder"));
        row.set_subtitle(&esc(&default_folder.to_string_lossy()));
        let open_btn = gtk4::Button::new();
        open_btn.set_icon_name("folder-open-symbolic");
        open_btn.set_valign(gtk4::Align::Center);
        open_btn.add_css_class(CSS_FLAT);
        let path = default_folder;
        open_btn.connect_clicked(move |_| {
            let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
        });
        row.add_suffix(&open_btn);
        guide_group.add(&row);
    }
    body.append(&guide_group);

    let start_btn = gtk4::Button::with_label(&crate::tr!("Start installation"));
    start_btn.add_css_class(CSS_SUGGESTED_ACTION);
    start_btn.set_size_request(-1, 44);

    let wizard_c = wizard.clone();
    let ist_c = ist.clone();
    let profile_row_c = profile_row;
    let wow64_row_c = wow64_row;
    let gamescope_row_c = gamescope_row;
    let db_c = state.borrow().db.clone();
    start_btn.connect_clicked(move |_| {
        let installers = ist_c.borrow().installers.clone();
        if installers.is_empty() {
            return;
        }
        let game_platform = installers
            .iter()
            .map(|p| game_platform(p, installer_type(p)))
            .find(|p| *p == GamePlatform::Windows)
            .unwrap_or(GamePlatform::Linux);

        {
            let mut ist = ist_c.borrow_mut();
            ist.profile_id = selected_profile_id(&profile_row_c, &db_c);
            ist.wow64 = wow64_row_c.is_active();
            ist.gamescope = gamescope_row_c.is_active();
            ist.game_platform = game_platform;
            ist.current_index = 0;
            let df = &ist.default_game_folder;
            ist.pre_snapshot = if df.as_os_str().is_empty() || !df.exists() {
                Vec::new()
            } else {
                snapshot_subdirs(df)
            };
        }
        start_installation(&wizard_c, &ist_c);
    });
    body.append(&start_btn);

    rebuild_installer_list(&list, ist);
}

fn rebuild_installer_list(list: &gtk4::ListBox, ist: &Rc<RefCell<InstallerState>>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let installers = ist.borrow().installers.clone();
    if installers.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title(&crate::tr!("No installers added yet"));
        row.set_subtitle(&crate::tr!(
            "Click \"Add installer…\" to select installer files"
        ));
        list.append(&row);
        return;
    }
    for (i, path) in installers.iter().enumerate() {
        let row = adw::ActionRow::new();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        row.set_title(&esc(name));

        let itype = installer_type(path);
        let badge = if itype == InstallerType::Linux && is_gog_makeself(path) {
            "GOG"
        } else if itype == InstallerType::Linux && is_linuxrulez(path) {
            "LinuxRulez"
        } else if itype == InstallerType::Windows && is_inno_setup(path) {
            "Inno Setup"
        } else if itype == InstallerType::Windows {
            "Windows"
        } else {
            "Linux"
        };
        row.set_subtitle(badge);

        let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        if i > 0 {
            let up_btn = gtk4::Button::new();
            up_btn.set_icon_name("go-up-symbolic");
            up_btn.add_css_class(CSS_FLAT);
            up_btn.set_valign(gtk4::Align::Center);
            let ist_c = ist.clone();
            let list_c = list.clone();
            up_btn.connect_clicked(move |_| {
                let mut s = ist_c.borrow_mut();
                s.installers.swap(i, i - 1);
                drop(s);
                rebuild_installer_list(&list_c, &ist_c);
            });
            btn_box.append(&up_btn);
        }
        if i + 1 < installers.len() {
            let down_btn = gtk4::Button::new();
            down_btn.set_icon_name("go-down-symbolic");
            down_btn.add_css_class(CSS_FLAT);
            down_btn.set_valign(gtk4::Align::Center);
            let ist_c = ist.clone();
            let list_c = list.clone();
            down_btn.connect_clicked(move |_| {
                let mut s = ist_c.borrow_mut();
                s.installers.swap(i, i + 1);
                drop(s);
                rebuild_installer_list(&list_c, &ist_c);
            });
            btn_box.append(&down_btn);
        }
        let rm_btn = gtk4::Button::new();
        rm_btn.set_icon_name("user-trash-symbolic");
        rm_btn.add_css_class(CSS_FLAT);
        rm_btn.set_valign(gtk4::Align::Center);
        let ist_c = ist.clone();
        let list_c = list.clone();
        rm_btn.connect_clicked(move |_| {
            ist_c.borrow_mut().installers.remove(i);
            rebuild_installer_list(&list_c, &ist_c);
        });
        btn_box.append(&rm_btn);
        row.add_suffix(&btn_box);
        list.append(&row);
    }
}

fn start_installation(wizard: &Rc<RefCell<Wizard>>, ist: &Rc<RefCell<InstallerState>>) {
    run_installer(wizard, ist, 0);
}

fn run_installer(wizard: &Rc<RefCell<Wizard>>, ist: &Rc<RefCell<InstallerState>>, index: usize) {
    let installer = ist.borrow().installers[index].clone();
    let itype = installer_type(&installer);
    let total = ist.borrow().installers.len();
    let name = installer
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("installer")
        .to_string();
    let status = crate::tr!("Running installer {}/{}: {}")
        .replacen("{}", &(index + 1).to_string(), 1)
        .replacen("{}", &total.to_string(), 1)
        .replacen("{}", &name, 1);
    set_status(wizard, &status);

    match itype {
        InstallerType::Windows => {
            if is_inno_setup(&installer) && innoextract_available() {
                run_silent_extraction(wizard, ist, index, ExtractionMethod::Inno);
            } else {
                run_wine_interactive(wizard, ist, index);
            }
        }
        InstallerType::Linux => {
            if is_gog_makeself(&installer) {
                run_silent_extraction(wizard, ist, index, ExtractionMethod::Gog);
            } else if is_linuxrulez(&installer)
                && ira_platforms::ysi_installer::is_ysi_installer(&installer)
            {
                run_silent_extraction(wizard, ist, index, ExtractionMethod::Ysi);
            } else {
                run_terminal_interactive(wizard, ist, index);
            }
        }
    }
}

enum ExtractionMethod {
    Inno,
    Gog,
    Ysi,
}

fn run_silent_extraction(
    wizard: &Rc<RefCell<Wizard>>,
    ist: &Rc<RefCell<InstallerState>>,
    index: usize,
    method: ExtractionMethod,
) {
    let installer = ist.borrow().installers[index].clone();
    let default_folder = ist.borrow().default_game_folder.clone();
    let stem = installer
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("game")
        .to_string();
    let dest = default_folder.join(&stem);

    let (tx, rx) = mpsc::channel::<WizardEvent>();
    let rx = Rc::new(RefCell::new(rx));

    std::thread::spawn(move || {
        let result = match method {
            ExtractionMethod::Inno => extract_inno(&installer, &dest).map(|_| ()),
            ExtractionMethod::Gog => {
                let split_tmp = dest.join(".gog_split");
                match split_gog_installer(&installer, &split_tmp) {
                    Ok(zip_path) => {
                        let r = extract_data_zip(&zip_path, &dest, |_, _| {});
                        let _ = std::fs::remove_dir_all(&split_tmp);
                        r
                    }
                    Err(e) => Err(e),
                }
            }
            ExtractionMethod::Ysi => {
                let progress: ira_platforms::ysi_installer::ProgressFn =
                    Box::new(move |_extracted, _total| {});
                let result = ira_platforms::ysi_installer::extract_ysi_installer(
                    &installer,
                    &dest,
                    Some(progress),
                    None,
                    None,
                );
                result.map(|_| ())
            }
        };
        match result {
            Ok(_) => {
                let _ = tx.send(WizardEvent::InstallDone);
            }
            Err(e) => {
                let _ = tx.send(WizardEvent::Failed(e));
            }
        }
    });

    let wizard_c = wizard.clone();
    let ist_c = ist.clone();
    glib::source::idle_add_local_full(glib::Priority::LOW, move || {
        match rx.borrow_mut().try_recv() {
            Ok(ev) => {
                match ev {
                    WizardEvent::InstallDone => {
                        on_installer_complete(&wizard_c, &ist_c, index, true);
                    }
                    WizardEvent::Failed(e) => {
                        show_error(&wizard_c, &e);
                        on_installer_complete(&wizard_c, &ist_c, index, false);
                    }
                    _ => {}
                }
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

fn run_wine_interactive(
    wizard: &Rc<RefCell<Wizard>>,
    ist: &Rc<RefCell<InstallerState>>,
    index: usize,
) {
    let installer = ist.borrow().installers[index].clone();
    let (profiles, profile_id, wow64, gamescope) = {
        let w = wizard.borrow();
        let s = ist.borrow();
        (w.profiles.clone(), s.profile_id, s.wow64, s.gamescope)
    };

    let mut wine = resolve_wine_config(&profiles, profile_id);
    wine.proton_wow64 = wow64;
    let wine_exe =
        ira_launcher::wine_launch::find_wine_binary(&wine.version, &wine.custom_wine_path)
            .unwrap_or_else(|_| "wine".to_string());
    let env = ira_launcher::wine_launch::build_wine_env(&wine, &wine_exe);

    let mut cmd = vec![wine_exe, installer.to_string_lossy().to_string()];
    if gamescope {
        let launch_cfg = ira_models::GameLaunchConfig {
            gamescope: Some(true),
            ..Default::default()
        };
        ira_launcher::env_builder::apply_performance(
            &mut cmd,
            &mut env.clone(),
            &launch_cfg,
            &wine,
        );
    }

    let mut command = std::process::Command::new(&cmd[0]);
    command.args(&cmd[1..]);
    for (k, v) in &env {
        command.env(k, v);
    }
    if let Err(e) = command.spawn() {
        show_error(
            wizard,
            &crate::tr!("Failed to start installer: {e}").replace("{e}", &e.to_string()),
        );
        on_installer_complete(wizard, ist, index, false);
        return;
    }

    show_interactive_done_page(wizard, ist, index);
}

fn run_terminal_interactive(
    wizard: &Rc<RefCell<Wizard>>,
    ist: &Rc<RefCell<InstallerState>>,
    index: usize,
) {
    let installer = ist.borrow().installers[index].clone();

    #[cfg(unix)]
    {
        let _ = std::fs::set_permissions(&installer, std::fs::Permissions::from_mode(0o755));
    }

    // Try running directly first — most Linux installers (LinuxRulez/YAD,
    // makeself, mojosetup) have their own GUI. If direct execution fails
    // immediately, fall back to a terminal emulator for TUI installers.
    match std::process::Command::new(&installer).spawn() {
        Ok(_) => {}
        Err(e) => {
            eprintln!("Direct execution failed ({}), trying terminal fallback", e);
            let installer_str = installer.to_string_lossy().to_string();
            if !spawn_terminal_fallback(&installer_str) {
                show_error(
                    wizard,
                    "Failed to start installer: no terminal emulator found",
                );
                on_installer_complete(wizard, ist, index, false);
                return;
            }
        }
    }

    show_interactive_done_page(wizard, ist, index);
}

/// Fallback: spawn a terminal emulator to run the installer.
fn spawn_terminal_fallback(cmd: &str) -> bool {
    let terminals: &[(&str, &[&str])] = &[
        ("gnome-terminal", &["--"]),
        ("konsole", &["-e"]),
        ("xfce4-terminal", &["-e"]),
        ("mate-terminal", &["--"]),
        ("alacritty", &["-e"]),
        ("kitty", &["-e"]),
        ("foot", &["-e"]),
        ("wezterm", &["start", "--"]),
        ("tilix", &["-e"]),
        ("qterminal", &["-e"]),
        ("lxterminal", &["-e"]),
        ("terminator", &["-e"]),
        ("xterm", &["-e"]),
    ];

    if let Ok(term) = std::env::var("TERMINAL") {
        let mut command = std::process::Command::new(&term);
        command.arg("-e").arg(cmd);
        if command.spawn().is_ok() {
            return true;
        }
    }

    for (term, args) in terminals {
        let mut command = std::process::Command::new(term);
        command.args(*args);
        command.arg(cmd);
        if command.spawn().is_ok() {
            return true;
        }
    }
    false
}

fn show_interactive_done_page(
    wizard: &Rc<RefCell<Wizard>>,
    ist: &Rc<RefCell<InstallerState>>,
    index: usize,
) {
    let content = wizard.borrow().content.clone();
    clear_children(&content);

    let status = adw::StatusPage::new();
    let total = ist.borrow().installers.len();
    let title = crate::tr!("Running installer {}/{}")
        .replacen("{}", &(index + 1).to_string(), 1)
        .replacen("{}", &total.to_string(), 1);
    status.set_title(&title);
    status.set_description(Some(&crate::tr!(
        "Click Done when the installer has finished."
    )));
    status.set_icon_name(Some("system-software-install-symbolic"));

    let done_btn = gtk4::Button::with_label(&crate::tr!("Done"));
    done_btn.add_css_class(CSS_SUGGESTED_ACTION);
    done_btn.set_halign(gtk4::Align::Center);

    let wizard_c = wizard.clone();
    let ist_c = ist.clone();
    done_btn.connect_clicked(move |_| {
        on_installer_complete(&wizard_c, &ist_c, index, true);
    });
    status.set_child(Some(&done_btn));
    content.append(&status);
}

fn on_installer_complete(
    wizard: &Rc<RefCell<Wizard>>,
    ist: &Rc<RefCell<InstallerState>>,
    index: usize,
    success: bool,
) {
    let content = wizard.borrow().content.clone();
    let total = ist.borrow().installers.len();
    let is_last = index + 1 >= total;
    let installer_name = ist.borrow().installers[index]
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("installer")
        .to_string();

    let title = if success {
        if is_last {
            crate::tr!("Installation complete")
        } else {
            crate::tr!("Installer finished")
        }
    } else {
        crate::tr!("Installer failed")
    };

    clear_children(&content);
    let status = adw::StatusPage::new();
    status.set_title(&title);
    status.set_description(
        Some(
            &crate::tr!("{} ({} of {})")
                .replacen("{}", &installer_name, 1)
                .replacen("{}", &(index + 1).to_string(), 1)
                .replacen("{}", &total.to_string(), 1),
        ),
    );
    status.set_icon_name(Some("system-software-install-symbolic"));

    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::Center);

    let cancel_btn = gtk4::Button::with_label(&crate::tr!("Cancel"));
    cancel_btn.add_css_class(CSS_FLAT);
    {
        let wizard_c = wizard.clone();
        cancel_btn.connect_clicked(move |_| wizard_c.borrow().win.close());
    }
    btn_row.append(&cancel_btn);

    if !success {
        let retry_btn = gtk4::Button::with_label(&crate::tr!("Retry"));
        retry_btn.add_css_class(CSS_SUGGESTED_ACTION);
        {
            let wizard_c = wizard.clone();
            let ist_c = ist.clone();
            retry_btn.connect_clicked(move |_| run_installer(&wizard_c, &ist_c, index));
        }
        btn_row.append(&retry_btn);

        let skip_btn = gtk4::Button::with_label(&crate::tr!("Skip"));
        skip_btn.add_css_class(CSS_FLAT);
        {
            let wizard_c = wizard.clone();
            let ist_c = ist.clone();
            skip_btn
                .connect_clicked(move |_| advance_after_complete(&wizard_c, &ist_c, index, is_last));
        }
        btn_row.append(&skip_btn);
    }

    let (advance_label, next_index) = if is_last {
        (crate::tr!("Continue"), None)
    } else {
        (crate::tr!("Next"), Some(index + 1))
    };
    let advance_btn = gtk4::Button::with_label(&advance_label);
    if success {
        advance_btn.add_css_class(CSS_SUGGESTED_ACTION);
    }
    {
        let wizard_c = wizard.clone();
        let ist_c = ist.clone();
        advance_btn.connect_clicked(move |_| match next_index {
            Some(next) => run_installer(&wizard_c, &ist_c, next),
            None => start_post_install(&wizard_c, &ist_c),
        });
    }
    btn_row.append(&advance_btn);

    status.set_child(Some(&btn_row));
    content.append(&status);
}

/// Skip/Next/Continue all move past the finished installer: on to the next
/// one, or to install detection when it was the last.
fn advance_after_complete(
    wizard: &Rc<RefCell<Wizard>>,
    ist: &Rc<RefCell<InstallerState>>,
    index: usize,
    is_last: bool,
) {
    if is_last {
        start_post_install(wizard, ist);
    } else {
        run_installer(wizard, ist, index + 1);
    }
}

fn start_post_install(wizard: &Rc<RefCell<Wizard>>, ist: &Rc<RefCell<InstallerState>>) {
    let default_folder = ist.borrow().default_game_folder.clone();
    if default_folder.as_os_str().is_empty() || !default_folder.exists() {
        pick_install_folder(wizard, ist);
        return;
    }

    let before = ist.borrow().pre_snapshot.clone();
    let new_dirs = detect_new_subdirs(&before, &default_folder);

    match new_dirs.len() {
        0 => pick_install_folder(wizard, ist),
        1 => {
            let folder = default_folder.join(&new_dirs[0]);
            let final_folder = flatten_linuxrulez_if_needed(wizard, &folder);
            ist.borrow_mut().detected_folder = Some(final_folder.clone());
            start_identify_from_install(wizard, ist, final_folder);
        }
        _ => pick_from_multiple(wizard, ist, new_dirs),
    }
}

/// LinuxRulez installers create a nested `game/game/` structure where the
/// actual game files live. If detected, flatten it by moving `game/game/*`
/// to a clean folder and renaming the original to `<name>.lrz`.
/// Returns the final game folder path (may be the same as input if no
/// flattening was needed).
fn flatten_linuxrulez_if_needed(wizard: &Rc<RefCell<Wizard>>, folder: &Path) -> PathBuf {
    let game_game = folder.join("game").join("game");
    if !game_game.is_dir() {
        return folder.to_path_buf();
    }

    let parent = folder.parent().unwrap_or(folder);
    let name = folder
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("game");
    let lrz_dir = parent.join(format!("{name}.lrz"));
    let clean_dir = parent.join(name);

    // Rename original folder to <name>.lrz
    if let Err(e) = std::fs::rename(folder, &lrz_dir) {
        eprintln!(
            "Failed to rename {} to {:?}: {}",
            folder.display(),
            lrz_dir,
            e
        );
        return folder.to_path_buf();
    }

    // Create clean folder and move game/game/* into it
    if let Err(e) = std::fs::create_dir_all(&clean_dir) {
        eprintln!("Failed to create {:?}: {}", clean_dir, e);
        // Rename back
        let _ = std::fs::rename(&lrz_dir, folder);
        return folder.to_path_buf();
    }

    let entries = match std::fs::read_dir(&game_game) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Failed to read game/game: {}", e);
            return clean_dir;
        }
    };

    for entry in entries.flatten() {
        let src = entry.path();
        let dst = clean_dir.join(entry.file_name());
        if let Err(e) = std::fs::rename(&src, &dst) {
            eprintln!("Failed to move {:?} \u{2192} {:?}: {}", src, dst, e);
        }
    }

    // Prompt user about deleting the .lrz folder
    let win = wizard.borrow().win.clone();
    let lrz_name = lrz_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("folder")
        .to_string();
    let alert = adw::AlertDialog::new(
        Some(&crate::tr!("Clean up installer files?")),
        Some(&crate::tr!("Game files were moved to a clean folder.\n\nDelete the leftover \"{lrz_name}\" folder (contains umu wrapper, desktop shortcuts, etc.)?")
            .replace("{lrz_name}", &lrz_name)),
    );
    alert.add_response("keep", &crate::tr!("Keep"));
    alert.add_response("delete", &crate::tr!("Delete"));
    alert.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    alert.set_default_response(Some("delete"));
    alert.set_close_response("keep");

    let lrz_dir_c = lrz_dir;
    alert.choose(
        Some(win.as_widget()),
        None::<&gtk4::gio::Cancellable>,
        move |response| {
            if response == "delete" {
                if let Err(e) = std::fs::remove_dir_all(&lrz_dir_c) {
                    eprintln!("Failed to delete {:?}: {}", lrz_dir_c, e);
                }
            }
        },
    );

    clean_dir
}

fn pick_install_folder(wizard: &Rc<RefCell<Wizard>>, ist: &Rc<RefCell<InstallerState>>) {
    let win = wizard.borrow().win.clone();
    let default_folder = ist.borrow().default_game_folder.clone();
    let dialog = gtk4::FileDialog::new();
    dialog.set_title(&crate::tr!("Select installed game folder"));
    super::helpers::set_initial_folder(&dialog, &default_folder.to_string_lossy());
    let wizard_c = wizard.clone();
    let ist_c = ist.clone();
    let Some(host) = super::helpers::hosting_window(win.as_widget()) else {
        return;
    };
    dialog.select_folder(Some(&host), None::<&gtk4::gio::Cancellable>, move |result| {
        if let Ok(file) = result {
            if let Some(path) = file.path() {
                ist_c.borrow_mut().detected_folder = Some(path.clone());
                start_identify_from_install(&wizard_c, &ist_c, path);
            }
        }
    });
}

fn pick_from_multiple(
    wizard: &Rc<RefCell<Wizard>>,
    ist: &Rc<RefCell<InstallerState>>,
    dirs: Vec<String>,
) {
    let content = wizard.borrow().content.clone();
    clear_children(&content);

    let status = adw::StatusPage::new();
    status.set_title(&crate::tr!("Multiple folders detected"));
    status.set_description(Some(&crate::tr!(
        "Select the folder where the game was installed."
    )));
    content.append(&status);

    let list = gtk4::ListBox::new();
    list.add_css_class(CSS_BOXED_LIST);
    list.set_selection_mode(gtk4::SelectionMode::Single);

    for (idx, dir) in dirs.iter().enumerate() {
        let row = adw::ActionRow::new();
        row.set_title(&esc(dir));
        row.set_activatable(true);
        list.append(&row);
        let _ = idx;
    }

    let wizard_c = wizard.clone();
    let ist_c = ist.clone();
    let dirs_c = dirs;
    list.connect_row_activated(move |_list, row| {
        let idx = row.index() as usize;
        if let Some(dir) = dirs_c.get(idx) {
            let folder = ist_c.borrow().default_game_folder.join(dir);
            ist_c.borrow_mut().detected_folder = Some(folder.clone());
            start_identify_from_install(&wizard_c, &ist_c, folder);
        }
    });
    content.append(&list);
}

fn start_identify_from_install(
    wizard: &Rc<RefCell<Wizard>>,
    ist: &Rc<RefCell<InstallerState>>,
    folder: PathBuf,
) {
    let (db, steam) = {
        let w = wizard.borrow();
        let s = w.state.borrow();
        (s.db.clone(), s.steam.clone())
    };
    set_status(wizard, &crate::tr!("Identifying game…"));

    let (tx, rx) = mpsc::channel::<WizardEvent>();
    let rx = Rc::new(RefCell::new(rx));
    spawn_identify_thread(tx, folder, None, db, steam);

    let wizard_c = wizard.clone();
    let ist_c = ist.clone();
    glib::source::idle_add_local_full(glib::Priority::LOW, move || {
        match rx.borrow_mut().try_recv() {
            Ok(ev) => {
                let terminal = !matches!(ev, WizardEvent::Status(_));
                handle_installer_identify_event(&wizard_c, &ist_c, ev);
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

fn handle_installer_identify_event(
    wizard: &Rc<RefCell<Wizard>>,
    ist: &Rc<RefCell<InstallerState>>,
    ev: WizardEvent,
) {
    match ev {
        WizardEvent::Status(msg) => set_status(wizard, &msg),
        WizardEvent::AlreadyExists => {
            show_error(wizard, "This folder is already in your library.");
        }
        WizardEvent::Failed(e) => {
            show_error(wizard, &e);
            show_manual_fallback(wizard, ist);
        }
        WizardEvent::NeedSteamSearch { .. } => show_manual_fallback(wizard, ist),
        WizardEvent::Identified(mut game) => {
            if ist.borrow().game_platform == GamePlatform::Windows {
                game.is_windows = true;
            }
            let profile_id = ist.borrow().profile_id;
            show_identified_form(wizard, *game, profile_id, true);
        }
        _ => {}
    }
}

/// The installer flow keeps the manual entry fallback: present the identified
/// form with an empty Steam app ID so non-Steam games can still be added.
fn show_manual_fallback(wizard: &Rc<RefCell<Wizard>>, ist: &Rc<RefCell<InstallerState>>) {
    let folder = ist.borrow().detected_folder.clone();
    let name = folder
        .as_ref()
        .and_then(|f| f.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("Game")
        .to_string();
    let is_windows = ist.borrow().game_platform == GamePlatform::Windows;
    if let Some(folder) = folder {
        let game = IdentifiedGame {
            app_id: String::new(),
            name,
            is_windows,
            game_folder: folder,
            exe: String::new(),
            variants: Vec::new(),
            logo_position: String::new(),
            logo_size: 0,
        };
        let profile_id = ist.borrow().profile_id;
        show_identified_form(wizard, game, profile_id, true);
    }
}

fn snapshot_subdirs(folder: &Path) -> Vec<String> {
    std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn detect_new_subdirs(before: &[String], folder: &Path) -> Vec<String> {
    let current = snapshot_subdirs(folder);
    current
        .into_iter()
        .filter(|d| !before.contains(d))
        .collect()
}
