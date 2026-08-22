//! Region pages for the profile editor: one page per controller region, one
//! row per *input* (not per binding), with unmapped inputs shown as dim
//! add-slots — mirroring Steam Input's per-input navigation. Rows open the
//! activator sheet for deep editing.

use super::helpers::esc;
use super::input_profile_activator_sheet::{show_input_sheet, InputSheetRequest};
use super::input_profile_editor_regions::{
    input_row, source_label, supported_button_sources, Region,
};
use adw::prelude::*;
use ira_input::{
    GamepadAxis, GamepadButton, InputMapping, InputProfile, InputSource, OutputAction,
    SourceMode, StickOutput,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub(crate) type ProfileRc = Rc<RefCell<InputProfile>>;

/// Everything the region pages and the sheet hooks need.
#[derive(Clone)]
pub(crate) struct PagesCtx {
    pub window: adw::Window,
    pub profile: ProfileRc,
    pub active_set: Rc<Cell<usize>>,
    pub device: Option<ira_input::DeviceInfo>,
    pub on_dirty: Rc<dyn Fn()>,
}

/// Handle to the region page contents; the Gyro and Action Sets pages are
/// static and stay inside the editor shell.
#[derive(Clone)]
pub(crate) struct RegionPages {
    pub region_boxes: Vec<gtk4::Box>,
}

pub(crate) fn region_sources(region: Region, device: Option<&ira_input::DeviceInfo>) -> Vec<InputSource> {
    let mut sources: Vec<InputSource> = match region {
        Region::FaceButtons => [GamepadButton::A, GamepadButton::B, GamepadButton::X, GamepadButton::Y]
            .into_iter()
            .map(InputSource::Button)
            .collect(),
        Region::Dpad => [
            GamepadButton::DpadUp,
            GamepadButton::DpadRight,
            GamepadButton::DpadDown,
            GamepadButton::DpadLeft,
        ]
        .into_iter()
        .map(InputSource::Button)
        .collect(),
        Region::TriggersBumpers => [
            InputSource::Button(GamepadButton::LeftShoulder),
            InputSource::Button(GamepadButton::RightShoulder),
            InputSource::Axis(GamepadAxis::LeftTrigger),
            InputSource::Axis(GamepadAxis::RightTrigger),
        ]
        .into_iter()
        .chain([
            InputSource::Button(GamepadButton::LeftTrigger),
            InputSource::Button(GamepadButton::RightTrigger),
        ])
        .collect(),
        Region::Sticks => [
            InputSource::Axis(GamepadAxis::LeftX),
            InputSource::Axis(GamepadAxis::LeftY),
            InputSource::Axis(GamepadAxis::RightX),
            InputSource::Axis(GamepadAxis::RightY),
            InputSource::Button(GamepadButton::LeftStick),
            InputSource::Button(GamepadButton::RightStick),
        ]
        .into_iter()
        .collect(),
        Region::SystemPaddles => [
            InputSource::Button(GamepadButton::Back),
            InputSource::Button(GamepadButton::Start),
            InputSource::Button(GamepadButton::Guide),
        ]
        .into_iter()
        .chain(
            supported_button_sources(device)
                .into_iter()
                .filter(|source| {
                    matches!(source, InputSource::Button(button) if button.is_paddle())
                }),
        )
        .collect(),
    };
    sources.dedup();
    sources
}

/// Rebuild every region page from the profile's active action set.
pub(crate) fn rebuild_region_pages(ctx: &PagesCtx, pages: &RegionPages) {
    let family = ctx
        .device
        .as_ref()
        .map(ira_input::DeviceInfo::family)
        .unwrap_or_default();
    let mappings = active_mappings(&ctx.profile, ctx.active_set.get());
    for (index, region) in Region::ALL.into_iter().enumerate() {
        let page = &pages.region_boxes[index];
        super::helpers::clear_children(page);
        let group = adw::PreferencesGroup::new();
        group.set_title(&region.title());
        for source in region_sources(region, ctx.device.as_ref()) {
            let mapping = mappings.iter().find(|mapping| mapping.source == source);
            group.add(&region_source_row(ctx, source, mapping, family));
        }
        page.append(&group);
    }
}

fn active_mappings(profile: &ProfileRc, active_set: usize) -> Vec<InputMapping> {
    profile
        .borrow()
        .action_sets
        .get(active_set)
        .map(|set| set.inputs.clone())
        .unwrap_or_default()
}

fn region_source_row(
    ctx: &PagesCtx,
    source: InputSource,
    mapping: Option<&InputMapping>,
    family: ira_input::ControllerFamily,
) -> adw::ActionRow {
    match mapping {
        Some(mapping) => {
            let on_edit = open_sheet_hook(ctx, source);
            let ctx_for_remove = ctx.clone();
            let on_remove: Rc<dyn Fn()> = Rc::new(move || {
                remove_mapping(&ctx_for_remove, source);
                (ctx_for_remove.on_dirty)();
            });
            input_row(mapping, family, &on_edit, &on_remove)
        }
        None => unmapped_row(ctx, source, family),
    }
}

fn unmapped_row(ctx: &PagesCtx, source: InputSource, family: ira_input::ControllerFamily) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&esc(&source_label(source)));
    row.set_subtitle(&crate::tr!("Not mapped"));
    let badge = super::input_profile_assets::source_badge(source, family);
    let badge_label = gtk4::Label::new(Some(&badge));
    badge_label.add_css_class(super::css::CSS_SOURCE_BADGE);
    badge_label.add_css_class(super::css::CSS_DIM_LABEL);
    badge_label.set_valign(gtk4::Align::Center);
    row.add_suffix(&badge_label);

    let add = gtk4::Button::from_icon_name("list-add-symbolic");
    add.add_css_class(super::css::CSS_FLAT);
    add.add_css_class(super::css::CSS_SQUARE_BUTTON);
    add.set_valign(gtk4::Align::Center);
    add.set_tooltip_text(Some(&crate::tr!("Add binding")));
    let ctx_for_add = ctx.clone();
    add.connect_clicked(move |_| {
        let mapping = default_mapping(source);
        insert_mapping(&ctx_for_add, mapping);
        (ctx_for_add.on_dirty)();
        open_sheet(&ctx_for_add, source);
    });
    row.add_suffix(&add);
    row
}

