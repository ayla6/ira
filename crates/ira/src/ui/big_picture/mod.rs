//! Big-picture (gamescope) mode: entry detection plus the big picture
//! shell. The detection is shared by main and the window builder so the
//! mode, its launch log line, and the UI changes it drives stay in
//! agreement; the shell's modules live beside it.

mod all_games;
mod game_menu;
mod groups;
mod keyboard;
mod home;
mod input;
mod marquee;
mod mouse;
mod running;
mod status;
mod view;

pub(super) use view::{build_window, refresh, BigPictureUi};

/// True when Ira was spawned under Gamescope's compositor, whose Wayland
/// socket is always `gamescope-0`.
pub fn running_in_gamescope() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok_and(|display| display.starts_with("gamescope"))
}

/// Big-picture mode: fullscreens the main window and drops the desktop
/// chrome. Opt in via `--big-picture`; automatic under Gamescope.
pub fn is_big_picture() -> bool {
    std::env::args().any(|arg| arg == "--big-picture") || running_in_gamescope()
}

/// Open the big-picture disc picker for a multi-disc game: one row per
/// disc, gamepad and keyboard navigable like every other menu.
pub(super) fn show_disc_picker(
    state: &crate::ui::state::SharedState,
    db_id: i64,
    variant_id: Option<i64>,
) {
    let (game_name, discs) = {
        let s = state.borrow();
        let name = s
            .games
            .iter()
            .find(|g| g.db_id == db_id)
            .map(|g| g.name.clone())
            .unwrap_or_default();
        (name, ira_db::get_discs(&s.db, db_id).unwrap_or_default())
    };
    if discs.len() <= 1 {
        return;
    }
    if let Some(big) = state.borrow().big_picture.clone() {
        big.game_menu.open(
            state,
            game_menu::MenuKind::Discs {
                db_id,
                variant_id,
                game_name,
                discs,
            },
        );
    }
}

/// Fade the big-picture shell into the black "game running" screen.
/// No-op outside big-picture mode.
pub(super) fn show_running(state: &crate::ui::state::SharedState, game_name: &str) {
    if let Some(big) = state.borrow().big_picture.clone() {
        big.show_running(game_name);
    }
}

/// Lift the black "game running" screen. No-op outside big-picture mode.
pub(super) fn hide_running(state: &crate::ui::state::SharedState) {
    if let Some(big) = state.borrow().big_picture.clone() {
        big.hide_running();
    }
}
