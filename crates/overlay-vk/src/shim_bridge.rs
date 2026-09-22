//! Bridge to the LD_PRELOAD input shim.
//!
//! The shim (`libira_overlay_shim.so`) is loaded via `LD_PRELOAD` and exports
//! C functions (`ira_overlay_poll_events`, `ira_overlay_is_visible`, etc.).
//! This module resolves them via `dlsym(RTLD_DEFAULT, ...)` at first use,
//! then calls them each frame to poll input events and sync visibility.
//!
//! If the shim is not loaded (dlsym returns NULL), all functions return
//! defaults — the overlay simply won't receive input.

use std::ffi::c_int;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use ira_overlay::capture;
use ira_overlay_ipc::{
    canvas_shm_path, command_from_raw, CanvasCommand, CanvasShm, InputEventRaw, MappedShm,
    ShmHeader, ACT_HIDE_OVERLAY, ACT_SCREENSHOT, ACT_TOGGLE_RECORD,
};

type PollEventsFn = unsafe extern "C" fn(*mut InputEventRaw, usize) -> usize;
type IsVisibleFn = unsafe extern "C" fn() -> c_int;
type SetVisibleFn = unsafe extern "C" fn(c_int);
type HasSdlFn = unsafe extern "C" fn() -> c_int;
type IncrementPresentFn = unsafe extern "C" fn();
type ResetPresentFn = unsafe extern "C" fn();
type ReadyForOverlayFn = unsafe extern "C" fn() -> c_int;
type EnforceCursorFn = unsafe extern "C" fn();

static POLL_EVENTS: OnceLock<Option<PollEventsFn>> = OnceLock::new();
static IS_VISIBLE: OnceLock<Option<IsVisibleFn>> = OnceLock::new();
static SET_VISIBLE: OnceLock<Option<SetVisibleFn>> = OnceLock::new();
static HAS_SDL: OnceLock<Option<HasSdlFn>> = OnceLock::new();
static INCREMENT_PRESENT: OnceLock<Option<IncrementPresentFn>> = OnceLock::new();
static RESET_PRESENT: OnceLock<Option<ResetPresentFn>> = OnceLock::new();
static READY_FOR_OVERLAY: OnceLock<Option<ReadyForOverlayFn>> = OnceLock::new();
static ENFORCE_CURSOR: OnceLock<Option<EnforceCursorFn>> = OnceLock::new();
static PRESENT_COUNT: AtomicU32 = AtomicU32::new(0);

fn poll_fn() -> Option<PollEventsFn> {
    *POLL_EVENTS.get_or_init(|| {
        let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"ira_overlay_poll_events".as_ptr()) };
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    })
}

fn visible_fn() -> Option<IsVisibleFn> {
    *IS_VISIBLE.get_or_init(|| {
        let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"ira_overlay_is_visible".as_ptr()) };
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    })
}

fn set_visible_fn() -> Option<SetVisibleFn> {
    *SET_VISIBLE.get_or_init(|| {
        let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"ira_overlay_set_visible".as_ptr()) };
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    })
}

/// Returns true if the overlay is visible (synced from the shim).
/// Returns false if the shim is not loaded.
pub fn is_visible() -> bool {
    visible_fn().is_some_and(|f| unsafe { f() != 0 })
}

/// Sets the overlay visibility. Called by the Wayland input handler
/// when Shift+Tab is pressed, or by any other input path that needs
/// to toggle the overlay.
pub fn set_visible(v: bool) {
    if let Some(f) = set_visible_fn() {
        unsafe { f(if v { 1 } else { 0 }) };
    }
}

/// Mirrors the shim visibility flag into the game SHM header so the
/// out-of-process host (gamescope window mode) can show/hide its window.
/// Called every present; writes only on change. Covers every toggle path
/// (Wayland, evdev, shim hooks) in one place.
pub fn sync_visible_to_shm() {
    static LAST_SYNCED: AtomicBool = AtomicBool::new(false);
    let visible = is_visible();
    if LAST_SYNCED.swap(visible, Ordering::SeqCst) != visible {
        if let Some(guard) = shm().and_then(|m| m.lock().ok()) {
            guard
                .header()
                .overlay_visible
                .store(u32::from(visible), Ordering::SeqCst);
        }
    }
}

