use super::css::CSS_FLAT;
use super::input_profile_store::controller_default_path_for_backend;
use super::input_profile_store::{
    copy_controller_default, ensure_controller_default_profile, list_profiles, StoredProfile,
};
use adw::prelude::*;
use ira_config::{Config, ControllerInputConfig};
use ira_models::ControllerInputMode;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

fn settings_page_container() -> gtk4::Box {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    page.set_margin_start(12);
    page.set_margin_end(12);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page
}

pub(super) struct InputPageWidgets {
    pub controller_defaults: Rc<RefCell<Vec<ControllerDefaultWidgets>>>,
}

pub(super) struct ConsoleProfileWidgets {
    pub console_id: String,
    pub profile_row: adw::ComboRow,
    pub profile_paths: Vec<Option<std::path::PathBuf>>,
}

#[derive(Clone)]
pub(super) struct ControllerDefaultWidgets {
    pub key: String,
    pub device_name: String,
    pub supported_buttons: Vec<ira_input::GamepadButton>,
    pub mode: adw::ComboRow,
    pub profile_path: Rc<RefCell<Option<std::path::PathBuf>>>,
    row: adw::ExpanderRow,
}

#[derive(Clone)]
struct ControllerDefaultState {
    config: ControllerInputConfig,
    profile_path: Option<std::path::PathBuf>,
}

#[derive(Clone)]
struct ControllerRowsParams {
    group: adw::PreferencesGroup,
    parent: adw::Window,
    save_dir: String,
    configured_defaults: HashMap<String, ControllerInputConfig>,
    widgets: Rc<RefCell<Vec<ControllerDefaultWidgets>>>,
    no_controllers_row: Rc<RefCell<Option<adw::ActionRow>>>,
    registry: std::sync::Arc<ira_input::ControllerRegistry>,
}

pub(super) fn build_input_settings_page(
    parent: &adw::Window,
    save_dir: &str,
    cfg: &Config,
    registry: std::sync::Arc<ira_input::ControllerRegistry>,
) -> (gtk4::Box, InputPageWidgets) {
    let page = settings_page_container();
    let profiles = list_profiles(save_dir).unwrap_or_else(|error| {
        eprintln!("Failed to list controller profiles: {error}");
        Vec::new()
    });
    let profiles_group = adw::PreferencesGroup::new();
    profiles_group.set_title("Profiles");
    for stored in &profiles {
        add_profile_row(
            &profiles_group,
            parent,
            save_dir,
            stored.clone(),
            registry.clone(),
        );
    }

    let monitor_group = adw::PreferencesGroup::new();
    monitor_group.set_title("Tools");
    let monitor_row = adw::ActionRow::new();
    monitor_row.set_title("Input monitor");
    let monitor_button = icon_button("input-gaming-symbolic", "Monitor input");
    monitor_row.add_suffix(&monitor_button);
    let parent_for_monitor = parent.clone();
    let registry_for_monitor = registry.clone();
    monitor_button.connect_clicked(move |_| {
        super::input_monitor_dialog::show_input_monitor_dialog(
            parent_for_monitor.upcast_ref(),
            registry_for_monitor.clone(),
        );
    });
    monitor_group.add(&monitor_row);

    let controller_group = adw::PreferencesGroup::new();
    controller_group.set_title("Controller defaults");
    let controller_defaults = Rc::new(RefCell::new(Vec::new()));
    let no_controllers_row = Rc::new(RefCell::new(None));
    let configured_defaults = cfg
        .controller_defaults
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<HashMap<_, _>>();
    let controller_rows_params = ControllerRowsParams {
        group: controller_group.clone(),
        parent: parent.clone(),
        save_dir: save_dir.to_string(),
        configured_defaults,
        widgets: controller_defaults.clone(),
        no_controllers_row: no_controllers_row.clone(),
        registry: registry.clone(),
    };
    rebuild_controller_rows(&controller_rows_params, &registry.snapshot());
    start_controller_registry_refresh(
        parent,
        &controller_group,
        save_dir,
        registry,
        controller_rows_params.configured_defaults.clone(),
        controller_defaults.clone(),
        no_controllers_row,
    );
    page.append(&controller_group);
    page.append(&monitor_group);
    page.append(&profiles_group);

    (
        page,
        InputPageWidgets {
            controller_defaults,
        },
    )
}

