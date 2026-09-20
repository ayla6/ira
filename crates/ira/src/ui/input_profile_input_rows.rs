//! One binding row per physical input: a native `AdwExpanderRow` whose
//! header identifies the input and summarizes its binding, and whose
//! children carry everything the old floating sheet held — command,
//! behavior, response settings, activators and mode shifts. Expansion state
//! survives page rebuilds through [`ExpansionState`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;

use super::css::{CSS_DIM_LABEL, CSS_SOURCE_BADGE};
use super::input_profile_assets::{set_source_asset, source_badge};
use super::input_profile_options::output_display_label;
use super::input_profile_editor_regions::source_label;
use super::input_profile_region_pages::rebind_hook;
use super::input_profile_region_pages::PagesCtx;
use super::input_profile_sheet_base::{
    find_mapping, stick_axis_pair, with_mapping, Reopen, SheetBase,
};
use super::input_profile_source_modes::{same_mode, ModeTarget};
use super::input_profile_widgets::{section_title_row, OptionChoice};
use ira_input::{AxisDirection, GamepadAxis, GamepadButton, InputMapping, InputSource, SourceMode};

/// Whether each input's settings expander is open, remembered across the
/// rebuilds that follow every edit.
pub(crate) type ExpansionState = Rc<RefCell<HashMap<InputSource, bool>>>;

/// Structural edits rebuild the expanded content through the same deferred,
/// coalesced pass the floating sheet used: removing rows mid-signal
/// unwinding finalizes widgets GTK still touches.
fn refill(base: &SheetBase, ctx: &PagesCtx) {
    if base.rebuild_pending.replace(true) {
        return;
    }
    let base = base.clone();
    let ctx = ctx.clone();
    gtk4::glib::idle_add_local_once(move || {
        base.rebuild_pending.set(false);
        fill_children(&base, &ctx);
    });
}

fn fill_children(base: &SheetBase, ctx: &PagesCtx) {
    let Some(expander) = base.child_expander.borrow().as_ref().cloned() else {
        return;
    };
    for stale in base.live_children.borrow_mut().drain(..) {
        expander.remove(&stale);
    }
    let reopen: Reopen = {
        let base = base.clone();
        let ctx = ctx.clone();
        Rc::new(move || refill(&base, &ctx))
    };
    for child in child_rows(ctx, base, &reopen) {
        expander.add_row(&child);
        base.live_children.borrow_mut().push(child);
    }
}

/// All child rows of one input's expander, for the current mapping.
fn child_rows(ctx: &PagesCtx, base: &SheetBase, reopen: &Reopen) -> Vec<gtk4::Widget> {
    let mapping = find_mapping(base);
    let mut rows: Vec<gtk4::Widget> = Vec::new();

    if matches!(base.source, InputSource::Axis(_) | InputSource::AxisDirection { .. }) {
        rows.push(behavior_row(ctx, base, reopen).upcast());
        if let Some(mode) = mapping.as_ref().and_then(|mapping| mapping.mode.as_ref()) {
            if matches!(mode, SourceMode::Joystick(_) | SourceMode::Mouse { .. }) {
                for (title, section) in
                    super::input_profile_stick_settings::stick_setting_sections(
                        base, mode, reopen,
                    )
                {
                    rows.push(section_title_row(&title).upcast());
                    for child in section {
                        rows.push(child.upcast());
                    }
                }
            } else {
                for child in super::input_profile_source_modes::mode_setting_rows(
                    base,
                    ModeTarget::Base,
                    mode,
                    reopen,
                ) {
                    rows.push(child.upcast());
                }
                if matches!(mode, SourceMode::Dpad { .. }) && !is_trigger_axis(base.source) {
                    rows.extend(direction_command_rows(ctx, base));
                }
            }
            if is_trigger_axis(base.source) {
                if let Some(mapping) = find_mapping(base) {
                    rows.extend(super::input_profile_activator_edit::activator_rows(
                        base, reopen, &mapping,
                    ));
                }
            }
        }
        rows.extend(super::input_profile_mode_shifts::shift_rows(base, reopen));
        return rows;
    }

    rows.push(command_child(ctx, base, mapping.as_ref()).upcast());
    if let Some(mapping) = find_mapping(base) {
        rows.extend(super::input_profile_activator_edit::activator_rows(
            base, reopen, &mapping,
        ));
        rows.extend(super::input_profile_mode_shifts::shift_rows(base, reopen));
    }
    rows
}

fn is_trigger_axis(source: InputSource) -> bool {
    matches!(
        source,
        InputSource::Axis(
            GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger
        ) | InputSource::AxisDirection {
            axis: GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger,
            ..
        }
    )
}

