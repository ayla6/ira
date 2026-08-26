//! Steam-style gyro output shaping rows for the editor's Gyro page:
//! momentum (the glide after the gyro deactivates) and trigger dampening
//! (gyro mouse output scaled down while a trigger is held).

use super::input_profile_sheet_base::combo_row;
use super::input_profile_widgets::{
    format_number, format_percent, slider_row, switch_row, SettingGroup, SliderSpec,
};
use adw::prelude::*;
use ira_input::{GyroConfig, TriggerDampening};
use std::cell::RefCell;
use std::rc::Rc;

pub(super) fn add_gyro_motion_groups(
    page: &gtk4::Box,
    gyro: &Rc<RefCell<GyroConfig>>,
    on_dirty: &Rc<dyn Fn()>,
) {
    page.append(&momentum_group(gyro, on_dirty));
    page.append(&dampening_group(gyro, on_dirty));
}

/// Momentum: after the gyro deactivates, its last motion keeps outputting
/// while friction bleeds it off — Steam's "Enable Momentum" panel.
fn momentum_group(gyro: &Rc<RefCell<GyroConfig>>, on_dirty: &Rc<dyn Fn()>) -> gtk4::Box {
    let group = SettingGroup::new(
        Some(&crate::tr!("Momentum")),
        Some(&crate::tr!(
            "When the gyro deactivates, its motion continues to output for a short time. Friction controls how quickly it slows down."
        )),
    );

    let (initial_enabled, initial_friction) = {
        let borrow = gyro.borrow();
        (borrow.momentum.enabled, borrow.momentum.friction)
    };
    let friction = slider_row(
        &crate::tr!("Friction"),
        Some(&crate::tr!(
            "How quickly the glide slows down; higher stops sooner"
        )),
        &SliderSpec(0.5, 10.0, 0.1, f64::from(initial_friction)),
        format_number,
        {
            let gyro = gyro.clone();
            let on_dirty = on_dirty.clone();
            move |value| {
                gyro.borrow_mut().momentum.friction = value as f32;
                on_dirty();
            }
        },
    );
    let enable = switch_row(
        &crate::tr!("Enable momentum"),
        Some(&crate::tr!(
            "Let the last gyro motion glide to a stop instead of cutting off"
        )),
        initial_enabled,
        {
            let gyro = gyro.clone();
            let on_dirty = on_dirty.clone();
            let friction = friction.clone();
            move |active| {
                gyro.borrow_mut().momentum.enabled = active;
                friction.set_sensitive(active);
                on_dirty();
            }
        },
    );
    friction.set_sensitive(initial_enabled);

    group.add(&enable);
    group.add(&friction);
    group.root
}

/// Trigger dampening: gyro mouse output scales down while the chosen
/// trigger is held, so aiming steadies while shooting.
fn dampening_group(gyro: &Rc<RefCell<GyroConfig>>, on_dirty: &Rc<dyn Fn()>) -> gtk4::Box {
    let group = SettingGroup::new(
        Some(&crate::tr!("Trigger dampening")),
        Some(&crate::tr!(
            "Reduces gyro mouse output while the chosen trigger is held"
        )),
    );

    let mode = combo_row(
        &dampening_labels(),
        dampening_index(gyro.borrow().trigger_dampening),
    );
    mode.set_title(&crate::tr!("Trigger press mouse dampening"));
    mode.set_subtitle(&crate::tr!("Which trigger state steadies the aim"));
    let amount = slider_row(
        &crate::tr!("Dampening amount"),
        Some(&crate::tr!(
            "How much gyro output is removed while the trigger is held"
        )),
        &SliderSpec(0.0, 1.0, 0.05, f64::from(gyro.borrow().dampening_amount)),
        format_percent,
        {
            let gyro = gyro.clone();
            let on_dirty = on_dirty.clone();
            move |value| {
                gyro.borrow_mut().dampening_amount = value as f32;
                on_dirty();
            }
        },
    );
    amount.set_sensitive(gyro.borrow().trigger_dampening != TriggerDampening::Off);

    let gyro_for_mode = gyro.clone();
    let on_dirty_for_mode = on_dirty.clone();
    let amount_for_mode = amount.clone();
    mode.connect_selected_notify(move |dropdown| {
        let selected = dampening_from_index(dropdown.selected());
        gyro_for_mode.borrow_mut().trigger_dampening = selected;
        amount_for_mode.set_sensitive(selected != TriggerDampening::Off);
        on_dirty_for_mode();
    });

    group.add(&mode);
    group.add(&amount);
    group.root
}

/// Dropdown labels in [`TriggerDampening`] declaration order.
fn dampening_labels() -> Vec<String> {
    [
        crate::tr!("Off"),
        crate::tr!("Right Trigger Soft Pull"),
        crate::tr!("Right Trigger Full Pull"),
        crate::tr!("Left Trigger Soft Pull"),
        crate::tr!("Left Trigger Full Pull"),
        crate::tr!("Both Triggers Full Pull"),
    ]
    .into_iter()
    .collect()
}

fn dampening_index(dampening: TriggerDampening) -> u32 {
    match dampening {
        TriggerDampening::Off => 0,
        TriggerDampening::RightTriggerSoftPull => 1,
        TriggerDampening::RightTriggerFullPull => 2,
        TriggerDampening::LeftTriggerSoftPull => 3,
        TriggerDampening::LeftTriggerFullPull => 4,
        TriggerDampening::BothTriggersFullPull => 5,
    }
}

fn dampening_from_index(index: u32) -> TriggerDampening {
    match index {
        1 => TriggerDampening::RightTriggerSoftPull,
        2 => TriggerDampening::RightTriggerFullPull,
        3 => TriggerDampening::LeftTriggerSoftPull,
        4 => TriggerDampening::LeftTriggerFullPull,
        5 => TriggerDampening::BothTriggersFullPull,
        _ => TriggerDampening::Off,
    }
}

#[cfg(test)]
mod tests {
    use super::{dampening_from_index, dampening_index};
    use ira_input::TriggerDampening;

    #[test]
    fn test_dampening_index_round_trip() {
        for dampening in [
            TriggerDampening::Off,
            TriggerDampening::RightTriggerSoftPull,
            TriggerDampening::RightTriggerFullPull,
            TriggerDampening::LeftTriggerSoftPull,
            TriggerDampening::LeftTriggerFullPull,
            TriggerDampening::BothTriggersFullPull,
        ] {
            assert_eq!(dampening_from_index(dampening_index(dampening)), dampening);
        }
        // Out-of-range selections fall back to Off rather than panicking.
        assert_eq!(dampening_from_index(99), TriggerDampening::Off);
    }
}
