use super::css::{CSS_ERROR, CSS_FLAT, CSS_SUGGESTED_ACTION};
use super::input_calibration_dialog::show_input_calibration_dialog;
use super::input_profile_binding::{
    add_binding_row, add_empty_page_state, binding_from_row, binding_page_index,
    binding_section_title, BindingRow, BindingRowContext, SectionGroups,
};
use super::input_profile_store::{new_managed_profile_path, read_profile, write_profile};
use adw::prelude::*;
use ira_input::{
    AxisDirection, Binding, DeviceInfo, GamepadAxis, GamepadButton, GyroAxis, InputProfile,
    InputSource, OutputAction, VirtualGamepadBackend,
};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

#[derive(Clone)]
struct ProfileForm {
    name: Rc<RefCell<String>>,
    rows: Rc<RefCell<Vec<BindingRow>>>,
    calibration: Rc<RefCell<ira_input::GyroCalibration>>,
    compatible_game_ids: Vec<i64>,
    game_id: Option<i64>,
    backend: Rc<RefCell<VirtualGamepadBackend>>,
}

#[derive(Clone)]
struct BindingCollectionContext {
    section_groups: SectionGroups,
    rows: Rc<RefCell<Vec<BindingRow>>>,
    device: Option<DeviceInfo>,
    backend: Rc<RefCell<VirtualGamepadBackend>>,
    mark_dirty: Rc<dyn Fn()>,
}

struct PageSetup {
    page_boxes: Vec<gtk4::Box>,
    section_groups: SectionGroups,
    backend_dropdown: adw::ComboRow,
}

pub(super) struct InputProfileEditorParams {
    pub save_dir: String,
    pub profile_path: Option<PathBuf>,
    pub game_id: Option<i64>,
    pub layout_name: Option<String>,
    pub registry: Arc<ira_input::ControllerRegistry>,
    pub device: Option<DeviceInfo>,
}

pub(super) fn show_input_profile_editor(
    parent: &gtk4::Window,
    params: InputProfileEditorParams,
    on_saved: impl Fn(PathBuf) + 'static,
) {
    let InputProfileEditorParams {
        save_dir,
        profile_path,
        game_id,
        layout_name,
        registry,
        device,
    } = params;
    let layout = super::helpers::dialog_layout(parent);
    layout.window.set_default_size(900, 720);
    layout.stack.set_hexpand(true);
    layout.stack.set_vexpand(true);

    let (mut profile, initial_status, initial_error) = match profile_path.as_deref() {
        Some(path) => match read_profile(path) {
            Ok(profile) => (profile, String::new(), false),
            Err(error) => (
                InputProfile::default(),
                format!("Could not load profile: {error}"),
                true,
            ),
        },
        None => (InputProfile::default(), String::new(), false),
    };
    if let Some(layout_name) = layout_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        if profile.name.trim().is_empty() {
            profile.name = layout_name.to_string();
        }
    }
    let calibration = Rc::new(RefCell::new(profile.gyro_calibration));
    let compatible_game_ids = profile.compatible_game_ids.clone();
    let profile_name = Rc::new(RefCell::new(profile.name.clone()));
    let detected_devices = registry.snapshot();
    let device_was_explicit = device.is_some();
    let device = device.or_else(|| detected_devices.first().cloned());
    let calibration_device = (device_was_explicit || detected_devices.len() == 1)
        .then(|| device.clone())
        .flatten();
    let original_profile = profile_with_game(profile.clone(), game_id);
    let backend = Rc::new(RefCell::new(profile.backend));

    let rows = Rc::new(RefCell::new(Vec::<BindingRow>::new()));
    let current_path = Rc::new(RefCell::new(profile_path));
    let status = gtk4::Label::new(Some(&initial_status));
    status.set_xalign(0.0);
    status.set_visible(!initial_status.is_empty());
    if initial_error {
        status.add_css_class(CSS_ERROR);
    }
    let save = gtk4::Button::with_label("Save");
    save.add_css_class(CSS_SUGGESTED_ACTION);
    save.set_sensitive(false);
    let mark_dirty: Rc<dyn Fn()> = {
        let save = save.clone();
        let rows = rows.clone();
        let name = profile_name.clone();
        let compatible_game_ids = compatible_game_ids.clone();
        let original_profile = original_profile.clone();
        let calibration = calibration.clone();
        let backend = backend.clone();
        Rc::new(move || {
            let result = build_profile(
                &name.borrow(),
                &rows.borrow(),
                *calibration.borrow(),
                &compatible_game_ids,
                game_id,
                *backend.borrow(),
            );
            save.set_sensitive(
                result
                    .as_ref()
                    .is_ok_and(|profile| profile != &original_profile),
            );
        })
    };

    let setup = setup_pages(
        &layout,
        &backend,
        &rows,
        &device,
        &mark_dirty,
        profile.bindings,
    );
    let page_boxes = setup.page_boxes;
    let section_groups = setup.section_groups;
    let backend_dropdown = setup.backend_dropdown;

    add_calibration_group(
        &page_boxes[4],
        &calibration,
        calibration_device,
        &registry,
        &mark_dirty,
    );

    let reset = reset_button(
        &layout.window,
        &page_boxes,
        &section_groups,
        &rows,
        &device,
        &backend,
        &mark_dirty,
    );
    connect_backend_change(
        &backend_dropdown,
        &backend,
        &page_boxes,
        &section_groups,
        &rows,
        &device,
        &mark_dirty,
    );
    layout.header.pack_end(&reset);

    setup_sidebar(&layout);

    let cancel = add_editor_footer(&layout, &save, &status, &profile_name, &mark_dirty);

    let form = ProfileForm {
        name: profile_name,
        rows,
        calibration,
        compatible_game_ids,
        game_id,
        backend,
    };
    let on_saved: Rc<dyn Fn(PathBuf)> = Rc::new(on_saved);
    let window_for_save = layout.window.clone();
    connect_save(
        &save,
        &save_dir,
        current_path,
        form,
        &status,
        &window_for_save,
        on_saved,
    );
    let window_for_cancel = layout.window.clone();
    cancel.connect_clicked(move |_| window_for_cancel.close());
    layout.window.present();
}