/// The behavior picker as an expander child. Swapping the behavior only
/// touches this expander — the header summary updates in place and the
/// children refill — so the rest of the editor keeps its scroll and
/// expansion state instead of reloading.
fn behavior_row(ctx: &PagesCtx, base: &SheetBase, reopen: &Reopen) -> adw::ComboRow {
    let picker = adw::ComboRow::new();
    picker.set_title(&crate::tr!("Behavior"));
    let modes = super::input_profile_source_modes::modes_for(base.source);
    let choices = behavior_choices(base.source);
    let titles: Vec<&str> = choices.iter().map(|choice| choice.title.as_str()).collect();
    picker.set_model(Some(&gtk4::StringList::new(&titles)));
    let current = find_mapping(base).and_then(|mapping| mapping.mode);
    let selected = current
        .as_ref()
        .and_then(|mode| modes.iter().position(|candidate| same_mode(candidate, mode)))
        .unwrap_or(0);
    picker.set_selected(selected as u32);

    let base_for_change = base.clone();
    let ctx_for_change = ctx.clone();
    let reopen_for_change = reopen.clone();
    picker.connect_selected_notify(move |picker| {
        let mode = modes.get(picker.selected() as usize).cloned().flatten();
        with_mapping(&base_for_change, |input| {
            input.mode = mode.clone();
        });
        sync_dpad_directions(&base_for_change, mode.as_ref());
        refresh_header_summary(&base_for_change);
        (ctx_for_change.on_adjusted)();
        (reopen_for_change)();
    });
    picker
}

/// Steam seeds a stick's Dpad behavior with the matching d-pad buttons: the
/// four direction slots carry their virtual button instead of sitting
/// empty, and the engine output is identical either way. Switching away
/// removes only the untouched seeds — custom direction commands survive.
fn sync_dpad_directions(base: &SheetBase, mode: Option<&SourceMode>) {
    let mut profile = base.profile.borrow_mut();
    let Some(inputs) = base.active_target.inputs_mut(&mut profile) else {
        return;
    };
    sync_stick_dpad_directions(inputs, base.source, mode);
}

/// The seeding itself, on the target's input list: `source` is the stick's
/// X axis (the pair's canonical mapping slot).
pub(crate) fn sync_stick_dpad_directions(
    inputs: &mut Vec<InputMapping>,
    source: InputSource,
    mode: Option<&SourceMode>,
) {
    let (x_axis, y_axis) = stick_axis_pair(source);
    let directions = [
        (GamepadButton::DpadUp, y_axis, AxisDirection::Negative),
        (GamepadButton::DpadDown, y_axis, AxisDirection::Positive),
        (GamepadButton::DpadLeft, x_axis, AxisDirection::Negative),
        (GamepadButton::DpadRight, x_axis, AxisDirection::Positive),
    ];
    let seeding = matches!(mode, Some(SourceMode::Dpad { .. }));
    for (button, axis, direction) in directions {
        let source = InputSource::AxisDirection { axis, direction };
        let seed = InputMapping::simple(
            source,
            ira_input::OutputAction::GamepadButton(button),
        );
        match (seeding, inputs.iter().position(|input| input.source == source)) {
            (true, None) => inputs.push(seed),
            (true, Some(index)) => {
                if inputs[index].activators.is_empty() {
                    inputs[index] = seed;
                }
            }
            (false, Some(index)) if inputs[index] == seed => {
                inputs.remove(index);
            }
            _ => {}
        }
    }
}

/// Refresh the expander header's summary label from the current mapping.
pub(crate) fn refresh_header_summary(base: &SheetBase) {
    if let Some(label) = base.header_summary.borrow().as_ref() {
        label.set_text(&summary_text(base.source, find_mapping(base).as_ref()));
    }
}

/// The digital input's command slot as an expander child: shows the current
/// output, and activating it opens the command picker.
fn command_child(
    ctx: &PagesCtx,
    base: &SheetBase,
    mapping: Option<&InputMapping>,
) -> adw::ActionRow {
    let child = command_row(ctx, base.source);
    child.set_title(&crate::tr!("Command"));
    let value = mapping
        .and_then(|mapping| mapping.activators.first())
        .map(|activator| {
            activator
                .outputs
                .iter()
                .map(output_display_label)
                .collect::<Vec<String>>()
                .join(", ")
        })
        .unwrap_or_else(|| crate::tr!("Add command").to_string());
    let value_label = gtk4::Label::new(Some(&value));
    value_label.add_css_class(CSS_DIM_LABEL);
    value_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    value_label.set_valign(gtk4::Align::Center);
    child.add_suffix(&value_label);
    child
}

