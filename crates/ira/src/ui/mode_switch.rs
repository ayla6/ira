//! Live switching between the desktop window and big picture: the window
//! is rebuilt in the other mode while the library, selection, search and
//! running games carry over in the shared state.

use super::background::{restore_content, teardown_content};
use super::state::SharedState;
use adw::prelude::*;
use std::rc::Rc;

/// The app action both views trigger (desktop menu, F11, the big picture
/// sort menu).
pub const TOGGLE_ACTION: &str = "toggle-big-picture";

/// Register the toggle action and its F11 accelerator on the application.
/// Actions outlive window rebuilds, so this runs once per process.
pub fn install(app: &adw::Application, state: &SharedState) {
    let action = gio::SimpleAction::new(TOGGLE_ACTION, None);
    let weak = Rc::downgrade(state);
    action.connect_activate(move |_, _| {
        if let Some(state) = weak.upgrade() {
            let target = !super::big_picture::is_big_picture();
            switch_to(&state, target);
        }
    });
    app.add_action(&action);
    app.set_accels_for_action(&format!("app.{TOGGLE_ACTION}"), &["F11"]);
}

/// Rebuild the window in big picture (`true`) or desktop (`false`) mode.
/// No-op when already there. Deferred to idle: callers run inside the
/// window's own handlers, which must not destroy it underneath them.
pub fn switch_to(state: &SharedState, big_picture: bool) {
    let weak = Rc::downgrade(state);
    glib::idle_add_local_once(move || {
        let Some(state) = weak.upgrade() else { return };
        if super::big_picture::is_big_picture() != big_picture {
            rebuild(&state, big_picture);
        }
    });
}

fn rebuild(state: &SharedState, big_picture: bool) {
    let (window, desktop_window, settings) = {
        let mut s = state.borrow_mut();
        // Dropping the shell stops its timers and controller reader.
        s.big_picture = None;
        (
            s.window.clone(),
            s.desktop_window,
            s.settings_data.as_ref().map(|data| data.window.clone()),
        )
    };
    if let Some(settings) = settings {
        settings.close();
    }
    let Some(app) = window
        .application()
        .and_then(|app| app.downcast::<adw::Application>().ok())
    else {
        eprintln!("Mode switch skipped: the window has no application");
        return;
    };
    teardown_content(state);
    ira_images::clear_texture_cache();
    super::big_picture::set_big_picture(big_picture);
    window.destroy();
    // Destroying can clear maximized; keep the live desktop geometry.
    state.borrow_mut().desktop_window = desktop_window;

    if big_picture {
        // The big picture view refreshes through the same library paths a
        // fresh start uses; the detached desktop widgets stand in unseen.
        state.borrow_mut().content_unloaded = false;
        super::big_picture::build_window(state, &app);
        show_running_game(state);
    } else {
        ira_widgets::font::pin_unhinted(false);
        super::window::build_window(state, &app);
        state.borrow().window.present();
        restore_content(state);
    }
    if let Some(tray) = state.borrow().tray.as_ref() {
        tray.set_big_picture(big_picture);
    }
}

/// A game launched from the desktop keeps running across the switch; the
/// big picture view covers itself with its running screen as it would
/// had the launch happened there.
fn show_running_game(state: &SharedState) {
    let name = {
        let s = state.borrow();
        let running = match s.running_games.lock() {
            Ok(running) => running.keys().copied().collect::<Vec<_>>(),
            Err(error) => {
                eprintln!("Running games lock poisoned: {error}");
                return;
            }
        };
        running.first().and_then(|db_id| {
            s.games
                .iter()
                .find(|game| game.db_id == *db_id && game.variant_id.is_none())
                .map(|game| game.name.clone())
        })
    };
    if let Some(name) = name {
        super::big_picture::show_running(state, &name);
    }
}
