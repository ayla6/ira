//! Region pages for the profile editor: Steam Input's binding list, one
//! page per controller region. Every input is a row with its current
//! command shown as a button — clicking it rebinds in one picker; the gear
//! opens the full per-input sheet. Sticks and triggers expose their analog
//! behavior through the same button pattern instead of a section dropdown.

use super::input_output_picker::{show_output_picker, OutputPickerScope};
use super::input_profile_input_rows::input_expander_row;
use super::input_profile_editor_regions::{region_groups, source_label, Region};
use adw::prelude::*;
use ira_input::{
    ActivatorKind, InputMapping, InputProfile, InputSource, OutputAction,
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
    /// Which inputs' settings expanders are open, preserved across the
    /// rebuilds every edit triggers.
    pub expansion: super::input_profile_input_rows::ExpansionState,
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
                widget.add(&input_expander_row(ctx, *source, mapping, family));
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

/// Hook for plain row activation: opens the one-window command picker that
/// swaps the input's click command, Steam-style.
pub(crate) fn rebind_hook(ctx: &PagesCtx, source: InputSource) -> Rc<dyn Fn()> {
    let ctx = ctx.clone();
    Rc::new(move || open_rebind_picker(&ctx, source))
}

/// One-window rebinding: pick a command and it replaces this input's click
/// output. Rarer activators (double press, soft pull, shifts) keep their
/// outputs; those stay in the advanced sheet.
fn primary_output(mapping: &InputMapping) -> Option<OutputAction> {
    mapping.activators.first().and_then(|activator| {
        activator.outputs.first()
    }).cloned()
}

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

#[cfg(test)]
mod tests {
    use super::super::input_profile_sheet_base::is_trigger_axis;
    use super::super::input_profile_source_modes::{mode_label, modes_for};
    use crate::ui::input_profile_input_rows::behavior_choices;
    use ira_input::{GamepadAxis, GamepadButton, InputSource};

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
        assert!(super::super::input_profile_input_rows::is_stick_source(
            InputSource::Axis(GamepadAxis::LeftX)
        ));
        assert!(super::super::input_profile_input_rows::is_stick_source(
            InputSource::Axis(GamepadAxis::RightX)
        ));
        // Y rides along inside the mode, and the click is a command row.
        assert!(!super::super::input_profile_input_rows::is_stick_source(
            InputSource::Axis(GamepadAxis::LeftY)
        ));
        assert!(!super::super::input_profile_input_rows::is_stick_source(
            InputSource::Button(GamepadButton::LeftStick)
        ));
    }

    #[test]
    fn test_default_stick_mapping_has_no_activators() {
        // A behavior pick on an unmapped stick creates the identity default
        // with the mode swapped in — no stray click bindings.
        let stick = super::super::input_profile_input_rows::default_mapping(
            InputSource::Axis(GamepadAxis::RightX),
        );
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
            expansion: std::rc::Rc::new(std::cell::RefCell::new(
                std::collections::HashMap::new(),
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