/// The command row for any bindable source — the input's own slot, or one
/// of a stick's dpad-direction slots. Activating opens the command picker
/// for that source.
fn command_row(ctx: &PagesCtx, source: InputSource) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_activatable(true);
    let on_rebind = rebind_hook(ctx, source);
    row.connect_activated(move |_| on_rebind());
    row
}

/// Steam's per-direction commands for the Dpad behavior: the stick's four
/// digital halves each get a command slot, and a bound command replaces
/// that direction's virtual dpad press in the engine.
fn direction_command_rows(ctx: &PagesCtx, base: &SheetBase) -> Vec<gtk4::Widget> {
    let (x_axis, y_axis) = stick_axis_pair(base.source);
    let directions = [
        (GamepadButton::DpadUp, y_axis, AxisDirection::Negative),
        (GamepadButton::DpadDown, y_axis, AxisDirection::Positive),
        (GamepadButton::DpadLeft, x_axis, AxisDirection::Negative),
        (GamepadButton::DpadRight, x_axis, AxisDirection::Positive),
    ];
    let mut rows: Vec<gtk4::Widget> = vec![section_title_row(&crate::tr!("Direction Commands")).upcast()];
    for (button, axis, direction) in directions {
        let source = InputSource::AxisDirection { axis, direction };
        let row = command_row(ctx, source);
        row.set_title(&source_label(InputSource::Button(button)));
        let value = base
            .active_target
            .find_mapping(&base.profile.borrow(), source)
            .and_then(|mapping| mapping.activators.first().cloned())
            .map(|activator| {
                activator
                    .outputs
                    .iter()
                    .map(output_display_label)
                    .collect::<Vec<String>>()
                    .join(", ")
            })
            .unwrap_or_else(|| crate::tr!("Add command").to_string());
        let value_label = gtk4::Label::new(Some(&value));
        value_label.add_css_class(CSS_DIM_LABEL);
        value_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        value_label.set_valign(gtk4::Align::Center);
        row.add_suffix(&value_label);
        rows.push(row.upcast());
    }
    rows
}

/// One binding row per input: the header identifies the input and carries
/// its summary; everything else lives in the expander children.
pub(crate) fn input_expander_row(
    ctx: &PagesCtx,
    source: InputSource,
    mapping: Option<&InputMapping>,
    family: ira_input::ControllerFamily,
) -> adw::ExpanderRow {
    let expander = adw::ExpanderRow::new();
    add_source_prefix(&expander, source, family, mapping.is_none());
    // The input's glyph — or its badge text when the glyph set lacks it —
    // identifies the row; it carries no text title. The bound command stays
    // text, right-aligned, "Not mapped" included.
    let value_label = gtk4::Label::new(Some(&summary_text(source, mapping)));
    value_label.add_css_class(CSS_DIM_LABEL);
    value_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    value_label.set_valign(gtk4::Align::Center);
    expander.add_suffix(&value_label);

    let base = SheetBase {
        gyro: ctx.gyro.clone(),
        child_expander: RefCell::new(Some(expander.clone())),
        live_children: RefCell::new(Vec::new()),
        header_summary: RefCell::new(Some(value_label.clone())),
        profile: ctx.profile.clone(),
        active_target: ctx.active_target.get(),
        source,
        device: ctx.device.clone(),
        backend: ctx.profile.borrow().backend,
        on_adjusted: {
            let ctx = ctx.clone();
            Rc::new(move || (ctx.on_adjusted)())
        },
        rebuild_pending: Rc::new(std::cell::Cell::new(false)),
    };
    let reopen: Reopen = {
        let base = base.clone();
        let ctx = ctx.clone();
        Rc::new(move || refill(&base, &ctx))
    };
    for child in child_rows(ctx, &base, &reopen) {
        expander.add_row(&child);
        base.live_children.borrow_mut().push(child);
    }

    let state = ctx.expansion.clone();
    if state.borrow().get(&source).copied().unwrap_or(false) {
        expander.set_enable_expansion(true);
    }
    expander.connect_enable_expansion_notify(move |expander| {
        state
            .borrow_mut()
            .insert(source, expander.enables_expansion());
    });
    expander
}

