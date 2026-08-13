use super::css::CSS_DIM_LABEL;
use super::input_profile_options::{axis_label, button_label};
use adw::prelude::*;
use ira_input::{
    ControllerRegistry, DeviceInfo, GamepadAxis, GamepadButton, InputEvent, InputProfile,
    InputSource, MappingEngine, OutputAction, OutputEvent, Sdl3SensorBackend,
};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
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

#[derive(Clone, Copy)]
pub(super) struct MonitorColors {
    pub foreground: gtk4::gdk::RGBA,
    pub accent: gtk4::gdk::RGBA,
}

pub(super) fn show_input_monitor_dialog(parent: &gtk4::Window, registry: Arc<ControllerRegistry>) {
    let window = adw::Window::new();
    window.set_title(Some("Input Monitor"));
    window.set_default_size(800, 680);
    window.set_modal(false);
    window.set_transient_for(Some(parent));

    let status = gtk4::Label::new(Some("Starting controller monitor..."));
    configure_status_label(&status);

    let values = Rc::new(RefCell::new(MonitorValues::default()));
    let drawing = gtk4::DrawingArea::new();
    drawing.set_content_width(640);
    drawing.set_content_height(410);
    drawing.set_hexpand(true);
    drawing.set_vexpand(true);
    set_monitor_draw_func(&drawing, values.clone());

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    content.append(&status);
    content.append(&drawing);

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&gtk4::Label::new(Some("Input Monitor"))));
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));
    window.set_content(Some(&toolbar));

    let stop = Arc::new(AtomicBool::new(false));
    let receiver = start_monitor(stop.clone(), None, registry);
    poll_monitor(receiver, values, drawing, status);

    let stop_for_close = stop.clone();
    window.connect_close_request(move |_| {
        stop_for_close.store(true, Ordering::Relaxed);
        glib::Propagation::Proceed
    });
    window.present();
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
        OutputAction::GamepadButton(button) => format!("Gamepad {}", button_label(*button)),
        OutputAction::GamepadAxis(axis) => format!("Gamepad {}", axis_label(*axis)),
        OutputAction::Keyboard { keycode } => format!("Keyboard key {keycode}"),
        OutputAction::MouseButton(button) => format!("Mouse {button:?}"),
        OutputAction::MouseAxis(axis) => format!("Mouse {axis:?}"),
        OutputAction::RecenterGyro => "Recenter gyro".to_string(),
    }
}

fn gyro_label(axis: ira_input::GyroAxis) -> &'static str {
    match axis {
        ira_input::GyroAxis::X => "Gyro X (Pitch)",
        ira_input::GyroAxis::Y => "Gyro Y (Yaw)",
        ira_input::GyroAxis::Z => "Gyro Z (Roll)",
    }
}

fn configure_status_label(label: &gtk4::Label) {
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_single_line_mode(true);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    label.set_height_request(24);
    label.add_css_class(CSS_DIM_LABEL);
}

fn poll_monitor(
    receiver: mpsc::Receiver<Result<MonitorValues, String>>,
    values: Rc<RefCell<MonitorValues>>,
    drawing: gtk4::DrawingArea,
    status: gtk4::Label,
) {
    glib::timeout_add_local(Duration::from_millis(33), move || {
        let mut latest = None;
        loop {
            match receiver.try_recv() {
                Ok(update) => latest = Some(update),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return glib::ControlFlow::Break,
            }
        }
        if let Some(update) = latest {
            match update {
                Ok(update) => {
                    let pressed = update
                        .buttons
                        .iter()
                        .map(|button| button_label(*button))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let controller_label = if update.controller_label.is_empty() {
                        "controller"
                    } else {
                        &update.controller_label
                    };
                    let text = if !update.controller_connected {
                        if update.controller_disconnected {
                            "Controller disconnected".to_string()
                        } else {
                            "Waiting for controller".to_string()
                        }
                    } else if pressed.is_empty() {
                        if update.gyro_available {
                            format!("Monitoring {controller_label}")
                        } else {
                            format!("Monitoring {controller_label} | Gyro unavailable")
                        }
                    } else {
                        format!("Monitoring {controller_label} | Pressed: {pressed}")
                    };
                    status.set_text(&text);
                    *values.borrow_mut() = update;
                }
                Err(error) => {
                    status.set_text(&error);
                    return glib::ControlFlow::Break;
                }
            }
        }
        drawing.queue_draw();
        glib::ControlFlow::Continue
    });
}