pub(super) fn add_console_profile_group(
    page: &gtk4::Box,
    cfg: &Config,
    save_dir: &str,
    console_id: &str,
    label: &str,
) -> ConsoleProfileWidgets {
    let group = adw::PreferencesGroup::new();
    group.set_title("Controller");
    let profiles = list_profiles(save_dir).unwrap_or_else(|error| {
        eprintln!("Failed to list controller profiles: {error}");
        Vec::new()
    });
    let widget = add_console_profile_row(
        &group,
        cfg,
        &profiles,
        console_id.to_string(),
        label.to_string(),
    );
    page.append(&group);
    widget
}

fn add_console_profile_row(
    group: &adw::PreferencesGroup,
    cfg: &Config,
    profiles: &[StoredProfile],
    console_id: String,
    label: String,
) -> ConsoleProfileWidgets {
    let profile_row = adw::ComboRow::new();
    profile_row.set_title("Default layout");
    profile_row.set_subtitle(&format!("Use controller default for {label}"));
    let mut labels = vec!["Use controller default".to_string()];
    let mut profile_paths = vec![None];
    for profile in profiles
        .iter()
        .filter(|profile| profile.profile.compatible_game_ids.is_empty())
    {
        labels.push(profile_label(profile));
        profile_paths.push(Some(profile.path.clone()));
    }
    let selected = profile_paths
        .iter()
        .position(|path| {
            path.as_ref().is_some_and(|path| {
                path == std::path::Path::new(&cfg.console(&console_id).controller_profile)
            })
        })
        .unwrap_or(0);
    let refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
    profile_row.set_model(Some(&gtk4::StringList::new(&refs)));
    profile_row.set_selected(selected as u32);
    group.add(&profile_row);
    ConsoleProfileWidgets {
        console_id,
        profile_row,
        profile_paths,
    }
}

fn rebuild_controller_rows(params: &ControllerRowsParams, devices: &[ira_input::DeviceInfo]) {
    let previous = params
        .widgets
        .borrow()
        .iter()
        .map(|widget| {
            (
                widget.key.clone(),
                ControllerDefaultState {
                    config: ControllerInputConfig {
                        mode: mode_from_selection(widget.mode.selected()),
                        profile: widget
                            .profile_path
                            .borrow()
                            .as_ref()
                            .map(|path| path.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                    },
                    profile_path: widget.profile_path.borrow().clone(),
                },
            )
        })
        .collect::<HashMap<_, _>>();
    clear_controller_rows(&params.group, &params.widgets, &params.no_controllers_row);
    let mut rebuilt = Vec::new();
    for device in devices {
        let key = ira_config::Config::controller_key(device.vendor, device.product);
        let config = params
            .configured_defaults
            .get(&key)
            .cloned()
            .unwrap_or_default();
        let configured =
            (!config.profile.is_empty()).then(|| std::path::PathBuf::from(&config.profile));
        let default_path = configured.filter(|path| path.is_file()).unwrap_or_else(|| {
            controller_default_path_for_backend(
                &params.save_dir,
                &key,
                backend_for_mode(config.mode),
            )
        });
        let default_state = ControllerDefaultState {
            config,
            profile_path: default_path.is_file().then_some(default_path),
        };
        let state = previous.get(&key).cloned().unwrap_or(default_state);
        rebuilt.push(add_controller_row(
            &params.group,
            &params.parent,
            &params.save_dir,
            device.clone(),
            state,
            params.registry.clone(),
        ));
    }
    if rebuilt.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title("No controllers detected");
        params.group.add(&row);
        *params.no_controllers_row.borrow_mut() = Some(row);
    }
    *params.widgets.borrow_mut() = rebuilt;
}

fn clear_controller_rows(
    group: &adw::PreferencesGroup,
    widgets: &Rc<RefCell<Vec<ControllerDefaultWidgets>>>,
    no_controllers_row: &Rc<RefCell<Option<adw::ActionRow>>>,
) {
    for widget in widgets.borrow().iter() {
        group.remove(&widget.row);
    }
    if let Some(row) = no_controllers_row.borrow_mut().take() {
        group.remove(&row);
    }
}

