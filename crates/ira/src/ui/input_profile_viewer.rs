use super::css::{CSS_DIM_LABEL, CSS_FLAT, CSS_VIEWER_CANVAS, CSS_VIEWER_CARD};
use super::input_monitor_dialog::{start_monitor, MonitorValues};
use super::input_profile_options::button_label;
use super::input_profile_store::read_profile;
use adw::prelude::*;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

pub(super) fn show_input_profile_viewer(
    parent: &gtk4::Window,
    path: &Path,
    registry: Arc<ira_input::ControllerRegistry>,
) {
    let window = adw::Window::new();
    window.set_default_size(900, 760);
    window.set_transient_for(Some(parent));
    window.set_modal(true);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.set_margin_top(16);
    content.set_margin_bottom(16);

    let stop = Arc::new(AtomicBool::new(false));
    let mut title = "Live layout preview".to_string();
    match read_profile(path) {
        Ok(profile) => {
            if !profile.name.trim().is_empty() {
                title = profile.name.clone();
            }
            let receiver = start_monitor(stop.clone(), Some(profile), registry);
            build_live_viewer(&window, &content, receiver);
        }
        Err(error) => {
            let error_label = gtk4::Label::new(Some(&error));
            error_label.set_wrap(true);
            error_label.set_xalign(0.0);
            content.append(&error_label);
        }
    }

    let close = gtk4::Button::with_label("Close");
    close.add_css_class(CSS_FLAT);
    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    actions.set_halign(gtk4::Align::End);
    actions.append(&close);
    content.append(&actions);
    let window_for_close = window.clone();
    close.connect_clicked(move |_| window_for_close.close());

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&gtk4::Label::new(Some(&title))));
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));
    window.set_content(Some(&toolbar));

    let stop_for_close = stop.clone();
    window.connect_close_request(move |_| {
        stop_for_close.store(true, Ordering::Relaxed);
        glib::Propagation::Proceed
    });
    window.present();
}

fn build_live_viewer(
    window: &adw::Window,
    content: &gtk4::Box,
    receiver: mpsc::Receiver<Result<MonitorValues, String>>,
) {
    let values = Rc::new(RefCell::new(MonitorValues::default()));
    let drawing = gtk4::DrawingArea::new();
    drawing.set_content_width(720);
    drawing.set_content_height(440);
    drawing.set_hexpand(true);
    drawing.set_vexpand(true);
    super::input_profile_gamepad::set_draw_func(&drawing, values.clone());
    let canvas = gtk4::Frame::new(None);
    canvas.add_css_class(CSS_VIEWER_CANVAS);
    canvas.set_child(Some(&drawing));
    canvas.set_hexpand(true);
    canvas.set_vexpand(true);

    let status = gtk4::Label::new(Some("Starting controller monitor..."));
    configure_status_label(&status, true);
    let status_card = viewer_card("Status", &status);

    let outputs = gtk4::Label::new(Some("No mapped output active"));
    configure_status_label(&outputs, false);
    let outputs_card = viewer_card("Active mappings", &outputs);

    content.append(&status_card);
    content.append(&canvas);
    content.append(&outputs_card);

    poll_viewer(receiver, values, drawing, status, outputs, window.clone());
}

fn viewer_card(title: &str, content: &gtk4::Label) -> gtk4::Box {
    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    card.add_css_class(CSS_VIEWER_CARD);
    let heading = gtk4::Label::new(Some(title));
    heading.set_xalign(0.0);
    card.append(&heading);
    card.append(content);
    card
}

fn poll_viewer(
    receiver: mpsc::Receiver<Result<MonitorValues, String>>,
    values: Rc<RefCell<MonitorValues>>,
    drawing: gtk4::DrawingArea,
    status: gtk4::Label,
    outputs: gtk4::Label,
    window: adw::Window,
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
                        .output_buttons
                        .iter()
                        .map(|button| button_label(*button))
                        .collect::<Vec<_>>()
                        .join(", ");
                    status.set_text(&viewer_status(&update, &pressed));
                    let outputs_text = update.active_outputs.join("  •  ");
                    outputs.set_text(if outputs_text.is_empty() {
                        "No mapped output active"
                    } else {
                        &outputs_text
                    });
                    *values.borrow_mut() = update;
                    drawing.queue_draw();
                }
                Err(error) => {
                    status.set_text(&error);
                    return glib::ControlFlow::Break;
                }
            }
        }
        if !window.is_visible() {
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}

fn viewer_status(update: &MonitorValues, pressed: &str) -> String {
    if !update.controller_connected {
        return if update.controller_disconnected {
            "Controller disconnected".to_string()
        } else {
            "Waiting for controller".to_string()
        };
    }
    if !pressed.is_empty() {
        return format!("Mapped virtual output | Pressed: {pressed}");
    }
    if update.gyro_available {
        "Monitoring mapped virtual output".to_string()
    } else {
        "Monitoring mapped virtual output | Gyro unavailable".to_string()
    }
}

fn configure_status_label(label: &gtk4::Label, dim: bool) {
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_single_line_mode(true);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    label.set_height_request(24);
    if dim {
        label.add_css_class(CSS_DIM_LABEL);
    }
}
