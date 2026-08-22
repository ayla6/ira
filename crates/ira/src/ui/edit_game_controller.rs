use super::css::CSS_SQUARE_BUTTON;
use super::input_profile_store::{
    add_game_compatibility, list_profiles, profile_matches_game, read_profile, StoredProfile,
};
use super::settings_dialog;
use super::state::SharedState;
use adw::prelude::*;
use glib::clone::Downgrade;
use ira_input::VirtualGamepadBackend;
use ira_models::{ControllerInputMode, Game, GameLaunchConfig};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

/// Quick access to a game's controller layout (game header button, context
/// menu): opens the input profile editor on the associated layout, or seeds a
/// new one bound to this game when none is associated yet. Saving from here
/// also enables ira-input for the game and points it at the saved layout.
pub(super) fn open_controller_settings(state: &SharedState, game: &Game) {
    let (window, save_dir, registry, associated) = {
        let s = state.borrow();
        let associated = ira_db::get_game_config(&s.db, game.db_id)
            .ok()
            .flatten()
            .and_then(|(launch, _, _)| launch.input_profile)
            .map(PathBuf::from);
        (
            s.window.clone(),
            s.save_dir.clone(),
            s.controller_registry.clone(),
            associated,
        )
    };
    let profile_path = associated.and_then(|path| {
        if path.is_file() {
            Some(path)
        } else {
            eprintln!(
                "Associated controller profile not found, starting a new layout: {}",
                path.display()
            );
            None
        }
    });
    let state_for_saved = state.clone();
    let game_for_saved = game.clone();
    super::input_profile_editor::show_input_profile_editor(
        window.upcast_ref(),
        super::input_profile_editor::InputProfileEditorParams {
            save_dir,
            profile_path,
            game_id: Some(game.db_id),
            layout_name: Some(game.name.clone()),
            registry,
            device: None,
        },
        move |path| {
            enable_input_for_game(&state_for_saved, &game_for_saved, &path);
        },
    );
}

/// Associate `path` with the game and switch its input mode on so the saved
/// layout is the one the launcher starts.
fn enable_input_for_game(state: &SharedState, game: &Game, path: &Path) {
    if let Err(error) = add_game_compatibility(path, game.db_id) {
        eprintln!("Failed to associate controller profile with game: {error}");
    }
    let mode = read_profile(path)
        .ok()
        .map(|profile| input_mode_for_backend(profile.backend));
    let db = state.borrow().db.clone();
    match ira_db::get_game_config(&db, game.db_id) {
        Ok(Some((mut launch, wine, profile_id))) => {
            launch.input_profile = Some(path.to_string_lossy().into_owned());
            if let Some(mode) = mode {
                launch.input_mode = Some(mode);
            }
            if let Err(error) =
                ira_db::save_game_config(&db, game.db_id, &launch, &wine, profile_id)
            {
                eprintln!("Failed to enable controller input for game: {error}");
            }
        }
        Ok(None) => {
            let launch = GameLaunchConfig {
                input_profile: Some(path.to_string_lossy().into_owned()),
                input_mode: mode,
                ..GameLaunchConfig::default()
            };
            if let Err(error) = ira_db::save_game_config(
                &db,
                game.db_id,
                &launch,
                &ira_models::WineConfig::default(),
                None,
            ) {
                eprintln!("Failed to enable controller input for game: {error}");
            }
        }
        Err(error) => eprintln!("Failed to read game config for input enable: {error}"),
    }
}

fn input_mode_for_backend(backend: VirtualGamepadBackend) -> ControllerInputMode {
    match backend {
        VirtualGamepadBackend::XInput => ControllerInputMode::VirtualXInput,
        VirtualGamepadBackend::DirectInput => ControllerInputMode::VirtualDirectInput,
        VirtualGamepadBackend::SwitchPro => ControllerInputMode::VirtualSwitchPro,
    }
}

#[derive(Clone)]
pub(super) struct ControllerWidgets {
    pub input_mode: Rc<RefCell<Option<ControllerInputMode>>>,
    pub input_profile_row: adw::ComboRow,
    pub input_profile_paths: Rc<RefCell<Vec<Option<PathBuf>>>>,
    pub pause_unfocused: Rc<RefCell<Option<bool>>>,
    pub motion_udp: Rc<RefCell<Option<bool>>>,
}

pub(super) struct ControllerPageParams<'a> {
    pub launch: &'a GameLaunchConfig,
    pub game: &'a Game,
    pub save_dir: &'a str,
    pub sidebar: &'a gtk4::ListBox,
    pub stack: &'a gtk4::Stack,
    pub registry: Arc<ira_input::ControllerRegistry>,
}

