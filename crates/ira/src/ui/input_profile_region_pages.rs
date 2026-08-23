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
        let sources = region_sources(region, ctx.device.as_ref());
        let group = adw::PreferencesGroup::new();
        group.set_title(&region.title());
        if !sources.is_empty() {
            group.add(&section_behavior_row(ctx, region, &mappings));
        }
        for source in &sources {
            let mapping = mappings.iter().find(|mapping| mapping.source == *source);
            group.add(&region_source_row(ctx, *source, mapping, family));
        }
        page.append(&group);
        if let Some(pairs) = swappable_pairs(region) {
            page.append(&swap_group(ctx, &pairs));
        }
    }
}

/// Steam-style section behavior: "Default" while every input of the region
/// carries its standard one-to-one mapping, "Custom" as soon as anything
/// differs. Picking Default restores the standard mappings for the section.
fn section_behavior_row(
    ctx: &PagesCtx,
    region: Region,
    mappings: &[InputMapping],
) -> adw::ComboRow {
    let row = adw::ComboRow::new();
    row.set_title(&crate::tr!("Behavior"));
    row.set_subtitle(&crate::tr!(
        "Apply this behavior to every binding in this section"
    ));
    row.set_model(Some(&gtk4::StringList::new(&[
        &crate::tr!("Default"),
        &crate::tr!("Custom"),
    ])));
    let is_default = region_is_at_defaults(region, ctx.device.as_ref(), mappings);
    row.set_selected(if is_default { 0 } else { 1 });
    // Only a manual flip onto Default mutates anything; the notify that
    // fires for the initial selection must not rewrite the profile.
    let applied = Rc::new(std::cell::Cell::new(is_default));
    let ctx_for_select = ctx.clone();
    row.connect_selected_notify(move |combo| {
        if combo.selected() != 0 || applied.get() {
            return;
        }
        applied.set(true);
        for source in region_sources(region, ctx_for_select.device.as_ref()) {
            insert_mapping(&ctx_for_select, default_mapping(source));
        }
        (ctx_for_select.on_dirty)();
    });
    row
}

/// True when every source of the region is mapped exactly to its default.
fn region_is_at_defaults(
    region: Region,
    device: Option<&ira_input::DeviceInfo>,
    mappings: &[InputMapping],
) -> bool {
    region_sources(region, device).into_iter().all(|source| {
        mappings
            .iter()
            .find(|mapping| mapping.source == source)
            .is_some_and(|mapping| *mapping == default_mapping(source))
    })
}

/// Source pairs whose bindings trade places with "Swap left with right".
type SourcePair = (InputSource, InputSource);

fn swappable_pairs(region: Region) -> Option<Vec<SourcePair>> {
    use GamepadAxis::{LeftTrigger, LeftX, LeftY, RightTrigger, RightX, RightY};
    use GamepadButton::{
        LeftShoulder, LeftStick, LeftTrigger as LeftTriggerButton, RightShoulder, RightStick,
        RightTrigger as RightTriggerButton,
    };
    let pairs = match region {
        Region::TriggersBumpers => vec![
            (
                InputSource::Axis(LeftTrigger),
                InputSource::Axis(RightTrigger),
            ),
            (
                InputSource::Button(LeftTriggerButton),
                InputSource::Button(RightTriggerButton),
            ),
            (
                InputSource::Button(LeftShoulder),
                InputSource::Button(RightShoulder),
            ),
        ],
        Region::Sticks => vec![
            (InputSource::Axis(LeftX), InputSource::Axis(RightX)),
            (InputSource::Axis(LeftY), InputSource::Axis(RightY)),
            (
                InputSource::Button(LeftStick),
                InputSource::Button(RightStick),
            ),
        ],
        _ => return None,
    };
    Some(pairs)
}