/// Returns true if SDL2 hooks are active (SDL2 detected via LD_PRELOAD).
/// When true, evdev gamepad polling is skipped since SDL hooks can consume
/// events (evdev can't).
pub fn has_sdl_hooks() -> bool {
    let f = *HAS_SDL.get_or_init(|| {
        let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"ira_overlay_has_sdl".as_ptr()) };
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    });
    f.is_some_and(|f| unsafe { f() != 0 })
}

/// Increments the present counter in the shim. Called on every queue_present.
pub fn increment_present_count() {
    let f = *INCREMENT_PRESENT.get_or_init(|| {
        let p = unsafe {
            libc::dlsym(
                libc::RTLD_DEFAULT,
                c"ira_overlay_increment_present_count".as_ptr(),
            )
        };
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    });
    if let Some(f) = f {
        unsafe { f() };
    }
}

/// Resets the present counter to zero. Called when a new swapchain is created.
pub fn reset_present_count() {
    let f = *RESET_PRESENT.get_or_init(|| {
        let p = unsafe {
            libc::dlsym(
                libc::RTLD_DEFAULT,
                c"ira_overlay_reset_present_count".as_ptr(),
            )
        };
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    });
    if let Some(f) = f {
        unsafe { f() };
    }
}

/// Re-asserts cursor visibility roughly twice a second while the overlay
/// is visible. Games re-grab/hide the pointer continuously, so the
/// toggle-time enforcement alone does not stick. Cheap: a few X roundtrips
/// at 0.5Hz, skipped entirely when hidden. `IRA_OVERLAY_NO_ENFORCE=1`
/// disables it for bisection.
pub fn enforce_cursor() {
    static DISABLED: OnceLock<bool> = OnceLock::new();
    if *DISABLED.get_or_init(|| std::env::var_os("IRA_OVERLAY_NO_ENFORCE").is_some()) {
        return;
    }
    if !PRESENT_COUNT.fetch_add(1, Ordering::Relaxed).is_multiple_of(30) {
        return;
    }
    let f = *ENFORCE_CURSOR.get_or_init(|| {
        let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"ira_overlay_enforce_cursor".as_ptr()) };
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    });
    if let Some(f) = f {
        unsafe { f() };
    }
}

/// Returns true if enough frames have been presented for the overlay to be safe.
/// If the shim isn't loaded (dlsym fails), returns true — no present count to wait for.
pub fn ready_for_overlay() -> bool {
    let f = *READY_FOR_OVERLAY.get_or_init(|| {
        let p = unsafe {
            libc::dlsym(
                libc::RTLD_DEFAULT,
                c"ira_overlay_ready_for_overlay".as_ptr(),
            )
        };
        (!p.is_null()).then(|| unsafe { std::mem::transmute(p) })
    });
    f.is_none_or(|f| unsafe { f() != 0 })
}

// ─── SHM-based hotkey config ───

static SHM: OnceLock<Option<Mutex<MappedShm>>> = OnceLock::new();
static SHM_WARNED: AtomicBool = AtomicBool::new(false);

fn shm() -> Option<&'static Mutex<MappedShm>> {
    SHM.get_or_init(|| {
        let path = match std::env::var_os("IRA_OVERLAY_SHM") {
            Some(p) => p,
            None => {
                warn_once(&SHM_WARNED, "ira-overlay: IRA_OVERLAY_SHM unset, input ring dead");
                return None;
            }
        };
        match MappedShm::open_rw(&path.to_string_lossy()) {
            Ok(m) => Some(Mutex::new(m)),
            Err(e) => {
                warn_once(
                    &SHM_WARNED,
                    &format!("ira-overlay: game SHM open failed ({e}), input ring dead"),
                );
                None
            }
        }
    })
    .as_ref()
}

/// Logs once per process (failures here never heal mid-session).
fn warn_once(flag: &AtomicBool, msg: &str) {
    if !flag.swap(true, Ordering::SeqCst) {
        eprintln!("{msg}");
    }
}

/// Reads hotkey config from SHM, falling back to defaults.
/// Returns (toggle_kc, toggle_mods, screenshot_kc, screenshot_mods, record_kc, record_mods).
pub fn hotkeys() -> (u32, u32, u32, u32, u32, u32) {
    let Some(shm) = shm().and_then(|m| m.lock().ok()) else {
        return ShmHeader::default_hotkeys();
    };
    shm.header().hotkeys()
}

pub fn gamepad_hotkeys() -> (u32, u32, u32) {
    let Some(shm) = shm().and_then(|m| m.lock().ok()) else {
        return ShmHeader::default_gamepad_hotkeys();
    };
    shm.header().gamepad_hotkeys()
}