fn reset_button(
    window: &adw::Window,
    page_boxes: &[gtk4::Box],
    section_groups: &SectionGroups,
    rows: &Rc<RefCell<Vec<BindingRow>>>,
    device: &Option<DeviceInfo>,
    backend: &Rc<RefCell<VirtualGamepadBackend>>,
    mark_dirty: &Rc<dyn Fn()>,
) -> gtk4::Button {
    let reset = gtk4::Button::new();
    reset.add_css_class(CSS_FLAT);
    reset.set_tooltip_text(Some(
        "Replace the layout with the standard default bindings for this controller",
    ));
    let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    content.append(&gtk4::Image::from_icon_name("view-refresh-symbolic"));
    content.append(&gtk4::Label::new(Some("Reset to defaults")));
    reset.set_child(Some(&content));
    let page_boxes = page_boxes.to_vec();
    let section_groups = section_groups.clone();
    let rows = rows.clone();
    let device = device.clone();
    let backend = backend.clone();
    let mark_dirty = mark_dirty.clone();
    let window = window.clone();
    reset.connect_clicked(move |_| {
        let dialog = adw::AlertDialog::new(
            Some("Reset to defaults?"),
            Some("Replace the current layout with the standard one-to-one default bindings for this controller."),
        );
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("reset", "Reset");
        dialog.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let page_boxes = page_boxes.clone();
        let section_groups = section_groups.clone();
        let rows = rows.clone();
        let device = device.clone();
        let backend = backend.clone();
        let mark_dirty = mark_dirty.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "reset" {
                reset_to_defaults(
                    &page_boxes,
                    &section_groups,
                    &rows,
                    device.as_ref(),
                    &backend,
                    &mark_dirty,
                );
            }
        });
        dialog.present(Some(&window));
    });
    reset
}

fn add_editor_footer(
    layout: &super::helpers::DialogLayout,
    save: &gtk4::Button,
    status: &gtk4::Label,
    profile_name: &Rc<RefCell<String>>,
    mark_dirty: &Rc<dyn Fn()>,
) -> gtk4::Button {
    let cancel = gtk4::Button::with_label("Cancel");
    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    actions.set_halign(gtk4::Align::End);
    actions.set_margin_start(16);
    actions.set_margin_end(16);
    actions.set_margin_top(8);
    actions.set_margin_bottom(12);
    actions.append(&cancel);
    actions.append(save);
    let name = adw::EntryRow::new();
    name.set_title("Profile name");
    name.set_text(&profile_name.borrow());
    let profile_name_for_changed = profile_name.clone();
    let mark_dirty_for_changed = mark_dirty.clone();
    name.connect_changed(move |entry| {
        *profile_name_for_changed.borrow_mut() = entry.text().to_string();
        mark_dirty_for_changed();
    });
    layout.content_area.append(&name);
    layout.content_area.append(status);
    layout.content_area.append(&actions);
    cancel
}

