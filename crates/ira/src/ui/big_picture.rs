//! Big-picture (gamescope / couch) entry detection, shared by main and the
//! window builder so the mode, its launch log line, and the UI changes it
//! drives stay in agreement.

/// True when Ira was spawned under Gamescope's compositor, whose Wayland
/// socket is always `gamescope-0`.
pub fn running_in_gamescope() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok_and(|display| display.starts_with("gamescope"))
}

/// Big-picture mode: fullscreens the main window and drops the desktop
/// chrome. Opt in via `--big-picture` or `IRA_BIG_PICTURE=1`; automatic
/// under Gamescope; opt out with `IRA_BIG_PICTURE=0`.
pub fn is_big_picture() -> bool {
    if std::env::args().any(|arg| arg == "--big-picture") {
        return true;
    }
    match std::env::var("IRA_BIG_PICTURE") {
        Ok(value) => value == "1",
        Err(_) => running_in_gamescope(),
    }
}