/// Hook that opens the sheet for one source; shared by edit buttons and row
/// activation.
fn open_sheet_hook(ctx: &PagesCtx, source: InputSource) -> Rc<dyn Fn()> {
    let ctx = ctx.clone();
    Rc::new(move || open_sheet(&ctx, source))
}

fn open_sheet(ctx: &PagesCtx, source: InputSource) {
    let backend = ctx.profile.borrow().backend;
    // The sheet mutates the profile directly; refresh summaries + dirty
    // state after every change it reports.
    let ctx_for_changes = ctx.clone();
    let on_changed: Rc<dyn Fn()> = Rc::new(move || (ctx_for_changes.on_dirty)());
    show_input_sheet(
        &ctx.window,
        InputSheetRequest {
            profile: ctx.profile.clone(),
            active_set: ctx.active_set.get(),
            source,
            device: ctx.device.clone(),
            backend,
            on_changed,
        },
    );
}

fn insert_mapping(ctx: &PagesCtx, mapping: InputMapping) {
    let set = ctx.active_set.get();
    let mut profile = ctx.profile.borrow_mut();
    if let Some(set) = profile.action_sets.get_mut(set) {
        set.inputs.retain(|input| input.source != mapping.source);
        set.inputs.push(mapping);
    }
}

fn remove_mapping(ctx: &PagesCtx, source: InputSource) {
    let set = ctx.active_set.get();
    let mut profile = ctx.profile.borrow_mut();
    if let Some(set) = profile.action_sets.get_mut(set) {
        set.inputs.retain(|input| input.source != source);
    }
}

/// Identity mapping for a freshly added input: buttons passthrough to their
/// virtual counterpart; sticks/triggers get their natural analog mode.
pub(crate) fn default_mapping(source: InputSource) -> InputMapping {
    match source {
        InputSource::Button(button) => {
            InputMapping::simple(source, OutputAction::GamepadButton(button))
        }
        InputSource::Axis(GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger) => {
            InputMapping {
                mode: Some(SourceMode::Trigger { threshold: 0.5 }),
                ..InputMapping::new(source)
            }
        }
        InputSource::Axis(axis) => InputMapping {
            mode: Some(SourceMode::Joystick {
                output: match axis {
                    GamepadAxis::RightX | GamepadAxis::RightY => StickOutput::Right,
                    _ => StickOutput::Left,
                },
                deadzone_inner: 0.1,
                deadzone_outer: 0.95,
                curve: 1.0,
            }),
            ..InputMapping::new(source)
        },
        InputSource::AxisDirection { .. } => InputMapping::simple(
            source,
            OutputAction::GamepadButton(GamepadButton::DpadUp),
        ),
    }
}