pub(super) fn build_controller_page(params: ControllerPageParams) -> ControllerWidgets {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    page.set_margin_start(8);
    page.set_margin_end(8);
    page.set_margin_top(8);
    page.set_margin_bottom(8);
    let initial_mode = params.launch.input_mode;
    let input_mode = Rc::new(RefCell::new(initial_mode));
    let input_mode_row = adw::ComboRow::new();
    input_mode_row.set_title(&crate::tr!("Input remapping"));
    let mode_strings = [
        crate::tr!("Inherit"),
        crate::tr!("Disabled"),
        crate::tr!("Virtual XInput"),
        crate::tr!("Virtual DirectInput"),
        crate::tr!("Nintendo Switch Pro Controller"),
    ];
    let mode_refs = mode_strings.iter().map(String::as_str).collect::<Vec<_>>();
    input_mode_row.set_model(Some(&gtk4::StringList::new(&mode_refs)));
    input_mode_row.set_selected(input_mode_index(initial_mode));
    let input_group = adw::PreferencesGroup::new();
    input_group.add(&input_mode_row);

    let pause_unfocused = Rc::new(RefCell::new(params.launch.input_pause_unfocused));
    let pause_row = adw::SwitchRow::builder()
        .title(crate::tr!("Pause when the game loses focus"))
        .subtitle(crate::tr!(
            "Suspend controller input and gyro while another window is focused"
        ))
        .active(params.launch.input_pause_unfocused.unwrap_or(true))
        .build();
    {
        let pause_cell = pause_unfocused.clone();
        pause_row.connect_active_notify(move |row| {
            *pause_cell.borrow_mut() = Some(row.is_active());
        });
    }
    input_group.add(&pause_row);

    let motion_udp = Rc::new(RefCell::new(params.launch.input_motion_udp));
    let motion_row = adw::SwitchRow::builder()
        .title(crate::tr!("Emulator motion stream"))
        .subtitle(crate::tr!(
            "Broadcast the raw gyro over cemuhook UDP for emulators with native motion support"
        ))
        .active(params.launch.input_motion_udp.unwrap_or(true))
        .build();
    {
        let motion_cell = motion_udp.clone();
        motion_row.connect_active_notify(move |row| {
            *motion_cell.borrow_mut() = Some(row.is_active());
        });
    }
    input_group.add(&motion_row);

    let current_path = params.launch.input_profile.as_deref().map(PathBuf::from);
    let (labels, paths, selected) =
        profile_choices(params.save_dir, params.game.db_id, current_path);
    let input_profile_row = adw::ComboRow::new();
    input_profile_row.set_title(&crate::tr!("Current layout"));
    let refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
    input_profile_row.set_model(Some(&gtk4::StringList::new(&refs)));
    input_profile_row.set_selected(selected);
    let input_profile_paths = Rc::new(RefCell::new(paths));
    let last_real = Rc::new(RefCell::new(selected));

    let mode_for_notify = input_mode.clone();
    let mode_row_for_notify = input_profile_row.clone();
    let mode_paths_for_notify = input_profile_paths.clone();
    let mode_save_dir = params.save_dir.to_string();
    let mode_game_id = params.game.db_id;
    let last_real_for_mode = last_real.clone();
    input_mode_row.connect_selected_notify(move |row| {
        let mode = input_mode_from_index(row.selected());
        *mode_for_notify.borrow_mut() = mode;
        let current = selected_path(&mode_row_for_notify, &mode_paths_for_notify);
        refresh_profile_choices(
            &mode_row_for_notify,
            &mode_paths_for_notify,
            &mode_save_dir,
            mode_game_id,
            current.as_deref(),
            &last_real_for_mode,
        );
    });

    let monitor_button = icon_button("input-gaming-symbolic", &crate::tr!("Monitor this layout"));
    let edit_button = icon_button("document-edit-symbolic", &crate::tr!("Edit profile"));
    monitor_button.set_sensitive(selected != 0);
    edit_button.set_sensitive(selected != 0);
    input_profile_row.add_suffix(&edit_button);
    input_profile_row.add_suffix(&monitor_button);

    let stack_for_monitor = Downgrade::downgrade(params.stack);
    let registry_for_monitor = params.registry.clone();
    let profile_row_for_monitor = Downgrade::downgrade(&input_profile_row);
    let profile_paths_for_monitor = input_profile_paths.clone();
    monitor_button.connect_clicked(move |_| {
        let Some(profile_row_for_monitor) = profile_row_for_monitor.upgrade() else {
            return;
        };
        let Some(path) = selected_path(&profile_row_for_monitor, &profile_paths_for_monitor) else {
            return;
        };
        let Some(stack_for_monitor) = stack_for_monitor.upgrade() else {
            return;
        };
        let Some(window) = stack_for_monitor
            .root()
            .and_then(|root| root.downcast::<gtk4::Window>().ok())
        else {
            return;
        };
        super::input_profile_viewer::show_input_profile_viewer(
            &window,
            &path,
            registry_for_monitor.clone(),
        );
    });

    let stack_for_edit = Downgrade::downgrade(params.stack);
    let profile_row_for_edit = Downgrade::downgrade(&input_profile_row);
    let profile_paths_for_edit = input_profile_paths.clone();
    let save_dir_for_edit = params.save_dir.to_string();
    let game_id_for_edit = params.game.db_id;
    let game_name_for_edit = params.game.name.clone();
    let registry_for_edit = params.registry.clone();
    let last_real_for_edit = last_real.clone();
    edit_button.connect_clicked(move |_| {
        let Some(profile_row_for_edit) = profile_row_for_edit.upgrade() else {
            return;
        };
        let Some(path) = selected_path(&profile_row_for_edit, &profile_paths_for_edit) else {
            return;
        };
        let Some(stack_for_edit) = stack_for_edit.upgrade() else {
            return;
        };
        let Some(window) = stack_for_edit
            .root()
            .and_then(|root| root.downcast::<gtk4::Window>().ok())
        else {
            return;
        };
        super::input_profile_editor::show_input_profile_editor(
            &window,
            super::input_profile_editor::InputProfileEditorParams {
                save_dir: save_dir_for_edit.clone(),
                profile_path: Some(path),
                game_id: Some(game_id_for_edit),
                layout_name: Some(game_name_for_edit.clone()),
                registry: registry_for_edit.clone(),
                device: None,
            },
            {
                let row = profile_row_for_edit.clone();
                let paths = profile_paths_for_edit.clone();
                let save_dir = save_dir_for_edit.clone();
                let last_real = last_real_for_edit.clone();
                move |saved| {
                    refresh_profile_choices(
                        &row,
                        &paths,
                        &save_dir,
                        game_id_for_edit,
                        Some(&saved),
                        &last_real,
                    )
                }
            },
        );
    });

    let last_real_for_new = last_real.clone();
    let new_profile_cb: Rc<dyn Fn()> = {
        let stack = Downgrade::downgrade(params.stack);
        let row = Downgrade::downgrade(&input_profile_row);
        let paths = input_profile_paths.clone();
        let save_dir = params.save_dir.to_string();
        let game_id = params.game.db_id;
        let game_name = params.game.name.clone();
        let registry = params.registry.clone();
        Rc::new(move || {
            let Some(row) = row.upgrade() else {
                return;
            };
            let Some(stack) = stack.upgrade() else {
                return;
            };
            let Some(window) = stack
                .root()
                .and_then(|root| root.downcast::<gtk4::Window>().ok())
            else {
                return;
            };
            let row = row.clone();
            let paths = paths.clone();
            let save_dir = save_dir.clone();
            let callback_save_dir = save_dir.clone();
            let last_real = last_real_for_new.clone();
            super::input_profile_editor::show_input_profile_editor(
                &window,
                super::input_profile_editor::InputProfileEditorParams {
                    save_dir,
                    profile_path: None,
                    game_id: Some(game_id),
                    layout_name: Some(game_name.clone()),
                    registry: registry.clone(),
                    device: None,
                },
                move |saved| {
                    refresh_profile_choices(
                        &row,
                        &paths,
                        &callback_save_dir,
                        game_id,
                        Some(&saved),
                        &last_real,
                    )
                },
            );
        })
    };

    let paths_for_sensitivity = input_profile_paths.clone();
    let last_real_for_notify = last_real;
    let new_profile_for_notify = new_profile_cb.clone();
    input_profile_row.connect_selected_notify({
        move |row| {
            let sentinel = paths_for_sensitivity.borrow().len().saturating_sub(1) as u32;
            let selected = row.selected();
            if selected == sentinel {
                let row_for_revert = row.clone();
                let previous = *last_real_for_notify.borrow();
                let create = new_profile_for_notify.clone();
                glib::idle_add_local(move || {
                    row_for_revert.set_selected(previous);
                    create();
                    glib::ControlFlow::Break
                });
                return;
            }
            *last_real_for_notify.borrow_mut() = selected;
            let has_profile = paths_for_sensitivity
                .borrow()
                .get(selected as usize)
                .and_then(Clone::clone)
                .is_some();
            edit_button.set_sensitive(has_profile);
            monitor_button.set_sensitive(has_profile);
        }
    });
    input_group.add(&input_profile_row);
    page.append(&input_group);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&page));
    scroll.set_vexpand(true);
    scroll.set_hexpand(true);
    params
        .sidebar
        .append(&settings_dialog::settings_sidebar_row(
            "input-gaming-symbolic",
            "Controller",
            "controller",
        ));
    params.stack.add_named(&scroll, Some("controller"));

    ControllerWidgets {
        input_mode,
        input_profile_row,
        input_profile_paths,
        pause_unfocused,
        motion_udp,
    }
}