fn add_controller_row(
    group: &adw::PreferencesGroup,
    parent: &adw::Window,
    save_dir: &str,
    device: ira_input::DeviceInfo,
    state: ControllerDefaultState,
    registry: std::sync::Arc<ira_input::ControllerRegistry>,
) -> ControllerDefaultWidgets {
    let key = ira_config::Config::controller_key(device.vendor, device.product);
    let device_name = device.name.clone();
    let supported_buttons = device.supported_buttons.clone();
    let expander = adw::ExpanderRow::new();
    expander.set_title(&device_name);
    update_controller_subtitle(&expander, &device, state.config.mode);
    let profile_path = Rc::new(RefCell::new(state.profile_path));
    let mode_model = gtk4::StringList::new(&["Disabled", "Virtual XInput", "Virtual DirectInput"]);
    let mode = adw::ComboRow::new();
    mode.set_title("Input mode");
    mode.set_model(Some(&mode_model));
    mode.set_selected(selection_for_mode(state.config.mode));
    let expander_for_mode = expander.clone();
    let device_for_mode = device.clone();
    let profile_path_for_mode = profile_path.clone();
    let save_dir_for_mode = save_dir.to_string();
    let key_for_mode = key.clone();
    mode.connect_selected_notify(move |row| {
        let mode = mode_from_selection(row.selected());
        update_controller_subtitle(&expander_for_mode, &device_for_mode, mode);
        if mode != ControllerInputMode::Disabled {
            let path = controller_default_path_for_backend(
                &save_dir_for_mode,
                &key_for_mode,
                backend_for_mode(mode),
            );
            *profile_path_for_mode.borrow_mut() = path.is_file().then_some(path);
        }
    });
    expander.set_expanded(state.config.mode != ControllerInputMode::Disabled);
    expander.add_row(&mode);

    let action_row = adw::ActionRow::new();
    action_row.set_title("Controller mapping");
    action_row.set_subtitle("Edit controller-specific bindings");
    let edit = icon_button("document-edit-symbolic", "Edit controller mapping");
    action_row.add_suffix(&edit);
    expander.add_row(&action_row);

    let copy_row = adw::ComboRow::new();
    copy_row.set_title("Copy mapping from");
    copy_row.set_subtitle("Use another configured controller as the starting point");
    let copy_model = gtk4::StringList::new(&["Select a configured controller"]);
    let mut copy_keys = Vec::new();
    // The key is the stable identity; names are intentionally not persisted here.
    for source_key in configured_controller_keys(save_dir, &key) {
        copy_model.append(&source_key);
        copy_keys.push(source_key);
    }
    copy_row.set_model(Some(&copy_model));
    expander.add_row(&copy_row);
    let save_dir_for_copy = save_dir.to_string();
    let target_key_for_copy = key.clone();
    let target_name_for_copy = device_name.clone();
    let buttons_for_copy = supported_buttons.clone();
    let profile_path_for_copy = profile_path.clone();
    let mode_for_copy = mode.clone();
    copy_row.connect_selected_notify(move |row| {
        let selected = row.selected();
        let Some(source_key) = selected
            .checked_sub(1)
            .and_then(|index| copy_keys.get(index as usize))
        else {
            return;
        };
        match copy_controller_default(
            &save_dir_for_copy,
            source_key,
            &target_key_for_copy,
            &target_name_for_copy,
            &buttons_for_copy,
        ) {
            Ok(path) => {
                if let Ok(profile) = super::input_profile_store::read_profile(&path) {
                    mode_for_copy.set_selected(selection_for_mode(match profile.backend {
                        ira_input::VirtualGamepadBackend::XInput => {
                            ControllerInputMode::VirtualXInput
                        }
                        ira_input::VirtualGamepadBackend::DirectInput => {
                            ControllerInputMode::VirtualDirectInput
                        }
                    }));
                }
                *profile_path_for_copy.borrow_mut() = Some(path);
                row.set_selected(0);
            }
            Err(error) => eprintln!("Failed to copy controller mapping: {error}"),
        }
    });

    let parent_for_edit = parent.clone();
    let save_dir_for_edit = save_dir.to_string();
    let profile_path_for_edit = profile_path.clone();
    let key_for_edit = key.clone();
    let device_name_for_edit = device_name.clone();
    let supported_buttons_for_edit = supported_buttons.clone();
    let device_for_edit = device.clone();
    let registry_for_edit = registry.clone();
    let mode_for_edit = mode.clone();
    edit.connect_clicked(move |_| {
        let backend = backend_for_mode(mode_from_selection(mode_for_edit.selected()));
        let path = match ensure_controller_default_profile(
            &save_dir_for_edit,
            &key_for_edit,
            &device_name_for_edit,
            &supported_buttons_for_edit,
            backend,
        ) {
            Ok(path) => {
                *profile_path_for_edit.borrow_mut() = Some(path.clone());
                path
            }
            Err(error) => {
                eprintln!("Failed to create controller mapping: {error}");
                return;
            }
        };
        super::input_profile_editor::show_input_profile_editor(
            parent_for_edit.upcast_ref(),
            super::input_profile_editor::InputProfileEditorParams {
                save_dir: save_dir_for_edit.clone(),
                profile_path: Some(path),
                game_id: None,
                layout_name: Some(device_name_for_edit.clone()),
                registry: registry_for_edit.clone(),
                device: Some(device_for_edit.clone()),
            },
            |_| {},
        );
    });

    group.add(&expander);
    ControllerDefaultWidgets {
        key,
        device_name,
        supported_buttons,
        mode,
        profile_path,
        row: expander,
    }
}

