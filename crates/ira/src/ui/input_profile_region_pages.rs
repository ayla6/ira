//! Region pages for the profile editor: Steam Input's binding list, one
//! page per controller region. Every input is a row with its current
//! command shown as a button — clicking it rebinds in one picker; the gear
//! opens the full per-input sheet. Sticks and triggers expose their analog
//! behavior through the same button pattern instead of a section dropdown.

use super::css::{CSS_DIM_LABEL, CSS_FLAT, CSS_SOURCE_BADGE, CSS_SQUARE_BUTTON};
use super::helpers::esc;
use super::input_output_picker::{show_output_picker, OutputPickerScope};
use super::input_profile_activator_sheet::{build_inline_input_sheet, InputSheetRequest};
use super::input_profile_assets::{set_source_asset, source_badge};
use super::input_profile_editor_regions::{region_groups, source_label, Region};
use super::input_profile_options::output_display_label;
use super::input_profile_sheet_base::is_trigger_axis;
use super::input_profile_source_modes::{modes_for, same_mode};
use super::input_profile_widgets::OptionChoice;
use adw::prelude::*;
use ira_input::{
    ActivatorKind, GamepadAxis, GamepadButton, InputMapping, InputProfile, InputSource,
    OutputAction, SourceMode, StickOutput,
};
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) type ProfileRc = Rc<RefCell<InputProfile>>;

pub(crate) use super::input_profile_sheet_base::EditingTarget;

/// Everything the region pages and the sheet hooks need.
#[derive(Clone)]
pub(crate) struct PagesCtx {
    pub window: adw::Dialog,
    pub profile: ProfileRc,
    /// The action set — or layer — whose bindings the region pages show and
    /// edit; mirrored by the set/layer indicator above the sidebar.
    pub active_target: Rc<std::cell::Cell<EditingTarget>>,
    /// The editor's top-left set/layer display, refreshed by every page
    /// that moves the editing target.
    pub indicator: Rc<super::input_profile_set_indicator::SetIndicator>,
    pub device: Option<ira_input::DeviceInfo>,
    pub on_dirty: Rc<dyn Fn()>,
    /// The editor's live gyro config: per-input sheets that touch shared
    /// calibration (the flick stick's dots per 360°) must write the same
    /// store the Gyro page edits, or the next save stomps their value.
    pub gyro: Rc<std::cell::RefCell<ira_input::GyroConfig>>,
}

/// Handle to the region page contents; the Gyro and Action Sets pages are
/// static and stay inside the editor shell.
#[derive(Clone)]
pub(crate) struct RegionPages {
    pub region_boxes: Vec<gtk4::Box>,
}

/// Rebuild every region page from the profile's active action set.
pub(crate) fn rebuild_region_pages(ctx: &PagesCtx, pages: &RegionPages) {
    let family = ctx
        .device
        .as_ref()
        .map(ira_input::DeviceInfo::family)
        .unwrap_or_default();
    let mappings = active_mappings(&ctx.profile, ctx.active_target.get());
    for (index, region) in Region::ALL.into_iter().enumerate() {
        let page = &pages.region_boxes[index];
        super::helpers::clear_children(page);
        for group in region_groups(region, ctx.device.as_ref()) {
            let widget = adw::PreferencesGroup::new();
            widget.set_title(&group.title);
            for source in &group.sources {
                let mapping = mappings.iter().find(|mapping| mapping.source == *source);
                let (row, sheet) = region_source_row(ctx, *source, mapping, family);
                widget.add(&row);
                // The input's settings expand right beneath its row.
                if let Some(sheet) = sheet {
                    widget.add(&sheet);
                }
            }
            page.append(&widget);
        }
    }
}

fn active_mappings(profile: &ProfileRc, target: EditingTarget) -> Vec<InputMapping> {
    let borrow = profile.borrow();
    match target {
        EditingTarget::Set(index) => borrow
            .action_sets
            .get(index)
            .map(|set| set.inputs.clone())
            .unwrap_or_default(),
        EditingTarget::Layer(index) => borrow
            .action_layers
            .get(index)
            .map(|layer| layer.inputs.clone())
            .unwrap_or_default(),
    }
}