fn selected_path(
    row: &adw::ComboRow,
    paths: &Rc<RefCell<Vec<Option<PathBuf>>>>,
) -> Option<PathBuf> {
    paths
        .borrow()
        .get(row.selected() as usize)
        .and_then(Clone::clone)
}

fn profile_choices(
    save_dir: &str,
    game_id: i64,
    current_path: Option<PathBuf>,
) -> (Vec<String>, Vec<Option<PathBuf>>, u32) {
    let profiles = list_profiles(save_dir).unwrap_or_else(|error| {
        eprintln!("Failed to list controller profiles: {error}");
        Vec::new()
    });
    let mut labels = vec![crate::tr!("No profile")];
    let mut paths = vec![None];
    for stored in profiles {
        if profile_matches_game(&stored.profile, game_id) {
            labels.push(profile_label(&stored));
            paths.push(Some(stored.path));
        }
    }
    let selected = current_path
        .as_ref()
        .and_then(|path| {
            paths
                .iter()
                .position(|candidate| candidate.as_ref() == Some(path))
        })
        .or_else(|| {
            current_path
                .as_ref()
                .filter(|path| path.is_file())
                .filter(|path| read_profile(path).is_ok())
                .map(|path| {
                    labels.push(crate::tr!("{} (current)").replacen("{}", &path_label(path), 1));
                    paths.push(Some(path.clone()));
                    paths.len() - 1
                })
        })
        .unwrap_or(0);
    labels.push(crate::tr!("Create new profile…"));
    paths.push(None);
    (labels, paths, selected as u32)
}

