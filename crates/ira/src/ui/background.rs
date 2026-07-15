use gtk4::prelude::*;
use adw::prelude::*;
use crate::strings as S;
use super::state::SharedState;
use super::state::malloc_trim;
use super::window::build_window;
use super::sidebar::{select_row_silently, rebuild_sidebar, find_game_index};
use super::grid_view::show_grid_view;
use super::game_display::display_game;
use super::message_handler::clear_content;
use super::helpers::clear_children;

pub fn show_close_choice_dialog(state: &SharedState) {
    let window = state.borrow().window.clone();
    let dialog = adw::AlertDialog::new(
        Some(S::CLOSE_VIEWER),
        Some(S::CLOSE_VIEWER_BODY),
    );
    dialog.add_response("cancel", S::CANCEL);
    dialog.add_response("background", S::HIDE_TO_BACKGROUND);
    dialog.add_response("quit", S::QUIT);
    dialog.set_response_appearance("background", adw::ResponseAppearance::Suggested);
    dialog.set_response_appearance("quit", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("background"));
    dialog.set_close_response("cancel");

    let state_clone = state.clone();
    dialog.connect_response(None, move |_, resp| {
        match resp {
            "background" => hide_to_background(&state_clone),
            "quit" => {
                let app = state_clone.borrow().window.application()
                    .expect("no application")
                    .downcast::<adw::Application>()
                    .expect("not an adw Application");
                app.quit();
            }
            _ => {}
        }
    });
    dialog.present(Some(&window));
}

pub fn hide_to_background(state: &SharedState) {
    teardown_content(state);

    let app = state.borrow().window.application()
        .expect("no application")
        .downcast::<adw::Application>()
        .expect("not an adw Application");

    state.borrow().window.destroy();

    build_window(state, &app);
    let win = state.borrow().window.clone();
    win.set_visible(false);

    ira_images::clear_texture_cache();

    unsafe { malloc_trim(0); }

    glib::timeout_add_local(std::time::Duration::from_millis(300), || {
        unsafe { malloc_trim(0); }
        glib::ControlFlow::Break
    });
}

fn teardown_content(state: &SharedState) {
    let (content_box, grid_header, sidebar_store) = {
        let s = state.borrow();
        (s.content_box.clone(), s.grid_header.clone(), s.sidebar_store.clone())
    };

    clear_children(&content_box);
    clear_children(&grid_header);
    sidebar_store.remove_all();

    let mut s = state.borrow_mut();
    s.content_unloaded = true;
}

pub fn restore_content(state: &SharedState) {
    if !state.borrow().content_unloaded {
        return;
    }
    let selected_id = state.borrow().selected_id.clone();
    state.borrow_mut().content_unloaded = false;
    rebuild_sidebar(state);

    if selected_id.is_empty() {
        clear_content(state);
        select_row_silently(state, Some(0));
        show_grid_view(state);
        return;
    }

    let db_id: i64 = selected_id.parse().unwrap_or(0);
    let game = state.borrow().games.iter().find(|g| g.db_id == db_id).cloned();
    if let Some(game) = game {
        display_game(&game, state);

        let index = find_game_index(state, db_id);
        select_row_silently(state, index);
    } else {
        state.borrow_mut().selected_id.clear();
        clear_content(state);
        select_row_silently(state, Some(0));
        show_grid_view(state);
    }
}