fn setup_pages(
    layout: &super::helpers::DialogLayout,
    backend: &Rc<RefCell<VirtualGamepadBackend>>,
    rows: &Rc<RefCell<Vec<BindingRow>>>,
    device: &Option<DeviceInfo>,
    mark_dirty: &Rc<dyn Fn()>,
    bindings: Vec<Binding>,
) -> PageSetup {
    let page_boxes = (0..5)
        .map(|_| gtk4::Box::new(gtk4::Orientation::Vertical, 10))
        .collect::<Vec<_>>();
    let section_groups = Rc::new(RefCell::new(
        (0..5)
            .map(|_| Vec::<(String, adw::PreferencesGroup)>::new())
            .collect(),
    ));
    let backend_dropdown = adw::ComboRow::new();
    backend_dropdown.set_title("Virtual gamepad backend");
    backend_dropdown.set_model(Some(&gtk4::StringList::new(&["XInput", "DirectInput"])));
    backend_dropdown.set_selected((*backend.borrow() as u32).min(1));
    layout.content_area.append(&backend_dropdown);
    for (index, (page_id, title, icon)) in page_descriptors().into_iter().enumerate() {
        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_child(Some(&page_boxes[index]));
        let stack_page = layout.stack.add_titled(&scroll, Some(page_id), title);
        stack_page.set_icon_name(icon);
    }
    let context = BindingCollectionContext {
        section_groups: section_groups.clone(),
        rows: rows.clone(),
        device: device.clone(),
        backend: backend.clone(),
        mark_dirty: mark_dirty.clone(),
    };
    for (page, default_binding) in page_boxes.iter().zip(default_bindings()) {
        let add_binding = gtk4::Button::from_icon_name("list-add-symbolic");
        add_binding.add_css_class(CSS_FLAT);
        add_binding.set_tooltip_text(Some("Add binding"));
        connect_add_binding(&add_binding, default_binding, &context, page);
        let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        actions.set_halign(gtk4::Align::End);
        actions.set_valign(gtk4::Align::Center);
        actions.set_margin_bottom(6);
        actions.append(&add_binding);
        page.append(&actions);
    }
    populate_bindings(
        &page_boxes,
        &section_groups,
        rows,
        bindings,
        device.as_ref(),
        backend,
        mark_dirty,
    );
    PageSetup {
        page_boxes,
        section_groups,
        backend_dropdown,
    }
}