fn region_source_row(
    ctx: &PagesCtx,
    source: InputSource,
    mapping: Option<&InputMapping>,
    family: ira_input::ControllerFamily,
) -> (gtk4::Widget, Option<gtk4::Revealer>) {
    if is_stick_source(source) || is_trigger_axis(source) {
        let (row, sheet) = analog_row(ctx, source, mapping, family);
        (row.upcast(), sheet)
    } else {
        command_row(ctx, source, mapping, family)
    }
}

/// The X axis stands for the whole stick; its Y rides along in the mode.
fn is_stick_source(source: InputSource) -> bool {
    matches!(
        source,
        InputSource::Axis(GamepadAxis::LeftX | GamepadAxis::RightX)
    )
}

/// A digital input's row: the whole row is the click target — it reopens
/// the command picker — with the current command shown as the row's value,
/// Steam's command slot in libadwaita clothing. The gear opens the full
/// sheet.
fn command_row(
    ctx: &PagesCtx,
    source: InputSource,
    mapping: Option<&InputMapping>,
    family: ira_input::ControllerFamily,
) -> (gtk4::Widget, Option<gtk4::Revealer>) {
    let row = adw::ActionRow::new();
    let unmapped = mapping.is_none();
    add_source_prefix(&row, source, family, unmapped);
    row.set_title(&esc(&source_label(source)));
    let value = match mapping.and_then(primary_output) {
        Some(output) => output_display_label(&output),
        None => crate::tr!("Not mapped"),
    };
    let value_label = gtk4::Label::new(Some(&value));
    value_label.add_css_class(CSS_DIM_LABEL);
    value_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    value_label.set_valign(gtk4::Align::Center);
    row.add_suffix(&value_label);
    let on_rebind = rebind_hook(ctx, source);
    row.set_activatable(true);
    row.connect_activated(move |_| on_rebind());
    if unmapped {
        return (row.upcast(), None);
    }
    let (gear, revealer) = gear_button(ctx, source);
    row.add_suffix(&gear);
    (row.upcast(), Some(revealer))
}

/// Steam's analog row as a native combo: the behavior is the row's own
/// selection, its description sits in the subtitle, and the gear opens the
/// full sheet. Sticks and triggers share the layout — the group header
/// carries the hardware name.
fn analog_row(
    ctx: &PagesCtx,
    source: InputSource,
    mapping: Option<&InputMapping>,
    family: ira_input::ControllerFamily,
) -> (gtk4::Widget, Option<gtk4::Revealer>) {
    let row = adw::ComboRow::new();
    add_source_prefix(&row, source, family, false);
    row.set_title(&crate::tr!("Behavior"));

    let choices = behavior_choices(source);
    let titles: Vec<&str> = choices.iter().map(|choice| choice.title.as_str()).collect();
    row.set_model(Some(&gtk4::StringList::new(&titles)));
    let modes = modes_for(source);
    let current = mapping.and_then(|mapping| mapping.mode.clone());
    let selected = current
        .as_ref()
        .and_then(|mode| {
            modes
                .iter()
                .position(|candidate| same_mode(candidate, mode))
        })
        .unwrap_or(0);
    row.set_selected(selected as u32);
    if let Some(description) = choices
        .get(selected)
        .and_then(|choice| choice.description.clone())
    {
        row.set_subtitle(&description);
    }

    // Connected after the model and selection are set so the initial
    // notify never writes back through set_mode.
    let ctx_for_pick = ctx.clone();
    let modes_for_pick = modes.clone();
    let choices_for_pick = choices.clone();
    row.connect_selected_notify(move |row| {
        let index = row.selected() as usize;
        let mode = modes_for_pick.get(index).cloned().flatten();
        set_mode(&ctx_for_pick, source, mode);
        if let Some(description) = choices_for_pick
            .get(index)
            .and_then(|choice| choice.description.clone())
        {
            row.set_subtitle(&description);
        }
        (ctx_for_pick.on_dirty)();
    });

    if mapping.is_none() {
        return (row.upcast(), None);
    }
    let (gear, revealer) = gear_button(ctx, source);
    row.add_suffix(&gear);
    (row.upcast(), Some(revealer))
}