fn swap_group(ctx: &PagesCtx, pairs: &[SourcePair]) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    let row = adw::ActionRow::new();
    row.set_title(&crate::tr!("Swap left with right"));
    row.set_subtitle(&crate::tr!(
        "Trade every left-side binding of this page with its right-side counterpart"
    ));
    let swap = gtk4::Button::with_label(&crate::tr!("Swap"));
    swap.add_css_class(super::css::CSS_FLAT);
    swap.set_valign(gtk4::Align::Center);
    row.add_suffix(&swap);
    group.add(&row);
    let ctx_for_swap = ctx.clone();
    let pairs = pairs.to_vec();
    swap.connect_clicked(move |_| {
        let set = ctx_for_swap.active_set.get();
        let mut profile = ctx_for_swap.profile.borrow_mut();
        if let Some(set) = profile.action_sets.get_mut(set) {
            for (left, right) in &pairs {
                let left_mapping = set.inputs.iter().find(|m| m.source == *left).cloned();
                let right_mapping = set.inputs.iter().find(|m| m.source == *right).cloned();
                match (left_mapping, right_mapping) {
                    (Some(l), Some(r)) => {
                        for input in &mut set.inputs {
                            if input.source == *left {
                                *input = r.clone();
                            } else if input.source == *right {
                                *input = l.clone();
                            }
                        }
                    }
                    (Some(l), None) => {
                        set.inputs.retain(|input| input.source != *left);
                        set.inputs.push(InputMapping {
                            source: *right,
                            ..l
                        });
                    }
                    (None, Some(r)) => {
                        set.inputs.retain(|input| input.source != *right);
                        set.inputs.push(InputMapping { source: *left, ..r });
                    }
                    (None, None) => {}
                }
            }
        }
        drop(profile);
        (ctx_for_swap.on_dirty)();
    });
    group
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
    row.add_css_class("unmapped-row");
    let badge = super::input_profile_assets::source_badge(source, family);
    let badge_label = gtk4::Label::new(Some(&badge));
    badge_label.add_css_class(super::css::CSS_SOURCE_BADGE);
    badge_label.add_css_class(super::css::CSS_DIM_LABEL);
    badge_label.set_valign(gtk4::Align::Center);
    row.add_suffix(&badge_label);

    // Whole-area affordance instead of a bare icon: the row itself adds.
    let add = super::helpers::icon_label_button("list-add-symbolic", &crate::tr!("Add"));
    row.add_suffix(&add);
    let ctx_for_add = ctx.clone();
    row.set_activatable(true);
    row.connect_activated(move |_| {
        let mapping = default_mapping(source);
        insert_mapping(&ctx_for_add, mapping);
        (ctx_for_add.on_dirty)();
        open_sheet(&ctx_for_add, source);
    });
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

#[cfg(test)]
mod tests {
    use super::{region_is_at_defaults, swappable_pairs, default_mapping, Region};
    use ira_input::{InputMapping, InputSource};

    #[test]
    fn test_region_is_at_defaults_true_for_identity_mappings() {
        let sources = super::region_sources(Region::Dpad, None);
        let mappings: Vec<InputMapping> = sources.iter().map(|s| default_mapping(*s)).collect();
        assert!(region_is_at_defaults(Region::Dpad, None, &mappings));
    }

    #[test]
    fn test_region_is_at_defaults_false_when_unmapped_or_changed() {
        let sources = super::region_sources(Region::FaceButtons, None);
        // One source missing entirely: not at defaults.
        let partial: Vec<InputMapping> = sources
            .iter()
            .skip(1)
            .map(|s| default_mapping(*s))
            .collect();
        assert!(!region_is_at_defaults(Region::FaceButtons, None, &partial));

        // An output that differs from identity reads as custom.
        let mut changed: Vec<InputMapping> =
            sources.iter().map(|s| default_mapping(*s)).collect();
        if let Some(first) = changed.first_mut() {
            first.activators[0].outputs.clear();
        }
        assert!(!region_is_at_defaults(Region::FaceButtons, None, &changed));
    }

    #[test]
    fn test_swappable_pairs_only_on_lateral_regions() {
        assert!(swappable_pairs(Region::TriggersBumpers).is_some());
        assert!(swappable_pairs(Region::Sticks).is_some());
        assert!(swappable_pairs(Region::FaceButtons).is_none());
        assert!(swappable_pairs(Region::SystemPaddles).is_none());
        for (left, right) in swappable_pairs(Region::Sticks).unwrap() {
            assert_ne!(left, right);
            assert_eq!(
                InputSource::category(left),
                InputSource::category(right)
            );
        }
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
    #[test]
    #[ignore]
    fn repro_region_rebuild_criticals() {
        let _ = gtk4::init();
        let _ = adw::init();
        let window = adw::Window::new();
        window.set_default_size(980, 740);
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
        window.set_content(Some(&root));

        let ctx = PagesCtx {
            window: window.clone(),
            profile: std::rc::Rc::new(std::cell::RefCell::new(InputProfile::default())),
            active_set: std::rc::Rc::new(std::cell::Cell::new(0usize)),
            device: None,
            on_dirty: std::rc::Rc::new(|| {}),
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
        window.present();
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