fn page_descriptors() -> [(&'static str, &'static str, &'static str); 5] {
    [
        ("buttons", "Buttons", "input-gaming-symbolic"),
        ("dpad", "D-pad", "view-grid-symbolic"),
        ("triggers", "Triggers", "media-seek-forward-symbolic"),
        ("joysticks", "Joysticks", "input-gaming-symbolic"),
        ("gyro", "Gyro", "view-refresh-symbolic"),
    ]
}

fn default_bindings() -> [Binding; 5] {
    [
        Binding::new(
            InputSource::Button(GamepadButton::A),
            OutputAction::GamepadButton(GamepadButton::A),
        ),
        Binding::new(
            InputSource::Button(GamepadButton::DpadUp),
            OutputAction::GamepadButton(GamepadButton::DpadUp),
        ),
        Binding::new(
            InputSource::Axis(GamepadAxis::LeftTrigger),
            OutputAction::GamepadAxis(GamepadAxis::LeftTrigger),
        ),
        Binding::new(
            InputSource::Axis(GamepadAxis::LeftX),
            OutputAction::GamepadAxis(GamepadAxis::LeftX),
        ),
        Binding::new(
            InputSource::Gyro(GyroAxis::X),
            OutputAction::GamepadAxis(GamepadAxis::RightX),
        ),
    ]
}

fn setup_sidebar(layout: &super::helpers::DialogLayout) {
    for (page_id, title, icon) in page_descriptors() {
        layout
            .sidebar
            .append(&super::settings_dialog::settings_sidebar_row(
                icon, title, page_id,
            ));
    }
    let stack = layout.stack.clone();
    layout.sidebar.connect_row_selected(move |_, row| {
        if let Some(row) = row {
            stack.set_visible_child_name(&row.widget_name());
        }
    });
    layout
        .sidebar
        .select_row(layout.sidebar.row_at_index(0).as_ref());
}

fn connect_backend_change(
    dropdown: &adw::ComboRow,
    backend: &Rc<RefCell<VirtualGamepadBackend>>,
    page_boxes: &[gtk4::Box],
    section_groups: &SectionGroups,
    rows: &Rc<RefCell<Vec<BindingRow>>>,
    device: &Option<DeviceInfo>,
    mark_dirty: &Rc<dyn Fn()>,
) {
    let backend = backend.clone();
    let page_boxes = page_boxes.to_vec();
    let section_groups = section_groups.clone();
    let rows = rows.clone();
    let device = device.clone();
    let mark_dirty = mark_dirty.clone();
    dropdown.connect_selected_notify(move |dropdown| {
        let selected = if dropdown.selected() == 1 {
            VirtualGamepadBackend::DirectInput
        } else {
            VirtualGamepadBackend::XInput
        };
        if *backend.borrow() == selected {
            return;
        }
        *backend.borrow_mut() = selected;
        reset_to_defaults(
            &page_boxes,
            &section_groups,
            &rows,
            device.as_ref(),
            &backend,
            &mark_dirty,
        );
    });
}

fn section_group(
    page: &gtk4::Box,
    groups: &SectionGroups,
    page_index: usize,
    title: &str,
) -> adw::PreferencesGroup {
    if let Some(group) = groups.borrow().get(page_index).and_then(|sections| {
        sections
            .iter()
            .find(|(section, _)| section == title)
            .map(|(_, group)| group.clone())
    }) {
        return group;
    }
    let group = adw::PreferencesGroup::new();
    group.set_title(title);
    if title == "Custom" {
        // Keep Custom immediately before the fixed add-binding controls.
        page.insert_child_after(&group, page.first_child().as_ref());
    } else {
        page.append(&group);
    }
    groups
        .borrow_mut()
        .get_mut(page_index)
        .expect("profile page index must be in range")
        .push((title.to_string(), group.clone()));
    group
}

fn add_calibration_group(
    page: &gtk4::Box,
    calibration: &Rc<RefCell<ira_input::GyroCalibration>>,
    device: Option<DeviceInfo>,
    registry: &Arc<ira_input::ControllerRegistry>,
    mark_dirty: &Rc<dyn Fn()>,
) {
    let group = adw::PreferencesGroup::new();
    group.set_title("Calibration");
    let summary = adw::ActionRow::new();
    summary.set_title("Profile gyro calibration");
    summary.set_subtitle("These bias values belong to this profile");
    let values = gtk4::Label::new(None);
    values.set_xalign(1.0);
    values.add_css_class("dim-label");
    values.set_text(&format_calibration(*calibration.borrow()));
    summary.add_suffix(&values);
    group.add(&summary);

    let actions = adw::ActionRow::new();
    actions.set_title("Calibrate while controller is flat and still");
    let calibrate = gtk4::Button::with_label("Calibrate");
    calibrate.set_valign(gtk4::Align::Center);
    let reset = gtk4::Button::with_label("Reset");
    reset.set_valign(gtk4::Align::Center);
    actions.add_suffix(&reset);
    actions.add_suffix(&calibrate);
    group.add(&actions);
    page.prepend(&group);

    let calibration_for_reset = calibration.clone();
    let values_for_reset = values.clone();
    let mark_dirty_for_reset = mark_dirty.clone();
    reset.connect_clicked(move |_| {
        *calibration_for_reset.borrow_mut() = ira_input::GyroCalibration::default();
        values_for_reset.set_text(&format_calibration(*calibration_for_reset.borrow()));
        mark_dirty_for_reset();
    });

    if device.is_none() {
        calibrate.set_sensitive(false);
        actions.set_subtitle("Calibration requires one connected gyro-capable controller");
    }
    let Some(device) = device else { return };
    let calibration_for_dialog = calibration.clone();
    let values_for_dialog = values.clone();
    let mark_dirty_for_dialog = mark_dirty.clone();
    let registry = registry.clone();
    calibrate.connect_clicked(move |button| {
        let Some(window) = button
            .root()
            .and_then(|root| root.downcast::<gtk4::Window>().ok())
        else {
            return;
        };
        let calibration = calibration_for_dialog.clone();
        let values = values_for_dialog.clone();
        let mark_dirty = mark_dirty_for_dialog.clone();
        show_input_calibration_dialog(&window, registry.clone(), device.clone(), move |result| {
            match result {
                Ok(value) => {
                    *calibration.borrow_mut() = value;
                    values.set_text(&format_calibration(value));
                    mark_dirty();
                }
                Err(error) => eprintln!("Gyro calibration failed: {error}"),
            }
        });
    });
}

fn format_calibration(calibration: ira_input::GyroCalibration) -> String {
    format!(
        "X {:.3}  Y {:.3}  Z {:.3}",
        calibration.x, calibration.y, calibration.z
    )
}

fn ensure_section_behavior(
    group: &adw::PreferencesGroup,
    title: &str,
    page: &gtk4::Box,
    context: &BindingCollectionContext,
) {
    if group.widget_name() == "section-behavior" {
        return;
    }
    group.set_widget_name("section-behavior");
    let choices = behavior_choices(title);
    let behavior = gtk4::DropDown::new(
        Some(gtk4::StringList::new(
            &choices.iter().map(String::as_str).collect::<Vec<_>>(),
        )),
        None::<&gtk4::Expression>,
    );
    behavior.set_selected(0);
    let row = adw::ActionRow::new();
    row.set_title("Behavior");
    row.set_subtitle("Apply this behavior to every binding in this section");
    behavior.set_valign(gtk4::Align::Center);
    row.add_suffix(&behavior);
    group.add(&row);
    let page = page.clone();
    let context = context.clone();
    let title = title.to_string();
    behavior.connect_selected_notify(move |dropdown| {
        if dropdown.selected() == 0 {
            return;
        }
        replace_section_bindings(&page, &title, dropdown.selected(), &context);
    });
}

fn behavior_choices(title: &str) -> Vec<String> {
    if matches!(title, "Left Stick" | "Right Stick") {
        vec!["Custom", "Default", "Stick", "Directional Pad"]
            .into_iter()
            .map(str::to_string)
            .collect()
    } else {
        vec!["Custom", "Default"]
            .into_iter()
            .map(str::to_string)
            .collect()
    }
}

fn section_behavior_bindings(
    title: &str,
    behavior: u32,
    device: Option<&DeviceInfo>,
    backend: VirtualGamepadBackend,
) -> Vec<Binding> {
    match behavior {
        1 => default_profile_bindings(device, backend)
            .into_iter()
            .filter(|binding| binding_section_title(binding) == title)
            .collect(),
        2 if title == "Left Stick" => vec![
            Binding::new(
                InputSource::Axis(GamepadAxis::LeftX),
                OutputAction::GamepadAxis(GamepadAxis::LeftX),
            ),
            Binding::new(
                InputSource::Axis(GamepadAxis::LeftY),
                OutputAction::GamepadAxis(GamepadAxis::LeftY),
            ),
        ],
        2 if title == "Right Stick" => vec![
            Binding::new(
                InputSource::Axis(GamepadAxis::RightX),
                OutputAction::GamepadAxis(GamepadAxis::RightX),
            ),
            Binding::new(
                InputSource::Axis(GamepadAxis::RightY),
                OutputAction::GamepadAxis(GamepadAxis::RightY),
            ),
        ],
        3 if title == "Left Stick" => {
            stick_to_dpad_bindings(GamepadAxis::LeftX, GamepadAxis::LeftY).to_vec()
        }
        3 if title == "Right Stick" => {
            stick_to_dpad_bindings(GamepadAxis::RightX, GamepadAxis::RightY).to_vec()
        }
        _ => Vec::new(),
    }
}

fn replace_section_bindings(
    page: &gtk4::Box,
    title: &str,
    behavior: u32,
    context: &BindingCollectionContext,
) {
    let removed = context
        .rows
        .borrow()
        .iter()
        .filter(|row| {
            binding_from_row(row).is_ok_and(|binding| binding_section_title(&binding) == title)
        })
        .map(|row| row.container.clone())
        .collect::<Vec<_>>();
    if let Some(group) = context
        .section_groups
        .borrow()
        .iter()
        .flatten()
        .find(|(name, _)| name == title)
        .map(|(_, group)| group.clone())
    {
        for container in &removed {
            group.remove(container);
        }
        context
            .rows
            .borrow_mut()
            .retain(|row| !removed.contains(&row.container));
        for binding in section_behavior_bindings(
            title,
            behavior,
            context.device.as_ref(),
            *context.backend.borrow(),
        ) {
            add_binding_row(
                group.as_ref(),
                binding,
                &BindingRowContext {
                    page: page.clone(),
                    section_groups: context.section_groups.clone(),
                    rows: context.rows.clone(),
                    device: context.device.clone(),
                    backend: *context.backend.borrow(),
                    on_dirty: context.mark_dirty.clone(),
                },
            );
        }
    }
    (context.mark_dirty)();
}

fn clear_empty_page_state(page: &gtk4::Box) {
    let mut child = page.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        if current.widget_name() == "input-empty-state" {
            page.remove(&current);
        }
    }
}

