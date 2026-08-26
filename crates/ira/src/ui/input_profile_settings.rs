use super::css::{CSS_FLAT, CSS_SQUARE_BUTTON};
use super::helpers::esc;
use super::input_profile_store::{
    controller_default_path_for_backend, find_controller_default_profile,
};
use super::input_profile_store::{ensure_controller_default_profile, list_profiles, StoredProfile};
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
    pub mode: Rc<RefCell<Option<ControllerInputMode>>>,
    pub profile_path: Rc<RefCell<Option<std::path::PathBuf>>>,
}

pub(super) fn add_pc_profile_group(
    page: &gtk4::Box,
    cfg: &Config,
    config_id: &str,
    label: &str,
    parent: &adw::Window,
    registry: std::sync::Arc<ira_input::ControllerRegistry>,
) -> ConsoleProfileWidgets {
    let group = adw::PreferencesGroup::new();
    group.set_title(label);
    let widget = add_console_remapping_rows(
        &group,
        cfg,
        config_id.to_string(),
        label.to_string(),
        parent,
        &cfg.save_dir,
        registry,
    );
    page.append(&group);
    widget
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
    let profiles_group = adw::PreferencesGroup::new();
    profiles_group.set_title(&crate::tr!("Profiles"));
    let profile_rows = Rc::new(RefCell::new(Vec::new()));
    rebuild_profile_rows(
        &profiles_group,
        parent,
        save_dir,
        registry.clone(),
        &profile_rows,
    );

    let controller_group = adw::PreferencesGroup::new();
    controller_group.set_title(&crate::tr!("Controller defaults"));
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
        registry.clone(),
        controller_rows_params.configured_defaults,
        controller_defaults.clone(),
        no_controllers_row,
    );
    page.append(&controller_group);
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
    parent: &adw::Window,
    cfg: &Config,
    save_dir: &str,
    console_id: &str,
    label: &str,
    registry: std::sync::Arc<ira_input::ControllerRegistry>,
) -> ConsoleProfileWidgets {
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Controller"));
    let widget = add_console_remapping_rows(
        &group,
        cfg,
        console_id.to_string(),
        label.to_string(),
        parent,
        save_dir,
        registry,
    );
    page.append(&group);
    widget
}

