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