fn connect_add_binding(
    button: &gtk4::Button,
    binding: Binding,
    context: &BindingCollectionContext,
    page: &gtk4::Box,
) {
    let page = page.clone();
    let context = context.clone();
    button.connect_clicked(move |_| {
        clear_empty_page_state(&page);
        let group = section_group(
            &page,
            &context.section_groups,
            binding_page_index(&binding),
            "Custom",
        );
        add_binding_row(
            &group,
            binding.clone(),
            &BindingRowContext {
                page: page.clone(),
                section_groups: context.section_groups.clone(),
                rows: context.rows.clone(),
                device: context.device.clone(),
                backend: *context.backend.borrow(),
                on_dirty: context.mark_dirty.clone(),
            },
        );
        (context.mark_dirty)();
    });
}

fn default_profile_bindings(
    device: Option<&DeviceInfo>,
    backend: VirtualGamepadBackend,
) -> Vec<Binding> {
    match device {
        Some(device) => {
            InputProfile::default_gamepad_for_backend_and_buttons(
                backend,
                &device.supported_buttons,
            )
            .bindings
        }
        None => InputProfile::default_gamepad_for_backend(backend).bindings,
    }
}

fn populate_bindings(
    page_boxes: &[gtk4::Box],
    section_groups: &SectionGroups,
    rows: &Rc<RefCell<Vec<BindingRow>>>,
    bindings: Vec<Binding>,
    device: Option<&DeviceInfo>,
    backend: &Rc<RefCell<VirtualGamepadBackend>>,
    mark_dirty: &Rc<dyn Fn()>,
) {
    let mut page_has_bindings = [false; 5];
    let mut bindings = bindings;
    bindings.sort_by_key(|binding| {
        (
            binding_page_index(binding),
            section_order(binding_section_title(binding)),
            binding_order(binding),
        )
    });
    for binding in bindings.into_iter() {
        let page_index = binding_page_index(&binding);
        let title = binding_section_title(&binding);
        page_has_bindings[page_index] = true;
        let group = section_group(&page_boxes[page_index], section_groups, page_index, title);
        let context = BindingCollectionContext {
            section_groups: section_groups.clone(),
            rows: rows.clone(),
            device: device.cloned(),
            backend: backend.clone(),
            mark_dirty: mark_dirty.clone(),
        };
        ensure_section_behavior(&group, title, &page_boxes[page_index], &context);
        add_binding_row(
            &group,
            binding,
            &BindingRowContext {
                page: page_boxes[page_index].clone(),
                section_groups: section_groups.clone(),
                rows: rows.clone(),
                device: device.cloned(),
                backend: *backend.borrow(),
                on_dirty: mark_dirty.clone(),
            },
        );
    }
    for (page, has_bindings) in page_boxes.iter().zip(page_has_bindings) {
        if !has_bindings {
            add_empty_page_state(page);
        }
    }
}