fn add_console_remapping_rows(
    group: &adw::PreferencesGroup,
    cfg: &Config,
    console_id: String,
    label: String,
    parent: &adw::Window,
    save_dir: &str,
    registry: std::sync::Arc<ira_input::ControllerRegistry>,
) -> ConsoleProfileWidgets {
    let mode = Rc::new(RefCell::new(cfg.console(&console_id).controller_mode));
    let mode_row = adw::ComboRow::new();
    mode_row.set_title(&crate::tr!("Input remapping"));
    let mode_strings = [
        crate::tr!("Inherit"),
        crate::tr!("Disabled"),
        crate::tr!("Virtual XInput"),
        crate::tr!("Virtual DirectInput"),
        crate::tr!("Nintendo Switch Pro Controller"),
        crate::tr!("DualShock 4 Controller"),
        crate::tr!("DualSense Controller"),
    ];
    let mode_refs: Vec<&str> = mode_strings.iter().map(String::as_str).collect();
    mode_row.set_model(Some(&gtk4::StringList::new(&mode_refs)));
    mode_row.set_selected(input_mode_index(*mode.borrow()));
    let mode_for_selection = mode.clone();
    mode_row.connect_selected_notify(move |row| {
        *mode_for_selection.borrow_mut() = input_mode_from_index(row.selected());
    });
    group.add(&mode_row);

    let profile_row = adw::ComboRow::new();
    profile_row.set_title(&crate::tr!("Layout"));
    let current_path = (!cfg.console(&console_id).controller_profile.is_empty())
        .then(|| std::path::PathBuf::from(&cfg.console(&console_id).controller_profile));
    let (labels, profile_paths, selected) = console_profile_choices(save_dir, current_path);
    profile_row.set_model(Some(&gtk4::StringList::new(
        &labels.iter().map(String::as_str).collect::<Vec<_>>(),
    )));
    profile_row.set_selected(selected);
    let profile_paths = Rc::new(RefCell::new(profile_paths));
    let profile_path = Rc::new(RefCell::new(selected_console_path(
        &profile_row,
        &profile_paths,
    )));
    let last_real = Rc::new(RefCell::new(selected));
    let profile_paths_for_selection = profile_paths.clone();
    let profile_path_for_selection = profile_path.clone();
    profile_row.connect_selected_notify(move |row| {
        *profile_path_for_selection.borrow_mut() = profile_paths_for_selection
            .borrow()
            .get(row.selected() as usize)
            .cloned()
            .flatten();
    });
    let monitor = icon_button("input-gaming-symbolic", &crate::tr!("Monitor this layout"));
    let edit = icon_button("document-edit-symbolic", &crate::tr!("Edit layout"));
    monitor.set_sensitive(true);
    edit.set_sensitive(selected != 0);
    profile_row.add_suffix(&edit);
    profile_row.add_suffix(&monitor);
    group.add(&profile_row);
    let parent = parent.clone();
    let save_dir = save_dir.to_string();
    let label_for_edit = label.clone();
    let profile_row_for_edit = profile_row.clone();
    let profile_paths_for_edit = profile_paths.clone();
    let profile_path_for_edit = profile_path.clone();
    let last_real_for_edit = last_real.clone();
    let parent_for_edit = parent.clone();
    let registry_for_edit = registry.clone();
    let save_dir_for_edit = save_dir.clone();
    edit.connect_clicked(move |_| {
        let Some(path) = profile_path_for_edit.borrow().clone() else {
            return;
        };
        let row = profile_row_for_edit.clone();
        let paths = profile_paths_for_edit.clone();
        let save_dir = save_dir_for_edit.clone();
        let last_real = last_real_for_edit.clone();
        super::input_profile_editor::show_input_profile_editor(
            parent_for_edit.upcast_ref(),
            super::input_profile_editor::InputProfileEditorParams {
                save_dir: save_dir.clone(),
                profile_path: Some(path),
                game_id: None,
                layout_name: Some(label_for_edit.clone()),
                registry: registry_for_edit.clone(),
                device: None,
            },
            move |saved| {
                refresh_console_profile_choices(&row, &paths, &save_dir, Some(&saved), &last_real)
            },
        );
    });
    let parent_for_monitor = parent.clone();
    let registry_for_monitor = registry.clone();
    monitor.connect_clicked(move |_| {
        super::input_profile_viewer::show_raw_input_viewer(
            parent_for_monitor.upcast_ref(),
            registry_for_monitor.clone(),
        );
    });
    let paths_for_notify = profile_paths;
    let last_real_for_notify = last_real;
    profile_row.connect_selected_notify({
        move |row| {
            let sentinel = paths_for_notify.borrow().len().saturating_sub(1) as u32;
            if row.selected() == sentinel {
                let row_for_revert = row.clone();
                let previous = *last_real_for_notify.borrow();
                let paths = paths_for_notify.clone();
                let last_real = last_real_for_notify.clone();
                let parent = parent.clone();
                let save_dir = save_dir.clone();
                let label = label.clone();
                let registry = registry.clone();
                glib::idle_add_local(move || {
                    row_for_revert.set_selected(previous);
                    let row_for_saved = row_for_revert.clone();
                    let paths_for_saved = paths.clone();
                    let save_dir_for_saved = save_dir.clone();
                    let last_real_for_saved = last_real.clone();
                    super::input_profile_editor::show_input_profile_editor(
                        parent.upcast_ref(),
                        super::input_profile_editor::InputProfileEditorParams {
                            save_dir: save_dir.clone(),
                            profile_path: None,
                            game_id: None,
                            layout_name: Some(label.clone()),
                            registry: registry.clone(),
                            device: None,
                        },
                        move |saved| {
                            refresh_console_profile_choices(
                                &row_for_saved,
                                &paths_for_saved,
                                &save_dir_for_saved,
                                Some(&saved),
                                &last_real_for_saved,
                            )
                        },
                    );
                    glib::ControlFlow::Break
                });
                return;
            }
            *last_real_for_notify.borrow_mut() = row.selected();
            let has_profile = selected_console_path(row, &paths_for_notify).is_some();
            edit.set_sensitive(has_profile);
            monitor.set_sensitive(true);
        }
    });
    ConsoleProfileWidgets {
        console_id,
        mode,
        profile_path,
    }
}

