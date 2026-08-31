use super::helpers::{esc, icon_button};
use super::input_profile_store::{
    controller_default_path, ensure_controller_default_profile, find_controller_default_profile,
    list_profiles, read_profile, StoredProfile,
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
    pub mode: Rc<RefCell<Option<ControllerInputMode>>>,
    pub profile_path: Rc<RefCell<Option<std::path::PathBuf>>>,
}

/// The settings host (an `adw::Window` since the settings dialog became
/// resizable), accepted as any widget so sub-dialogs can present over it.
pub(super) fn add_pc_profile_group(
    page: &gtk4::Box,
    cfg: &Config,
    config_id: &str,
    label: &str,
    parent: &impl IsA<gtk4::Widget>,
    registry: std::sync::Arc<ira_input::ControllerRegistry>,
) -> ConsoleProfileWidgets {
    let parent = parent.clone().upcast();
    let group = adw::PreferencesGroup::new();
    group.set_title(label);
    let widget = add_console_remapping_rows(
        &group,
        cfg,
        config_id.to_string(),
        label.to_string(),
        &parent,
        &cfg.save_dir,
        registry,
    );
    page.append(&group);
    widget
}

#[derive(Clone)]
pub(super) struct ControllerDefaultWidgets {
    pub key: String,
    pub layout: adw::ComboRow,
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
    parent: gtk4::Widget,
    save_dir: String,
    configured_defaults: HashMap<String, ControllerInputConfig>,
    widgets: Rc<RefCell<Vec<ControllerDefaultWidgets>>>,
    no_controllers_row: Rc<RefCell<Option<adw::ActionRow>>>,
    registry: std::sync::Arc<ira_input::ControllerRegistry>,
}