fn add_source_prefix(
    row: &impl gtk4::glib::object::IsA<adw::ActionRow>,
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
    row.add_prefix(&asset);
    row.add_prefix(&badge);
}

/// The gear every Steam binding row carries: it toggles the input's
/// settings inline — the sheet's groups expand beneath the row instead of
/// a floating window. Content is built lazily on first expansion.
fn gear_button(ctx: &PagesCtx, source: InputSource) -> (gtk4::Button, gtk4::Revealer) {
    let gear = gtk4::Button::from_icon_name("emblem-system-symbolic");
    gear.add_css_class(CSS_FLAT);
    gear.add_css_class(CSS_SQUARE_BUTTON);
    gear.set_valign(gtk4::Align::Center);
    gear.set_tooltip_text(Some(&crate::tr!("Edit")));
    let revealer = gtk4::Revealer::new();
    revealer.set_transition_type(gtk4::RevealerTransitionType::SlideDown);
    revealer.set_visible(false);
    let built = std::rc::Rc::new(std::cell::Cell::new(false));
    let ctx = ctx.clone();
    let revealer_for_toggle = revealer.clone();
    gear.connect_clicked(move |gear| {
        let reveal = !revealer_for_toggle.reveals_child();
        if reveal && !built.replace(true) {
            revealer_for_toggle
                .set_child(Some(&inline_sheet_content(&ctx, source)));
            revealer_for_toggle.set_visible(true);
        }
        revealer_for_toggle.set_reveal_child(reveal);
        gear.set_tooltip_text(Some(&crate::tr!("Collapse")));
    });
    (gear, revealer)
}

