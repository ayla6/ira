use std::path::Path;

use crate::{InputEvent, MappingEngine, OutputEvent, VirtualGamepad, VirtualKeyboard, VirtualMouse};

use super::super::TraceState;
use super::devices::{create_keyboard, create_mouse, load_profile};

pub(crate) fn reload_profile(
    mapper: &mut MappingEngine,
    virtual_gamepad: &mut VirtualGamepad,
    keyboard: &mut Option<VirtualKeyboard>,
    mouse: &mut Option<VirtualMouse>,
    pad: &mut crate::PadState,
    path: &Path,
    trace: &mut TraceState,
) -> Result<(), String> {
    let profile = load_profile(Some(path))?;
    let new_mapper = MappingEngine::new(profile)?;
    let backend_changed = mapper.profile().backend != new_mapper.profile().backend;
    if backend_changed {
        return Err("virtual gamepad backend changes require restarting the game".to_string());
    }
    // Same class of problem as a backend change: the uhid controller is
    // created once at startup, and the engine routes gyro output for the
    // transport it was built with. Silently reloading a profile that flips
    // native motion leaves the gyro writing to no device at all, so refuse
    // instead — the running session keeps the previous profile.
    if mapper.profile().wants_native_controller() != new_mapper.profile().wants_native_controller()
    {
        return Err(
            "native motion transport changes require restarting the game".to_string(),
        );
    }
    let keycodes_changed =
        mapper.profile().keyboard_keycodes() != new_mapper.profile().keyboard_keycodes();
    let mouse_changed = mapper.profile().uses_mouse() != new_mapper.profile().uses_mouse();
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
        mapper.reset(),
        OutputTargets {
            gamepad: virtual_gamepad,
            keyboard: keyboard.as_mut(),
            mouse: mouse.as_mut(),
            pad,
        },
        trace,
    )?;
    if keycodes_changed {
        *keyboard = replacement_keyboard;
    }
    if mouse_changed {
        *mouse = replacement_mouse;
    }
    *mapper = new_mapper;
    eprintln!(
        "ira-input: reloaded {} mapped inputs from {}",
        mapper
            .profile()
            .action_sets
            .iter()
            .map(|set| set.inputs.len())
            .sum::<usize>(),
        path.display()
    );
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
