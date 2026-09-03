use std::time::Instant;

use crate::{MappingEngine, ReportRateEstimator};

use super::outputs::{emit_mapped, emit_outputs, OutputTargets};
use super::super::TraceState;

/// Maps the hub-forwarded input events and stashes motion samples for the
/// tick. Both arrive pre-routed: this session owns focus.
pub(crate) fn process_pad_events(
    pending: &mut Vec<super::super::hub::PadEvent>,
    samples: &mut Vec<crate::SensorSample>,
    mapper: &mut MappingEngine,
    report_rate: &mut ReportRateEstimator,
    mut targets: OutputTargets<'_>,
    trace: &mut TraceState,
) -> Result<(), String> {
    // The report rate describes the pad's event cadence: one observation
    // per drained batch, like the direct-read loop this replaced, not one
    // per buffered event (those share microseconds).
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
            super::super::hub::PadEvent::Sample(sample) => samples.push(sample),
            // Control variants are consumed in the session loop.
            _ => {}
        }
    }
    if observed {
        report_rate.observe(Instant::now());
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