fn set_monitor_draw_func(drawing: &gtk4::DrawingArea, values: Rc<RefCell<MonitorValues>>) {
    drawing.set_draw_func(move |area, cr, width, height| {
        let values = values.borrow();
        let width = width as f64;
        let height = height as f64;
        let colors = MonitorColors {
            foreground: area.color(),
            accent: adw::StyleManager::default().accent_color().to_rgba(),
        };

        let pad_size = (width.min(640.0) / 2.0 - 48.0).min(180.0);
        draw_stick(
            cr,
            (24.0, 32.0),
            pad_size,
            (values.axes[0], values.axes[1]),
            "Left stick",
            colors,
        );
        draw_stick(
            cr,
            (width / 2.0 + 24.0, 32.0),
            pad_size,
            (values.axes[2], values.axes[3]),
            "Right stick",
            colors,
        );

        let bars_y = 250.0;
        draw_bar(
            cr,
            24.0,
            bars_y,
            width - 48.0,
            values.axes[4],
            "Left trigger",
            colors,
        );
        draw_bar(
            cr,
            24.0,
            bars_y + 42.0,
            width - 48.0,
            values.axes[5],
            "Right trigger",
            colors,
        );
        draw_bar(
            cr,
            24.0,
            bars_y + 100.0,
            width - 48.0,
            values.gyro[0] / 10.0,
            "Gyro X (Pitch)",
            colors,
        );
        draw_bar(
            cr,
            24.0,
            bars_y + 142.0,
            width - 48.0,
            values.gyro[1] / 10.0,
            "Gyro Y (Yaw)",
            colors,
        );
        draw_bar(
            cr,
            24.0,
            bars_y + 184.0,
            width - 48.0,
            values.gyro[2] / 10.0,
            "Gyro Z (Roll)",
            colors,
        );
        let _ = height;
    });
}

fn draw_stick(
    cr: &gtk4::cairo::Context,
    position: (f64, f64),
    size: f64,
    value: (f32, f32),
    label: &str,
    colors: MonitorColors,
) {
    let (x, y) = position;
    let (x_value, y_value) = value;
    set_cairo_color(cr, colors.foreground.with_alpha(0.08));
    cr.rectangle(x, y, size, size);
    let _ = cr.fill();
    set_cairo_color(cr, colors.foreground.with_alpha(0.28));
    cr.move_to(x + size / 2.0, y);
    cr.line_to(x + size / 2.0, y + size);
    cr.move_to(x, y + size / 2.0);
    cr.line_to(x + size, y + size / 2.0);
    let _ = cr.stroke();
    set_cairo_color(cr, colors.accent);
    cr.arc(
        x + (x_value.clamp(-1.0, 1.0) as f64 + 1.0) * size / 2.0,
        // Gamepad positive Y is up; Cairo's positive Y points down.
        y + (1.0 - y_value.clamp(-1.0, 1.0) as f64) * size / 2.0,
        9.0,
        0.0,
        std::f64::consts::TAU,
    );
    let _ = cr.fill();
    set_cairo_color(cr, colors.foreground);
    cr.select_font_face(
        "Sans",
        gtk4::cairo::FontSlant::Normal,
        gtk4::cairo::FontWeight::Normal,
    );
    cr.set_font_size(13.0);
    cr.move_to(x, y + size + 20.0);
    let _ = cr.show_text(&format!("{label}: {x_value:.2}, {y_value:.2}"));
}

fn draw_bar(
    cr: &gtk4::cairo::Context,
    x: f64,
    y: f64,
    width: f64,
    value: f32,
    label: &str,
    colors: MonitorColors,
) {
    let value = value.clamp(-1.0, 1.0) as f64;
    set_cairo_color(cr, colors.foreground.with_alpha(0.08));
    cr.rectangle(x, y, width, 20.0);
    let _ = cr.fill();
    set_cairo_color(cr, colors.accent);
    if value >= 0.0 {
        cr.rectangle(x + width / 2.0, y, value * width / 2.0, 20.0);
    } else {
        cr.rectangle(
            x + (1.0 + value) * width / 2.0,
            y,
            -value * width / 2.0,
            20.0,
        );
    }
    let _ = cr.fill();
    set_cairo_color(cr, colors.foreground);
    cr.set_font_size(12.0);
    cr.move_to(x, y - 5.0);
    let _ = cr.show_text(&format!("{label}: {value:.2}"));
}

pub(super) fn set_cairo_color(cr: &gtk4::cairo::Context, color: gtk4::gdk::RGBA) {
    cr.set_source_rgba(
        color.red() as f64,
        color.green() as f64,
        color.blue() as f64,
        color.alpha() as f64,
    );
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
