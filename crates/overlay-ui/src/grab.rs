//! Desktop input authority for gamescope mode (no `/dev/input` needed).
//!
//! The gamescope X server (`:2`) delivers effectively no input events to
//! external clients: passive grabs there never fire, so neither the chord
//! nor arrows nor the pointer can be read from it. The desktop server
//! (`:0`) always can — the user types and points there no matter what the
//! session shows. Two threads live on X displays for this:
//!
//! * `chord`: the toggle chord (Shift+Tab) plus navigation keys on the
//!   session display, where the chord demonstrably fires;
//! * `desktop`: the toggle chord plus pointer buttons on the desktop
//!   display, mapped into overlay space through the gamescope window rect.
//!
//! Narrow grabs only: the game keeps every other key, and the desktop
//! keeps everything the moment focus leaves the gamescope window (or the
//! overlay hides; button releases get a 300ms grace so a closing click
//! never leaks its release anywhere). X releases client grabs on
//! disconnect, so even a host crash cannot wedge input. Runs on its own
//! threads, gamescope+X11 only.

/// Pointer input the desktop thread reports back to the GTK loop, already
/// mapped into overlay (window) coordinates.
pub enum PointerEvent {
    /// Pointer position, on change only.
    Move(f64, f64),
    /// Button press at overlay coordinates (X11 button number).
    Press(f64, f64, u8),
    /// Button release at overlay coordinates.
    Release(f64, f64, u8),
}

/// Starts the session-display chord fallback: Shift+Tab flips overlay
/// visibility through the game SHM, same as the desktop chord. The :2
/// server delivers nothing else usable, so this thread grabs the chord
/// only — no wide steal, no navigation, no pointer. The SHM debounce
/// covers double flips when both threads hear one press.
pub fn start_chord_fallback(shm_name: Option<String>) {
    if std::env::var_os("IRA_OVERLAY_GAMESCOPE").is_none() {
        return;
    }
    let Some(shm_name) = shm_name else {
        return;
    };
    if std::thread::Builder::new()
        .name("ira-chord".to_string())
        .spawn(move || super::chord::chord_loop(&shm_name))
        .is_err()
    {
        eprintln!("ira-overlay-ui: chord fallback failed to spawn");
    }
}

/// Starts the desktop input thread, returning the channel its pointer
/// reports arrive on. `None` when there is nothing to watch: gamescope
/// mode off, no game SHM, no desktop display, or no desktop X connection.
pub fn start_desktop_input(
    shm_name: Option<String>,
    desktop_display: Option<String>,
    desktop_auth: Option<String>,
) -> Option<std::sync::mpsc::Receiver<PointerEvent>> {
    std::env::var_os("IRA_OVERLAY_GAMESCOPE")?;
    let shm_name = shm_name?;
    let desktop_display = desktop_display?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("ira-grab".to_string())
        .spawn(move || {
            super::desktop::grab_loop(&shm_name, &desktop_display, desktop_auth.as_deref(), tx)
        })
        .ok()?;
    Some(rx)
}