fn console_profile_choices(
    save_dir: &str,
    current_path: Option<std::path::PathBuf>,
) -> (Vec<String>, Vec<Option<std::path::PathBuf>>, u32) {
    let profiles = list_profiles(save_dir).unwrap_or_else(|error| {
        eprintln!("Failed to list controller profiles: {error}");
        Vec::new()
    });
    let mut labels = vec![crate::tr!("Inherit")];
    let mut paths = vec![None];
    for profile in profiles
        .into_iter()
        .filter(|profile| profile.profile.compatible_game_ids.is_empty())
    {
        labels.push(profile_label(&profile));
        paths.push(Some(profile.path));
    }
    let selected = current_path
        .as_ref()
        .and_then(|path| {
            paths
                .iter()
                .position(|candidate| candidate.as_ref() == Some(path))
        })
        .unwrap_or(0);
    labels.push(crate::tr!("Create new profile..."));
    paths.push(None);
    (labels, paths, selected as u32)
}

fn selected_console_path(
    row: &adw::ComboRow,
    paths: &Rc<RefCell<Vec<Option<std::path::PathBuf>>>>,
) -> Option<std::path::PathBuf> {
    paths
        .borrow()
        .get(row.selected() as usize)
        .and_then(Clone::clone)
}

fn refresh_console_profile_choices(
    row: &adw::ComboRow,
    paths: &Rc<RefCell<Vec<Option<std::path::PathBuf>>>>,
    save_dir: &str,
    selected_path: Option<&std::path::Path>,
    last_real: &Rc<RefCell<u32>>,
) {
    let (labels, updated_paths, selected) =
        console_profile_choices(save_dir, selected_path.map(std::path::Path::to_path_buf));
    *paths.borrow_mut() = updated_paths;
    row.set_model(Some(&gtk4::StringList::new(
        &labels.iter().map(String::as_str).collect::<Vec<_>>(),
    )));
    row.set_selected(selected);
    *last_real.borrow_mut() = selected;
}

fn input_mode_index(mode: Option<ControllerInputMode>) -> u32 {
    match mode {
        None => 0,
        Some(ControllerInputMode::Disabled) => 1,
        Some(ControllerInputMode::Enabled) => 2,
    }
}

