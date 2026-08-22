//! The per-input editor sheet: every activator of one input, Steam-style —
//! press patterns, outputs (with keyboard/mouse capture), gating, toggling,
//! repeat rates, stick behaviors, and mode shifts. Mutations apply straight
//! to the profile; `on_changed` fires after each one so the region pages can
//! refresh their summaries.

use super::css::{CSS_FLAT, CSS_SQUARE_BUTTON};
use super::helpers::esc;
use super::input_profile_editor_regions::{
    activator_kind_label, source_label, supported_button_sources,
};
use super::input_profile_options::{
    activation_index, activation_labels, output_display_label, output_index, output_labels,
    output_option, OutputOption,
};
use super::input_profile_output_capture::{
    show_keyboard_output_capture, show_mouse_output_capture,
};
use adw::prelude::*;
use gtk4::prelude::*;
use ira_input::{
    Activator, ActivatorKind, Activation, GamepadButton, InputMapping, InputSource, OutputAction,
    SourceMode, StickOutput, VirtualGamepadBackend,
};
use std::cell::RefCell;
use std::rc::Rc;

type ProfileRc = Rc<RefCell<ira_input::InputProfile>>;
type OnChanged = Rc<dyn Fn()>;
/// Structural edits rebuild the sheet contents through this hook.
type Reopen = Rc<dyn Fn()>;

const DEFAULT_DOUBLE_MS: f64 = 320.0;
const DEFAULT_LONG_MS: f64 = 600.0;

fn find_mapping(
    profile: &ProfileRc,
    active_set: usize,
    source: InputSource,
) -> Option<InputMapping> {
    let borrow = profile.borrow();
    borrow
        .action_sets
        .get(active_set)?
        .inputs
        .iter()
        .find(|input| input.source == source)
        .cloned()
}

