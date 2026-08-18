use super::input_profile_binding::{
    add_binding_row, binding_from_row, binding_section_title, BindingRow, BindingRowContext,
    SectionGroups,
};
use adw::prelude::*;
use ira_input::{
    AxisDirection, Binding, DeviceInfo, GamepadAxis, GamepadButton, InputProfile, InputSource,
    OutputAction, VirtualGamepadBackend,
};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
pub(super) struct BindingCollectionContext {
    pub(super) section_groups: SectionGroups,
    pub(super) rows: Rc<RefCell<Vec<BindingRow>>>,
    pub(super) device: Option<DeviceInfo>,
    pub(super) backend: Rc<RefCell<VirtualGamepadBackend>>,
    pub(super) mark_dirty: Rc<dyn Fn()>,
}

pub(super) fn section_group(
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
    let display_title = super::input_profile_binding::section_title_label(title);
    group.set_title(&super::helpers::esc(&display_title));
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

pub(super) fn ensure_section_behavior(
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
    row.set_title(&crate::tr!("Behavior"));
    row.set_subtitle(&crate::tr!(
        "Apply this behavior to every binding in this section"
    ));
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
        [
            crate::tr!("Custom"),
            crate::tr!("Default"),
            crate::tr!("Stick"),
            crate::tr!("Directional Pad"),
        ]
        .into_iter()
        .collect()
    } else {
        [crate::tr!("Custom"), crate::tr!("Default")]
            .into_iter()
            .collect()
    }
}

pub(super) fn section_behavior_bindings(
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

pub(super) fn connect_add_binding(
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
            super::input_profile_binding::binding_page_index(&binding),
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

pub(super) fn default_profile_bindings(
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

pub(super) fn stick_to_dpad_bindings(x_axis: GamepadAxis, y_axis: GamepadAxis) -> [Binding; 4] {
    [
        axis_button_binding(x_axis, AxisDirection::Negative, GamepadButton::DpadLeft),
        axis_button_binding(x_axis, AxisDirection::Positive, GamepadButton::DpadRight),
        axis_button_binding(y_axis, AxisDirection::Negative, GamepadButton::DpadUp),
        axis_button_binding(y_axis, AxisDirection::Positive, GamepadButton::DpadDown),
    ]
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
