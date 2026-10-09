//! ira-overlay-ui — out-of-process GTK overlay host.
//!
//! Reads game data from the Ira app's SHM region, renders a real Adwaita
//! achievement panel in a helper window, and publishes pixel frames to the
//! canvas SHM region for the injected Vulkan layer to composite. Input flows
//! back as [`CanvasCommand`]s, applied to real widgets (no event synthesis).
//!
//! Without `IRA_OVERLAY_SHM` the host renders mock data (`--mock` dev mode,
//! canvas `/ira_canvas_0`). The layer derives the same canvas path from the
//! game header's `db_id`, so no extra env var is needed.

mod chord;
mod commands;
mod cursor;
mod desktop;
mod focus;
mod game;
mod gamescope;
mod grab;
mod host;
mod i18n;
mod keys;
mod proc;
mod render;
mod stub;
mod stub_dispatch;
mod tick;
mod ui;
mod widgets;
mod xauth;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk4::prelude::{ApplicationExt, ApplicationExtManual};
use ira_overlay_ipc::{CANVAS_VERSION, CanvasShm, MappedShm, canvas_shm_path};

use game::Fingerprint;

struct State {
    canvas: CanvasShm,
    game: Option<MappedShm>,
    ui: Rc<RefCell<ui::OverlayUi>>,
    renderer: Option<render::FrameRenderer>,
    window: gtk4::Window,
    cmd_read: u32,
    last_fp: Option<Fingerprint>,
    last_visible: bool,
    /// Last applied window size: the window follows the game extent.
    last_panel_size: (u32, u32),
    /// Game window actually unmapped for deterministic unfocus (None when
    /// nothing was unmapped).
    unmapped_game: Option<u32>,
    /// Settled-geometry logged for the current showing (reset on hide):
    /// the show-edge measurements fire pre-layout and read zeros.
    showgeom_logged: bool,
    /// The game fought the unmap (remapped behind our back): stop trying
    /// instead of fighting it forever.
    unmap_retreat: bool,
    /// Persistent focus connection (gamescope mode): owns X focus while
    /// visible instead of reconnecting per steal.
    focus_conn: Option<gamescope::FocusConn>,
    /// Cached overlay X id (queried once the surface exists).
    overlay_xid: Option<u32>,
    /// Last repaint (rate cap below).
    last_frame_render: std::time::Instant,
    /// Transition window end: fresh visual changes keep repainting
    /// briefly so CSS transitions play instead of freezing.
    tail_until: std::time::Instant,
    /// Layer ack consumed by the last render: repaints couple to the
    /// game's present rate instead of free-running past it.
    last_render_ack: u32,
    /// Live cursor serial last published (None until the first XFixes
    /// image lands; the file cursor covers the gap).
    last_cursor_serial: Option<u32>,
    dirty: bool,
    frame_no: u64,
    gamescope: bool,
    /// Footer glyph family last painted; the daemon publishes the
    /// physical pad's family for the overlay to follow.
    last_family: ira_input::ControllerFamily,
    /// Desktop pointer reports (gamescope mode only).
    input_rx: Option<std::sync::mpsc::Receiver<grab::PointerEvent>>,
}

/// Starts the stub compositor in injected mode. Returns the socket name
/// (kept alive by the compositor thread; dies with the process), or `None`
/// in gamescope mode / on failure (caller falls back to a minimized
/// helper window). `IRA_OVERLAY_NO_STUB=1` skips the stub for testing
/// against a real display.
fn stub_or_none() -> Option<String> {
    if gamescope::is_gamescope_mode() {
        return None;
    }
    if std::env::var_os("IRA_OVERLAY_NO_STUB").is_some() {
        eprintln!("ira-overlay-ui: stub disabled, using ambient display");
        return None;
    }
    stub::start()
}

/// Gamescope mode must live on the session's Xwayland server, not the
/// inherited desktop display: gamescope only composites external planes
/// from its own X server, and focus steal/restore below needs the same
/// connection. Must precede any GTK/GDK use (the display is opened
/// lazily on first use).
fn setup_gamescope_display() {
    if gamescope::is_gamescope_mode() && std::env::var("IRA_OVERLAY_XWAYLAND_DISPLAY").is_err() {
        match gamescope::discover_xwayland_display() {
            Some(num) => {
                eprintln!("ira-overlay-ui: using gamescope Xwayland display :{num}");
                unsafe { std::env::set_var("DISPLAY", format!(":{num}")) };
            }
            None => eprintln!("ira-overlay-ui: no gamescope Xwayland found, keeping $DISPLAY"),
        }
    }
}

