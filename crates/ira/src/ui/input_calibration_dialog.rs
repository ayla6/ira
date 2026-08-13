use super::css::{CSS_ERROR, CSS_SUGGESTED_ACTION};
use adw::prelude::*;
use ira_input::{GyroCalibration, GyroSample};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

type Completion = Box<dyn FnOnce(Result<GyroCalibration, String>) + 'static>;
const CALIBRATION_DURATION: Duration = Duration::from_millis(700);

enum Update {
    Countdown(u8),
    Progress(f64),
    Complete(Result<GyroCalibration, String>),
}

#[derive(Clone)]
struct CalibrationWidgets {
    window: adw::Window,
    status: gtk4::Label,
    progress: gtk4::ProgressBar,
    start: gtk4::Button,
}

pub(super) fn show_input_calibration_dialog(
    parent: &gtk4::Window,
    registry: Arc<ira_input::ControllerRegistry>,
    device: ira_input::DeviceInfo,
    on_complete: impl FnOnce(Result<GyroCalibration, String>) + 'static,
) {
    let window = adw::Window::new();
    window.set_default_size(460, 190);
    window.set_resizable(false);
    window.set_transient_for(Some(parent));
    window.set_modal(true);

    let status = gtk4::Label::new(Some(&crate::tr!(
        "Place the controller flat on a stable surface. Keep it still during calibration."
    )));
    status.set_wrap(true);
    status.set_xalign(0.0);
    let progress = gtk4::ProgressBar::new();
    progress.set_show_text(true);
    progress.set_text(Some(&crate::tr!("Ready")));
    let start = gtk4::Button::with_label(&crate::tr!("Start calibration"));
    start.add_css_class(CSS_SUGGESTED_ACTION);
    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    actions.set_halign(gtk4::Align::End);
    actions.append(&start);
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.append(&status);
    content.append(&progress);
    content.append(&actions);
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&gtk4::Label::new(Some(&crate::tr!("Calibrate Gyro")))));
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));
    window.set_content(Some(&toolbar));

    let cancelled = Arc::new(AtomicBool::new(false));
    let cancelled_for_request = cancelled.clone();
    window.connect_close_request(move |_| {
        cancelled_for_request.store(true, Ordering::Release);
        glib::Propagation::Proceed
    });
    connect_start(
        CalibrationWidgets {
            window: window.clone(),
            status,
            progress,
            start,
        },
        registry,
        device,
        cancelled,
        Box::new(on_complete),
    );
    window.present();
}

fn connect_start(
    widgets: CalibrationWidgets,
    registry: Arc<ira_input::ControllerRegistry>,
    device: ira_input::DeviceInfo,
    cancelled: Arc<AtomicBool>,
    on_complete: Completion,
) {
    let callback = Rc::new(RefCell::new(Some(on_complete)));
    let completed = Rc::new(Cell::new(false));
    widgets.start.clone().connect_clicked(move |_| {
        if completed.get() {
            widgets.window.close();
            return;
        }
        widgets.start.set_sensitive(false);
        widgets.start.set_label(&crate::tr!("Calibrating..."));
        widgets.status.remove_css_class(CSS_ERROR);
        widgets
            .status
            .set_text(&crate::tr!("Get ready... calibration starts in 1 second."));
        widgets.progress.set_fraction(0.0);
        widgets.progress.set_text(Some(&crate::tr!("Preparing")));
        let (sender, receiver) = mpsc::channel();
        let worker_cancelled = cancelled.clone();
        let registry = registry.clone();
        let device = device.clone();
        std::thread::spawn(move || {
            let result = collect_calibration(&registry, &device, &worker_cancelled, &sender);
            let _ = sender.send(Update::Complete(result));
        });
        poll_updates(
            receiver,
            widgets.status.clone(),
            widgets.progress.clone(),
            widgets.start.clone(),
            cancelled.clone(),
            callback.clone(),
            completed.clone(),
        );
    });
}