fn clear_page_bindings(
    page_boxes: &[gtk4::Box],
    section_groups: &SectionGroups,
    rows: &Rc<RefCell<Vec<BindingRow>>>,
) {
    rows.borrow_mut().clear();
    section_groups.borrow_mut().iter_mut().for_each(Vec::clear);
    for page in page_boxes {
        let mut child = page.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            if current.is::<adw::PreferencesGroup>() || current.widget_name() == "input-empty-state"
            {
                page.remove(&current);
            }
        }
    }
}

fn reset_to_defaults(
    page_boxes: &[gtk4::Box],
    section_groups: &SectionGroups,
    rows: &Rc<RefCell<Vec<BindingRow>>>,
    device: Option<&DeviceInfo>,
    backend: &Rc<RefCell<VirtualGamepadBackend>>,
    mark_dirty: &Rc<dyn Fn()>,
) {
    clear_page_bindings(page_boxes, section_groups, rows);
    populate_bindings(
        page_boxes,
        section_groups,
        rows,
        default_profile_bindings(device, *backend.borrow()),
        device,
        backend,
        mark_dirty,
    );
    mark_dirty();
}

fn stick_to_dpad_bindings(x_axis: GamepadAxis, y_axis: GamepadAxis) -> [Binding; 4] {
    [
        axis_button_binding(x_axis, AxisDirection::Negative, GamepadButton::DpadLeft),
        axis_button_binding(x_axis, AxisDirection::Positive, GamepadButton::DpadRight),
        axis_button_binding(y_axis, AxisDirection::Negative, GamepadButton::DpadUp),
        axis_button_binding(y_axis, AxisDirection::Positive, GamepadButton::DpadDown),
    ]
}

fn section_order(title: &str) -> usize {
    match title {
        "Face Buttons" => 0,
        "Bumpers" => 1,
        "Extended Buttons" => 2,
        "Menu Buttons" => 3,
        "Stick Clicks" => 4,
        "D-pad" => 0,
        "Triggers" => 0,
        "Left Stick" => 0,
        "Right Stick" => 1,
        "Gyro" => 0,
        "Custom" => 0,
        _ => usize::MAX,
    }
}