fn summary_text(source: InputSource, mapping: Option<&InputMapping>) -> String {
    match source {
        // Whole axes summarize their behavior mode; direction halves are
        // digital command slots and summarize like buttons do.
        InputSource::Axis(_) => mapping
            .and_then(|mapping| mapping.mode.as_ref())
            .map(|mode| {
                super::input_profile_source_modes::mode_label(
                    &Some(mode.clone()),
                    is_trigger_axis(source),
                )
            })
            .unwrap_or_else(|| crate::tr!("Unbound").to_string()),
        _ => mapping
            .and_then(|mapping| mapping.activators.first())
            .map(|activator| {
                activator
                    .outputs
                    .iter()
                    .map(output_display_label)
                    .collect::<Vec<String>>()
                    .join(", ")
            })
            .unwrap_or_else(|| crate::tr!("Not mapped").to_string()),
    }
}

fn add_source_prefix(
    expander: &adw::ExpanderRow,
    source: InputSource,
    family: ira_input::ControllerFamily,
    dim: bool,
) {
    let badge = gtk4::Label::new(Some(&source_badge(source, family)));
    badge.add_css_class(CSS_SOURCE_BADGE);
    if dim {
        badge.add_css_class(CSS_DIM_LABEL);
    }
    badge.set_valign(gtk4::Align::Center);
    let asset = gtk4::Image::new();
    asset.set_pixel_size(24);
    set_source_asset(&asset, &badge, source, family);
    expander.add_prefix(&asset);
    expander.add_prefix(&badge);
}

/// Steam's behavior list for an analog source: the modes it can take, each
/// with its one-line description. Titles must match `mode_label` — the
/// tests pin them together. The d-pad group differs on purpose: its bare
/// "no mode" state is the standard directional pad (identity bindings),
/// while Steam's inert None rides the stick Dpad mode, which the engine
/// runs as a no-op on button sources.
pub(crate) fn behavior_choices(source: InputSource) -> Vec<OptionChoice> {
    if matches!(source, InputSource::Button(button) if button.is_dpad()) {
        return vec![
            OptionChoice {
                title: crate::tr!("None"),
                description: Some(crate::tr!("The d-pad sends nothing")),
            },
            OptionChoice {
                title: crate::tr!("Directional Pad"),
                description: Some(crate::tr!(
                    "The standard d-pad; each direction keeps its own command"
                )),
            },
            OptionChoice {
                title: crate::tr!("Button Pad"),
                description: Some(crate::tr!(
                    "Each direction is its own independent button, with no d-pad semantics"
                )),
            },
            OptionChoice {
                title: crate::tr!("Joystick"),
                description: Some(crate::tr!(
                    "The four directions deflect a virtual joystick — for games and menus that only read the stick"
                )),
            },
        ];
    }
    if is_trigger_axis(source) {
        return vec![
            OptionChoice {
                title: crate::tr!("None"),
                description: Some(crate::tr!("The trigger only runs its click bindings")),
            },
            OptionChoice {
                title: crate::tr!("Trigger"),
                description: Some(crate::tr!(
                    "The trigger sends an analog value; soft and full pulls get their own bindings"
                )),
            },
        ];
    }
    vec![
        OptionChoice {
            title: crate::tr!("None"),
            description: Some(crate::tr!("The stick sends nothing")),
        },
        OptionChoice {
            title: crate::tr!("Joystick"),
            description: Some(crate::tr!(
                "Deflection drives a virtual joystick — the standard analog movement"
            )),
        },
        OptionChoice {
            title: crate::tr!("Directional Pad"),
            description: Some(crate::tr!("Deflection presses the d-pad directions")),
        },
        OptionChoice {
            title: crate::tr!("Joystick Mouse"),
            description: Some(crate::tr!("Deflection moves the mouse pointer")),
        },
        OptionChoice {
            title: crate::tr!("Flick Stick"),
            description: Some(crate::tr!(
                "Direction sets the facing; a flick turns instantly — pairs well with gyro"
            )),
        },
    ]
}

/// Identity mapping for a freshly added input: buttons passthrough to their
/// virtual counterpart; sticks and triggers get their natural analog mode.
/// Test-only: product code leaves unmapped inputs unmapped until edited.
#[cfg(test)]
pub(crate) fn default_mapping(source: InputSource) -> InputMapping {
    match source {
        InputSource::Axis(axis @ (GamepadAxis::LeftX | GamepadAxis::RightX)) => {
            let output = if axis == GamepadAxis::LeftX {
                ira_input::StickOutput::Left
            } else {
                ira_input::StickOutput::Right
            };
            InputMapping {
                mode: Some(SourceMode::joystick(output)),
                ..InputMapping::new(source)
            }
        }
        InputSource::Axis(GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger) => InputMapping {
            mode: Some(SourceMode::Trigger { threshold: 0.5 }),
            ..InputMapping::new(source)
        },
        InputSource::Axis(_) => InputMapping::new(source),
        InputSource::AxisDirection { .. } => InputMapping::new(source),
        InputSource::Button(button) => InputMapping::simple(
            source,
            ira_input::OutputAction::GamepadButton(button),
        ),
    }
}

