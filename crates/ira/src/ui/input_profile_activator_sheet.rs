//! The per-input editor sheet: every activator of one input, Steam-style —
//! press patterns, outputs picked from the tabbed command picker, gating
//! (hold/toggle/chord/analog), toggling, repeat rates, stick behaviors, and
//! mode shifts. Mutations apply straight to the profile; `on_changed` fires
//! after each one so the region pages can refresh their summaries.

use super::css::{CSS_FLAT, CSS_SQUARE_BUTTON};
use super::helpers::esc;
use super::input_profile_editor_regions::{source_label, supported_button_sources};
use adw::prelude::*;
use ira_input::{GamepadAxis, InputMapping, InputSource, SourceMode, StickOutput, VirtualGamepadBackend};
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) type ProfileRc = Rc<RefCell<ira_input::InputProfile>>;
pub(crate) type OnChanged = Rc<dyn Fn()>;
/// Structural edits rebuild the sheet contents through this hook.
pub(crate) type Reopen = Rc<dyn Fn()>;

/// Everything one input's sheet needs; built by the region pages.
pub(crate) struct InputSheetRequest {
    pub profile: ProfileRc,
    pub active_set: usize,
    pub source: InputSource,
    pub device: Option<ira_input::DeviceInfo>,
    pub backend: VirtualGamepadBackend,
    pub on_changed: OnChanged,
}

/// Cloneable sheet state shared by every edit closure. `fill_sheet` derives
/// its own reopen hook from this so any depth of rebuilds keeps working.
#[derive(Clone)]
pub(crate) struct SheetBase {
    pub(crate) content: gtk4::Box,
    pub(crate) profile: ProfileRc,
    pub(crate) active_set: usize,
    pub(crate) source: InputSource,
    pub(crate) device: Option<ira_input::DeviceInfo>,
    pub(crate) backend: VirtualGamepadBackend,
    pub(crate) on_changed: OnChanged,
}

pub(crate) fn find_mapping(base: &SheetBase) -> Option<InputMapping> {
    base.profile
        .borrow()
        .action_sets
        .get(base.active_set)?
        .inputs
        .iter()
        .find(|input| input.source == base.source)
        .cloned()
}

pub(crate) fn with_mapping(base: &SheetBase, apply: impl FnOnce(&mut InputMapping)) {
    let mut borrow = base.profile.borrow_mut();
    if let Some(set) = borrow.action_sets.get_mut(base.active_set) {
        if let Some(input) = set.inputs.iter_mut().find(|input| input.source == base.source) {
            apply(input);
        }
    }
}

pub(crate) fn show_input_sheet(parent: &adw::Window, request: InputSheetRequest) {
    let window = adw::Window::new();
    window.set_modal(true);
    window.set_transient_for(Some(parent));
    window.set_destroy_with_parent(true);
    window.set_hide_on_close(false);
    window.set_default_size(560, 700);
    window.set_title(Some(&sheet_title(&request)));

    let header_bar = adw::HeaderBar::new();
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&content));
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.append(&header_bar);
    root.append(&scroll);
    window.set_content(Some(&root));

    let base = SheetBase {
        content,
        profile: request.profile,
        active_set: request.active_set,
        source: request.source,
        device: request.device,
        backend: request.backend,
        on_changed: request.on_changed,
    };
    fill_sheet(&base);
    window.present();
}

fn sheet_title(request: &InputSheetRequest) -> String {
    crate::tr!("Edit {input}")
        .replace("{input}", &source_label(request.source))
}

