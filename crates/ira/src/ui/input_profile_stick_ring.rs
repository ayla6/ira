//! Outer Ring rows for the stick sheet: the command held while the stick
//! sits past the ring radius, its radius slider, and the invert switch.

use super::input_output_picker::{show_output_picker, OutputPickerScope};
use super::input_profile_options::output_display_label;
use super::input_profile_sheet_base::{Reopen, SheetBase};
use super::input_profile_source_modes::mode_slider_row;
use super::input_profile_source_modes::ModeTarget;
use super::input_profile_stick_settings::{mode_switch_row, processing_of, write_processing};
use super::input_profile_widgets::SliderSpec;
use adw::prelude::*;
use ira_input::{OutputAction, SourceMode, StickProcessing};

/// The Outer Ring rows: a command held while the stick sits past the ring
/// radius. Unconfigured, the group offers the command slot; configured, it
/// shows the command, its radius, and the invert switch.
pub(super) fn outer_ring_rows(
    base: &SheetBase,
    target: ModeTarget,
    reopen: &Reopen,
    processing: &StickProcessing,
) -> Vec<gtk4::ListBoxRow> {
    let Some(ring) = processing.outer_ring.as_ref() else {
        let row = adw::ActionRow::new();
        row.set_title(&crate::tr!("Outer Ring Command"));
        row.set_subtitle(&crate::tr!(
            "Hold a button or key while the stick is at the edge — a sprint key, for example"
        ));
        let add =
            super::helpers::icon_label_button("list-add-symbolic", &crate::tr!("Add command"));
        add.set_valign(gtk4::Align::Center);
        row.add_suffix(&add);
        let base_for_add = base.clone();
        let reopen_for_add = reopen.clone();
        add.connect_clicked(move |_| {
            pick_outer_ring_command(&base_for_add, target, None, &reopen_for_add);
        });
        return vec![row.upcast()];
    };

    let command_row = adw::ActionRow::new();
    command_row.set_title(&crate::tr!("Outer Ring Command"));
    command_row.set_subtitle(&crate::tr!(
        "When outside this radius on the Joystick, the assigned button or key will be sent"
    ));
    let pick_btn = gtk4::Button::with_label(&output_display_label(&ring.output));
    pick_btn.add_css_class(super::css::CSS_FLAT);
    pick_btn.set_valign(gtk4::Align::Center);
    {
        let base_for_pick = base.clone();
        let reopen_for_pick = reopen.clone();
        let current = ring.output.clone();
        pick_btn.connect_clicked(move |_| {
            pick_outer_ring_command(&base_for_pick, target, Some(&current), &reopen_for_pick);
        });
    }
    command_row.add_suffix(&pick_btn);

    let mut rows = vec![command_row.upcast()];
    rows.push(
        mode_slider_row(
            base,
            target,
            &crate::tr!("Outer Ring Command Radius"),
            Some(&crate::tr!(
                "The slider can be visualized as extending a radius from the center outward, with the point being where the outer ring begins"
            )),
            &SliderSpec(0.05, 1.0, 0.01, f64::from(ring.radius)),

            |mode, value| {
                if let Some(ring) = outer_ring_of(mode) {
                    ring.radius = value as f32;
                }
            },
        )
        .upcast(),
    );
    rows.push(
        mode_switch_row(
            base,
            target,
            &crate::tr!("Outer Ring Command Invert"),
            Some(&crate::tr!(
                "If set, the command will be sent when inside the radius instead of outside"
            )),
            ring.invert,
            |mode, enabled| {
                if let Some(ring) = outer_ring_of(mode) {
                    ring.invert = enabled;
                }
            },
        )
        .upcast(),
    );

    let remove_row = adw::ActionRow::new();
    remove_row.set_title(&crate::tr!("Remove outer ring command"));
    let trash = gtk4::Button::from_icon_name("user-trash-symbolic");
    trash.add_css_class(super::css::CSS_FLAT);
    trash.add_css_class(super::css::CSS_SQUARE_BUTTON);
    trash.set_valign(gtk4::Align::Center);
    remove_row.add_suffix(&trash);
    let base_for_remove = base.clone();
    let reopen_for_remove = reopen.clone();
    trash.connect_clicked(move |_| {
        write_processing(&base_for_remove, target, |processing| {
            processing.outer_ring = None;
        });
        reopen_for_remove();
    });
    rows.push(remove_row.upcast());
    rows
}

/// The ring command a mode carries, whichever behavior it belongs to.
fn outer_ring_of(mode: &mut SourceMode) -> Option<&mut ira_input::OuterRingCommand> {
    processing_of(mode).and_then(|processing| processing.outer_ring.as_mut())
}

/// Open the output picker for the outer ring command and write the pick
/// into the targeted mode's ring.
fn pick_outer_ring_command(
    base: &SheetBase,
    target: ModeTarget,
    current: Option<&OutputAction>,
    reopen: &Reopen,
) {
    let Some(window) = base
        .child_expander
        .borrow()
        .as_ref()
        .and_then(|expander| {
            expander
                .upcast_ref::<gtk4::Widget>()
                .root()
                .and_then(|root| root.downcast::<gtk4::Window>().ok())
        })
    else {
        return;
    };
    let profile = base.profile.borrow();
    let scope = OutputPickerScope {
        backend: base.backend,
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
    let base_for_pick = base.clone();
    let reopen_for_pick = reopen.clone();
    show_output_picker(
        &window,
        &crate::tr!("Outer Ring Command"),
        &scope,
        current,
        move |output| {
            write_processing(&base_for_pick, target, |processing| {
                processing.outer_ring = Some(ira_input::OuterRingCommand {
                    output: output.clone(),
                    ..Default::default()
                });
            });
            reopen_for_pick();
        },
    );
}
