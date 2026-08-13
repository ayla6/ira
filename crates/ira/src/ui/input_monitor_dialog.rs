use super::input_profile_options::{axis_label, button_label};
use ira_input::{
    ControllerRegistry, DeviceInfo, GamepadAxis, GamepadButton, InputEvent, InputProfile,
    InputSource, MappingEngine, OutputAction, OutputEvent, Sdl3SensorBackend,
};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Clone, Default)]
pub(super) struct MonitorValues {
    pub axes: [f32; 6],
    pub gyro: [f32; 3],
    pub buttons: Vec<GamepadButton>,
    pub output_axes: [f32; 6],
    pub output_buttons: Vec<GamepadButton>,
    pub active_outputs: Vec<String>,
    pub gyro_available: bool,
    pub controller_connected: bool,
    pub controller_disconnected: bool,
    pub controller_label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DeviceIdentity {
    vendor: u16,
    product: u16,
    name: String,
}

pub(super) fn start_monitor(
    stop: Arc<AtomicBool>,
    profile: Option<InputProfile>,
    registry: Arc<ControllerRegistry>,
) -> mpsc::Receiver<Result<MonitorValues, String>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = monitor_loop(&stop, &sender, profile, registry);
        if let Err(error) = result {
            let _ = sender.send(Err(error));
        }
    });
    receiver
}

fn monitor_loop(
    stop: &AtomicBool,
    sender: &mpsc::Sender<Result<MonitorValues, String>>,
    profile: Option<InputProfile>,
    registry: Arc<ControllerRegistry>,
) -> Result<(), String> {
    let mut engine = profile.map(MappingEngine::new).transpose()?;
    let mut values = MonitorValues::default();
    let mut gamepad = None;
    let mut sensor = None;
    let mut selected_identity = None;
    let mut generation = u64::MAX;
    let mut retry_at = std::time::Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let now = std::time::Instant::now();
        let registry_generation = registry.generation();
        if registry_generation != generation || (gamepad.is_none() && now >= retry_at) {
            let reopened = reconcile_device(
                &registry,
                &mut selected_identity,
                &mut gamepad,
                &mut sensor,
                &mut values,
            );
            if reopened {
                if let Some(engine) = engine.as_mut() {
                    engine.reset();
                }
            }
            retry_at = now + Duration::from_millis(200);
        }
        generation = registry_generation;

        let Some(current_gamepad) = gamepad.as_mut() else {
            values = MonitorValues::default();
            sender
                .send(Ok(values.clone()))
                .map_err(|_| "Input monitor closed".to_string())?;
            thread::sleep(Duration::from_millis(33));
            continue;
        };
        for event in current_gamepad.fetch_events()? {
            update_values(&mut values, event);
            if let Some(engine) = engine.as_mut() {
                let events = engine.process(event);
                update_mapped_values(&mut values, &events);
                update_outputs(
                    &mut values.active_outputs,
                    &events,
                    event.source,
                    engine.profile(),
                );
            }
        }
        let sensor_failed = if let Some(sensor_backend) = sensor.as_mut() {
            match sensor_backend.read(timestamp_us()) {
                Ok(Some(sample)) => {
                    values.gyro = [sample.x, sample.y, sample.z];
                    if let Some(engine) = engine.as_mut() {
                        for event in sample.input_events() {
                            let events = engine.process(event);
                            update_mapped_values(&mut values, &events);
                            update_outputs(
                                &mut values.active_outputs,
                                &events,
                                event.source,
                                engine.profile(),
                            );
                        }
                    }
                    false
                }
                Ok(None) => false,
                Err(error) => {
                    eprintln!("SDL3 gyro read failed; continuing without gyro: {error}");
                    true
                }
            }
        } else {
            false
        };
        let disconnected = !current_gamepad.is_connected();
        if disconnected {
            if let Some(engine) = engine.as_mut() {
                engine.reset();
            }
            gamepad = None;
            sensor = None;
            values = MonitorValues::default();
            values.controller_disconnected = true;
            retry_at = now + Duration::from_millis(200);
        }
        if sensor_failed {
            sensor = None;
            values.gyro_available = false;
        }
        values.controller_connected = gamepad.is_some();
        sender
            .send(Ok(values.clone()))
            .map_err(|_| "Input monitor closed".to_string())?;
        thread::sleep(Duration::from_millis(16));
    }
    Ok(())
}