fn input_mode_from_index(index: u32) -> Option<ControllerInputMode> {
    match index {
        1 => Some(ControllerInputMode::Disabled),
        2 => Some(ControllerInputMode::Enabled),
        _ => None,
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
                        mode: if widget.mode.selected() == 0 {
                            ControllerInputMode::Disabled
                        } else {
                            ControllerInputMode::Enabled
                        },
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
        let default_path = configured
            .filter(|path| path.is_file())
            .or_else(|| find_controller_default_profile(&params.save_dir, &key))
            .unwrap_or_else(|| {
                controller_default_path_for_backend(
                    &params.save_dir,
                    &key,
                    ira_input::VirtualGamepadBackend::XInput,
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
        row.set_title(&crate::tr!("No controllers detected"));
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
    expander.set_title(&esc(&device_name));
    let initial_selection = stored_selection(state.config.mode, state.profile_path.as_deref());
    let initial_flavor = backend_for_selection(initial_selection);
    update_controller_subtitle(&expander, &device, initial_flavor);
    let profile_path = Rc::new(RefCell::new(state.profile_path));
    let mode_strings = [
        crate::tr!("Disabled"),
        crate::tr!("Virtual XInput"),
        crate::tr!("Virtual DirectInput"),
        crate::tr!("Nintendo Switch Pro Controller"),
        crate::tr!("DualShock 4 Controller"),
        crate::tr!("DualSense Controller"),
    ];
    let mode_refs: Vec<&str> = mode_strings.iter().map(String::as_str).collect();
    let mode_model = gtk4::StringList::new(&mode_refs);
    let mode = adw::ComboRow::new();
    mode.set_title(&crate::tr!("Default layout"));
    mode.set_model(Some(&mode_model));
    mode.set_selected(initial_selection);
    let expander_for_mode = expander.clone();
    let device_for_mode = device.clone();
    let profile_path_for_mode = profile_path.clone();
    let save_dir_for_mode = save_dir.to_string();
    let key_for_mode = key.clone();
    mode.connect_selected_notify(move |row| {
        let flavor = backend_for_selection(row.selected());
        update_controller_subtitle(&expander_for_mode, &device_for_mode, flavor);
        if let Some(backend) = flavor {
            let path =
                controller_default_path_for_backend(&save_dir_for_mode, &key_for_mode, backend);
            *profile_path_for_mode.borrow_mut() = path.is_file().then_some(path);
        }
    });
    let enabled = state.config.mode != ControllerInputMode::Disabled;
    expander.set_expanded(enabled);
    expander.add_row(&mode);

    let action_row = adw::ActionRow::new();
    action_row.set_title(&crate::tr!("Controller mapping"));
    action_row.set_subtitle(&crate::tr!("Edit controller-specific bindings"));
    let edit = icon_button(
        "document-edit-symbolic",
        &crate::tr!("Edit controller mapping"),
    );
    action_row.add_suffix(&edit);
    expander.add_row(&action_row);

    // Calibration is per controller: deadzone preference and gyro bias for
    // this pad, stored outside any profile.
    super::input_calibration_settings::add_controller_calibration(
        &expander,
        &ira_input::calibration_store_path(save_dir),
        &device,
        registry.clone(),
    );

    let parent_for_edit = parent.clone();
    let save_dir_for_edit = save_dir.to_string();
    let profile_path_for_edit = profile_path.clone();
    let key_for_edit = key.clone();
    let device_name_for_edit = device_name.clone();
    let supported_buttons_for_edit = supported_buttons.clone();
    let device_for_edit = device;
    let registry_for_edit = registry;
    let mode_for_edit = mode.clone();
    edit.connect_clicked(move |_| {
        let Some(backend) = backend_for_selection(mode_for_edit.selected()) else {
            return;
        };
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

fn update_controller_subtitle(
    row: &adw::ExpanderRow,
    device: &ira_input::DeviceInfo,
    flavor: Option<ira_input::VirtualGamepadBackend>,
) {
    use ira_input::VirtualGamepadBackend;
    let virtualization = match flavor {
        None => crate::tr!("Input virtualization disabled"),
        Some(VirtualGamepadBackend::XInput) => crate::tr!("Virtual XInput layout"),
        Some(VirtualGamepadBackend::DirectInput) => crate::tr!("Virtual DirectInput layout"),
        Some(VirtualGamepadBackend::SwitchPro) => {
            crate::tr!("Nintendo Switch Pro Controller layout")
        }
        Some(VirtualGamepadBackend::DualShock4) => crate::tr!("DualShock 4 Controller layout"),
        Some(VirtualGamepadBackend::DualSense) => crate::tr!("DualSense Controller layout"),
        Some(VirtualGamepadBackend::Dsu) => crate::tr!("DSU (cemuhook) controller layout"),
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
    let widgets_for_refresh = widgets;
    let generation = Rc::new(Cell::new(registry.generation()));
    let generation_for_refresh = generation;
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

/// Combo index for an enabled default: derived from the flavor recorded in
/// the layout itself, since backends live on profiles now.
fn stored_selection(mode: ControllerInputMode, profile_path: Option<&std::path::Path>) -> u32 {
    if mode == ControllerInputMode::Disabled {
        return 0;
    }
    let backend = profile_path
        .and_then(|path| super::input_profile_store::read_profile(path).ok())
        .map(|profile| profile.backend)
        .unwrap_or(ira_input::VirtualGamepadBackend::XInput);
    selection_for_backend(backend)
}

fn selection_for_backend(backend: ira_input::VirtualGamepadBackend) -> u32 {
    use ira_input::VirtualGamepadBackend;
    match backend {
        VirtualGamepadBackend::DirectInput => 2,
        VirtualGamepadBackend::SwitchPro => 3,
        VirtualGamepadBackend::DualShock4 => 4,
        VirtualGamepadBackend::DualSense => 5,
        VirtualGamepadBackend::Dsu => 6,
        VirtualGamepadBackend::XInput => 1,
    }
}

/// `None` means the "Disabled" entry.
pub(super) fn backend_for_selection(selection: u32) -> Option<ira_input::VirtualGamepadBackend> {
    use ira_input::VirtualGamepadBackend;
    match selection {
        1 => Some(VirtualGamepadBackend::XInput),
        2 => Some(VirtualGamepadBackend::DirectInput),
        3 => Some(VirtualGamepadBackend::SwitchPro),
        4 => Some(VirtualGamepadBackend::DualShock4),
        5 => Some(VirtualGamepadBackend::DualSense),
        6 => Some(VirtualGamepadBackend::Dsu),
        _ => None,
    }
}

fn rebuild_profile_rows(
    group: &adw::PreferencesGroup,
    parent: &adw::Window,
    save_dir: &str,
    registry: std::sync::Arc<ira_input::ControllerRegistry>,
    rows: &Rc<RefCell<Vec<gtk4::Widget>>>,
) {
    for row in rows.borrow_mut().drain(..) {
        group.remove(&row);
    }
    for stored in list_profiles(save_dir).unwrap_or_else(|error| {
        eprintln!("Failed to list controller profiles: {error}");
        Vec::new()
    }) {
        rows.borrow_mut().push(
            add_profile_row(group, parent, save_dir, stored, registry.clone(), rows).upcast(),
        );
    }
    let new_profile = icon_button("list-add-symbolic", &crate::tr!("Create profile"));
    let group_for_new = group.clone();
    let parent_for_new = parent.clone();
    let save_dir_for_new = save_dir.to_string();
    let registry_for_new = registry.clone();
    let rows_for_new = rows.clone();
    new_profile.connect_clicked(move |_| {
        let group = group_for_new.clone();
        let parent = parent_for_new.clone();
        let parent_for_saved = parent.clone();
        let save_dir = save_dir_for_new.clone();
        let registry = registry_for_new.clone();
        let rows_for_saved = rows_for_new.clone();
        super::input_profile_editor::show_input_profile_editor(
            parent.upcast_ref(),
            super::input_profile_editor::InputProfileEditorParams {
                save_dir: save_dir.clone(),
                profile_path: None,
                game_id: None,
                layout_name: None,
                registry: registry.clone(),
                device: None,
            },
            move |_| {
                rebuild_profile_rows(
                    &group,
                    &parent_for_saved,
                    &save_dir,
                    registry.clone(),
                    &rows_for_saved,
                )
            },
        );
    });
    let preview = icon_button(
        "input-gaming-symbolic",
        &crate::tr!("View raw controller input"),
    );
    let preview_row = adw::ActionRow::new();
    preview_row.set_title(&crate::tr!("Controller preview"));
    preview_row.add_suffix(&preview);
    let parent_for_preview = parent.clone();
    let registry_for_preview = registry;
    preview.connect_clicked(move |_| {
        super::input_profile_viewer::show_raw_input_viewer(
            parent_for_preview.upcast_ref(),
            registry_for_preview.clone(),
        );
    });
    group.add(&preview_row);
    group.set_header_suffix(Some(&new_profile));
    rows.borrow_mut()
        .extend([preview_row.upcast::<gtk4::Widget>()]);
}

fn add_profile_row(
    group: &adw::PreferencesGroup,
    parent: &adw::Window,
    save_dir: &str,
    stored: StoredProfile,
    registry: std::sync::Arc<ira_input::ControllerRegistry>,
    rows: &Rc<RefCell<Vec<gtk4::Widget>>>,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&esc(&profile_label(&stored)));
    let preview = icon_button(
        "input-gaming-symbolic",
        &crate::tr!("Preview layout output"),
    );
    let edit = icon_button("document-edit-symbolic", &crate::tr!("Edit profile"));
    let delete = icon_button("user-trash-symbolic", &crate::tr!("Delete profile"));
    row.add_suffix(&preview);
    row.add_suffix(&edit);
    row.add_suffix(&delete);
    let parent_for_edit = parent.clone();
    let save_dir_for_edit = save_dir.to_string();
    let registry_for_edit = registry.clone();
    let path_for_edit = stored.path.clone();
    let group_for_edit = group.clone();
    let rows_for_edit = rows.clone();
    edit.connect_clicked(move |_| {
        let rows_for_saved = rows_for_edit.clone();
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
            {
                let group = group_for_edit.clone();
                let parent = parent_for_edit.clone();
                let save_dir = save_dir_for_edit.clone();
                let registry = registry_for_edit.clone();
                move |_| {
                    rebuild_profile_rows(
                        &group,
                        &parent,
                        &save_dir,
                        registry.clone(),
                        &rows_for_saved,
                    )
                }
            },
        );
    });
    let parent_for_preview = parent.clone();
    let path_for_preview = stored.path.clone();
    let registry_for_preview = registry.clone();
    preview.connect_clicked(move |_| {
        super::input_profile_viewer::show_input_profile_viewer(
            parent_for_preview.upcast_ref(),
            &path_for_preview,
            registry_for_preview.clone(),
        );
    });
    let parent_for_delete = parent.clone();
    let path_for_delete = stored.path;
    let group_for_delete = group.clone();
    let save_dir_for_delete = save_dir.to_string();
    let rows_for_delete = rows.clone();
    delete.connect_clicked(move |_| {
        let alert = adw::AlertDialog::new(
            Some(&crate::tr!("Delete layout")),
            Some(&crate::tr!("This removes the game layout from Ira.")),
        );
        alert.add_response("cancel", &crate::tr!("Cancel"));
        alert.add_response("delete", &crate::tr!("Delete"));
        alert.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        alert.set_default_response(Some("cancel"));
        alert.set_close_response("cancel");
        let path = path_for_delete.clone();
        let group = group_for_delete.clone();
        let parent = parent_for_delete.clone();
        let save_dir = save_dir_for_delete.clone();
        let registry = registry.clone();
        let rows = rows_for_delete.clone();
        alert.connect_response(None, move |_, response| {
            if response == "delete" {
                if let Err(error) = std::fs::remove_file(&path) {
                    eprintln!("Failed to delete controller layout: {error}");
                } else {
                    rebuild_profile_rows(&group, &parent, &save_dir, registry.clone(), &rows);
                }
            }
        });
        alert.present(Some(&parent_for_delete));
    });
    group.add(&row);
    row
}

fn icon_button(icon: &str, tooltip: &str) -> gtk4::Button {
    let button = gtk4::Button::from_icon_name(icon);
    button.add_css_class(CSS_FLAT);
    button.add_css_class(CSS_SQUARE_BUTTON);
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
    use super::{
        backend_for_selection, input_mode_from_index, selection_for_backend, stored_selection,
    };
    use ira_input::VirtualGamepadBackend;
    use ira_models::ControllerInputMode;

    #[test]
    fn test_backend_for_selection_round_trip() {
        for backend in [
            VirtualGamepadBackend::XInput,
            VirtualGamepadBackend::DirectInput,
            VirtualGamepadBackend::SwitchPro,
            VirtualGamepadBackend::DualShock4,
            VirtualGamepadBackend::DualSense,
            VirtualGamepadBackend::Dsu,
        ] {
            assert_eq!(
                backend_for_selection(selection_for_backend(backend)),
                Some(backend)
            );
        }
        assert_eq!(backend_for_selection(0), None);
        assert_eq!(backend_for_selection(99), None);
    }

    #[test]
    fn test_stored_selection_disabled_is_zero_and_enabled_falls_back_to_xinput() {
        assert_eq!(stored_selection(ControllerInputMode::Disabled, None), 0);
        // An enabled default without a readable layout assumes XInput.
        assert_eq!(stored_selection(ControllerInputMode::Enabled, None), 1);
    }

    #[test]
    fn test_input_mode_from_index_preserves_inheritance() {
        assert_eq!(input_mode_from_index(0), None);
        assert_eq!(
            input_mode_from_index(1),
            Some(ControllerInputMode::Disabled)
        );
        assert_eq!(input_mode_from_index(2), Some(ControllerInputMode::Enabled));
    }
}
