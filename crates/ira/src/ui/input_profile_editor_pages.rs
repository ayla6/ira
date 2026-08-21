use super::input_profile_binding::{
    add_binding_row, add_empty_page_state, binding_page_index, binding_section_title, BindingRow,
    BindingRowContext, SectionGroups,
};
use super::input_profile_editor_sections::{
    connect_add_binding, default_profile_bindings, ensure_section_behavior, section_group,
    BindingCollectionContext,
};
use adw::prelude::*;
use ira_input::{
    Binding, DeviceInfo, GamepadAxis, GamepadButton, InputSource, OutputAction,
    VirtualGamepadBackend,
};
use std::cell::RefCell;
use std::rc::Rc;

pub(super) struct PageSetup {
    pub(super) page_boxes: Vec<gtk4::Box>,
    pub(super) section_groups: SectionGroups,
    pub(super) backend_dropdown: adw::ComboRow,
}

pub(super) fn setup_pages(
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
    backend_dropdown.set_title(&crate::tr!("Virtual gamepad backend"));
    let backend_strings = [
        crate::tr!("XInput"),
        crate::tr!("DirectInput"),
        crate::tr!("Nintendo Switch Pro"),
    ];
    let backend_refs: Vec<&str> = backend_strings.iter().map(String::as_str).collect();
    backend_dropdown.set_model(Some(&gtk4::StringList::new(&backend_refs)));
    backend_dropdown.set_selected(match *backend.borrow() {
        VirtualGamepadBackend::XInput => 0,
        VirtualGamepadBackend::DirectInput => 1,
        VirtualGamepadBackend::SwitchPro => 2,
    });
    layout.content_area.append(&backend_dropdown);
    for (index, (page_id, title, icon)) in page_descriptors().into_iter().enumerate() {
        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_child(Some(&page_boxes[index]));
        let stack_page = layout.stack.add_titled(&scroll, Some(page_id), &title);
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
        let Some(default_binding) = default_binding else {
            continue;
        };
        let add_binding = gtk4::Button::from_icon_name("list-add-symbolic");
        add_binding.add_css_class(super::css::CSS_FLAT);
        add_binding.set_tooltip_text(Some(&crate::tr!("Add binding")));
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

fn page_descriptors() -> Vec<(&'static str, String, &'static str)> {
    vec![
        ("buttons", crate::tr!("Buttons"), "input-gaming-symbolic"),
        ("dpad", crate::tr!("D-pad"), "view-grid-symbolic"),
        (
            "triggers",
            crate::tr!("Triggers"),
            "media-seek-forward-symbolic",
        ),
        (
            "joysticks",
            crate::tr!("Joysticks"),
            "input-gaming-symbolic",
        ),
        ("gyro", crate::tr!("Gyro"), "view-refresh-symbolic"),
    ]
}

fn default_bindings() -> [Option<Binding>; 5] {
    [
        Some(Binding::new(
            InputSource::Button(GamepadButton::A),
            OutputAction::GamepadButton(GamepadButton::A),
        )),
        Some(Binding::new(
            InputSource::Button(GamepadButton::DpadUp),
            OutputAction::GamepadButton(GamepadButton::DpadUp),
        )),
        Some(Binding::new(
            InputSource::Axis(GamepadAxis::LeftTrigger),
            OutputAction::GamepadAxis(GamepadAxis::LeftTrigger),
        )),
        Some(Binding::new(
            InputSource::Axis(GamepadAxis::LeftX),
            OutputAction::GamepadAxis(GamepadAxis::LeftX),
        )),
        // The gyro page has no bindings: it holds the whole-controller gyro
        // config card instead.
        None,
    ]
}

pub(super) fn setup_sidebar(layout: &super::helpers::DialogLayout) {
    for (page_id, title, icon) in page_descriptors() {
        layout
            .sidebar
            .append(&super::settings_dialog::settings_sidebar_row(
                icon, &title, page_id,
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

pub(super) fn connect_backend_change(
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
        let selected = match dropdown.selected() {
            1 => VirtualGamepadBackend::DirectInput,
            2 => VirtualGamepadBackend::SwitchPro,
            _ => VirtualGamepadBackend::XInput,
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

pub(super) fn reset_to_defaults(
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

pub(super) fn section_order(title: &str) -> usize {
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
    }
}
