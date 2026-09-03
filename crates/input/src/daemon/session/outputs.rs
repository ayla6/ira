use std::path::Path;

use crate::{InputEvent, MappingEngine, OutputEvent, VirtualGamepad, VirtualKeyboard, VirtualMouse};

use super::super::TraceState;
use super::devices::{create_keyboard, create_mouse, load_profile};
use super::sensor::SensorPipeline;

/// Everything a profile reload may rebuild, borrowed from the session loop.
pub(crate) struct LiveOutputs<'a> {
    pub(crate) mapper: &'a mut MappingEngine,
    pub(crate) gamepad: &'a mut VirtualGamepad,
    pub(crate) keyboard: &'a mut Option<VirtualKeyboard>,
    pub(crate) mouse: &'a mut Option<VirtualMouse>,
    pub(crate) pad: &'a mut crate::PadState,
    pub(crate) pipeline: &'a mut SensorPipeline,
    pub(crate) motion_enabled: bool,
}

pub(crate) fn reload_profile(
    outputs: &mut LiveOutputs<'_>,
    path: &Path,
    trace: &mut TraceState,
) -> Result<(), String> {
    let profile = load_profile(Some(path))?;
    let new_mapper = MappingEngine::new(profile)?;
    if super::output_stack::reload_needs_full_rebuild(
        outputs.mapper.profile(),
        new_mapper.profile(),
    ) {
        rebuild_output_stack(outputs, new_mapper, trace);
    } else {
        hot_apply_mappings(outputs, new_mapper, trace)?;
    }
    eprintln!(
        "ira-input: reloaded {} mapped inputs from {}",
        outputs
            .mapper
            .profile()
            .action_sets
            .iter()
            .map(|set| set.inputs.len())
            .sum::<usize>(),
        path.display()
    );
    Ok(())
}

/// The controller kind changed: release everything held on the outgoing
/// devices, then rebuild the pad (and twins, motion node, cemuhook stream)
/// for the new one. Games that re-enumerate see a fresh controller; games
/// that only bind at startup keep the old one until they are restarted —
/// the swap is best-effort by nature, not by refusal.
fn rebuild_output_stack(
    outputs: &mut LiveOutputs<'_>,
    new_mapper: MappingEngine,
    trace: &mut TraceState,
) {
    let stack = super::output_stack::build_virtual_stack(
        outputs.pipeline.motion_alive(),
        outputs.motion_enabled,
        new_mapper.profile(),
    );
    emit_outputs(
        outputs.mapper.reset(),
        OutputTargets {
            gamepad: outputs.gamepad,
            keyboard: outputs.keyboard.as_mut(),
            mouse: outputs.mouse.as_mut(),
            pad: outputs.pad,
        },
        trace,
    )
    .unwrap_or_else(|error| {
        eprintln!("ira-input: failed to release outputs before the controller swap: {error}")
    });
    eprintln!(
        "ira-input: controller kind changed; the game sees a new {:?} controller",
        new_mapper.profile().backend
    );
    *outputs.gamepad = stack.gamepad;
    *outputs.keyboard = stack.keyboard;
    *outputs.mouse = stack.mouse;
    outputs.pipeline.motion = stack.motion;
    outputs.pipeline.motion_device = stack.motion_device;
    outputs.pipeline.ds4_hid = stack.ds4_hid;
    outputs.pipeline.dualsense_hid = stack.dualsense_hid;
    outputs.pipeline.switch_pro_hid = stack.switch_pro_hid;
    outputs.pipeline.imu_hid = stack.imu_hid;
    *outputs.mapper = new_mapper;
}

/// Same controller kind: recreate only the devices whose needs changed and
/// hot-apply the new mappings, leaving the pad itself untouched.
fn hot_apply_mappings(
    outputs: &mut LiveOutputs<'_>,
    new_mapper: MappingEngine,
    trace: &mut TraceState,
) -> Result<(), String> {
    let keycodes_changed = outputs.mapper.profile().keyboard_keycodes()
        != new_mapper.profile().keyboard_keycodes();
    let mouse_changed =
        outputs.mapper.profile().uses_mouse() != new_mapper.profile().uses_mouse();
    let replacement_keyboard = if keycodes_changed {
        create_keyboard(new_mapper.profile().keyboard_keycodes())?
    } else {
        None
    };
    let replacement_mouse = if mouse_changed {
        create_mouse(new_mapper.profile().uses_mouse())?
    } else {
        None
    };
    emit_outputs(
        outputs.mapper.reset(),
        OutputTargets {
            gamepad: outputs.gamepad,
            keyboard: outputs.keyboard.as_mut(),
            mouse: outputs.mouse.as_mut(),
            pad: outputs.pad,
        },
        trace,
    )?;
    if keycodes_changed {
        *outputs.keyboard = replacement_keyboard;
    }
    if mouse_changed {
        *outputs.mouse = replacement_mouse;
    }
    *outputs.mapper = new_mapper;
    Ok(())
}

/// The virtual devices mapped events are written to, borrowed per loop pass.
/// `pad` shadows the gamepad outputs so the DSU stream can carry the full
/// controller state without re-deriving it from the mapping engine.
pub(crate) struct OutputTargets<'a> {
    pub(crate) gamepad: &'a mut VirtualGamepad,
    pub(crate) keyboard: Option<&'a mut VirtualKeyboard>,
    pub(crate) mouse: Option<&'a mut VirtualMouse>,
    pub(crate) pad: &'a mut crate::PadState,
}

pub(crate) fn stop_child(child: &mut Option<std::process::Child>) {
    if let Some(child) = child.as_mut() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub(crate) fn emit_mapped(
    mapper: &mut MappingEngine,
    targets: OutputTargets<'_>,
    event: InputEvent,
    trace: &mut TraceState,
) -> Result<(), String> {
    trace.record_input(event);
    emit_outputs(mapper.process(event), targets, trace)
}

pub(crate) fn emit_outputs(
    outputs: Vec<OutputEvent>,
    mut targets: OutputTargets<'_>,
    trace: &mut TraceState,
) -> Result<(), String> {
    for output in outputs {
        trace.record_output(&output);
        targets.pad.apply_output(&output);
        targets
            .gamepad
            .emit(&output)
            .map_err(|error| format!("failed to emit virtual input: {error}"))?;
        if let Some(keyboard) = targets.keyboard.as_deref_mut() {
            keyboard
                .emit(&output)
                .map_err(|error| format!("failed to emit virtual keyboard input: {error}"))?;
        }
        if let Some(mouse) = targets.mouse.as_deref_mut() {
            mouse
                .emit(&output)
                .map_err(|error| format!("failed to emit virtual mouse input: {error}"))?;
        }
    }
    targets
        .gamepad
        .flush()
        .map_err(|error| format!("failed to emit virtual input: {error}"))?;
    if let Some(mouse) = targets.mouse {
        mouse
            .flush()
            .map_err(|error| format!("failed to emit virtual mouse input: {error}"))?;
    }
    Ok(())
}