/// Polls input events from the shim and forwards them to the GTK host as
/// canvas commands. Screenshot/record hotkeys execute game-side (capture
/// readback lives in the layer). Call every frame from `queue_present`.
pub fn poll_and_forward(swapchain: u64) {
    let Some(f) = poll_fn() else { return };

    let mut buf = [InputEventRaw::default(); 64];
    let count = unsafe { f(buf.as_mut_ptr(), buf.len()) };

    if count > 0 && poll_debug() {
        static LAST_LOG_MS: AtomicU32 = AtomicU32::new(0);
        let now = crate::canvas::now_ms();
        if now.wrapping_sub(LAST_LOG_MS.load(Ordering::Relaxed)) > 1000 {
            LAST_LOG_MS.store(now, Ordering::Relaxed);
            let kinds: Vec<u32> = buf[..count].iter().map(|r| r.event_type).collect();
            eprintln!("ira-overlay: polled {count} shim events: {kinds:?}");
        }
    }

    for raw in &buf[..count] {
        convert_and_forward(raw, swapchain);
    }
}

/// One-shot env check for input-path diagnostics (`IRA_OVERLAY_DEBUG=1`).
fn poll_debug() -> bool {
    static DEBUG: OnceLock<bool> = OnceLock::new();
    *DEBUG.get_or_init(|| std::env::var_os("IRA_OVERLAY_DEBUG").is_some())
}

fn convert_and_forward(raw: &InputEventRaw, swapchain: u64) {
    match raw.event_type {
        // Mouse: root pixels become client pixels via the swapchain's game
        // window (identity for fullscreen), then canvas pixels. Screen
        // position always feeds the layer-drawn cursor.
        0..=2 => {
            let (sx, sy) = crate::canvas::translate_to_client(
                swapchain,
                raw.x as f32,
                raw.y as f32,
            );
            crate::canvas::set_last_mouse(sx, sy);
            let Some((cx, cy)) = crate::canvas::screen_to_canvas(sx, sy)
            else {
                if poll_debug() {
                    static LAST_DROP_MS: AtomicU32 = AtomicU32::new(0);
                    let now = crate::canvas::now_ms();
                    if now.wrapping_sub(LAST_DROP_MS.load(Ordering::Relaxed)) > 2000 {
                        LAST_DROP_MS.store(now, Ordering::Relaxed);
                        eprintln!(
                            "ira-overlay: mouse ({} {}) off-panel, dropped",
                            raw.x, raw.y
                        );
                    }
                }
                return;
            };
            let cmd = match raw.event_type {
                0 => CanvasCommand::mouse_move(cx, cy),
                1 => CanvasCommand::mouse_down(cx, cy, raw.button),
                _ => CanvasCommand::mouse_up(cx, cy, raw.button),
            };
            // Coordinate-bug tracing: raw screen coords next to the mapped
            // canvas coords they produced. Buttons log every press (rare);
            // motion throttles (hot path).
            if poll_debug() {
                if raw.event_type == 0 {
                    static LAST_MOTION_MS: AtomicU32 = AtomicU32::new(0);
                    let now = crate::canvas::now_ms();
                    if now.wrapping_sub(LAST_MOTION_MS.load(Ordering::Relaxed)) > 2000 {
                        LAST_MOTION_MS.store(now, Ordering::Relaxed);
                        eprintln!(
                            "ira-overlay: motion raw=({},{}) canvas=({cx:.0},{cy:.0})",
                            raw.x, raw.y
                        );
                    }
                } else {
                    eprintln!(
                        "ira-overlay: button{} raw=({},{}) canvas=({cx:.0},{cy:.0})",
                        raw.event_type, raw.x, raw.y
                    );
                }
            }
            push_canvas_command(cmd);
        }
        // Key press — navigation keys become commands via the shared table.
        3 => {
            if let Some(cmd) = command_from_raw(3, 0, 0, 0, raw.keycode) {
                push_canvas_command(cmd);
            }
        }
        // Key release — not used by the host UI.
        4 => {}
        // Screenshot hotkey (F12).
        5 => {
            capture::request_screenshot();
        }
        // Recording toggle hotkey (F11).
        6 => {
            capture::toggle_recording();
        }
        // Mouse scroll event.
        7 => {
            if let Some(cmd) = command_from_raw(7, 0, raw.y, 0, 0) {
                push_canvas_command(cmd);
            }
        }
        _ => {}
    }
}