#[cfg(test)]
pub(crate) fn is_stick_source(source: InputSource) -> bool {
    matches!(
        source,
        InputSource::Axis(GamepadAxis::LeftX | GamepadAxis::RightX)
    )
}

#[cfg(test)]
mod tests {
    use super::{sync_stick_dpad_directions, InputMapping, InputSource, SourceMode};
    use ira_input::{AxisDirection, GamepadAxis, GamepadButton, OutputAction};

    fn direction_of(
        inputs: &[InputMapping],
        axis: GamepadAxis,
        direction: AxisDirection,
    ) -> Option<&InputMapping> {
        inputs
            .iter()
            .find(|input| input.source == InputSource::AxisDirection { axis, direction })
    }

    #[test]
    fn test_switching_to_dpad_seeds_all_four_direction_buttons() {
        let mut inputs = vec![InputMapping::new(InputSource::Axis(GamepadAxis::LeftX))];
        sync_stick_dpad_directions(
            &mut inputs,
            InputSource::Axis(GamepadAxis::LeftX),
            Some(&SourceMode::Dpad { threshold: 0.5 }),
        );
        let expected = [
            (GamepadAxis::LeftY, AxisDirection::Negative, GamepadButton::DpadUp),
            (GamepadAxis::LeftY, AxisDirection::Positive, GamepadButton::DpadDown),
            (GamepadAxis::LeftX, AxisDirection::Negative, GamepadButton::DpadLeft),
            (GamepadAxis::LeftX, AxisDirection::Positive, GamepadButton::DpadRight),
        ];
        for (axis, direction, button) in expected {
            let mapping = direction_of(&inputs, axis, direction)
                .unwrap_or_else(|| panic!("{button:?} direction was not seeded"));
            assert_eq!(
                mapping.activators.first().and_then(|a| a.outputs.first()),
                Some(&OutputAction::GamepadButton(button))
            );
        }
    }

    #[test]
    fn test_switching_away_removes_untouched_seeds_only() {
        let mut inputs = vec![InputMapping::new(InputSource::Axis(GamepadAxis::LeftX))];
        sync_stick_dpad_directions(
            &mut inputs,
            InputSource::Axis(GamepadAxis::LeftX),
            Some(&SourceMode::Dpad { threshold: 0.5 }),
        );
        // Rebind one direction to a keyboard key: a custom command.
        let up = InputSource::AxisDirection {
            axis: GamepadAxis::LeftY,
            direction: AxisDirection::Negative,
        };
        let index = inputs.iter().position(|input| input.source == up).unwrap();
        inputs[index] = InputMapping::simple(up, OutputAction::Keyboard { keycode: 17 });

        sync_stick_dpad_directions(
            &mut inputs,
            InputSource::Axis(GamepadAxis::LeftX),
            Some(&SourceMode::joystick(ira_input::StickOutput::Left)),
        );
        // The custom up survives, the untouched seeds are gone.
        assert!(direction_of(&inputs, GamepadAxis::LeftY, AxisDirection::Negative).is_some());
        assert!(direction_of(&inputs, GamepadAxis::LeftY, AxisDirection::Positive).is_none());
        assert!(direction_of(&inputs, GamepadAxis::LeftX, AxisDirection::Negative).is_none());
        assert!(direction_of(&inputs, GamepadAxis::LeftX, AxisDirection::Positive).is_none());
    }

    #[test]
    fn test_seeding_fills_empty_direction_slots_not_custom_ones() {
        // An existing but empty direction mapping (cleared via the picker)
        // gets the default button back; a bound one is left alone.
        let down = InputSource::AxisDirection {
            axis: GamepadAxis::LeftY,
            direction: AxisDirection::Positive,
        };
        let mut inputs = vec![
            InputMapping::new(InputSource::Axis(GamepadAxis::RightX)),
            InputMapping::new(down),
        ];
        sync_stick_dpad_directions(
            &mut inputs,
            InputSource::Axis(GamepadAxis::RightX),
            Some(&SourceMode::Dpad { threshold: 0.5 }),
        );
        let mapping = direction_of(&inputs, GamepadAxis::RightY, AxisDirection::Positive)
            .expect("empty slot must be filled");
        assert_eq!(
            mapping.activators.first().and_then(|a| a.outputs.first()),
            Some(&OutputAction::GamepadButton(GamepadButton::DpadDown))
        );
    }
}