fn backend_for_mode(mode: ControllerInputMode) -> ira_input::VirtualGamepadBackend {
    match mode {
        ControllerInputMode::VirtualDirectInput => ira_input::VirtualGamepadBackend::DirectInput,
        ControllerInputMode::Disabled | ControllerInputMode::VirtualXInput => {
            ira_input::VirtualGamepadBackend::XInput
        }
    }
}

fn update_controller_subtitle(
    row: &adw::ExpanderRow,
    device: &ira_input::DeviceInfo,
    mode: ControllerInputMode,
) {
    let virtualization = match mode {
        ControllerInputMode::Disabled => "Input virtualization disabled",
        ControllerInputMode::VirtualXInput => "Virtual XInput layout",
        ControllerInputMode::VirtualDirectInput => "Virtual DirectInput layout",
    };
    row.set_subtitle(&format!(
        "{} | Linux reports {}",
        virtualization,
        device.reported_input_mode().label()
    ));
}

fn start_controller_registry_refresh(
    parent: &adw::Window,
    group: &adw::PreferencesGroup,
    save_dir: &str,
    registry: std::sync::Arc<ira_input::ControllerRegistry>,
    configured_defaults: HashMap<String, ControllerInputConfig>,
    widgets: Rc<RefCell<Vec<ControllerDefaultWidgets>>>,
    no_controllers_row: Rc<RefCell<Option<adw::ActionRow>>>,
) {
    let group_for_refresh = group.clone();
    let parent_weak = parent.downgrade();
    let closing = Rc::new(Cell::new(false));
    let closing_for_parent = closing.clone();
    parent.connect_close_request(move |_| {
        closing_for_parent.set(true);
        glib::Propagation::Proceed
    });
    let save_dir_for_refresh = save_dir.to_string();
    let widgets_for_refresh = widgets.clone();
    let generation = Rc::new(Cell::new(registry.generation()));
    let generation_for_refresh = generation.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if closing.get() {
            return glib::ControlFlow::Break;
        }
        let Some(parent) = parent_weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if !parent.is_visible() {
            return glib::ControlFlow::Break;
        }
        if registry.generation() != generation_for_refresh.get() {
            generation_for_refresh.set(registry.generation());
            let params = ControllerRowsParams {
                group: group_for_refresh.clone(),
                parent,
                save_dir: save_dir_for_refresh.clone(),
                configured_defaults: configured_defaults.clone(),
                widgets: widgets_for_refresh.clone(),
                no_controllers_row: no_controllers_row.clone(),
                registry: registry.clone(),
            };
            rebuild_controller_rows(&params, &registry.snapshot());
        }
        glib::ControlFlow::Continue
    });
}