/// Rebuild the whole sheet. Called after every structural change (mode or
/// activator add/remove); value edits mutate widgets in place.
fn fill_sheet(base: &SheetBase) {
    let reopen: Reopen = {
        let base = base.clone();
        Rc::new(move || fill_sheet(&base))
    };
    super::helpers::clear_children(&base.content);

    if matches!(base.source, InputSource::Axis(_)) {
        base.content
            .append(&behavior_group(base, &reopen, modes_for(base.source)));
        if let Some(mapping) = find_mapping(base) {
            if let Some(mode) = mapping.mode {
                base.content
                    .append(&mode_settings_group(base, &mode));
            }
        }
        return;
    }

    if let Some(mapping) = find_mapping(base) {
        base.content.append(&super::input_profile_activator_edit::activators_group(
            base, &reopen, &mapping,
        ));
        base.content.append(&shifts_group(base, &reopen));
    }
}

// ---------------------------------------------------------------------------
// Behavior (analog inputs)
// ---------------------------------------------------------------------------

fn modes_for(source: InputSource) -> Vec<Option<SourceMode>> {
    if is_trigger_axis(source) {
        vec![None, Some(SourceMode::Trigger { threshold: 0.5 })]
    } else {
        vec![
            None,
            Some(SourceMode::Joystick {
                output: default_stick_output(source),
                deadzone_inner: 0.1,
                deadzone_outer: 0.95,
                curve: 1.0,
            }),
            Some(SourceMode::Dpad { threshold: 0.5 }),
            Some(SourceMode::Mouse { sensitivity: 1.0 }),
            Some(SourceMode::Flickstick {
                rotation_sensitivity: 1.0,
                flick_duration_ms: 100,
            }),
        ]
    }
}

fn is_trigger_axis(source: InputSource) -> bool {
    matches!(
        source,
        InputSource::Axis(GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger)
    )
}

fn mode_label(mode: &Option<SourceMode>, is_trigger: bool) -> String {
    match mode {
        None => crate::tr!("None"),
        Some(SourceMode::Joystick { .. }) => crate::tr!("Joystick"),
        Some(SourceMode::Dpad { .. }) => crate::tr!("Directional Pad"),
        Some(SourceMode::Mouse { .. }) => crate::tr!("Joystick Mouse"),
        Some(SourceMode::Flickstick { .. }) => crate::tr!("Flick Stick"),
        Some(SourceMode::Trigger { .. }) if is_trigger => crate::tr!("Trigger"),
        _ => crate::tr!("Other"),
    }
}

fn same_mode(left: &Option<SourceMode>, right: &SourceMode) -> bool {
    match left {
        Some(a) => std::mem::discriminant(a) == std::mem::discriminant(right),
        None => false,
    }
}

fn default_stick_output(source: InputSource) -> StickOutput {
    match source {
        InputSource::Axis(GamepadAxis::RightX | GamepadAxis::RightY) => StickOutput::Right,
        _ => StickOutput::Left,
    }
}

fn behavior_group(base: &SheetBase, reopen: &Reopen, modes: Vec<Option<SourceMode>>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Behavior"));
    group.set_description(Some(&crate::tr!("What this stick or trigger does")));

    let is_trigger = is_trigger_axis(base.source);
    let current = find_mapping(base).and_then(|mapping| mapping.mode);
    let selected = current
        .as_ref()
        .and_then(|mode| modes.iter().position(|candidate| same_mode(candidate, mode)))
        .unwrap_or(0);
    let labels: Vec<String> = modes.iter().map(|mode| mode_label(mode, is_trigger)).collect();

    let dropdown = combo_row(&labels, selected as u32);
    group.add(&row_with_control(&crate::tr!("Behavior"), &dropdown));

    let base_for_change = base.clone();
    let reopen_for_change = reopen.clone();
    dropdown.connect_selected_notify(move |dropdown| {
        let mode = modes
            .get(dropdown.selected() as usize)
            .cloned()
            .flatten();
        with_mapping(&base_for_change, |input| {
            input.mode = mode;
        });
        (base_for_change.on_changed)();
        reopen_for_change();
    });

    group
}

/// Returns a closure that mutates the current mode in place, if any.
fn mode_writer(base: &SheetBase) -> impl Fn(&mut dyn FnMut(&mut SourceMode)) + '_ {
    let base = base.clone();
    move |mutate: &mut dyn FnMut(&mut SourceMode)| {
        with_mapping(&base, |input| {
            if let Some(current) = input.mode.as_mut() {
                mutate(current);
            }
        });
    }
}