/// Opens the game SHM (or mock data when `IRA_OVERLAY_UI_MOCK` is set).
/// `None` when there is nothing to show.
fn load_game_source(mock_mode: bool) -> Option<(Option<MappedShm>, Option<game::GameData>, i64)> {
    match std::env::var("IRA_OVERLAY_SHM") {
        // Read-write: the Hide path mirrors visibility into the game SHM
        // (freeze watcher follows it); a read-only mapping aborts there.
        Ok(path) => match MappedShm::open_rw(&path) {
            Ok(shm) => {
                let db_id = shm.header().game_db_id;
                eprintln!("ira-overlay-ui: game SHM '{path}' (db_id={db_id})");
                Some((Some(shm), None, db_id))
            }
            Err(e) => {
                eprintln!("ira-overlay-ui: cannot open game SHM '{path}': {e}");
                None
            }
        },
        Err(_) if mock_mode => {
            eprintln!("ira-overlay-ui: mock mode, canvas /ira_canvas_0");
            Some((None, Some(game::GameData::mock()), 0))
        }
        Err(_) => {
            eprintln!("ira-overlay-ui: IRA_OVERLAY_SHM not set (try --mock)");
            None
        }
    }
}

/// Creates and initializes the canvas, then claims the single-host lock.
/// `None` when the canvas cannot be created.
fn create_canvas(db_id: i64) -> Option<CanvasShm> {
    let mut canvas = match CanvasShm::create(db_id) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ira-overlay-ui: canvas create failed: {e}");
            return None;
        }
    };
    canvas.init_header();
    eprintln!("ira-overlay-ui: canvas '{}'", canvas_shm_path(db_id));
    // Single instance per game: a previous host (crashed sessions leave
    // them behind) would composite a second, possibly frozen, plane over
    // or under this one, making the overlay look dead to all input.
    claim_host_lock(db_id);
    Some(canvas)
}

fn main() {
    eprintln!("ira-overlay-ui: starting (canvas v{CANVAS_VERSION})");
    // Build fingerprint: proves which binary a log came from, so stale
    // builds are identifiable instead of debatable.
    if let Ok(exe) = std::env::current_exe()
        && let Ok(meta) = std::fs::metadata(&exe)
        && let Ok(modified) = meta.modified()
    {
        eprintln!("ira-overlay-ui: host built at {modified:?}");
    }
    // Same catalog as the app, before GTK initializes text rendering.
    i18n::init();
    // Same type as big picture, before GTK initializes text rendering.
    ira_widgets::font::install();
    setup_gamescope_display();
    // Injected mode renders on our own stub compositor: zero visible
    // output anywhere (snapshots require a mapped window).
    // Gamescope mode keeps its real window — the compositor is its display.
    let stub_name = stub_or_none();
    // Mock flag is env, not argv: GTK/Adwaita claim unknown CLI flags.
    let mock_mode = std::env::var_os("IRA_OVERLAY_UI_MOCK").is_some();
    let Some((game, mock, db_id)) = load_game_source(mock_mode) else {
        return;
    };
    let Some(canvas) = create_canvas(db_id) else {
        return;
    };

    // Desktop pointer + keyboard authority for gamescope mode (no
    // /dev/input needed): the chord opens, nav keys and pointer buttons
    // are stolen while visible and engaged. The :2 server delivers no
    // input events to external clients, so this lives on the desktop
    // display — passed explicitly, since this process inherits gamescope's
    // DISPLAY. Dies with the process.
    let input_rx = grab::start_desktop_input(
        std::env::var("IRA_OVERLAY_SHM").ok(),
        std::env::var("IRA_OVERLAY_DESKTOP_DISPLAY").ok(),
        std::env::var("IRA_OVERLAY_DESKTOP_XAUTHORITY").ok(),
    );
    // Session chord fallback: Shift+Tab on :2 costs nothing when that
    // server is mute, and restores the toggle where it does deliver.
    grab::start_chord_fallback(std::env::var("IRA_OVERLAY_SHM").ok());

    let app = adw::Application::new(Some("org.ira.OverlayUi"), gio::ApplicationFlags::FLAGS_NONE);
    // connect_activate takes Fn: hand owned state over via take-once cells.
    let game = RefCell::new(Some(game));
    let mock = RefCell::new(mock);
    let canvas = RefCell::new(Some(canvas));
    let input_rx = RefCell::new(input_rx);
    app.connect_activate(move |app| {
        let (Some(game), mock, Some(canvas)) = (game.take(), mock.take(), canvas.take()) else {
            return;
        };
        host::activate(
            app,
            game,
            mock,
            canvas,
            stub_name.is_none(),
            input_rx.take(),
        );
    });
    std::process::exit(i32::from(app.run()));
}

