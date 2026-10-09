use std::collections::HashSet;
use std::time::Instant;

use crate::{InputEvent, InputSource, MappingEngine, ReportRateEstimator};

use super::outputs::{emit_mapped, emit_outputs, OutputTargets};
use super::super::TraceState;

/// Maps hub-forwarded input events and stashes motion samples for the tick.
/// The overlay toggle chord never reaches here; the session loop filters it first.
pub(crate) fn process_pad_events(
    pending: &mut Vec<super::super::hub::PadEvent>,
    samples: &mut Vec<crate::SensorSample>,
    mapper: &mut MappingEngine,
    report_rate: &mut ReportRateEstimator,
    mut targets: OutputTargets<'_>,
    trace: &mut TraceState,
) -> Result<(), String> {
    // One observation per drained batch, not per buffered event (those share
    // microseconds); motion counts too, or gyro-only sessions never tighten.
    let mut observed = false;
    for event in std::mem::take(pending) {
        match event {
            super::super::hub::PadEvent::Input(event) => {
                observed = true;
                emit_mapped(
                    mapper,
                    OutputTargets {
                        gamepad: targets.gamepad,
                        keyboard: targets.keyboard.as_deref_mut(),
                        mouse: targets.mouse.as_deref_mut(),
                        pad: targets.pad,
                    },
                    event,
                    trace,
                )?;
            }
            super::super::hub::PadEvent::Sample(sample) => {
                observed = true;
                samples.push(sample);
            }
            // Control variants are consumed in the session loop.
            _ => {}
        }
    }
    if observed {
        report_rate.observe(Instant::now());
    }
    Ok(())
}

/// One cursor-visibility poll, switching to the profile's action set for the
/// new state. Held outputs of the old set are released through the returned events.
pub(crate) fn poll_cursor_switch(
    watcher: &crate::CursorWatcher,
    last_visible: &mut Option<bool>,
    mapper: &mut MappingEngine,
    targets: OutputTargets<'_>,
    trace: &mut TraceState,
) -> Result<(), String> {
    let Some(visible) = watcher.cursor_visible() else {
        return Ok(());
    };
    if *last_visible == Some(visible) {
        return Ok(());
    }
    *last_visible = Some(visible);
    let profile = mapper.profile();
    let target = if visible {
        profile.action_set_when_cursor_shown
    } else {
        profile.action_set_when_cursor_hidden
    };
    let Some(target) = target else {
        return Ok(());
    };
    let releases = mapper.request_action_set(target);
    eprintln!(
        "ira-input: cursor {}; switching to action set {target}",
        if visible { "shown" } else { "hidden" }
    );
    emit_outputs(releases, targets, trace)
}

/// Synthetic releases for mapper-held buttons the pad reports up, so gaps lose
/// no release and stick forever. Pure for testability.
pub(crate) fn releases_for_missing(
    held: Vec<InputSource>,
    pressed: &HashSet<InputSource>,
    timestamp_us: u64,
) -> Vec<InputEvent> {
    held.into_iter()
        .filter(|source| !pressed.contains(source))
        .map(|source| InputEvent {
            source,
            value: 0.0,
            timestamp_us,
        })
        .collect()
}

/// Whether an event value reads as a release: buttons below threshold, sticks
/// near center. Pure for testability.
pub(crate) fn is_release(source: &InputSource, value: f32) -> bool {
    match source {
        InputSource::Button(_) => value <= 0.5,
        InputSource::Axis(_) => value.abs() < 0.5,
        // Directions are derived edges, never raw hub state; they clear
        // with their axis.
        InputSource::AxisDirection { .. } => false,
    }
}
/// Synthetic presses for physically held buttons the mapper doesn't hold
/// (overlay-close hand-back). Pure for testability.
pub(crate) fn presses_for_held(
    held: Vec<InputSource>,
    pressed: &HashSet<InputSource>,
    timestamp_us: u64,
) -> Vec<InputEvent> {
    let held_set: HashSet<InputSource> = held.into_iter().collect();
    pressed
        .iter()
        .filter(|source| {
            matches!(source, InputSource::Button(_)) && !held_set.contains(source)
        })
        .map(|source| InputEvent {
            source: *source,
            value: 1.0,
            timestamp_us,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GamepadButton;

    #[test]
    fn test_is_release_follows_source_thresholds() {
        use crate::{AxisDirection, GamepadAxis, GamepadButton};
        // Buttons release at or below the press threshold.
        assert!(is_release(
            &InputSource::Button(GamepadButton::A),
            0.0
        ));
        assert!(!is_release(
            &InputSource::Button(GamepadButton::A),
            1.0
        ));
        // Sticks release near center only.
        assert!(is_release(&InputSource::Axis(GamepadAxis::LeftX), 0.1));
        assert!(!is_release(&InputSource::Axis(GamepadAxis::LeftX), 0.8));
        // Derived directions never read as releases on their own.
        assert!(!is_release(
            &InputSource::AxisDirection {
                axis: GamepadAxis::LeftX,
                direction: AxisDirection::Negative,
            },
            0.0
        ));
    }

    #[test]
    fn test_releases_for_missing_sends_only_physically_up_buttons() {
        // Arrange: mapper holds A and B, pad reports only B down.
        let held = vec![
            InputSource::Button(GamepadButton::A),
            InputSource::Button(GamepadButton::B),
        ];
        let pressed: HashSet<InputSource> =
            [InputSource::Button(GamepadButton::B)].into_iter().collect();
        let releases = releases_for_missing(held, &pressed, 7);
        // Assert: A gets a real release event, B untouched.
        assert_eq!(
            releases,
            vec![InputEvent {
                source: InputSource::Button(GamepadButton::A),
                value: 0.0,
                timestamp_us: 7,
            }]
        );
    }

    #[test]
    fn test_releases_for_missing_empty_when_nothing_stuck() {
        // Mapper and pad agree: no synthetic traffic.
        let held = vec![InputSource::Button(GamepadButton::A)];
        let pressed: HashSet<InputSource> =
            [InputSource::Button(GamepadButton::A)].into_iter().collect();
        assert!(releases_for_missing(held, &pressed, 0).is_empty());
        assert!(releases_for_missing(Vec::new(), &pressed, 0).is_empty());
    }

    #[test]
    fn test_presses_for_held_represses_gripped_buttons() {
        // Arrange: mapper clean (open-edge release), pad reports A held.
        let pressed: HashSet<InputSource> =
            [InputSource::Button(GamepadButton::A)].into_iter().collect();
        let presses = presses_for_held(Vec::new(), &pressed, 9);
        // Assert: one synthetic press restoring the held button.
        assert_eq!(
            presses,
            vec![InputEvent {
                source: InputSource::Button(GamepadButton::A),
                value: 1.0,
                timestamp_us: 9,
            }]
        );
        // Already-held buttons are left alone (no duplicate edges).
        let held = vec![InputSource::Button(GamepadButton::A)];
        assert!(presses_for_held(held, &pressed, 9).is_empty());
    }
}