fn mode_settings_group(base: &SheetBase, mode: &SourceMode) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Response"));
    match mode {
        SourceMode::Joystick {
            deadzone_inner,
            deadzone_outer,
            curve,
            ..
        } => joystick_rows(base, &group, *deadzone_inner, *deadzone_outer, *curve),
        SourceMode::Mouse { sensitivity } => {
            group.add(&mode_spin_row(
                base,
                &crate::tr!("Sensitivity"),
                0.05,
                20.0,
                0.05,
                f64::from(*sensitivity),
                |mode, value| {
                    if let SourceMode::Mouse { sensitivity } = mode {
                        *sensitivity = value as f32;
                    }
                },
            ));
        }
        SourceMode::Dpad { threshold } => {
            group.add(&mode_spin_row(
                base,
                &crate::tr!("Activation threshold"),
                0.2,
                0.95,
                0.05,
                f64::from(*threshold),
                |mode, value| {
                    if let SourceMode::Dpad { threshold } = mode {
                        *threshold = value as f32;
                    }
                },
            ));
        }
        SourceMode::Trigger { threshold } => {
            group.add(&mode_spin_row(
                base,
                &crate::tr!("Full pull threshold"),
                0.1,
                1.0,
                0.05,
                f64::from(*threshold),
                |mode, value| {
                    if let SourceMode::Trigger { threshold } = mode {
                        *threshold = value as f32;
                    }
                },
            ));
        }
        SourceMode::Flickstick {
            rotation_sensitivity,
            flick_duration_ms,
        } => {
            group.add(&mode_spin_row(
                base,
                &crate::tr!("Rotation sensitivity"),
                0.1,
                10.0,
                0.1,
                f64::from(*rotation_sensitivity),
                |mode, value| {
                    if let SourceMode::Flickstick {
                        rotation_sensitivity,
                        ..
                    } = mode
                    {
                        *rotation_sensitivity = value as f32;
                    }
                },
            ));
            group.add(&mode_spin_row(
                base,
                &crate::tr!("Flick duration (ms)"),
                40.0,
                400.0,
                10.0,
                f64::from(*flick_duration_ms),
                |mode, value| {
                    if let SourceMode::Flickstick {
                        flick_duration_ms, ..
                    } = mode
                    {
                        *flick_duration_ms = value as u32;
                    }
                },
            ));
        }
    }
    group
}

fn joystick_rows(
    base: &SheetBase,
    group: &adw::PreferencesGroup,
    inner: f32,
    outer: f32,
    curve: f32,
) {
    group.add(&mode_spin_row(
        base,
        &crate::tr!("Inner dead zone"),
        0.0,
        0.9,
        0.01,
        f64::from(inner),
        |mode, value| {
            if let SourceMode::Joystick { deadzone_inner, .. } = mode {
                *deadzone_inner = value as f32;
            }
        },
    ));
    group.add(&mode_spin_row(
        base,
        &crate::tr!("Outer dead zone"),
        0.1,
        1.0,
        0.01,
        f64::from(outer),
        |mode, value| {
            if let SourceMode::Joystick {
                deadzone_outer, ..
            } = mode
            {
                *deadzone_outer = value as f32;
            }
        },
    ));
    group.add(&mode_spin_row(
        base,
        &crate::tr!("Response curve"),
        0.2,
        3.0,
        0.05,
        f64::from(curve),
        |mode, value| {
            if let SourceMode::Joystick { curve, .. } = mode {
                *curve = value as f32;
            }
        },
    ));
}

