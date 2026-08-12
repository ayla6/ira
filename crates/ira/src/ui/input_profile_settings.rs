use super::css::CSS_FLAT;
use super::input_profile_store::controller_default_path;
use super::input_profile_store::{ensure_controller_default_profile, list_profiles, StoredProfile};
use adw::prelude::*;
use ira_config::Config;
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

#[derive(Clone)]
pub(super) struct ControllerDefaultWidgets {
    pub key: String,
    pub device_name: String,
    pub supported_buttons: Vec<ira_input::GamepadButton>,
    pub always_on: gtk4::Switch,
    pub profile_path: Rc<RefCell<Option<std::path::PathBuf>>>,
    row: adw::ExpanderRow,
}

#[derive(Clone)]
struct ControllerDefaultState {
    always_on: bool,
    profile_path: Option<std::path::PathBuf>,
}

#[derive(Clone)]
struct ControllerRowsParams {
    group: adw::PreferencesGroup,
    parent: adw::Window,
    save_dir: String,
    configured_defaults: HashMap<String, bool>,
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
    for stored in profiles {
        add_profile_row(&profiles_group, parent, save_dir, stored, registry.clone());
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
        .map(|(key, value)| (key.clone(), value.always_on))
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

fn rebuild_controller_rows(params: &ControllerRowsParams, devices: &[ira_input::DeviceInfo]) {
    let previous = params
        .widgets
        .borrow()
        .iter()
        .map(|widget| {
            (
                widget.key.clone(),
                ControllerDefaultState {
                    always_on: widget.always_on.is_active(),
                    profile_path: widget.profile_path.borrow().clone(),
                },
            )
        })
        .collect::<HashMap<_, _>>();
    clear_controller_rows(&params.group, &params.widgets, &params.no_controllers_row);
    let mut rebuilt = Vec::new();
    for device in devices {
        let key = ira_config::Config::controller_key(device.vendor, device.product);
        let default_path = controller_default_path(&params.save_dir, &key);
        let default_state = ControllerDefaultState {
            always_on: params
                .configured_defaults
                .get(&key)
                .copied()
                .unwrap_or(false),
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
    expander.set_subtitle("Generic XInput layout");
    let always_on = gtk4::Switch::new();
    always_on.set_valign(gtk4::Align::Center);
    always_on.set_active(state.always_on);
    expander.add_suffix(&always_on);
    expander.set_expanded(state.always_on);

    let profile_path = Rc::new(RefCell::new(state.profile_path));
    let action_row = adw::ActionRow::new();
    action_row.set_title("Controller mapping");
    action_row.set_subtitle("Edit paddle remaps and other controller-specific bindings");
    let edit = icon_button("document-edit-symbolic", "Edit controller mapping");
    action_row.add_suffix(&edit);
    expander.add_row(&action_row);

    let parent_for_edit = parent.clone();
    let save_dir_for_edit = save_dir.to_string();
    let profile_path_for_edit = profile_path.clone();
    let key_for_edit = key.clone();
    let device_name_for_edit = device_name.clone();
    let supported_buttons_for_edit = supported_buttons.clone();
    let device_for_edit = device.clone();
    let registry_for_edit = registry.clone();
    edit.connect_clicked(move |_| {
        let path = match ensure_controller_default_profile(
            &save_dir_for_edit,
            &key_for_edit,
            &device_name_for_edit,
            &supported_buttons_for_edit,
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
        always_on,
        profile_path,
        row: expander,
    }
}

fn start_controller_registry_refresh(
    parent: &adw::Window,
    group: &adw::PreferencesGroup,
    save_dir: &str,
    registry: std::sync::Arc<ira_input::ControllerRegistry>,
    configured_defaults: HashMap<String, bool>,
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