// ─── Canvas SHM (layer side: command producer, action consumer) ───

static CANVAS_SHM: Mutex<Option<CanvasShm>> = Mutex::new(None);
static LAST_ACT_READ: AtomicU32 = AtomicU32::new(0);

fn game_db_id() -> Option<i64> {
    shm()
        .and_then(|m| m.lock().ok())
        .map(|g| g.header().game_db_id)
}

fn open_canvas() -> Option<CanvasShm> {
    static ABI_DEAD: AtomicBool = AtomicBool::new(false);
    static OPEN_FAILS: AtomicU32 = AtomicU32::new(0);
    // A proven mismatch never heals within this process (fresh SHM every
    // launch); missing files still retry until the host creates them.
    if ABI_DEAD.load(Ordering::SeqCst) {
        return None;
    }
    let db_id = game_db_id()?;
    let shm = match CanvasShm::open_rw(&canvas_shm_path(db_id)) {
        Ok(s) => s,
        Err(e) => {
            // The host starts alongside the game; only complain once it is
            // clearly not coming (~5s of presents).
            if OPEN_FAILS.fetch_add(1, Ordering::Relaxed) == 300 {
                eprintln!("ira-overlay: canvas SHM not open ({e}), input ring dead");
            }
            return None;
        }
    };
    OPEN_FAILS.store(0, Ordering::Relaxed);
    // Fail loud on ABI drift (stale layer/host binaries): silent garbage
    // is worse than no overlay.
    let hdr = shm.header();
    if hdr.magic != ira_overlay_ipc::CANVAS_MAGIC
        || hdr.version != ira_overlay_ipc::CANVAS_VERSION
    {
        eprintln!(
            "ira-overlay: canvas ABI mismatch (magic={:#x} version={}), overlay disabled",
            hdr.magic, hdr.version
        );
        ABI_DEAD.store(true, Ordering::SeqCst);
        return None;
    }
    static LOGGED_OPEN: OnceLock<()> = OnceLock::new();
    LOGGED_OPEN.get_or_init(|| {
        eprintln!(
            "ira-overlay: canvas v{} open (client-space input)",
            ira_overlay_ipc::CANVAS_VERSION
        );
    });
    Some(shm)
}

/// Pushes one input command to the host (retries the SHM open until the
/// host has created the canvas region).
pub fn push_canvas_command(cmd: CanvasCommand) {
    use ira_overlay_ipc::CMD_MOUSE_MOVE;
    static DEBUG: OnceLock<bool> = OnceLock::new();
    if *DEBUG.get_or_init(|| std::env::var_os("IRA_OVERLAY_DEBUG").is_some())
        && cmd.kind != CMD_MOUSE_MOVE
    {
        eprintln!(
            "ira-overlay: push kind={} a={} b={} c={}",
            cmd.kind, cmd.a, cmd.b, cmd.c
        );
    }
    let mut guard = CANVAS_SHM.lock().unwrap();
    if guard.is_none() {
        *guard = open_canvas();
    }
    if let Some(shm) = guard.as_mut() {
        shm.push_command(cmd);
    }
}

/// Runs `f` against the canvas SHM, opening it on first use.
pub fn with_canvas<R>(f: impl FnOnce(&mut CanvasShm) -> R) -> Option<R> {
    let mut guard = CANVAS_SHM.lock().unwrap();
    if guard.is_none() {
        *guard = open_canvas();
    }
    guard.as_mut().map(f)
}

/// Overlay position (`OverlayPosition` as u32) from the game SHM header.
pub fn overlay_position() -> u32 {
    shm()
        .and_then(|m| m.lock().ok())
        .map(|g| g.header().overlay_position)
        .unwrap_or(0)
}

/// Drains host→layer actions (GTK button presses) and executes them
/// game-side, where swapchain readback lives.
pub fn drain_host_actions() {
    let read = LAST_ACT_READ.load(Ordering::SeqCst);
    let drained = with_canvas(|shm| shm.drain_actions(read));
    let Some((next, acts)) = drained else { return };
    LAST_ACT_READ.store(next, Ordering::SeqCst);
    for act in acts {
        match act.kind {
            ACT_SCREENSHOT => capture::request_screenshot(),
            ACT_TOGGLE_RECORD => capture::toggle_recording(),
            ACT_HIDE_OVERLAY => set_visible(false),
            _ => {}
        }
    }
}