pub(super) fn build_input_settings_page(
    parent: &impl IsA<gtk4::Widget>,
    save_dir: &str,
    cfg: &Config,
    steam: std::sync::Arc<ira_api::SteamDataClient>,
    registry: std::sync::Arc<ira_input::ControllerRegistry>,
) -> (gtk4::Box, InputPageWidgets) {
    let parent = parent.clone().upcast();
    let page = settings_page_container();
    let profiles_group = adw::PreferencesGroup::new();
    profiles_group.set_title(&crate::tr!("Profiles"));
    let profile_rows = Rc::new(RefCell::new(Vec::new()));
    rebuild_profile_rows(
        &profiles_group,
        &parent,
        save_dir,
        steam.clone(),
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
        &parent,
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
    parent: &impl IsA<gtk4::Widget>,
    cfg: &Config,
    save_dir: &str,
    console_id: &str,
    label: &str,
    registry: std::sync::Arc<ira_input::ControllerRegistry>,
) -> ConsoleProfileWidgets {
    let parent = parent.clone().upcast();
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Controller"));
    let widget = add_console_remapping_rows(
        &group,
        cfg,
        console_id.to_string(),
        label.to_string(),
        &parent,
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
    parent: &gtk4::Widget,
    save_dir: &str,
    registry: std::sync::Arc<ira_input::ControllerRegistry>,
) -> ConsoleProfileWidgets {
    let mode = Rc::new(RefCell::new(cfg.console(&console_id).controller_mode));
    let mode_row = adw::ComboRow::new();
    mode_row.set_title(&crate::tr!("Input remapping"));
    // Same three-entry gate as the game controller page: which virtual
    // controller the emulator sees is decided by the selected layout itself.
    mode_row.set_subtitle(&crate::tr!(
        "Whether the input broker runs; the virtual controller type comes from the selected layout"
    ));
    let mode_strings = [
        crate::tr!("Inherit"),
        crate::tr!("Disabled"),
        crate::tr!("Enabled"),
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
    let edit = icon_button("document-edit-symbolic", &crate::tr!("Edit layout"));
    edit.set_sensitive(selected != 0);
    profile_row.add_suffix(&edit);
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
            &parent_for_edit,
            super::input_profile_editor::InputProfileEditorParams {
                save_dir: save_dir.clone(),
                profile_path: Some(path),
                game_id: None,
                platform_id: None,
                layout_name: Some(label_for_edit.clone()),
                registry: registry_for_edit.clone(),
                device: None,
            },
            move |saved| {
                refresh_console_profile_choices(&row, &paths, &save_dir, Some(&saved), &last_real)
            },
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
                        &parent,
                        super::input_profile_editor::InputProfileEditorParams {
                            save_dir: save_dir.clone(),
                            profile_path: None,
                            game_id: None,
                platform_id: None,
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
        }
    });
    ConsoleProfileWidgets {
        console_id,
        mode,
        profile_path,
    }
}

/// Entries for a "choose a layout" combo, shared by the console and
/// controller-default pickers: the none-style choice first, then every
/// layout not scoped to a specific game, then the create-new sentinel.
/// The selected layout owns the virtual backend; these rows never pick it.
fn layout_choices(
    save_dir: &str,
    current_path: Option<std::path::PathBuf>,
    none_label: &str,
) -> (Vec<String>, Vec<Option<std::path::PathBuf>>, u32) {
    let profiles = list_profiles(save_dir).unwrap_or_else(|error| {
        eprintln!("Failed to list controller profiles: {error}");
        Vec::new()
    });
    let mut labels = vec![none_label.to_string()];
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

fn console_profile_choices(
    save_dir: &str,
    current_path: Option<std::path::PathBuf>,
) -> (Vec<String>, Vec<Option<std::path::PathBuf>>, u32) {
    layout_choices(save_dir, current_path, &crate::tr!("Inherit"))
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
                        mode: if widget.layout.selected() == 0 {
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
            .unwrap_or_else(|| controller_default_path(&params.save_dir, &key));
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
    parent: &gtk4::Widget,
    save_dir: &str,
    device: ira_input::DeviceInfo,
    state: ControllerDefaultState,
    registry: std::sync::Arc<ira_input::ControllerRegistry>,
) -> ControllerDefaultWidgets {
    let key = ira_config::Config::controller_key(device.vendor, device.product);
    let device_name = device.name.clone();
    let expander = adw::ExpanderRow::new();
    expander.set_title(&esc(&device_name));

    let action_row = adw::ActionRow::new();
    action_row.set_title(&crate::tr!("Controller mapping"));
    action_row.set_subtitle(&crate::tr!("Edit controller-specific bindings"));
    let edit = icon_button(
        "document-edit-symbolic",
        &crate::tr!("Edit controller mapping"),
    );
    action_row.add_suffix(&edit);
    expander.add_row(&action_row);

    let layout = ControllerDefaultLayout::new(
        &expander,
        parent,
        save_dir,
        &device,
        &state,
        registry.clone(),
    );
    let enabled = state.config.mode != ControllerInputMode::Disabled;
    expander.set_expanded(enabled);
    edit.set_sensitive(layout.row.selected() != 0);
    let layout_for_edit = layout.clone();
    edit.connect_clicked(move |_| {
        let Some(path) = layout_for_edit.chosen.borrow().clone() else {
            return;
        };
        layout_for_edit.open_editor(path);
    });

    // Calibration is per controller: deadzone preference and gyro bias for
    // this pad, stored outside any profile.
    super::input_calibration_settings::add_controller_calibration(
        &expander,
        &ira_input::calibration_store_path(save_dir),
        &device,
        registry,
    );

    group.add(&expander);
    ControllerDefaultWidgets {
        key,
        layout: layout.row.clone(),
        profile_path: layout.chosen.clone(),
        row: expander,
    }
}

/// One controller-default expander's layout picker plus everything needed to
/// create or edit the chosen layout. The picker lists layouts; which virtual
/// controller a layout produces lives in the layout itself.
#[derive(Clone)]
struct ControllerDefaultLayout {
    parent: gtk4::Widget,
    save_dir: String,
    device: ira_input::DeviceInfo,
    registry: std::sync::Arc<ira_input::ControllerRegistry>,
    row: adw::ComboRow,
    expander: adw::ExpanderRow,
    paths: Rc<RefCell<Vec<Option<std::path::PathBuf>>>>,
    last_real: Rc<RefCell<u32>>,
    /// The layout the device default points at. Kept while Disabled is
    /// selected so re-enabling restores it.
    chosen: Rc<RefCell<Option<std::path::PathBuf>>>,
}

impl ControllerDefaultLayout {
    fn new(
        expander: &adw::ExpanderRow,
        parent: &gtk4::Widget,
        save_dir: &str,
        device: &ira_input::DeviceInfo,
        state: &ControllerDefaultState,
        registry: std::sync::Arc<ira_input::ControllerRegistry>,
    ) -> Self {
        let enabled = state.config.mode != ControllerInputMode::Disabled;
        let current = enabled.then(|| state.profile_path.clone()).flatten();
        let (labels, paths, selected) =
            layout_choices(save_dir, current, &crate::tr!("Disabled"));
        let row = adw::ComboRow::new();
        row.set_title(&crate::tr!("Default layout"));
        row.set_model(Some(&gtk4::StringList::new(
            &labels.iter().map(String::as_str).collect::<Vec<_>>(),
        )));
        row.set_selected(selected);
        expander.add_row(&row);
        let layout = Self {
            parent: parent.clone(),
            save_dir: save_dir.to_string(),
            device: device.clone(),
            registry,
            row,
            expander: expander.clone(),
            paths: Rc::new(RefCell::new(paths)),
            last_real: Rc::new(RefCell::new(selected)),
            chosen: Rc::new(RefCell::new(state.profile_path.clone())),
        };
        layout.update_subtitle();
        layout.connect_selection_changed();
        layout
    }

    fn selected_path(&self) -> Option<std::path::PathBuf> {
        self.paths
            .borrow()
            .get(self.row.selected() as usize)
            .cloned()
            .flatten()
    }

    /// Virtual backend of the selected layout; `None` for the Disabled
    /// entry. The backend is read from the layout file, never from the row.
    fn selected_backend(&self) -> Option<ira_input::VirtualGamepadBackend> {
        let path = self.selected_path()?;
        read_profile(&path).ok().map(|profile| profile.backend)
    }

    fn update_subtitle(&self) {
        update_controller_subtitle(&self.expander, &self.device, self.selected_backend());
    }

    fn connect_selection_changed(&self) {
        let layout = self.clone();
        self.row.connect_selected_notify(move |row| {
            let sentinel = layout.paths.borrow().len().saturating_sub(1) as u32;
            if row.selected() == sentinel {
                // The create-new entry is not a layout: snap back and open
                // the editor on the device's default layout instead.
                let layout = layout.clone();
                let previous = *layout.last_real.borrow();
                glib::idle_add_local(move || {
                    layout.row.set_selected(previous);
                    layout.create_and_open_editor();
                    glib::ControlFlow::Break
                });
                return;
            }
            *layout.last_real.borrow_mut() = row.selected();
            if row.selected() != 0 {
                *layout.chosen.borrow_mut() = layout.selected_path();
            }
            layout.update_subtitle();
        });
    }

    /// Create the device's default layout if it does not exist yet, then
    /// open it in the editor.
    fn create_and_open_editor(&self) {
        let key = ira_config::Config::controller_key(self.device.vendor, self.device.product);
        let path = match ensure_controller_default_profile(
            &self.save_dir,
            &key,
            &self.device.name,
            &self.device.supported_buttons,
        ) {
            Ok(path) => path,
            Err(error) => {
                eprintln!("Failed to create controller mapping: {error}");
                return;
            }
        };
        self.open_editor(path);
    }

    fn open_editor(&self, profile_path: std::path::PathBuf) {
        let layout = self.clone();
        super::input_profile_editor::show_input_profile_editor(
            &self.parent,
            super::input_profile_editor::InputProfileEditorParams {
                save_dir: self.save_dir.clone(),
                profile_path: Some(profile_path),
                game_id: None,
                platform_id: None,
                layout_name: Some(self.device.name.clone()),
                registry: self.registry.clone(),
                device: Some(self.device.clone()),
            },
            move |saved| layout.refresh(Some(&saved)),
        );
    }

    /// Reload the picker after a layout was created or saved, keeping the
    /// saved file selected and the subtitle in step with its backend.
    fn refresh(&self, selected_path: Option<&std::path::Path>) {
        let (labels, paths, selected) = layout_choices(
            &self.save_dir,
            selected_path.map(std::path::Path::to_path_buf),
            &crate::tr!("Disabled"),
        );
        *self.paths.borrow_mut() = paths;
        self.row.set_model(Some(&gtk4::StringList::new(
            &labels.iter().map(String::as_str).collect::<Vec<_>>(),
        )));
        self.row.set_selected(selected);
        *self.last_real.borrow_mut() = selected;
        if selected != 0 {
            *self.chosen.borrow_mut() = selected_path.map(std::path::Path::to_path_buf);
        }
        self.update_subtitle();
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
    parent: &gtk4::Widget,
    group: &adw::PreferencesGroup,
    save_dir: &str,
    registry: std::sync::Arc<ira_input::ControllerRegistry>,
    configured_defaults: HashMap<String, ControllerInputConfig>,
    widgets: Rc<RefCell<Vec<ControllerDefaultWidgets>>>,
    no_controllers_row: Rc<RefCell<Option<adw::ActionRow>>>,
) {
    let group_for_refresh = group.clone();
    let parent_weak = parent.downgrade();
    let save_dir_for_refresh = save_dir.to_string();
    let widgets_for_refresh = widgets;
    let generation = Rc::new(Cell::new(registry.generation()));
    let generation_for_refresh = generation;
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let Some(parent) = parent_weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        // A closed settings window stays "visible" until destroyed, but it
        // stops being mapped, which ends the poll.
        if !parent.is_mapped() {
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

fn rebuild_profile_rows(
    group: &adw::PreferencesGroup,
    parent: &gtk4::Widget,
    save_dir: &str,
    steam: std::sync::Arc<ira_api::SteamDataClient>,
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
            add_profile_row(
                group,
                parent,
                save_dir,
                steam.clone(),
                stored,
                registry.clone(),
                rows,
            )
            .upcast(),
        );
    }
    let header_actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let import_from_steam = icon_button(
        "folder-download-symbolic",
        &crate::tr!("Import layout from Steam"),
    );
    let import_parent = parent.clone();
    let import_save_dir = save_dir.to_string();
    let import_steam = steam.clone();
    let import_registry = registry.clone();
    let import_group = group.clone();
    let import_rows = rows.clone();
    import_from_steam.connect_clicked(move |_| {
        let group = import_group.clone();
        let parent_for_saved = import_parent.clone();
        let save_dir = import_save_dir.clone();
        let steam_for_saved = import_steam.clone();
        let registry_for_saved = import_registry.clone();
        let rows = import_rows.clone();
        super::input_profile_search::show_steam_layout_search(
            &import_parent,
            &import_steam,
            &import_save_dir,
            None,
            Rc::new(move |_| {
                rebuild_profile_rows(
                    &group,
                    &parent_for_saved,
                    &save_dir,
                    steam_for_saved.clone(),
                    registry_for_saved.clone(),
                    &rows,
                )
            }),
        );
    });
    header_actions.append(&import_from_steam);

    let new_profile = icon_button("list-add-symbolic", &crate::tr!("Create profile"));
    let create_group = group.clone();
    let create_parent = parent.clone();
    let create_save_dir = save_dir.to_string();
    let create_steam = steam.clone();
    let create_registry = registry.clone();
    let create_rows = rows.clone();
    new_profile.connect_clicked(move |_| {
        let group = create_group.clone();
        let parent_for_saved = create_parent.clone();
        let save_dir = create_save_dir.clone();
        let steam_for_saved = create_steam.clone();
        let registry_for_saved = create_registry.clone();
        let rows = create_rows.clone();
        super::input_profile_editor::show_input_profile_editor(
            &create_parent,
            super::input_profile_editor::InputProfileEditorParams {
                save_dir: save_dir.clone(),
                profile_path: None,
                game_id: None,
                platform_id: None,
                layout_name: None,
                registry: create_registry.clone(),
                device: None,
            },
            move |_| {
                rebuild_profile_rows(
                    &group,
                    &parent_for_saved,
                    &save_dir,
                    steam_for_saved.clone(),
                    registry_for_saved.clone(),
                    &rows,
                )
            },
        );
    });
    header_actions.append(&new_profile);
    group.set_header_suffix(Some(&header_actions));
}

fn add_profile_row(
    group: &adw::PreferencesGroup,
    parent: &gtk4::Widget,
    save_dir: &str,
    steam: std::sync::Arc<ira_api::SteamDataClient>,
    stored: StoredProfile,
    registry: std::sync::Arc<ira_input::ControllerRegistry>,
    rows: &Rc<RefCell<Vec<gtk4::Widget>>>,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&esc(&profile_label(&stored)));
    let edit = icon_button("document-edit-symbolic", &crate::tr!("Edit profile"));
    let delete = icon_button("user-trash-symbolic", &crate::tr!("Delete profile"));
    row.add_suffix(&edit);
    row.add_suffix(&delete);
    let parent_for_edit = parent.clone();
    let save_dir_for_edit = save_dir.to_string();
    let steam_for_edit = steam.clone();
    let registry_for_edit = registry.clone();
    let path_for_edit = stored.path.clone();
    let group_for_edit = group.clone();
    let rows_for_edit = rows.clone();
    edit.connect_clicked(move |_| {
        let rows_for_saved = rows_for_edit.clone();
        super::input_profile_editor::show_input_profile_editor(
            &parent_for_edit,
            super::input_profile_editor::InputProfileEditorParams {
                save_dir: save_dir_for_edit.clone(),
                profile_path: Some(path_for_edit.clone()),
                game_id: None,
                platform_id: None,
                layout_name: None,
                registry: registry_for_edit.clone(),
                device: None,
            },
            {
                let group = group_for_edit.clone();
                let parent = parent_for_edit.clone();
                let save_dir = save_dir_for_edit.clone();
                let steam = steam_for_edit.clone();
                let registry = registry_for_edit.clone();
                move |_| {
                    rebuild_profile_rows(
                        &group,
                        &parent,
                        &save_dir,
                        steam.clone(),
                        registry.clone(),
                        &rows_for_saved,
                    )
                }
            },
        );
    });
    let parent_for_delete = parent.clone();
    let path_for_delete = stored.path;
    let group_for_delete = group.clone();
    let save_dir_for_delete = save_dir.to_string();
    let steam_for_delete = steam;
    let registry_for_delete = registry.clone();
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
        let steam = steam_for_delete.clone();
        let registry = registry_for_delete.clone();
        let rows = rows_for_delete.clone();
        alert.connect_response(None, move |_, response| {
            if response == "delete" {
                if let Err(error) = std::fs::remove_file(&path) {
                    eprintln!("Failed to delete controller layout: {error}");
                } else {
                    rebuild_profile_rows(
                        &group,
                        &parent,
                        &save_dir,
                        steam.clone(),
                        registry.clone(),
                        &rows,
                    );
                }
            }
        });
        alert.present(Some(&parent_for_delete));
    });
    group.add(&row);
    row
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
    use super::{input_mode_from_index, input_mode_index, layout_choices};
    use crate::ui::input_profile_store::{managed_profile_path, write_profile};
    use ira_input::InputProfile;
    use ira_models::ControllerInputMode;

    #[test]
    fn test_input_mode_from_index_preserves_inheritance() {
        assert_eq!(input_mode_from_index(0), None);
        assert_eq!(
            input_mode_from_index(1),
            Some(ControllerInputMode::Disabled)
        );
        assert_eq!(input_mode_from_index(2), Some(ControllerInputMode::Enabled));
    }

    #[test]
    fn test_input_mode_index_round_trip_all_three_entries() {
        for mode in [
            None,
            Some(ControllerInputMode::Disabled),
            Some(ControllerInputMode::Enabled),
        ] {
            assert_eq!(input_mode_from_index(input_mode_index(mode)), mode);
        }
    }

    #[test]
    fn test_input_mode_from_index_out_of_range_falls_back_to_inherit() {
        // The old dropdown offered controller types at indices 3-6; those
        // selections always behaved as Inherit and now cannot be produced.
        for index in [3u32, 4, 5, 6, 99] {
            assert_eq!(input_mode_from_index(index), None);
        }
    }

    #[test]
    fn test_layout_choices_empty_store_has_none_and_sentinel_only() {
        let tmp = tempfile::tempdir().unwrap();
        let (labels, paths, selected) =
            layout_choices(tmp.path().to_str().unwrap(), None, "Disabled");
        assert_eq!(labels, vec!["Disabled", "Create new profile..."]);
        assert_eq!(paths, vec![None, None]);
        assert_eq!(selected, 0);
    }

    #[test]
    fn test_layout_choices_lists_unscoped_hides_game_scoped() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().to_str().unwrap();
        write_profile(
            &managed_profile_path(save_dir, "Global"),
            &InputProfile {
                name: "Global".to_string(),
                ..InputProfile::default()
            },
        )
        .unwrap();
        write_profile(
            &managed_profile_path(save_dir, "Game only"),
            &InputProfile {
                name: "Game only".to_string(),
                compatible_game_ids: vec![7],
                ..InputProfile::default()
            },
        )
        .unwrap();
        let (labels, paths, selected) = layout_choices(save_dir, None, "Disabled");
        assert_eq!(labels, vec!["Disabled", "Global", "Create new profile..."]);
        assert!(paths[1].is_some());
        assert_eq!(selected, 0);
    }

    #[test]
    fn test_layout_choices_selects_the_current_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().to_str().unwrap();
        let path = managed_profile_path(save_dir, "Global");
        write_profile(
            &path,
            &InputProfile {
                name: "Global".to_string(),
                ..InputProfile::default()
            },
        )
        .unwrap();
        let (labels, _, selected) =
            layout_choices(save_dir, Some(path), "Disabled");
        assert_eq!(labels[selected as usize], "Global");
    }

    #[test]
    fn test_layout_choices_unknown_current_falls_back_to_none_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let save_dir = tmp.path().to_str().unwrap();
        write_profile(
            &managed_profile_path(save_dir, "Global"),
            &InputProfile {
                name: "Global".to_string(),
                ..InputProfile::default()
            },
        )
        .unwrap();
        let missing = tmp.path().join("elsewhere.json");
        let (_, _, selected) = layout_choices(save_dir, Some(missing), "Disabled");
        assert_eq!(selected, 0);
    }
}