/// One spin row bound to a field of the input's current SourceMode.
fn mode_spin_row(
    base: &SheetBase,
    title: &str,
    min: f64,
    max: f64,
    step: f64,
    value: f64,
    mutate: fn(&mut SourceMode, f64),
) -> adw::SpinRow {
    let base = base.clone();
    spin_row(title, min, max, step, value, Rc::new(move |value| {
        let write = mode_writer(&base);
        write(&mut |mode| mutate(mode, value));
        (base.on_changed)();
    }))
}

// ---------------------------------------------------------------------------
// Activators (button inputs)
// ---------------------------------------------------------------------------


// ---------------------------------------------------------------------------
// Mode shifts
// ---------------------------------------------------------------------------

fn shifts_group(base: &SheetBase, reopen: &Reopen) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Mode shifts"));
    group.set_description(Some(&crate::tr!(
        "While the trigger is held, this input uses the shifted behavior"
    )));

    if let Some(mapping) = find_mapping(base) {
        for (shift_index, shift) in mapping.mode_shifts.iter().enumerate() {
            let row = adw::ActionRow::new();
            row.set_title(&esc(&format!(
                "{} {}",
                crate::tr!("While holding"),
                source_label(shift.trigger)
            )));
            let trash = gtk4::Button::from_icon_name("user-trash-symbolic");
            trash.add_css_class(CSS_FLAT);
            trash.add_css_class(CSS_SQUARE_BUTTON);
            trash.set_valign(gtk4::Align::Center);
            row.add_suffix(&trash);
            let base = base.clone();
            let reopen = reopen.clone();
            trash.connect_clicked(move |_| {
                with_mapping(&base, |input| {
                    if shift_index < input.mode_shifts.len() {
                        input.mode_shifts.remove(shift_index);
                    }
                });
                (base.on_changed)();
                reopen();
            });
            group.add(&row);
        }
    }

    let sources: Vec<(InputSource, String)> = supported_button_sources(base.device.as_ref())
        .into_iter()
        .map(|source| (source, source_label(source)))
        .collect();
    let labels: Vec<String> = sources.iter().map(|(_, label)| label.clone()).collect();
    let picker = combo_row(&labels, 0);
    let add = gtk4::Button::with_label(&crate::tr!("Add shift"));
    add.add_css_class(CSS_FLAT);
    add.set_valign(gtk4::Align::Center);
    let picker_row = adw::ActionRow::new();
    picker_row.set_title(&crate::tr!("Shift while holding"));
    picker_row.add_suffix(&picker);
    picker_row.add_suffix(&add);
    group.add(&picker_row);
    let base = base.clone();
    let reopen = reopen.clone();
    add.connect_clicked(move |_| {
        if let Some((trigger, _)) = sources.get(picker.selected() as usize) {
            let trigger = *trigger;
            with_mapping(&base, |input| {
                input.mode_shifts.push(ira_input::ModeShift {
                    trigger,
                    mode: None,
                    activators: Vec::new(),
                });
            });
            (base.on_changed)();
            reopen();
        }
    });

    group
}

// ---------------------------------------------------------------------------
// Widget helpers
// ---------------------------------------------------------------------------

pub(crate) fn combo_row(labels: &[String], selected: u32) -> gtk4::DropDown {
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let row = gtk4::DropDown::new(
        Some(gtk4::StringList::new(&refs)),
        None::<&gtk4::Expression>,
    );
    row.set_selected(selected);
    row
}

pub(crate) fn row_with_control(title: &str, control: &impl IsA<gtk4::Widget>) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&esc(title));
    control.add_css_class(super::css::CSS_BINDING_SUFFIX);
    control.set_valign(gtk4::Align::Center);
    row.add_suffix(control);
    row
}

pub(crate) type SpinChange = Rc<dyn Fn(f64)>;

pub(crate) fn spin_row(
    title: &str,
    min: f64,
    max: f64,
    step: f64,
    value: f64,
    on_change: SpinChange,
) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(min, max, step);
    row.set_title(&esc(title));
    row.set_value(value);
    row.connect_value_notify(move |row| {
        on_change(row.value());
    });
    row
}
