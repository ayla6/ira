//! Big-picture (gamescope) mode: entry detection plus the big picture
//! shell. The detection is shared by main and the window builder so the
//! mode, its launch log line, and the UI changes it drives stay in
//! agreement; the shell's modules live beside it.

mod all_games;
mod disc_picker;
mod disc_tile;
mod game_menu;
mod groups;
mod home;
mod icon_center;
mod icon_row;
mod input;
mod keyboard;
mod marquee;
mod mouse;
mod running;
mod scroll_glide;
mod status;
mod view;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

pub(super) use view::{build_window, refresh, BigPictureUi};

/// True when Ira was spawned under Gamescope's compositor, whose Wayland
/// socket is always `gamescope-0`.
pub fn running_in_gamescope() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok_and(|display| display.starts_with("gamescope"))
}

/// The live mode. Seeded from the launch (`--big-picture`, or automatic
/// under Gamescope) and flipped by `crate::ui::mode_switch`; launch workers
/// read it off the main thread, hence the atomic.
fn mode() -> &'static AtomicBool {
    static MODE: OnceLock<AtomicBool> = OnceLock::new();
    MODE.get_or_init(|| {
        AtomicBool::new(
            std::env::args().any(|arg| arg == "--big-picture") || running_in_gamescope(),
        )
    })
}

/// Big-picture mode: fullscreens the main window and drops the desktop
/// chrome.
pub fn is_big_picture() -> bool {
    mode().load(Ordering::Relaxed)
}

/// Record the mode the window is about to be rebuilt in.
pub(super) fn set_big_picture(big_picture: bool) {
    mode().store(big_picture, Ordering::Relaxed);
}

/// Open the big-picture disc picker for a multi-disc game: a centered
/// sheet of disc tiles, gamepad and keyboard navigable. Refused while a
/// game runs — picking another disc is starting something else.
pub(super) fn show_disc_picker(
    state: &crate::ui::state::SharedState,
    db_id: i64,
    variant_id: Option<i64>,
) {
    if launch_locked(state) {
        return;
    }
    let discs = {
        let s = state.borrow();
        ira_db::get_discs(&s.db, db_id).unwrap_or_default()
    };
    if discs.len() <= 1 {
        return;
    }
    if let Some(big) = state.borrow().big_picture.clone() {
        big.disc_picker.open(state, db_id, variant_id, &discs);
    }
}

/// True while any game runs: after one launch lands, nothing else may
/// start — not another game, not even the disc picker. The launcher
/// tracks running games itself, so the lock clears on exit with no
/// extra state to go stale.
pub(super) fn launch_locked(state: &crate::ui::state::SharedState) -> bool {
    state
        .borrow()
        .running_games
        .lock()
        .map(|running| !running.is_empty())
        .unwrap_or(false)
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
