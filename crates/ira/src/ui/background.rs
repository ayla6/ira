use super::game_display::display_game;
use super::grid_view::show_grid_view;
use super::helpers::clear_children;
use super::message_helpers::clear_content;
use super::sidebar::{find_game_index, rebuild_sidebar, select_row_silently};
use super::state::malloc_trim;
use super::state::SharedState;
use super::window::build_window;
use adw::prelude::*;

/// The running application for window operations: downcast once here
/// instead of at every hide/quit site.
fn app_of(state: &SharedState) -> adw::Application {
    state
        .borrow()
        .window
        .application()
        .expect("no application")
        .downcast::<adw::Application>()
        .expect("not an adw Application")
}

pub fn hide_to_background(state: &SharedState) {
    let window = state.borrow().window.clone();
    window.set_visible(false);
    let weak = std::rc::Rc::downgrade(state);
    // Let GTK hide the window before freeing the library's widgets and textures.
    glib::idle_add_local_once(move || {
        let Some(state) = weak.upgrade() else { return };
        if window.is_visible() || state.borrow().window != window {
            return;
        }
        release_background_content(&state);
    });
}

fn release_background_content(state: &SharedState) {
    teardown_content(state);
    ira_images::clear_texture_cache();

    let app = app_of(state);

    let (window, desktop_window) = {
        let state = state.borrow();
        (state.window.clone(), state.desktop_window)
    };
    window.destroy();
    state.borrow_mut().desktop_window = desktop_window;

    build_window(state, &app);
    let win = state.borrow().window.clone();
    win.set_visible(false);

    unsafe {
        malloc_trim(0);
    }

    glib::timeout_add_local(std::time::Duration::from_millis(300), || {
        unsafe {
            malloc_trim(0);
        }
        glib::ControlFlow::Break
    });
}

fn teardown_content(state: &SharedState) {
    let (content_box, grid_header, sidebar_store) = {
        let s = state.borrow();
        (
            s.content_box.clone(),
            s.grid_header.clone(),
            s.sidebar_store.clone(),
        )
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
        select_row_silently(state, None);
        show_grid_view(state);
        return;
    }

    let db_id = ira_models::parse_db_id(&selected_id);
    let variant_id = selected_id
        .split("-v")
        .nth(1)
        .and_then(|s| s.parse::<i64>().ok());
    let game = state.borrow().find_game(db_id, variant_id);
    if let Some(game) = game {
        display_game(&game, state);

        let index = find_game_index(state, db_id, variant_id);
        select_row_silently(state, index);
    } else {
        state.borrow_mut().selected_id.clear();
        clear_content(state);
        select_row_silently(state, None);
        show_grid_view(state);
    }
}