fn binding_order(binding: &Binding) -> usize {
    match binding.source {
        InputSource::Button(button) => match button {
            GamepadButton::A => 0,
            GamepadButton::B => 1,
            GamepadButton::X => 2,
            GamepadButton::Y => 3,
            GamepadButton::LeftShoulder => 0,
            GamepadButton::RightShoulder => 1,
            GamepadButton::Paddle2 => 0,
            GamepadButton::Paddle1 => 1,
            GamepadButton::Paddle4 => 2,
            GamepadButton::Paddle3 => 3,
            GamepadButton::Paddle5 => 4,
            GamepadButton::Paddle6 => 5,
            GamepadButton::Paddle7 => 6,
            GamepadButton::Paddle8 => 7,
            GamepadButton::Back => 0,
            GamepadButton::Start => 1,
            GamepadButton::Guide => 2,
            GamepadButton::LeftStick => 0,
            GamepadButton::RightStick => 1,
            GamepadButton::LeftTrigger => 0,
            GamepadButton::RightTrigger => 1,
            GamepadButton::DpadUp => 0,
            GamepadButton::DpadDown => 1,
            GamepadButton::DpadLeft => 2,
            GamepadButton::DpadRight => 3,
        },
        InputSource::Axis(axis) => match axis {
            GamepadAxis::LeftX => 0,
            GamepadAxis::LeftY => 1,
            GamepadAxis::RightX => 2,
            GamepadAxis::RightY => 3,
            GamepadAxis::LeftTrigger => 0,
            GamepadAxis::RightTrigger => 1,
        },
        InputSource::AxisDirection { axis, .. } => binding_order(&Binding::new(
            InputSource::Axis(axis),
            OutputAction::GamepadAxis(axis),
        )),
        InputSource::Gyro(axis) => axis as usize,
    }
}

fn axis_button_binding(
    axis: GamepadAxis,
    direction: AxisDirection,
    button: GamepadButton,
) -> Binding {
    Binding::new(
        InputSource::AxisDirection { axis, direction },
        OutputAction::GamepadButton(button),
    )
}

fn connect_save(
    button: &gtk4::Button,
    save_dir: &str,
    current_path: Rc<RefCell<Option<PathBuf>>>,
    form: ProfileForm,
    status: &gtk4::Label,
    window: &adw::Window,
    on_saved: Rc<dyn Fn(PathBuf)>,
) {
    let save_dir = save_dir.to_string();
    let form = form.clone();
    let status = status.clone();
    let window = window.clone();
    let button_for_save = button.clone();
    button.connect_clicked(move |_| {
        let result = build_profile(
            &form.name.borrow(),
            &form.rows.borrow(),
            *form.calibration.borrow(),
            &form.compatible_game_ids,
            form.game_id,
            *form.backend.borrow(),
        );
        save_result(
            result,
            &save_dir,
            &current_path,
            &status,
            &button_for_save,
            &window,
            &on_saved,
        );
    });
}

fn save_result(
    result: Result<InputProfile, String>,
    save_dir: &str,
    current_path: &Rc<RefCell<Option<PathBuf>>>,
    status: &gtk4::Label,
    button: &gtk4::Button,
    window: &adw::Window,
    on_saved: &Rc<dyn Fn(PathBuf)>,
) {
    let result = result.and_then(|profile| {
        let path = profile_path_for_save(save_dir, current_path.borrow().as_deref(), &profile.name);
        write_profile(&path, &profile)?;
        Ok(path)
    });
    match result {
        Ok(path) => {
            *current_path.borrow_mut() = Some(path.clone());
            button.set_sensitive(false);
            on_saved(path);
            window.close();
        }
        Err(error) => {
            set_error(status, &error);
        }
    }
}

fn profile_path_for_save(save_dir: &str, current_path: Option<&Path>, name: &str) -> PathBuf {
    current_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| new_managed_profile_path(save_dir, name))
}

fn build_profile(
    name: &str,
    rows: &[BindingRow],
    calibration: ira_input::GyroCalibration,
    compatible_game_ids: &[i64],
    game_id: Option<i64>,
    backend: VirtualGamepadBackend,
) -> Result<InputProfile, String> {
    let mut compatible_game_ids = compatible_game_ids.to_vec();
    if let Some(game_id) = game_id {
        if !compatible_game_ids.contains(&game_id) {
            compatible_game_ids.push(game_id);
        }
    }
    let profile = InputProfile {
        name: name.trim().to_string(),
        bindings: rows
            .iter()
            .map(binding_from_row)
            .collect::<Result<_, _>>()?,
        gyro_calibration: calibration,
        compatible_game_ids,
        backend,
        ..InputProfile::default()
    };
    profile.validate()?;
    Ok(profile)
}

fn profile_with_game(mut profile: InputProfile, game_id: Option<i64>) -> InputProfile {
    if let Some(game_id) = game_id {
        if !profile.compatible_game_ids.contains(&game_id) {
            profile.compatible_game_ids.push(game_id);
        }
    }
    profile
}

fn set_error(status: &gtk4::Label, text: &str) {
    status.set_text(text);
    status.set_visible(true);
    status.set_css_classes(&[CSS_ERROR]);
}