fn with_mapping(
    profile: &ProfileRc,
    active_set: usize,
    source: InputSource,
    apply: impl FnOnce(&mut InputMapping),
) {
    let mut borrow = profile.borrow_mut();
    if let Some(set) = borrow.action_sets.get_mut(active_set) {
        if let Some(input) = set.inputs.iter_mut().find(|input| input.source == source) {
            apply(input);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn show_input_sheet(
    parent: &adw::Window,
    profile: &ProfileRc,
    active_set: usize,
    source: InputSource,
    device: Option<&ira_input::DeviceInfo>,
    backend: VirtualGamepadBackend,
    family: ira_input::ControllerFamily,
    on_changed: OnChanged,
) {
    let window = adw::Window::new();
    window.set_modal(true);
    window.set_transient_for(Some(parent));
    window.set_destroy_with_parent(true);
    window.set_hide_on_close(false);
    window.set_default_size(540, 680);
    window.set_title(Some(&source_label(source)));

    let header_bar = adw::HeaderBar::new();
    header_bar.set_show_title(false);
    let close = gtk4::Button::from_icon_name("window-close-symbolic");
    close.add_css_class(CSS_FLAT);
    header_bar.pack_end(&close);

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

    close.connect_clicked(move |button| {
        if let Some(host) = button
            .ancestor(gtk4::Window::static_type())
            .and_downcast::<adw::Window>()
        {
            host.close();
        }
    });

    let reopen: Reopen = {
        let window = window.clone();
        let content = content.clone();
        let profile = profile.clone();
        let device = device.cloned();
        let on_changed = on_changed.clone();
        Rc::new(move || {
            fill_sheet(
                &window,
                &content,
                &profile,
                active_set,
                source,
                device.as_ref(),
                backend,
                family,
                on_changed.clone(),
                Rc::new(|| {}),
            );
        })
    };

    fill_sheet(
        &window,
        &content,
        &profile,
        active_set,
        source,
        device,
        backend,
        family,
        on_changed.clone(),
        reopen,
    );

    window.present();
}

/// Rebuild the whole sheet. Called after every structural change (mode or
/// activator add/remove); value edits mutate widgets in place.
#[allow(clippy::too_many_arguments)]
fn fill_sheet(
    window: &adw::Window,
    content: &gtk4::Box,
    profile: &ProfileRc,
    active_set: usize,
    source: InputSource,
    device: Option<&ira_input::DeviceInfo>,
    backend: VirtualGamepadBackend,
    family: ira_input::ControllerFamily,
    on_changed: OnChanged,
    reopen: Reopen,
) {
    let _ = family;
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }

    if matches!(source, InputSource::Axis(_)) {
        content.append(&behavior_group(
            profile,
            active_set,
            source,
            modes_for(source),
            on_changed,
            reopen,
        ));
        if let Some(mapping) = find_mapping(profile, active_set, source) {
            if let Some(mode) = mapping.mode {
                content.append(&mode_settings_group(profile, active_set, source, &mode));
            }
        }
        return;
    }

    if let Some(mapping) = find_mapping(profile, active_set, source) {
        content.append(&activators_group(
            window,
            profile,
            active_set,
            source,
            &mapping,
            device,
            backend,
            on_changed.clone(),
            reopen.clone(),
        ));
        content.append(&shifts_group(
            profile,
            active_set,
            source,
            on_changed.clone(),
            reopen.clone(),
        ));
    }
}

// ---------------------------------------------------------------------------
// Behavior (analog inputs)
// ---------------------------------------------------------------------------

fn modes_for(source: InputSource) -> Vec<Option<SourceMode>> {
    let is_trigger = matches!(
        source,
        InputSource::Axis(ira_input::GamepadAxis::LeftTrigger)
            | InputSource::Axis(ira_input::GamepadAxis::RightTrigger)
    );
    if is_trigger {
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
        ]
    }
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
    match (left, right) {
        (Some(a), b) => std::mem::discriminant(a) == std::mem::discriminant(b),
        (None, _) => false,
    }
}

fn default_stick_output(source: InputSource) -> StickOutput {
    match source {
        InputSource::Axis(ira_input::GamepadAxis::RightX)
        | InputSource::Axis(ira_input::GamepadAxis::RightY) => StickOutput::Right,
        _ => StickOutput::Left,
    }
}

fn behavior_group(
    profile: &ProfileRc,
    active_set: usize,
    source: InputSource,
    modes: Vec<Option<SourceMode>>,
    on_changed: OnChanged,
    reopen: Reopen,
) -> adw::PreferencesGroup {
    let modes = modes;
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Behavior"));
    group.set_description(Some(&crate::tr!("What this stick or trigger does")));

    let is_trigger = matches!(
        source,
        InputSource::Axis(ira_input::GamepadAxis::LeftTrigger)
            | InputSource::Axis(ira_input::GamepadAxis::RightTrigger)
    );
    let current = find_mapping(profile, active_set, source).and_then(|mapping| mapping.mode);
    let selected = current
        .as_ref()
        .and_then(|mode| modes.iter().position(|candidate| same_mode(candidate, mode)))
        .unwrap_or(0);
    let labels: Vec<String> = modes
        .iter()
        .map(|mode| mode_label(mode, is_trigger))
        .collect();

    let dropdown = combo_row(&labels, selected as u32);
    let row = row_with_control(&crate::tr!("Behavior"), &dropdown);
    group.add(&row);

    let profile_for_change = profile.clone();
    dropdown.connect_selected_notify(move |dropdown| {
        let mode = modes
            .get(dropdown.selected() as usize)
            .cloned()
            .flatten();
        with_mapping(&profile_for_change, active_set, source, |input| {
            input.mode = mode;
        });
        on_changed();
        reopen();
    });

    group
}

fn write_mode(
    profile: ProfileRc,
    active_set: usize,
    source: InputSource,
) -> impl Fn(&mut dyn FnMut(&mut SourceMode)) {
    move |mutate: &mut dyn FnMut(&mut SourceMode)| {
        with_mapping(&profile, active_set, source, |input| {
            if let Some(current) = input.mode.as_mut() {
                mutate(current);
            }
        });
    }
}

#[allow(clippy::too_many_lines)]
fn mode_settings_group(
    profile: &ProfileRc,
    active_set: usize,
    source: InputSource,
    mode: &SourceMode,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Response"));
    match mode {
        SourceMode::Joystick {
            deadzone_inner,
            deadzone_outer,
            curve,
            ..
        } => {
            let mut write = write_mode(profile.clone(), active_set, source);
            let inner_value = *deadzone_inner;
            group.add(&spin_row(
                &crate::tr!("Inner dead zone"),
                0.0,
                0.9,
                0.01,
                inner_value as f64,
                Rc::new(move |value| {
                    write(&mut |mode| {
                        if let SourceMode::Joystick { deadzone_inner, .. } = mode {
                            *deadzone_inner = value as f32;
                        }
                    });
                }),
            ));
            let outer_value = *deadzone_outer;
            let mut write = write_mode(profile.clone(), active_set, source);
            group.add(&spin_row(
                &crate::tr!("Outer dead zone"),
                0.1,
                1.0,
                0.01,
                outer_value as f64,
                Rc::new(move |value| {
                    write(&mut |mode| {
                        if let SourceMode::Joystick { deadzone_outer, .. } = mode {
                            *deadzone_outer = value as f32;
                        }
                    });
                }),
            ));
            let curve_value = *curve;
            let mut write = write_mode(profile.clone(), active_set, source);
            group.add(&spin_row(
                &crate::tr!("Response curve"),
                0.2,
                3.0,
                0.05,
                curve_value as f64,
                Rc::new(move |value| {
                    write(&mut |mode| {
                        if let SourceMode::Joystick { curve, .. } = mode {
                            *curve = value as f32;
                        }
                    });
                }),
            ));
        }
        SourceMode::Mouse { sensitivity } => {
            let sensitivity_value = *sensitivity;
            let mut write = write_mode(profile.clone(), active_set, source);
            group.add(&spin_row(
                &crate::tr!("Sensitivity"),
                0.05,
                20.0,
                0.05,
                sensitivity_value as f64,
                Rc::new(move |value| {
                    write(&mut |mode| {
                        if let SourceMode::Mouse { sensitivity } = mode {
                            *sensitivity = value as f32;
                        }
                    });
                }),
            ));
        }
        SourceMode::Dpad { threshold } => {
            let threshold_value = *threshold;
            let mut write = write_mode(profile.clone(), active_set, source);
            group.add(&spin_row(
                &crate::tr!("Activation threshold"),
                0.2,
                0.95,
                0.05,
                threshold_value as f64,
                Rc::new(move |value| {
                    write(&mut |mode| {
                        if let SourceMode::Dpad { threshold } = mode {
                            *threshold = value as f32;
                        }
                    });
                }),
            ));
        }
        SourceMode::Trigger { threshold } => {
            let threshold_value = *threshold;
            let mut write = write_mode(profile.clone(), active_set, source);
            group.add(&spin_row(
                &crate::tr!("Full pull threshold"),
                0.1,
                1.0,
                0.05,
                threshold_value as f64,
                Rc::new(move |value| {
                    write(&mut |mode| {
                        if let SourceMode::Trigger { threshold } = mode {
                            *threshold = value as f32;
                        }
                    });
                }),
            ));
        }
        other => {
            let _ = other;
        }
    }
    group
}

// ---------------------------------------------------------------------------
// Activators (button inputs)
// ---------------------------------------------------------------------------

const ACTIVATOR_KINDS: usize = 5;

fn kind_index(kind: &ActivatorKind) -> u32 {
    match kind {
        ActivatorKind::DoublePress { .. } => 1,
        ActivatorKind::LongPress { .. } => 2,
        ActivatorKind::StartPress => 3,
        ActivatorKind::Release => 4,
        _ => 0,
    }
}

fn make_kind(index: u32, window_ms: u32, duration_ms: u32) -> ActivatorKind {
    match index {
        1 => ActivatorKind::DoublePress { window_ms },
        2 => ActivatorKind::LongPress { duration_ms },
        3 => ActivatorKind::StartPress,
        4 => ActivatorKind::Release,
        _ => ActivatorKind::FullPress,
    }
}

fn kind_labels() -> Vec<String> {
    [
        crate::tr!("Click"),
        crate::tr!("Double press"),
        crate::tr!("Long press"),
        crate::tr!("On press down"),
        crate::tr!("On release"),
    ]
    .into_iter()
    .collect()
}

#[allow(clippy::too_many_arguments)]
fn activators_group(
    window: &adw::Window,
    profile: &ProfileRc,
    active_set: usize,
    source: InputSource,
    mapping: &InputMapping,
    device: Option<&ira_input::DeviceInfo>,
    backend: VirtualGamepadBackend,
    on_changed: OnChanged,
    reopen: Reopen,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Activators"));
    group.set_description(Some(&crate::tr!(
        "Different actions by how the input is pressed"
    )));

    for (index, activator) in mapping.activators.iter().enumerate() {
        group.add(&activator_expander(
            window,
            profile,
            active_set,
            source,
            index,
            activator,
            device,
            backend,
            on_changed.clone(),
            reopen.clone(),
        ));
    }

    let add = gtk4::Button::with_label(&crate::tr!("Add activator"));
    add.add_css_class(CSS_FLAT);
    add.set_halign(gtk4::Align::Start);
    {
        let profile = profile.clone();
        let on_changed = on_changed.clone();
        let reopen = reopen.clone();
        add.connect_clicked(move |_| {
            with_mapping(&profile, active_set, source, |input| {
                input
                    .activators
                    .push(Activator::full_press(vec![OutputAction::GamepadButton(
                        GamepadButton::A,
                    )]));
            });
            on_changed();
            reopen();
        });
    }
    group.add(&add);
    group
}

#[allow(clippy::too_many_arguments)]
fn activator_expander(
    window: &adw::Window,
    profile: &ProfileRc,
    active_set: usize,
    source: InputSource,
    index: usize,
    activator: &Activator,
    device: Option<&ira_input::DeviceInfo>,
    backend: VirtualGamepadBackend,
    on_changed: OnChanged,
    reopen: Reopen,
) -> adw::ExpanderRow {
    let expander = adw::ExpanderRow::new();
    expander.set_title(&activator_kind_label(&activator.kind));
    let outputs_summary: Vec<String> = activator
        .outputs
        .iter()
        .map(output_display_label)
        .collect();
    expander.set_subtitle(&esc(&outputs_summary.join(", ")));

    let remove = gtk4::Button::from_icon_name("user-trash-symbolic");
    remove.add_css_class(CSS_FLAT);
    remove.add_css_class(CSS_SQUARE_BUTTON);
    remove.set_valign(gtk4::Align::Center);
    remove.set_tooltip_text(Some(&crate::tr!("Remove activator")));
    expander.add_suffix(&remove);
    {
        let profile = profile.clone();
        let on_changed = on_changed.clone();
        let reopen = reopen.clone();
        remove.connect_clicked(move |_| {
            with_mapping(&profile, active_set, source, |input| {
                if index < input.activators.len() {
                    input.activators.remove(index);
                }
            });
            on_changed();
            reopen();
        });
    }

    // Press pattern.
    let timing_window = match &activator.kind {
        ActivatorKind::DoublePress { window_ms } => u32::from(*window_ms),
        _ => 320,
    };
    let timing_duration = match &activator.kind {
        ActivatorKind::LongPress { duration_ms } => u32::from(*duration_ms),
        _ => 600,
    };
    let kind = combo_row(&kind_labels(), kind_index(&activator.kind));
    expander.add_row(&row_with_control(&crate::tr!("Press pattern"), &kind));
    {
        let profile = profile.clone();
        let on_changed = on_changed.clone();
        let reopen = reopen.clone();
        kind.connect_selected_notify(move |dropdown| {
            with_mapping(&profile, active_set, source, |input| {
                if let Some(activator) = input.activators.get_mut(index) {
                    activator.kind =
                        make_kind(dropdown.selected(), timing_window, timing_duration);
                }
            });
            on_changed();
            reopen();
        });
    }

    // Timing for double/long patterns.
    if let Some(window_ms) = match &activator.kind {
        ActivatorKind::DoublePress { window_ms } => Some(f64::from(*window_ms)),
        _ => None,
    } {
        expander.add_row(&spin_row(
            &crate::tr!("Double-press window"),
            100.0,
            1000.0,
            10.0,
            window_ms,
            Rc::new({
                let profile = profile.clone();
                let on_changed = on_changed.clone();
                move |value| {
                    with_mapping(&profile, active_set, source, |input| {
                        if let Some(activator) = input.activators.get_mut(index) {
                            if let ActivatorKind::DoublePress { window_ms } =
                                &mut activator.kind
                            {
                                *window_ms = value as u32;
                            }
                        }
                    });
                    on_changed();
                }
            }),
        ));
    }
    if let Some(duration_ms) = match &activator.kind {
        ActivatorKind::LongPress { duration_ms } => Some(f64::from(*duration_ms)),
        _ => None,
    } {
        expander.add_row(&spin_row(
            &crate::tr!("Hold duration"),
            200.0,
            2000.0,
            25.0,
            duration_ms,
            Rc::new({
                let profile = profile.clone();
                let on_changed = on_changed.clone();
                move |value| {
                    with_mapping(&profile, active_set, source, |input| {
                        if let Some(activator) = input.activators.get_mut(index) {
                            if let ActivatorKind::LongPress { duration_ms } =
                                &mut activator.kind
                            {
                                *duration_ms = value as u32;
                            }
                        }
                    });
                    on_changed();
                }
            }),
        ));
    }

    // Outputs.
    for (output_index_at, output) in activator.outputs.iter().enumerate() {
        let output_row = adw::ActionRow::new();
        output_row.set_title(&esc(&output_display_label(output)));
        let trash = gtk4::Button::from_icon_name("user-trash-symbolic");
        trash.add_css_class(CSS_FLAT);
        trash.add_css_class(CSS_SQUARE_BUTTON);
        trash.set_valign(gtk4::Align::Center);
        output_row.add_suffix(&trash);
        {
            let profile = profile.clone();
            let on_changed = on_changed.clone();
            let reopen = reopen.clone();
            trash.connect_clicked(move |_| {
                with_mapping(&profile, active_set, source, |input| {
                    if let Some(activator) = input.activators.get_mut(index) {
                        if output_index_at < activator.outputs.len() {
                            activator.outputs.remove(output_index_at);
                        }
                    }
                });
                on_changed();
                reopen();
            });
        }
        expander.add_row(&output_row);
    }

    let labels = output_labels(backend);
    let add_output = combo_row(&labels, 0);
    let add_output_row = row_with_control(&crate::tr!("Add output"), &add_output);
    expander.add_row(&add_output_row);
    {
        let parent_window = window.clone();
        let profile = profile.clone();
        let on_changed = on_changed.clone();
        let reopen = reopen.clone();
        add_output.connect_selected_notify(move |dropdown| {
            let selected = dropdown.selected() as usize;
            let Some(option) = output_option(selected as u32, backend) else {
                return;
            };
            match option {
                OutputOption::Action(action) => {
                    with_mapping(&profile, active_set, source, |input| {
                        if let Some(activator) = input.activators.get_mut(index) {
                            activator.outputs.push(action);
                        }
                    });
                    on_changed();
                    reopen();
                }
                capture @ (OutputOption::CaptureKeyboard | OutputOption::CaptureMouseButton) => {
                    dropdown.set_selected(output_index(
                        &find_first_output(&profile, active_set, source, index),
                        backend,
                    ));
                    if let Some(host) = parent_window
                        .transient_for()
                        .and_downcast::<gtk4::Window>()
                    {
                        let profile = profile.clone();
                        let on_changed = on_changed.clone();
                        let reopen = reopen.clone();
                        match capture {
                            OutputOption::CaptureKeyboard => show_keyboard_output_capture(
                                &host,
                                move |keycode| {
                                    with_mapping(&profile, active_set, source, |input| {
                                        if let Some(activator) = input.activators.get_mut(index) {
                                            activator
                                                .outputs
                                                .push(OutputAction::Keyboard { keycode });
                                        }
                                    });
                                    on_changed();
                                    reopen();
                                },
                            ),
                            _ => show_mouse_output_capture(&host, move |button| {
                                with_mapping(&profile, active_set, source, |input| {
                                    if let Some(activator) = input.activators.get_mut(index) {
                                        activator.outputs.push(OutputAction::MouseButton(button));
                                    }
                                });
                                on_changed();
                                reopen();
                            }),
                        }
                    }
                }
                OutputOption::Action(_) => {}
            }
        });
    }

    // Gating.
    let gate = combo_row(&activation_labels(), activation_index(&activator.activation));
    expander.add_row(&row_with_control(&crate::tr!("Requires"), &gate));
    {
        let profile = profile.clone();
        let on_changed = on_changed.clone();
        gate.connect_selected_notify(move |dropdown| {
            with_mapping(&profile, active_set, source, |input| {
                if let Some(activator) = input.activators.get_mut(index) {
                    activator.activation = match dropdown.selected() {
                        1 => Activation::Hold(InputSource::Button(default_gate_button(
                            &activator.activation,
                        ))),
                        2 => Activation::Toggle(InputSource::Button(default_gate_button(
                            &activator.activation,
                        ))),
                        3 => Activation::DisableWhile(InputSource::Button(default_gate_button(
                            &activator.activation,
                        ))),
                        _ => Activation::Always,
                    };
                }
            });
            on_changed();
            reopen();
        });
    }

    if !matches!(activator.activation, Activation::Always) {
        let gate_sources: Vec<(InputSource, String)> = supported_button_sources(device)
            .into_iter()
            .map(|source| (source, source_label(source)))
            .collect();
        let gate_labels: Vec<String> = gate_sources
            .iter()
            .map(|(_, label)| label.clone())
            .collect();
        let gate_source = combo_row(&gate_labels, 0);
        expander.add_row(&row_with_control(&crate::tr!("While holding"), &gate_source));
        {
            let profile = profile.clone();
            let on_changed = on_changed.clone();
            gate_source.connect_selected_notify(move |dropdown| {
                if let Some((gate_input, _)) = gate_sources.get(dropdown.selected() as usize) {
                    with_mapping(&profile, active_set, source, |input| {
                        if let Some(activator) = input.activators.get_mut(index) {
                            match activator.activation {
                                Activation::Hold(_) => activator.activation =
                                    Activation::Hold(*gate_input),
                                Activation::Toggle(_) => activator.activation =
                                    Activation::Toggle(*gate_input),
                                Activation::DisableWhile(_) => {
                                    activator.activation = Activation::DisableWhile(*gate_input)
                                }
                                Activation::Always | Activation::Chord { .. } => {}
                            }
                        }
                    });
                    on_changed();
                }
            });
        }
    }

    // Toggle instead of hold.
    let toggle = gtk4::Switch::new();
    toggle.set_active(activator.settings.toggle);
    toggle.set_valign(gtk4::Align::Center);
    let toggle_row = adw::ActionRow::new();
    toggle_row.set_title(&crate::tr!("Toggle instead of hold"));
    toggle_row.add_suffix(&toggle);
    expander.add_row(&toggle_row);
    {
        let profile = profile.clone();
        let on_changed = on_changed.clone();
        toggle.connect_active_notify(move |switch| {
            with_mapping(&profile, active_set, source, |input| {
                if let Some(activator) = input.activators.get_mut(index) {
                    activator.settings.toggle = switch.is_active();
                }
            });
            on_changed();
        });
    }

    // Repeat rate (full presses only).
    if matches!(activator.kind, ActivatorKind::FullPress) {
        let repeat_ms = activator.settings.repeat_rate_ms.map(f64::from).unwrap_or(0.0);
        expander.add_row(&spin_row(
            &crate::tr!("Repeat every"),
            0.0,
            1000.0,
            50.0,
            repeat_ms,
            Rc::new({
                let profile = profile.clone();
                let on_changed = on_changed.clone();
                move |value| {
                    with_mapping(&profile, active_set, source, |input| {
                        if let Some(activator) = input.activators.get_mut(index) {
                            activator.settings.repeat_rate_ms = if value >= 50.0 {
                                Some(value as u32)
                            } else {
                                None
                            };
                        }
                    });
                    on_changed();
                }
            }),
        ));
    }

    expander
}

fn default_gate_button(activation: &Activation) -> GamepadButton {
    if let Activation::Hold(InputSource::Button(button))
    | Activation::Toggle(InputSource::Button(button))
    | Activation::DisableWhile(InputSource::Button(button)) = activation
    {
        return *button;
    }
    GamepadButton::LeftTrigger
}

fn find_first_output(
    profile: &ProfileRc,
    active_set: usize,
    source: InputSource,
    index: usize,
) -> OutputAction {
    find_mapping(profile, active_set, source)
        .and_then(|mapping| mapping.activators.get(index)?.outputs.first().cloned())
        .unwrap_or(OutputAction::GamepadButton(GamepadButton::A))
}

// ---------------------------------------------------------------------------
// Mode shifts
// ---------------------------------------------------------------------------

fn shifts_group(
    profile: &ProfileRc,
    active_set: usize,
    source: InputSource,
    on_changed: OnChanged,
    reopen: Reopen,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Mode shifts"));
    group.set_description(Some(&crate::tr!(
        "While the trigger is held, this input uses the shifted behavior"
    )));

    if let Some(mapping) = find_mapping(profile, active_set, source) {
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
            {
                let profile = profile.clone();
                let on_changed = on_changed.clone();
                let reopen = reopen.clone();
                trash.connect_clicked(move |_| {
                    with_mapping(&profile, active_set, source, |input| {
                        if shift_index < input.mode_shifts.len() {
                            input.mode_shifts.remove(shift_index);
                        }
                    });
                    on_changed();
                    reopen();
                });
            }
            group.add(&row);
        }
    }

    let sources: Vec<(InputSource, String)> = supported_button_sources(None)
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
    {
        let profile = profile.clone();
        let on_changed = on_changed.clone();
        let reopen = reopen.clone();
        add.connect_clicked(move |_| {
            if let Some((trigger, _)) = sources.get(picker.selected() as usize) {
                let trigger = *trigger;
                with_mapping(&profile, active_set, source, |input| {
                    input.mode_shifts.push(ira_input::ModeShift {
                        trigger,
                        mode: None,
                        activators: Vec::new(),
                    });
                });
                on_changed();
                reopen();
            }
        });
    }

    group
}

// ---------------------------------------------------------------------------
// Widget helpers
// ---------------------------------------------------------------------------

fn combo_row(labels: &[String], selected: u32) -> gtk4::DropDown {
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let row = gtk4::DropDown::new(
        Some(gtk4::StringList::new(&refs)),
        None::<&gtk4::Expression>,
    );
    row.set_selected(selected);
    row
}

fn row_with_control(title: &str, control: &impl IsA<gtk4::Widget>) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&esc(title));
    control.add_css_class(super::css::CSS_BINDING_SUFFIX);
    control.set_valign(gtk4::Align::Center);
    row.add_suffix(control);
    row
}

type SpinChange = Rc<dyn Fn(f64)>;

fn spin_row(
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
