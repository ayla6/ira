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
use super::input_profile_assets::{
    output_asset_name, set_asset_from_name, set_output_asset, set_source_asset, source_badge,
};
use super::input_profile_editor_regions::source_label;
use super::input_profile_options::output_display_label;
use super::input_profile_region_pages::{rebind_hook, PagesCtx};
use super::input_profile_sheet_base::{find_mapping, with_mapping, Reopen, SheetBase};
use super::input_profile_source_modes::{same_mode, ModeTarget};
use super::input_profile_widgets::{section_title_row, OptionChoice};
use ira_input::{GamepadAxis, InputMapping, InputSource, SourceMode};

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

/// The behavior picker as an expander child; changing it updates the
/// header's summary through the rebuild.
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
            input.mode = mode;
        });
        (ctx_for_change.on_dirty)();
        (reopen_for_change)();
    });
    picker
}

/// The digital input's command slot as an expander child: shows the current
/// output, and activating it opens the command picker. Glyphs stand in for
/// the output names when the Steam glyph set covers them all.
fn command_child(
    ctx: &PagesCtx,
    base: &SheetBase,
    mapping: Option<&InputMapping>,
) -> adw::ActionRow {
    let child = adw::ActionRow::new();
    child.set_title(&crate::tr!("Command"));
    let outputs: Vec<ira_input::OutputAction> = mapping
        .and_then(|mapping| mapping.activators.first())
        .map(|activator| activator.outputs.clone())
        .unwrap_or_default();
    let family = base
        .device
        .as_ref()
        .map(ira_input::DeviceInfo::family)
        .unwrap_or_default();
    let glyphs: Vec<&str> = outputs
        .iter()
        .filter_map(|output| output_asset_name(output, family))
        .collect();
    if !outputs.is_empty() && glyphs.len() == outputs.len() {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        row.set_valign(gtk4::Align::Center);
        for name in glyphs {
            let image = gtk4::Image::new();
            image.set_pixel_size(16);
            image.set_valign(gtk4::Align::Center);
            set_asset_from_name(&image, Some(name));
            row.append(&image);
        }
        child.add_suffix(&row);
    } else {
        let value = if outputs.is_empty() {
            crate::tr!("Not mapped").to_string()
        } else {
            outputs
                .iter()
                .map(output_display_label)
                .collect::<Vec<String>>()
                .join(", ")
        };
        let value_label = gtk4::Label::new(Some(&value));
        value_label.add_css_class(CSS_DIM_LABEL);
        value_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        value_label.set_valign(gtk4::Align::Center);
        child.add_suffix(&value_label);
    }
    child.set_activatable(true);
    let on_rebind = rebind_hook(ctx, base.source);
    child.connect_activated(move |_| on_rebind());
    child
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
    expander.set_title(&source_label(source));
    // The bound command's glyph replaces the text name when the Steam glyph
    // set covers it; the name is only the fallback.
    let mut shows_glyph = false;
    if matches!(source, InputSource::Button(_)) {
        if let Some(output) = mapping
            .and_then(|mapping| mapping.activators.first())
            .and_then(|activator| activator.outputs.first())
        {
            let glyph = gtk4::Image::new();
            glyph.set_pixel_size(18);
            glyph.set_valign(gtk4::Align::Center);
            shows_glyph = set_output_asset(&glyph, output, family);
            if shows_glyph {
                expander.add_suffix(&glyph);
            }
        }
    }
    if !shows_glyph {
        expander.set_subtitle(&summary_text(source, mapping));
    }

    let base = SheetBase {
        gyro: ctx.gyro.clone(),
        child_expander: RefCell::new(Some(expander.clone())),
        live_children: RefCell::new(Vec::new()),
        profile: ctx.profile.clone(),
        active_target: ctx.active_target.get(),
        source,
        device: ctx.device.clone(),
        backend: ctx.profile.borrow().backend,
        on_changed: {
            let ctx = ctx.clone();
            Rc::new(move || (ctx.on_dirty)())
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
        InputSource::Axis(_) | InputSource::AxisDirection { .. } => mapping
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
/// tests pin them together.
pub(crate) fn behavior_choices(source: InputSource) -> Vec<OptionChoice> {
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