fn reconcile_device(
    registry: &ControllerRegistry,
    selected_identity: &mut Option<DeviceIdentity>,
    gamepad: &mut Option<ira_input::PhysicalGamepad>,
    sensor: &mut Option<Sdl3SensorBackend>,
    values: &mut MonitorValues,
) -> bool {
    let selected = select_device(&registry.snapshot(), selected_identity.as_ref());
    let needs_reopen = should_reopen(
        gamepad
            .as_ref()
            .map(|current| current.info().path.as_path()),
        selected.as_ref(),
    );
    if !needs_reopen {
        return false;
    }
    *gamepad = None;
    *sensor = None;
    *values = MonitorValues::default();
    let Some(device) = selected else {
        return true;
    };
    *selected_identity = Some(device_identity(&device));
    values.controller_label = device_label(&device);
    match ira_input::PhysicalGamepad::open(&device.path, false) {
        Ok(opened) => {
            *sensor = match Sdl3SensorBackend::open(&device) {
                Ok(sensor) => sensor,
                Err(error) => {
                    eprintln!("SDL3 gyro unavailable: {error}");
                    None
                }
            };
            values.gyro_available = sensor.is_some();
            values.controller_connected = true;
            values.controller_disconnected = false;
            *gamepad = Some(opened);
        }
        Err(error) => eprintln!("failed to open controller: {error}"),
    }
    true
}

fn device_label(device: &DeviceInfo) -> String {
    format!(
        "{} | Linux reports {} ({:04x}:{:04x})",
        device.name,
        device.reported_input_mode().label(),
        device.vendor,
        device.product,
    )
}

fn device_identity(device: &DeviceInfo) -> DeviceIdentity {
    DeviceIdentity {
        vendor: device.vendor,
        product: device.product,
        name: device.name.clone(),
    }
}

fn select_device(
    devices: &[DeviceInfo],
    selected_identity: Option<&DeviceIdentity>,
) -> Option<DeviceInfo> {
    match selected_identity {
        Some(identity) => devices
            .iter()
            .filter(|device| device_identity(device) == *identity)
            .min_by(|left, right| left.path.cmp(&right.path))
            .cloned(),
        None => devices
            .iter()
            .min_by(|left, right| left.path.cmp(&right.path))
            .cloned(),
    }
}

fn should_reopen(current_path: Option<&Path>, selected: Option<&DeviceInfo>) -> bool {
    match (current_path, selected) {
        (Some(current), Some(next)) => current != next.path,
        (Some(_), None) | (None, Some(_)) => true,
        (None, None) => false,
    }
}

fn update_values(values: &mut MonitorValues, event: InputEvent) {
    match event.source {
        InputSource::Axis(axis) => values.axes[axis_index(axis)] = event.value,
        InputSource::Button(button) => {
            if event.value > 0.5 {
                if !values.buttons.contains(&button) {
                    values.buttons.push(button);
                }
            } else {
                values.buttons.retain(|candidate| *candidate != button);
            }
        }
        InputSource::AxisDirection { .. } => {}
        InputSource::Gyro(_) => {}
    }
}

fn update_mapped_values(values: &mut MonitorValues, events: &[OutputEvent]) {
    for event in events {
        match event {
            OutputEvent::GamepadButton { button, pressed } => {
                if *pressed {
                    if !values.output_buttons.contains(button) {
                        values.output_buttons.push(*button);
                    }
                } else {
                    values
                        .output_buttons
                        .retain(|candidate| candidate != button);
                }
            }
            OutputEvent::GamepadAxis { axis, value } => {
                values.output_axes[axis_index(*axis)] = *value;
            }
            _ => {}
        }
    }
}

fn update_outputs(
    outputs: &mut Vec<String>,
    events: &[OutputEvent],
    source: InputSource,
    profile: &InputProfile,
) {
    for event in events {
        let (output, active) = match &event {
            OutputEvent::GamepadButton { button, pressed } => {
                (OutputAction::GamepadButton(*button), *pressed)
            }
            OutputEvent::GamepadAxis { axis, value } => {
                (OutputAction::GamepadAxis(*axis), value.abs() > 0.01)
            }
            OutputEvent::Key { keycode, pressed } => {
                (OutputAction::Keyboard { keycode: *keycode }, *pressed)
            }
            OutputEvent::MouseButton { button, pressed } => {
                (OutputAction::MouseButton(*button), *pressed)
            }
            OutputEvent::MouseMotion { axis, value } => {
                (OutputAction::MouseAxis(*axis), value.abs() > 0.01)
            }
            OutputEvent::RecenterGyro => continue,
        };
        let relations = profile
            .bindings
            .iter()
            .filter(|binding| binding.output == output && source_matches(source, binding.source))
            .map(|binding| {
                format!(
                    "{} -> {}",
                    input_source_label(binding.source),
                    output_label(&output)
                )
            })
            .collect::<Vec<_>>();
        let labels = if relations.is_empty() {
            vec![output_label(&output)]
        } else {
            relations
        };
        for label in labels {
            if active {
                if !outputs.contains(&label) {
                    outputs.push(label);
                }
            } else {
                outputs.retain(|existing| existing != &label);
            }
        }
    }
}

fn source_matches(event: InputSource, binding: InputSource) -> bool {
    event == binding
        || matches!(
            (event, binding),
            (
                InputSource::Axis(event_axis),
                InputSource::AxisDirection { axis: binding_axis, .. }
            ) if event_axis == binding_axis
        )
}