fn poll_updates(
    receiver: mpsc::Receiver<Update>,
    status: gtk4::Label,
    progress: gtk4::ProgressBar,
    start: gtk4::Button,
    cancelled: Arc<AtomicBool>,
    callback: Rc<RefCell<Option<Completion>>>,
    completed: Rc<Cell<bool>>,
) {
    let receiver = Rc::new(RefCell::new(receiver));
    glib::timeout_add_local(Duration::from_millis(80), move || {
        if cancelled.load(Ordering::Acquire) {
            return glib::ControlFlow::Break;
        }
        let mut latest = None;
        loop {
            match receiver.borrow_mut().try_recv() {
                Ok(update) => latest = Some(update),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        }
        match latest {
            Some(Update::Countdown(seconds)) => {
                progress.set_fraction(0.0);
                progress.set_text(Some(&crate::tr!("Preparing")));
                status.set_text(
                    &crate::tr!("Release the controller. Calibration starts in {seconds}...")
                        .replace("{seconds}", &seconds.to_string()),
                );
                glib::ControlFlow::Continue
            }
            Some(Update::Progress(value)) => {
                progress.set_fraction(value);
                progress.set_text(Some(&crate::tr!("Calibrating")));
                status.set_text(&crate::tr!(
                    "Keep the controller still while samples are collected..."
                ));
                glib::ControlFlow::Continue
            }
            Some(Update::Complete(result)) => {
                match &result {
                    Ok(value) => {
                        progress.set_fraction(1.0);
                        progress.set_text(Some(&crate::tr!("Complete")));
                        status.set_text(&format_calibration(*value));
                        start.set_label(&crate::tr!("Done"));
                        completed.set(true);
                        if let Some(callback) = callback.borrow_mut().take() {
                            callback(Ok(*value));
                        }
                    }
                    Err(error) => {
                        progress.set_text(Some(&crate::tr!("Failed")));
                        set_error(&status, error);
                        start.set_label(&crate::tr!("Retry"));
                    }
                }
                start.set_sensitive(true);
                glib::ControlFlow::Break
            }
            None => glib::ControlFlow::Continue,
        }
    });
}

fn collect_calibration(
    registry: &ira_input::ControllerRegistry,
    identity: &ira_input::DeviceInfo,
    cancelled: &AtomicBool,
    sender: &mpsc::Sender<Update>,
) -> Result<GyroCalibration, String> {
    let device = registry
        .snapshot()
        .into_iter()
        .find(|device| same_device(device, identity))
        .ok_or_else(|| format!("Controller '{}' is no longer connected", identity.name))?;
    let mut sensor = ira_input::Sdl3SensorBackend::open(&device)?
        .ok_or_else(|| "No SDL3 gyro is available for this controller".to_string())?;
    if cancelled.load(Ordering::Acquire) {
        return Err("Calibration cancelled".to_string());
    }
    let _ = sender.send(Update::Countdown(1));
    std::thread::sleep(Duration::from_secs(1));
    let deadline = Instant::now() + CALIBRATION_DURATION;
    let mut samples = Vec::<GyroSample>::new();
    while Instant::now() < deadline {
        if cancelled.load(Ordering::Acquire) {
            return Err("Calibration cancelled".to_string());
        }
        if let Some(sample) = sensor.read(0)? {
            samples.push(sample);
        }
        let fraction = 0.1
            + 0.9
                * (1.0
                    - deadline
                        .saturating_duration_since(Instant::now())
                        .as_secs_f64()
                        / CALIBRATION_DURATION.as_secs_f64());
        let _ = sender.send(Update::Progress(fraction.min(1.0)));
        std::thread::sleep(Duration::from_millis(4));
    }
    GyroCalibration::from_samples(&samples)
        .ok_or_else(|| "No gyro samples were received during calibration".to_string())
}

fn same_device(left: &ira_input::DeviceInfo, right: &ira_input::DeviceInfo) -> bool {
    left.vendor == right.vendor && left.product == right.product && left.name == right.name
}

fn format_calibration(calibration: GyroCalibration) -> String {
    format!(
        "Calibration complete. Bias: X={:.4}, Y={:.4}, Z={:.4}",
        calibration.x, calibration.y, calibration.z
    )
}

fn set_error(status: &gtk4::Label, message: &str) {
    status.set_text(message);
    status.add_css_class(CSS_ERROR);
}