/// Publishes the theme cursor once: the layer draws it per-present at
/// native size, so pointer tracking never waits on the panel frame loop.
fn publish_theme_cursor(canvas: &mut CanvasShm) {
    let size = cursor::cursor_size();
    match cursor::load_cursor("left_ptr", size) {
        Some(img) => {
            if let Err(e) = canvas.publish_cursor(
                img.width,
                img.height,
                img.xhot as i32,
                img.yhot as i32,
                &img.pixels,
            ) {
                eprintln!("ira-overlay-ui: cursor publish failed: {e}");
            }
        }
        None => eprintln!("ira-overlay-ui: no cursor theme found; overlay pointer hidden"),
    }
}

/// Lock file pinning one overlay host per game canvas.
fn host_lock_path(db_id: i64) -> std::path::PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    dir.join(format!("ira-overlay-host-{db_id}.lock"))
}

/// Single instance per game canvas: reads the previous host's pid from
/// the lock file and terminates it before claiming the lock. A leftover
/// host composites a second plane (possibly a frozen frame) that makes
/// the live overlay look dead to all input.
fn claim_host_lock(db_id: i64) {
    let path = host_lock_path(db_id);
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(pid) = text.trim().parse::<i32>()
        && pid != std::process::id() as i32
        && unsafe { libc::kill(pid, 0) } == 0
    {
        eprintln!("ira-overlay-ui: terminating stale host (pid={pid})");
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(2) && unsafe { libc::kill(pid, 0) } == 0 {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    if let Err(e) = std::fs::write(&path, std::process::id().to_string()) {
        eprintln!("ira-overlay-ui: cannot write host lock: {e}");
    }
}

/// Display size published by Ira (`IRA_OVERLAY_SCREEN_W/H`), if any.
fn screen_size_from_env() -> Option<(u32, u32)> {
    let w: u32 = std::env::var("IRA_OVERLAY_SCREEN_W").ok()?.parse().ok()?;
    let h: u32 = std::env::var("IRA_OVERLAY_SCREEN_H").ok()?.parse().ok()?;
    (w > 0 && h > 0).then_some((w, h))
}

/// Display size with the desktop thread's fallback: one truth for the env
/// parsing (positive-only), separate defaults per caller.
fn screen_size() -> (u32, u32) {
    screen_size_from_env().unwrap_or((1920, 1080))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EnvGuard {
        key: &'static str,
        prev: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var_os(key);
            unsafe { std::env::set_var(key, value) };
            Self { key, prev }
        }

        fn remove(key: &'static str) -> Self {
            let prev = std::env::var_os(key);
            unsafe { std::env::remove_var(key) };
            Self { key, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => unsafe { std::env::set_var(self.key, v) },
                None => unsafe { std::env::remove_var(self.key) },
            }
        }
    }

    #[test]
    fn test_screen_size_from_env_accepts_positive_only() {
        let _gw = EnvGuard::set("IRA_OVERLAY_SCREEN_W", "1920");
        let _gh = EnvGuard::set("IRA_OVERLAY_SCREEN_H", "1080");
        // Act + Assert
        assert_eq!(screen_size_from_env(), Some((1920, 1080)));
        assert_eq!(screen_size(), (1920, 1080));
    }

    #[test]
    fn test_screen_size_from_env_rejects_non_positive() {
        let _gw = EnvGuard::set("IRA_OVERLAY_SCREEN_W", "0");
        let _gh = EnvGuard::set("IRA_OVERLAY_SCREEN_H", "1080");
        assert_eq!(screen_size_from_env(), None);
        assert_eq!(screen_size(), (1920, 1080));
    }

    #[test]
    fn test_screen_size_from_env_rejects_garbage() {
        let _gw = EnvGuard::set("IRA_OVERLAY_SCREEN_W", "wide");
        let _gh = EnvGuard::set("IRA_OVERLAY_SCREEN_H", "1080");
        assert_eq!(screen_size_from_env(), None);
    }

    #[test]
    fn test_host_lock_path_uses_runtime_dir() {
        let _g = EnvGuard::set("XDG_RUNTIME_DIR", "/run/user/1000");
        assert_eq!(
            host_lock_path(7),
            std::path::PathBuf::from("/run/user/1000/ira-overlay-host-7.lock")
        );
        drop(_g);
        let _g = EnvGuard::remove("XDG_RUNTIME_DIR");
        assert_eq!(
            host_lock_path(7),
            std::path::PathBuf::from("/tmp/ira-overlay-host-7.lock")
        );
    }
}
