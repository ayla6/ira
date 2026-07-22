use gtk4::prelude::*;
use adw::prelude::*;
use std::sync::Arc;
use std::sync::Mutex;
use super::state::SharedState;

pub fn show_log_dialog(state: &SharedState, db_id: i64) {
    let (window, save_dir, game_name) = {
        let s = state.borrow();
        let name = s.games.iter()
            .find(|g| g.db_id == db_id)
            .map(|g| g.name.clone())
            .unwrap_or_else(|| format!("Game {}", db_id));
        (s.window.clone(), s.save_dir.clone(), name)
    };

    let log_path = ira_launcher::wrapper::game_log_path(&save_dir, db_id);

    let dialog = adw::Window::new();
    dialog.set_title(Some(&format!("{} — Log", game_name)));
    dialog.set_default_size(700, 500);
    dialog.set_modal(true);
    dialog.set_transient_for(Some(&window));

    let header = adw::HeaderBar::new();

    let toast_overlay = adw::ToastOverlay::new();

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);

    let text_view = gtk4::TextView::new();
    text_view.set_editable(false);
    text_view.set_monospace(true);
    text_view.set_wrap_mode(gtk4::WrapMode::WordChar);
    text_view.set_top_margin(8);
    text_view.set_bottom_margin(8);
    text_view.set_left_margin(8);
    text_view.set_right_margin(8);

    let buffer = text_view.buffer();

    let shared_initial = Arc::new(Mutex::new(None::<String>));
    let shared_c = shared_initial.clone();
    let log_path_c = log_path.clone();
    std::thread::spawn(move || {
        let content = std::fs::read_to_string(&log_path_c)
            .unwrap_or_else(|_| format!("No log file found at:\n{}\n\nThe log will be created when the game is launched.", log_path_c));
        *shared_c.lock().unwrap() = Some(content.replace('\0', "\u{FFFD}"));
    });
    let buffer_c = buffer.clone();
    glib::source::idle_add_local_full(glib::Priority::LOW, move || {
        if let Some(content) = shared_initial.lock().unwrap().take() {
            buffer_c.set_text(&content);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });

    let mark = buffer.create_mark(None, &buffer.end_iter(), false);

    scrolled.set_child(Some(&text_view));
    toast_overlay.set_child(Some(&scrolled));

    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    box_.append(&header);
    box_.append(&toast_overlay);
    dialog.set_content(Some(&box_));

    dialog.present();

    let tv_clone = text_view.clone();
    let buf_clone = buffer.clone();
    let mark_clone = mark.clone();
    let log_path_clone = log_path.clone();
    let is_running = state.borrow().running_games.lock().unwrap().contains_key(&db_id);
    if is_running {
        let shared = Arc::new(Mutex::new(None::<String>));
        glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
            let shared_c = shared.clone();
            let lpc = log_path_clone.clone();
            std::thread::spawn(move || {
                if let Ok(content) = std::fs::read_to_string(&lpc) {
                    *shared_c.lock().unwrap() = Some(content);
                }
            });
            if let Some(content) = shared.try_lock().ok().and_then(|mut g| g.take()) {
                let current = buf_clone.text(&buf_clone.start_iter(), &buf_clone.end_iter(), false);
                if current != content {
                    buf_clone.set_text(&content.replace('\0', "\u{FFFD}"));
                    buf_clone.move_mark(&mark_clone, &buf_clone.end_iter());
                    tv_clone.scroll_to_mark(&mark_clone, 0.0, false, 0.0, 0.0);
                }
            }
            glib::ControlFlow::Continue
        });
    }
}