fn configured_controller_keys(save_dir: &str, target_key: &str) -> Vec<String> {
    std::fs::read_dir(
        super::input_profile_store::controller_default_path(save_dir, "")
            .parent()
            .unwrap_or_else(|| std::path::Path::new(save_dir)),
    )
    .ok()
    .into_iter()
    .flatten()
    .filter_map(Result::ok)
    .filter(|entry| entry.file_type().is_ok_and(|file_type| file_type.is_file()))
    .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
    .filter_map(|entry| {
        entry
            .path()
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::to_owned)
    })
    .filter(|key| key != target_key)
    .collect()
}

fn selection_for_mode(mode: ControllerInputMode) -> u32 {
    match mode {
        ControllerInputMode::Disabled => 0,
        ControllerInputMode::VirtualXInput => 1,
        ControllerInputMode::VirtualDirectInput => 2,
    }
}

pub(super) fn mode_from_selection(selection: u32) -> ControllerInputMode {
    match selection {
        1 => ControllerInputMode::VirtualXInput,
        2 => ControllerInputMode::VirtualDirectInput,
        _ => ControllerInputMode::Disabled,
    }
}

fn add_profile_row(
    group: &adw::PreferencesGroup,
    parent: &adw::Window,
    save_dir: &str,
    stored: StoredProfile,
    registry: std::sync::Arc<ira_input::ControllerRegistry>,
) {
    let row = adw::ActionRow::new();
    row.set_title(&profile_label(&stored));
    let edit = icon_button("document-edit-symbolic", "Edit profile");
    let delete = icon_button("user-trash-symbolic", "Delete profile");
    row.add_suffix(&edit);
    row.add_suffix(&delete);
    let parent_for_edit = parent.clone();
    let save_dir_for_edit = save_dir.to_string();
    let registry_for_edit = registry.clone();
    let path_for_edit = stored.path.clone();
    edit.connect_clicked(move |_| {
        super::input_profile_editor::show_input_profile_editor(
            parent_for_edit.upcast_ref(),
            super::input_profile_editor::InputProfileEditorParams {
                save_dir: save_dir_for_edit.clone(),
                profile_path: Some(path_for_edit.clone()),
                game_id: None,
                layout_name: None,
                registry: registry_for_edit.clone(),
                device: None,
            },
            |_| {},
        );
    });
    let parent_for_delete = parent.clone();
    let path_for_delete = stored.path.clone();
    let row_for_delete = row.clone();
    let group_for_delete = group.clone();
    delete.connect_clicked(move |_| {
        let alert = adw::AlertDialog::new(
            Some("Delete layout"),
            Some("This removes the game layout from Ira."),
        );
        alert.add_response("cancel", "Cancel");
        alert.add_response("delete", "Delete");
        alert.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        alert.set_default_response(Some("cancel"));
        alert.set_close_response("cancel");
        let path = path_for_delete.clone();
        let row = row_for_delete.clone();
        let group = group_for_delete.clone();
        alert.connect_response(None, move |_, response| {
            if response == "delete" {
                if let Err(error) = std::fs::remove_file(&path) {
                    eprintln!("Failed to delete controller layout: {error}");
                } else {
                    group.remove(&row);
                }
            }
        });
        alert.present(Some(&parent_for_delete));
    });
    group.add(&row);
}

fn icon_button(icon: &str, tooltip: &str) -> gtk4::Button {
    let button = gtk4::Button::from_icon_name(icon);
    button.add_css_class(CSS_FLAT);
    button.set_tooltip_text(Some(tooltip));
    button
}

fn profile_label(stored: &StoredProfile) -> String {
    if stored.profile.name.trim().is_empty() {
        stored
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("Unnamed")
            .to_string()
    } else {
        stored.profile.name.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::mode_from_selection;
    use ira_models::ControllerInputMode;

    #[test]
    fn test_mode_from_selection_defaults_to_disabled() {
        assert_eq!(mode_from_selection(0), ControllerInputMode::Disabled);
        assert_eq!(mode_from_selection(99), ControllerInputMode::Disabled);
    }

    #[test]
    fn test_mode_from_selection_selects_virtual_backends() {
        assert_eq!(mode_from_selection(1), ControllerInputMode::VirtualXInput);
        assert_eq!(
            mode_from_selection(2),
            ControllerInputMode::VirtualDirectInput
        );
    }
}
