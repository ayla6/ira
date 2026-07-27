use gtk4::prelude::*;
use adw::prelude::*;
use super::state::SharedState;

pub fn show_log_dialog(state: &SharedState, db_id: i64) {
    let (window, game_name) = {
        let s = state.borrow();
        let name = s.games.iter()
            .find(|g| g.db_id == db_id)
            .map(|g| g.name.clone())
            .unwrap_or_else(|| format!("Game {}", db_id));
        (s.window.clone(), name)
    };

    let dialog = adw::Window::new();
    dialog.set_title(Some(&format!("{} — Log", game_name)));
    dialog.set_default_size(700, 500);
    dialog.set_modal(true);
    dialog.set_transient_for(Some(&window));

    let header = adw::HeaderBar::new();

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

    // Load initial content from the in-memory log buffer.
    let log = ira_launcher::wrapper::get_game_log(db_id);
    let initial: String = {
        let lines = log.lock().unwrap();
        if lines.is_empty() {
            "No log output yet. The log will populate when the game is launched.".to_string()
        } else {
            lines.join("\n")
        }
    };
    buffer.set_text(&initial.replace('\0', "\u{FFFD}"));

    let mark = buffer.create_mark(None, &buffer.end_iter(), false);

    scrolled.set_child(Some(&text_view));

    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    box_.append(&header);
    box_.append(&scrolled);
    dialog.set_content(Some(&box_));

    dialog.present();

    // If the game is running, poll the in-memory buffer for new lines.
    let is_running = state.borrow().running_games.lock().unwrap().contains_key(&db_id);
    if is_running {
        let log = ira_launcher::wrapper::get_game_log(db_id);
        let buf_clone = buffer.clone();
        let tv_clone = text_view.clone();
        let mark_clone = mark.clone();
        let mut last_len: usize = {
            let lines = log.lock().unwrap();
            lines.len()
        };
        glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
            let lines = log.lock().unwrap();
            if lines.len() > last_len {
                // Append only new lines
                let new_text = lines[last_len..].join("\n");
                let current_text = buf_clone.text(&buf_clone.start_iter(), &buf_clone.end_iter(), false);
                let separator = if current_text.is_empty() || current_text.ends_with('\n') { "" } else { "\n" };
                let insert_text = format!("{}{}", separator, new_text);
                let mut end = buf_clone.end_iter();
                buf_clone.insert(&mut end, &insert_text);
                buf_clone.move_mark(&mark_clone, &buf_clone.end_iter());
                tv_clone.scroll_to_mark(&mark_clone, 0.0, false, 0.0, 0.0);
                last_len = lines.len();
            }
            glib::ControlFlow::Continue
        });
    }
}