/// Steam's behavior list for an analog source: the modes it can take, each
/// with its one-line description. Titles must match `mode_label` — the
/// tests pin them together.
fn behavior_choices(source: InputSource) -> Vec<OptionChoice> {
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

/// Behavior picks write the mode onto the mapping, creating the mapping
/// from the identity default when the input was unmapped.
fn set_mode(ctx: &PagesCtx, source: InputSource, mode: Option<SourceMode>) {
    let target = ctx.active_target.get();
    let mut profile = ctx.profile.borrow_mut();
    if let Some(inputs) = target.inputs_mut(&mut profile) {
        match inputs.iter_mut().find(|input| input.source == source) {
            Some(input) => input.mode = mode,
            None => {
                if mode.is_some() {
                    let mut mapping = default_mapping(source);
                    mapping.mode = mode;
                    inputs.push(mapping);
                }
            }
        }
    }
}

/// Hook for plain row activation: opens the one-window command picker that
/// swaps the input's click command, Steam-style.
fn rebind_hook(ctx: &PagesCtx, source: InputSource) -> Rc<dyn Fn()> {
    let ctx = ctx.clone();
    Rc::new(move || open_rebind_picker(&ctx, source))
}

/// The output the quick rebind targets: the click activator's command, or
/// whatever advanced activator carries the first output.
fn primary_output(mapping: &InputMapping) -> Option<OutputAction> {
    mapping
        .activators
        .iter()
        .find(|activator| matches!(activator.kind, ActivatorKind::FullPress))
        .or_else(|| mapping.activators.first())
        .and_then(|activator| activator.outputs.first())
        .cloned()
}

/// One-window rebinding: pick a command and it replaces this input's click
/// output. Rarer activators (double press, soft pull, shifts) keep their
/// outputs; those stay in the advanced sheet.
fn open_rebind_picker(ctx: &PagesCtx, source: InputSource) {
    let current = active_mappings(&ctx.profile, ctx.active_target.get())
        .iter()
        .find(|mapping| mapping.source == source)
        .and_then(primary_output);
    let profile = ctx.profile.borrow();
    let scope = OutputPickerScope {
        backend: profile.backend,
        set_names: profile
            .action_sets
            .iter()
            .map(|set| set.name.clone())
            .collect(),
        layer_names: profile
            .action_layers
            .iter()
            .map(|layer| layer.name.clone())
            .collect(),
    };
    drop(profile);
    let title = crate::tr!("Bind {}").replacen("{}", &source_label(source), 1);
    let ctx_for_pick = ctx.clone();
    show_output_picker(
        &ctx.window,
        &title,
        &scope,
        current.as_ref(),
        move |output| {
            apply_quick_rebind(&ctx_for_pick, source, output);
        },
    );
}

fn apply_quick_rebind(ctx: &PagesCtx, source: InputSource, output: OutputAction) {
    {
        let target = ctx.active_target.get();
        let mut profile = ctx.profile.borrow_mut();
        let Some(inputs) = target.inputs_mut(&mut profile) else {
            return;
        };
        match inputs.iter_mut().find(|input| input.source == source) {
            Some(input) => {
                match input
                    .activators
                    .iter_mut()
                    .find(|activator| matches!(activator.kind, ActivatorKind::FullPress))
                {
                    Some(activator) => activator.outputs = vec![output],
                    None => input
                        .activators
                        .push(ira_input::Activator::full_press(vec![output])),
                }
            }
            None => inputs.push(InputMapping::simple(source, output)),
        }
    }
    (ctx.on_dirty)();
}

fn inline_sheet_content(ctx: &PagesCtx, source: InputSource) -> gtk4::Box {
    let backend = ctx.profile.borrow().backend;
    // The sheet mutates the profile directly; refresh summaries + dirty
    // state after every change it reports.
    let ctx_for_changes = ctx.clone();
    let on_changed: Rc<dyn Fn()> = Rc::new(move || (ctx_for_changes.on_dirty)());
    build_inline_input_sheet(InputSheetRequest {
        profile: ctx.profile.clone(),
        gyro: ctx.gyro.clone(),
        active_target: ctx.active_target.get(),
        source,
        device: ctx.device.clone(),
        backend,
        on_changed,
    })
}

/// Identity mapping for a freshly added input: buttons passthrough to their
/// virtual counterpart; sticks/triggers get their natural analog mode.
pub(crate) fn default_mapping(source: InputSource) -> InputMapping {
    match source {
        InputSource::Button(button) => {
            // Identity: sources and outputs are positional on every backend,
            // matching how SDL names a real controller of the same kind.
            InputMapping::simple(source, OutputAction::GamepadButton(button))
        }
        InputSource::Axis(GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger) => InputMapping {
            mode: Some(SourceMode::Trigger { threshold: 0.5 }),
            ..InputMapping::new(source)
        },
        InputSource::Axis(axis) => {
            let output = match axis {
                GamepadAxis::RightX | GamepadAxis::RightY => StickOutput::Right,
                _ => StickOutput::Left,
            };
            InputMapping {
                mode: Some(SourceMode::joystick(output)),
                ..InputMapping::new(source)
            }
        }
        InputSource::AxisDirection { .. } => {
            InputMapping::simple(source, OutputAction::GamepadButton(GamepadButton::DpadUp))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::input_profile_sheet_base::is_trigger_axis;
    use super::super::input_profile_source_modes::{mode_label, modes_for};
    use super::{behavior_choices, default_mapping, is_stick_source};
    use ira_input::{GamepadAxis, GamepadButton, InputSource, OutputAction};

    #[test]
    fn test_default_mapping_is_identity_on_every_backend() {
        // Positional identity, including Switch Pro: a virtual pad must
        // identify like the real controller SDL names after.
        let south = default_mapping(InputSource::Button(GamepadButton::A));
        assert!(south.activators[0]
            .outputs
            .contains(&OutputAction::GamepadButton(GamepadButton::A)));
        let east = default_mapping(InputSource::Button(GamepadButton::B));
        assert!(east.activators[0]
            .outputs
            .contains(&OutputAction::GamepadButton(GamepadButton::B)));
    }

    #[test]
    fn test_behavior_choices_align_with_modes_for() {
        for source in [
            InputSource::Axis(GamepadAxis::LeftX),
            InputSource::Axis(GamepadAxis::RightX),
            InputSource::Axis(GamepadAxis::LeftTrigger),
        ] {
            let trigger = is_trigger_axis(source);
            let modes = modes_for(source);
            let choices = behavior_choices(source);
            assert_eq!(choices.len(), modes.len());
            for (choice, mode) in choices.iter().zip(modes.iter()) {
                assert_eq!(choice.title, mode_label(mode, trigger));
            }
        }
    }

    #[test]
    fn test_stick_rows_are_the_stick_x_axes() {
        assert!(is_stick_source(InputSource::Axis(GamepadAxis::LeftX)));
        assert!(is_stick_source(InputSource::Axis(GamepadAxis::RightX)));
        // Y rides along inside the mode, and the click is a command row.
        assert!(!is_stick_source(InputSource::Axis(GamepadAxis::LeftY)));
        assert!(!is_stick_source(InputSource::Button(
            GamepadButton::LeftStick
        )));
    }

    #[test]
    fn test_default_stick_mapping_has_no_activators() {
        // A behavior pick on an unmapped stick creates the identity default
        // with the mode swapped in — no stray click bindings.
        let stick = default_mapping(InputSource::Axis(GamepadAxis::RightX));
        assert!(stick.activators.is_empty());
        assert!(stick.mode.is_some());
    }
}

#[cfg(test)]
mod gtk_repro {
    use super::*;

    // Manual regression check for the recurring editor critical bursts:
    // run with `cargo test -p ira --lib gtk_repro -- --ignored --nocapture`
    // and watch stderr; the fixed bug was adw::ButtonContent suffixes, whose
    // construction inside preferences groups finalized widgets GTK still
    // touched (get_parent/add_css_class pairs per page per rebuild).
    // GTK initializes once per process, so run the tests in this module one
    // filter at a time.
    #[test]
    #[ignore]
    fn repro_region_rebuild_criticals() {
        let _ = gtk4::init();
        let _ = adw::init();
        let window = adw::Dialog::new();
        window.set_content_width(980);
        window.set_content_height(740);
        let header = adw::HeaderBar::new();
        let sidebar_scroll = gtk4::ScrolledWindow::new();
        let sidebar = gtk4::ListBox::new();
        sidebar.add_css_class("navigation-sidebar");
        sidebar_scroll.set_child(Some(&sidebar));
        sidebar_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        let stack = gtk4::Stack::new();
        stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
        let layout = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        layout.append(&sidebar_scroll);
        layout.append(&stack);
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root.append(&header);
        root.append(&layout);
        window.set_child(Some(&root));

        let ctx = PagesCtx {
            window: window.clone(),
            profile: std::rc::Rc::new(std::cell::RefCell::new(InputProfile::default())),
            active_target: std::rc::Rc::new(std::cell::Cell::new(
                EditingTarget::Set(0),
            )),
            indicator: std::rc::Rc::new(
                super::super::input_profile_set_indicator::SetIndicator::new(),
            ),
            device: None,
            on_dirty: std::rc::Rc::new(|| {}),
            gyro: std::rc::Rc::new(std::cell::RefCell::new(
                InputProfile::default().gyro,
            )),
        };
        let mut region_boxes = Vec::new();
        for region in Region::ALL {
            let content = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
            let scroll = gtk4::ScrolledWindow::new();
            scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
            scroll.set_child(Some(&content));
            stack.add_named(&scroll, Some(region.id()));
            sidebar.append(&super::super::settings_dialog::settings_sidebar_row(
                region.icon(),
                &region.title(),
                region.id(),
            ));
            region_boxes.push(content);
        }
        let pages = RegionPages { region_boxes };
        window.present(None::<&gtk4::Widget>);
        let main = gtk4::glib::MainContext::default();
        for _ in 0..3 {
            let ctx = ctx.clone();
            let pages = pages.clone();
            gtk4::glib::idle_add_local_once(move || {
                rebuild_region_pages(&ctx, &pages);
            });
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
            while std::time::Instant::now() < deadline {
                while main.iteration(false) {}
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        window.close();
    }
}
