use super::state::SharedState;
use adw::prelude::*;
use gtk4::gdk::RGBA;
use std::cell::RefCell;
use std::rc::Rc;

pub fn show_log_dialog(state: &SharedState, db_id: i64) {
    let game_name = {
        let s = state.borrow();
        s.games
            .iter()
            .find(|g| g.db_id == db_id)
            .map(|g| g.name.clone())
            .unwrap_or_else(|| format!("Game {}", db_id))
    };

    let window = adw::Window::new();
    window.set_title(Some(&crate::tr!("{} — Log").replacen("{}", &game_name, 1)));
    window.set_default_size(700, 500);
    window.set_transient_for(Some(&state.borrow().window));
    window.set_destroy_with_parent(true);

    let header = adw::HeaderBar::new();

    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some(&crate::tr!("Search log…")));
    search_entry.set_width_chars(28);
    header.pack_start(&search_entry);

    let title_label = gtk4::Label::new(Some(&crate::tr!("{} — Log").replacen("{}", &game_name, 1)));
    title_label.add_css_class("heading");
    header.set_title_widget(Some(&title_label));

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
    window.set_content(Some(&box_));

    window.present();

    let search_state: Rc<RefCell<SearchState>> = Rc::new(RefCell::new(SearchState::new()));
    if let Some(ref tag) = buffer.create_tag(Some("search-highlight"), &[]) {
        tag.set_background_rgba(Some(&highlight_color()));
    }

    {
        let buffer = buffer.clone();
        let search_state = search_state.clone();
        let text_view = text_view.clone();
        search_entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string();
            clear_highlights(&buffer);
            let mut st = search_state.borrow_mut();
            st.matches.clear();
            st.current = 0;
            if query.is_empty() {
                st.last_query.clear();
                return;
            }
            st.last_query = query.clone();
            find_matches(&buffer, &query, &mut st);
            if !st.matches.is_empty() {
                drop(st);
                jump_to_match(&buffer, &text_view, &search_state, 0);
            }
        });
    }

    {
        let search_state = search_state.clone();
        let buffer = buffer.clone();
        let text_view = text_view.clone();
        search_entry.connect_activate(move |_| {
            let st = search_state.borrow();
            if st.matches.is_empty() {
                return;
            }
            let next = (st.current + 1) % st.matches.len();
            drop(st);
            jump_to_match(&buffer, &text_view, &search_state, next);
        });
    }

    {
        let search_state = search_state.clone();
        let buffer = buffer.clone();
        let text_view = text_view.clone();
        let controller = gtk4::EventControllerKey::new();
        controller.connect_key_pressed(move |_, keyval, _, mods| {
            let is_g = matches!(keyval, gtk4::gdk::Key::g | gtk4::gdk::Key::G);
            if !is_g {
                return glib::Propagation::Proceed;
            }
            let st = search_state.borrow();
            if st.matches.is_empty() {
                return glib::Propagation::Proceed;
            }
            let len = st.matches.len();
            let next = if mods.contains(gtk4::gdk::ModifierType::SHIFT_MASK) {
                if st.current == 0 {
                    len - 1
                } else {
                    st.current - 1
                }
            } else {
                (st.current + 1) % len
            };
            drop(st);
            jump_to_match(&buffer, &text_view, &search_state, next);
            glib::Propagation::Stop
        });
        search_entry.add_controller(controller);
    }

    let is_running = state
        .borrow()
        .running_games
        .lock()
        .unwrap()
        .contains_key(&db_id);
    if is_running {
        let log = ira_launcher::wrapper::get_game_log(db_id);
        let window_weak = window.downgrade();
        let buf_clone = buffer;
        let tv_clone = text_view;
        let mark_clone = mark;
        let mut last_len: usize = {
            let lines = log.lock().unwrap();
            lines.len()
        };
        glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
            if window_weak.upgrade().is_none() {
                return glib::ControlFlow::Break;
            }
            let lines = log.lock().unwrap();
            if lines.len() > last_len {
                let new_text = lines[last_len..].join("\n");
                let current_text =
                    buf_clone.text(&buf_clone.start_iter(), &buf_clone.end_iter(), false);
                let separator = if current_text.is_empty() || current_text.ends_with('\n') {
                    ""
                } else {
                    "\n"
                };
                let insert_text = format!("{}{}", separator, new_text);
                let mut end = buf_clone.end_iter();
                buf_clone.insert(&mut end, &insert_text);
                buf_clone.move_mark(&mark_clone, &buf_clone.end_iter());
                tv_clone.scroll_to_mark(&mark_clone, 0.0, false, 0.0, 0.0);
                last_len = lines.len();

                let st = search_state.borrow();
                if !st.last_query.is_empty() {
                    drop(st);
                    let query = search_state.borrow().last_query.clone();
                    clear_highlights(&buf_clone);
                    let mut st = search_state.borrow_mut();
                    st.matches.clear();
                    st.current = 0;
                    find_matches(&buf_clone, &query, &mut st);
                }
            }
            glib::ControlFlow::Continue
        });
    }
}

struct SearchState {
    matches: Vec<gtk4::TextIter>,
    current: usize,
    last_query: String,
}

impl SearchState {
    fn new() -> Self {
        Self {
            matches: Vec::new(),
            current: 0,
            last_query: String::new(),
        }
    }
}

fn highlight_color() -> RGBA {
    RGBA::new(0.85, 0.65, 0.15, 0.7)
}

fn clear_highlights(buffer: &gtk4::TextBuffer) {
    let start = buffer.start_iter();
    let end = buffer.end_iter();
    buffer.remove_tag_by_name("search-highlight", &start, &end);
}

fn find_matches(buffer: &gtk4::TextBuffer, query: &str, st: &mut SearchState) {
    if query.is_empty() {
        return;
    }
    let mut iter = buffer.start_iter();
    let mut matches = Vec::new();
    let limit = 5000;
    while matches.len() < limit {
        let Some((match_start, match_end)) =
            iter.forward_search(query, gtk4::TextSearchFlags::CASE_INSENSITIVE, None)
        else {
            break;
        };
        buffer.apply_tag_by_name("search-highlight", &match_start, &match_end);
        matches.push(match_start);
        // Advance past the match; use forward_char instead of forward_cursor_position
        // to avoid "Char offset off the end of the line" crashes on long lines.
        let mut after = match_end;
        if !after.forward_char() {
            break;
        }
        iter = after;
    }
    st.matches = matches;
}

fn jump_to_match(
    buffer: &gtk4::TextBuffer,
    text_view: &gtk4::TextView,
    search_state: &Rc<RefCell<SearchState>>,
    index: usize,
) {
    let mut st = search_state.borrow_mut();
    if index >= st.matches.len() {
        return;
    }
    st.current = index;
    let match_iter = st.matches[index];
    drop(st);

    let mark = buffer.create_mark(None, &match_iter, false);
    text_view.scroll_to_mark(&mark, 0.0, true, 0.0, 0.5);
    buffer.delete_mark(&mark);
}