fn input_source_label(source: InputSource) -> String {
    match source {
        InputSource::Button(button) => button_label(button),
        InputSource::Axis(axis) => axis_label(axis),
        InputSource::AxisDirection { axis, direction } => format!(
            "{} ({})",
            axis_label(axis),
            match direction {
                ira_input::AxisDirection::Negative => "-",
                ira_input::AxisDirection::Positive => "+",
            }
        ),
        InputSource::Gyro(axis) => gyro_label(axis).to_string(),
    }
}

fn output_label(output: &OutputAction) -> String {
    match output {
        OutputAction::GamepadButton(button) => {
            crate::tr!("Gamepad {}").replacen("{}", &button_label(*button), 1)
        }
        OutputAction::GamepadAxis(axis) => {
            crate::tr!("Gamepad {}").replacen("{}", &axis_label(*axis), 1)
        }
        OutputAction::Keyboard { keycode } => {
            crate::tr!("Keyboard key {keycode}").replace("{keycode}", &keycode.to_string())
        }
        OutputAction::MouseButton(button) => {
            crate::tr!("Mouse {button:?}").replace("{button:?}", &format!("{button:?}"))
        }
        OutputAction::MouseAxis(axis) => {
            crate::tr!("Mouse {axis:?}").replace("{axis:?}", &format!("{axis:?}"))
        }
        OutputAction::RecenterGyro => crate::tr!("Recenter gyro"),
    }
}

fn gyro_label(axis: ira_input::GyroAxis) -> &'static str {
    match axis {
        ira_input::GyroAxis::X => "Gyro X (Pitch)",
        ira_input::GyroAxis::Y => "Gyro Y (Yaw)",
        ira_input::GyroAxis::Z => "Gyro Z (Roll)",
    }
}

fn axis_index(axis: GamepadAxis) -> usize {
    match axis {
        GamepadAxis::LeftX => 0,
        GamepadAxis::LeftY => 1,
        GamepadAxis::RightX => 2,
        GamepadAxis::RightY => 3,
        GamepadAxis::LeftTrigger => 4,
        GamepadAxis::RightTrigger => 5,
    }
}

fn timestamp_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

#[cfg(test)]
mod tests {
    use super::{
        select_device, should_reopen, update_mapped_values, DeviceIdentity, MonitorValues,
    };
    use ira_input::{DeviceInfo, GamepadAxis, GamepadButton, OutputEvent};
    use std::path::PathBuf;

    fn device(path: &str, name: &str, vendor: u16, product: u16) -> DeviceInfo {
        DeviceInfo {
            path: PathBuf::from(path),
            name: name.to_string(),
            vendor,
            product,
            version: 0,
            has_evdev_gyro: false,
            supported_buttons: Vec::new(),
        }
    }

    #[test]
    fn test_select_device_prefers_identity_and_lowest_path() {
        let devices = [
            device("/dev/input/event9", "Pad", 1, 2),
            device("/dev/input/event2", "Other", 3, 4),
            device("/dev/input/event1", "Pad", 1, 2),
        ];
        let identity = DeviceIdentity {
            vendor: 1,
            product: 2,
            name: "Pad".to_string(),
        };
        assert_eq!(
            select_device(&devices, Some(&identity)).unwrap().path,
            PathBuf::from("/dev/input/event1")
        );
    }

    #[test]
    fn test_select_device_waits_when_identity_is_gone() {
        let devices = [device("/dev/input/event4", "Other", 3, 4)];
        let identity = DeviceIdentity {
            vendor: 1,
            product: 2,
            name: "Pad".to_string(),
        };
        assert!(select_device(&devices, Some(&identity)).is_none());
    }

    #[test]
    fn test_should_reopen_when_event_path_changes() {
        let next = device("/dev/input/event7", "Pad", 1, 2);
        assert!(should_reopen(
            Some(PathBuf::from("/dev/input/event3").as_path()),
            Some(&next)
        ));
        assert!(!should_reopen(Some(next.path.as_path()), Some(&next)));
        assert!(should_reopen(None, Some(&next)));
    }

    #[test]
    fn test_update_mapped_values_tracks_virtual_gamepad_output() {
        let mut values = MonitorValues::default();
        update_mapped_values(
            &mut values,
            &[
                OutputEvent::GamepadButton {
                    button: GamepadButton::Paddle4,
                    pressed: true,
                },
                OutputEvent::GamepadAxis {
                    axis: GamepadAxis::RightX,
                    value: 0.75,
                },
            ],
        );

        assert_eq!(values.output_buttons, vec![GamepadButton::Paddle4]);
        assert_eq!(values.output_axes[2], 0.75);
        assert!(values.buttons.is_empty());
        assert_eq!(values.axes, [0.0; 6]);

        update_mapped_values(
            &mut values,
            &[
                OutputEvent::GamepadButton {
                    button: GamepadButton::Paddle4,
                    pressed: false,
                },
                OutputEvent::GamepadAxis {
                    axis: GamepadAxis::RightX,
                    value: 0.0,
                },
            ],
        );
        assert!(values.output_buttons.is_empty());
        assert_eq!(values.output_axes, [0.0; 6]);
    }
}