fn refresh_profile_choices(
    row: &adw::ComboRow,
    paths: &Rc<RefCell<Vec<Option<PathBuf>>>>,
    save_dir: &str,
    game_id: i64,
    selected_path: Option<&Path>,
    last_real: &Rc<RefCell<u32>>,
) {
    let (labels, profile_paths, selected) =
        profile_choices(save_dir, game_id, selected_path.map(Path::to_path_buf));
    let refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
    *paths.borrow_mut() = profile_paths;
    row.set_model(Some(&gtk4::StringList::new(&refs)));
    row.set_selected(selected);
    *last_real.borrow_mut() = selected;
}

fn profile_label(stored: &StoredProfile) -> String {
    if stored.profile.name.trim().is_empty() {
        path_label(&stored.path)
    } else {
        stored.profile.name.clone()
    }
}

fn path_label(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("Unnamed")
        .to_string()
}

fn input_mode_index(mode: Option<ControllerInputMode>) -> u32 {
    match mode {
        None => 0,
        Some(ControllerInputMode::Disabled) => 1,
        Some(ControllerInputMode::VirtualXInput) => 2,
        Some(ControllerInputMode::VirtualDirectInput) => 3,
        Some(ControllerInputMode::VirtualSwitchPro) => 4,
    }
}

fn input_mode_from_index(index: u32) -> Option<ControllerInputMode> {
    match index {
        0 => None,
        1 => Some(ControllerInputMode::Disabled),
        2 => Some(ControllerInputMode::VirtualXInput),
        3 => Some(ControllerInputMode::VirtualDirectInput),
        4 => Some(ControllerInputMode::VirtualSwitchPro),
        _ => None,
    }
}

fn icon_button(icon: &str, tooltip: &str) -> gtk4::Button {
    let button = gtk4::Button::from_icon_name(icon);
    button.add_css_class(super::css::CSS_FLAT);
    button.add_css_class(CSS_SQUARE_BUTTON);
    button.set_tooltip_text(Some(tooltip));
    button.set_valign(gtk4::Align::Center);
    button
}

#[cfg(test)]
mod tests {
    use super::{input_mode_from_index, input_mode_index};
    use ira_models::ControllerInputMode;

    #[test]
    fn test_input_mode_index_round_trip() {
        for mode in [
            None,
            Some(ControllerInputMode::Disabled),
            Some(ControllerInputMode::VirtualXInput),
            Some(ControllerInputMode::VirtualDirectInput),
            Some(ControllerInputMode::VirtualSwitchPro),
        ] {
            assert_eq!(input_mode_from_index(input_mode_index(mode)), mode);
        }
    }
}