#[cfg(test)]
mod tests {
    use super::{
        build_profile, default_profile_bindings, profile_path_for_save, section_behavior_bindings,
        section_order, stick_to_dpad_bindings,
    };
    use ira_input::{
        AxisDirection, DeviceInfo, GamepadAxis, GamepadButton, GyroCalibration, InputSource,
        OutputAction, VirtualGamepadBackend,
    };
    use std::path::PathBuf;

    #[test]
    fn test_default_profile_bindings_without_device_omit_paddles() {
        let bindings = default_profile_bindings(None, VirtualGamepadBackend::XInput);
        assert!(!bindings.is_empty());
        assert!(!bindings.iter().any(
            |binding| matches!(binding.source, InputSource::Button(button) if button.is_paddle())
        ));
    }

    #[test]
    fn test_default_profile_bindings_include_only_supported_buttons() {
        let device = DeviceInfo {
            path: PathBuf::from("/dev/input/event0"),
            name: "Test controller".to_string(),
            vendor: 0,
            product: 0,
            version: 0,
            has_evdev_gyro: false,
            supported_buttons: vec![GamepadButton::A, GamepadButton::Paddle1],
        };
        let bindings = default_profile_bindings(Some(&device), VirtualGamepadBackend::XInput);
        assert!(bindings
            .iter()
            .any(|binding| binding.source == InputSource::Button(GamepadButton::A)));
        assert!(!bindings
            .iter()
            .any(|binding| { binding.source == InputSource::Button(GamepadButton::Paddle1) }));
        assert!(!bindings
            .iter()
            .any(|binding| binding.source == InputSource::Button(GamepadButton::B)));
    }

    #[test]
    fn test_stick_to_dpad_preset_assigns_each_direction() {
        let bindings = stick_to_dpad_bindings(GamepadAxis::LeftX, GamepadAxis::LeftY);
        assert_eq!(
            bindings[0].source,
            InputSource::AxisDirection {
                axis: GamepadAxis::LeftX,
                direction: AxisDirection::Negative,
            }
        );
        assert_eq!(
            bindings[1].output,
            OutputAction::GamepadButton(GamepadButton::DpadRight)
        );
        assert_eq!(
            bindings[2].output,
            OutputAction::GamepadButton(GamepadButton::DpadUp)
        );
        assert_eq!(
            bindings[3].output,
            OutputAction::GamepadButton(GamepadButton::DpadDown)
        );
    }

    #[test]
    fn test_buttons_sections_follow_steam_order() {
        assert!(section_order("Face Buttons") < section_order("Bumpers"));
        assert!(section_order("Bumpers") < section_order("Extended Buttons"));
        assert!(section_order("Extended Buttons") < section_order("Menu Buttons"));
        assert!(section_order("Menu Buttons") < section_order("Stick Clicks"));
    }

    #[test]
    fn test_stick_behavior_replaces_with_directional_bindings() {
        let bindings =
            section_behavior_bindings("Left Stick", 3, None, VirtualGamepadBackend::XInput);
        assert_eq!(bindings.len(), 4);
        assert!(bindings.iter().all(|binding| matches!(
            binding.output,
            OutputAction::GamepadButton(
                GamepadButton::DpadUp
                    | GamepadButton::DpadDown
                    | GamepadButton::DpadLeft
                    | GamepadButton::DpadRight
            )
        )));
        let default =
            section_behavior_bindings("Left Stick", 2, None, VirtualGamepadBackend::XInput);
        assert_eq!(default.len(), 2);
        assert_ne!(default, bindings);
    }

    #[test]
    fn test_custom_section_sorts_before_standard_sections() {
        assert!(section_order("Custom") < section_order("Stick Clicks"));
    }

    #[test]
    fn test_profile_save_keeps_existing_path() {
        let tmp = tempfile::tempdir().unwrap();
        let old_path = tmp.path().join("old-name.json");
        let saved_path =
            profile_path_for_save(tmp.path().to_str().unwrap(), Some(&old_path), "New name");
        assert_eq!(saved_path, old_path);
    }

    #[test]
    fn test_build_profile_uses_updated_calibration() {
        let calibration = GyroCalibration {
            x: 1.0,
            y: -2.0,
            z: 3.0,
        };
        let profile = build_profile(
            "Test",
            &[],
            calibration,
            &[],
            None,
            VirtualGamepadBackend::XInput,
        )
        .unwrap();
        assert_eq!(profile.gyro_calibration, calibration);
    }

    #[test]
    fn test_default_calibration_is_zeroed() {
        let calibration = GyroCalibration::default();
        assert_eq!(
            (calibration.x, calibration.y, calibration.z),
            (0.0, 0.0, 0.0)
        );
    }
}
