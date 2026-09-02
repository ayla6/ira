use std::time::Instant;

use crate::{MappingEngine, PhysicalGamepad, ReportRateEstimator};

use super::outputs::{emit_mapped, emit_outputs, OutputTargets};
use super::sensor::SensorPipeline;
use super::super::TraceState;

pub(crate) fn process_physical_inputs(
    gamepad: &mut Option<PhysicalGamepad>,
    pipeline: &mut SensorPipeline,
    mapper: &mut MappingEngine,
    report_rate: &mut ReportRateEstimator,
    mut targets: OutputTargets<'_>,
    trace: &mut TraceState,
) -> Result<(), String> {
    // The Switch-protocol driver owns this pad's inputs: its report-mode
    // switch makes whatever generic HID parses from evdev unreliable.
    if let Some(switch) = pipeline.switch_hidraw.as_mut() {
        let events = switch.take_events();
        if !events.is_empty() {
            report_rate.observe(Instant::now());
        }
        for event in events {
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
        return Ok(());
    }
    let Some(gamepad) = gamepad.as_mut() else {
        return Ok(());
    };
    let events = gamepad.fetch_events()?;
    if !events.is_empty() {
        report_rate.observe(Instant::now());
    }
    for event in events {
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
    Ok(())
}

/// One cursor-visibility poll: when the shown/hidden state flipped and the
/// profile maps that state to an action set, switch to it. Held outputs of
/// the old set are released through the returned events.
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
