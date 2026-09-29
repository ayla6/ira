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
mod cursor;
mod desktop;
mod focus;
mod game;
mod gamescope;
mod grab;
mod i18n;
mod keys;
mod proc;
mod render;
mod stub;
mod ui;
mod widgets;
mod xauth;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use glib::object::Cast as _;
use gtk4::prelude::{ApplicationExt, ApplicationExtManual, GtkWindowExt, WidgetExt};
use ira_overlay_ipc::{
    CANVAS_VERSION, CanvasAction, CanvasCommand, CanvasShm, MappedShm, canvas_shm_path,
    fit_panel_size,
};

use game::{Fingerprint, fingerprint};
use render::{CANVAS_H, CANVAS_W, FrameRenderer};
use ui::{OverlayUi, UiEffect};

struct State {
    canvas: CanvasShm,
    game: Option<MappedShm>,
    ui: Rc<RefCell<OverlayUi>>,
    renderer: Option<FrameRenderer>,
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
    // Gamescope mode must live on the session's Xwayland server, not the
    // inherited desktop display: gamescope only composites external planes
    // from its own X server, and focus steal/restore below needs the same
    // connection. Must precede any GTK/GDK use (the display is opened
    // lazily on first use).
    if gamescope::is_gamescope_mode() && std::env::var("IRA_OVERLAY_XWAYLAND_DISPLAY").is_err() {
        match gamescope::discover_xwayland_display() {
            Some(num) => {
                eprintln!("ira-overlay-ui: using gamescope Xwayland display :{num}");
                unsafe { std::env::set_var("DISPLAY", format!(":{num}")) };
            }
            None => eprintln!("ira-overlay-ui: no gamescope Xwayland found, keeping $DISPLAY"),
        }
    }
    // Injected mode renders on our own stub compositor: zero visible
    // output anywhere (snapshots require a mapped window).
    // Gamescope mode keeps its real window — the compositor is its display.
    let stub_name = stub_or_none();
    // Mock flag is env, not argv: GTK/Adwaita claim unknown CLI flags.
    let mock_mode = std::env::var_os("IRA_OVERLAY_UI_MOCK").is_some();
    let (game, mock, db_id) = match std::env::var("IRA_OVERLAY_SHM") {
        // Read-write: the Hide path mirrors visibility into the game SHM
        // (freeze watcher follows it); a read-only mapping aborts there.
        Ok(path) => match MappedShm::open_rw(&path) {
            Ok(shm) => {
                let db_id = shm.header().game_db_id;
                eprintln!("ira-overlay-ui: game SHM '{path}' (db_id={db_id})");
                (Some(shm), None, db_id)
            }
            Err(e) => {
                eprintln!("ira-overlay-ui: cannot open game SHM '{path}': {e}");
                return;
            }
        },
        Err(_) if mock_mode => {
            eprintln!("ira-overlay-ui: mock mode, canvas /ira_canvas_0");
            (None, Some(game::GameData::mock()), 0)
        }
        Err(_) => {
            eprintln!("ira-overlay-ui: IRA_OVERLAY_SHM not set (try --mock)");
            return;
        }
    };

    let mut canvas = match CanvasShm::create(db_id) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ira-overlay-ui: canvas create failed: {e}");
            return;
        }
    };
    canvas.init_header();
    eprintln!("ira-overlay-ui: canvas '{}'", canvas_shm_path(db_id));
    // Single instance per game: a previous host (crashed sessions leave
    // them behind) would composite a second, possibly frozen, plane over
    // or under this one, making the overlay look dead to all input.
    claim_host_lock(db_id);

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
        activate(
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
        while start.elapsed() < std::time::Duration::from_secs(2)
            && unsafe { libc::kill(pid, 0) } == 0
        {
            std::thread::sleep(std::time::Duration::from_millis(50));
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

/// Builds the helper window + UI and starts the 30Hz frame loop.
/// Takes `game`/`canvas` by value — the activate closure runs once.
fn activate(
    app: &adw::Application,
    game: Option<MappedShm>,
    mock: Option<game::GameData>,
    mut canvas: CanvasShm,
    no_stub: bool,
    input_rx: Option<std::sync::mpsc::Receiver<grab::PointerEvent>>,
) {
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::PreferDark);
    publish_theme_cursor(&mut canvas);
    // Same stylesheet as big picture (1.0 = the 1080p reference): the
    // overlay speaks the same visual language instead of a hand-rolled
    // clone. Injected mode renders on our own stub compositor and stays
    // un-dimmed: the renderer snapshots the window and the layer blends
    // it over the game.
    let gamescope = gamescope::is_gamescope_mode();
    ira_widgets::style::install_bp_css(1.0);

    let window = gtk4::Window::new();
    window.set_application(Some(app));
    window.set_title(Some(&crate::tr!("Ira Overlay")));
    window.set_decorated(false);
    if gamescope {
        // Big-picture dim behind the content (same class as the app's
        // menu sheets). Injected mode stays transparent for snapshot
        // blending.
        window.add_css_class(ira_widgets::style::CSS_BP_MENU_DIM);
    }
    // Startup size from the display (published by Ira; inherited through
    // spawn) so the first frame is already right: fit the screen the same
    // way the per-tick resize does, defaulting to 512x800 without it.
    let (win_w, win_h) = match screen_size_from_env() {
        Some((sw, sh)) => {
            let (w, h) = fit_panel_size(sw, sh);
            (w as i32, h as i32)
        }
        None => (CANVAS_W as i32, CANVAS_H as i32),
    };
    window.set_default_size(win_w, win_h);
    if !gamescope {
        // The injected helper is never user-facing: bury it below other
        // windows (a fullscreen game covers it regardless).
        gamescope::background_helper(&window);
    }

    let state = Rc::new(RefCell::new(None::<State>));
    let push_action = {
        let state = state.clone();
        Rc::new(move |act: CanvasAction| {
            if act.kind == ira_overlay_ipc::ACT_EXIT_GAME {
                // Exit Game: terminate the session leader; its cleanup
                // reaps the game and this host with it. Best-effort:
                // without a session ancestor there is nothing to exit
                // here — but the action is still forwarded below, and the
                // in-process layer terminates the game itself on receipt.
                if let Some(pid) = proc::session_leader_pid() {
                    eprintln!("ira-overlay-ui: exiting game (session leader pid={pid})");
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                } else {
                    eprintln!("ira-overlay-ui: exit game requested, no session leader found");
                }
            }
            if let Some(s) = state.borrow_mut().as_mut() {
                s.canvas.push_action(act);
            }
        })
    };
    let hide_state = state.clone();
    let on_hide = Rc::new(move || {
        if let Some(s) = hide_state.borrow_mut().as_mut() {
            apply_hide(s);
        }
    });
    let ui = OverlayUi::new(push_action, on_hide, gamescope::is_gamescope_mode());
    let initial = match (&game, &mock) {
        (Some(shm), _) => game::load(shm),
        (None, Some(m)) => m.clone(),
        (None, None) => game::GameData::mock(),
    };
    ui.borrow_mut().rebuild(&initial);
    window.set_child(Some(ui.borrow().root()));
    window.present();

    // The renderer needs the mapped surface; the window was just presented,
    // so pump the main context until it is mapped (spike precedent).
    // Non-blocking iterations only: iteration(true) would wait forever on
    // a dead compositor thread, hanging startup with zero further output.
    let ctx = glib::MainContext::default();
    let start = std::time::Instant::now();
    while !window.is_mapped() {
        if start.elapsed() > Duration::from_secs(5) {
            eprintln!("ira-overlay-ui: helper window never mapped");
            return;
        }
        ctx.iteration(false);
        std::thread::sleep(Duration::from_millis(10));
    }
    if gamescope {
        gamescope::setup_external_overlay(&window);
    }
    // Gamescope shows the window itself; only the injected path snapshots it.
    // The window maps on the stub compositor (invisible anywhere), so only
    // the real-desktop fallback minimizes the helper.
    let renderer = if gamescope {
        None
    } else {
        let renderer = match FrameRenderer::for_window(&window) {
            Some(r) => Some(r),
            None => {
                eprintln!("ira-overlay-ui: no GSK renderer for helper surface");
                return;
            }
        };
        if no_stub {
            window.minimize();
        }
        renderer
    };

    let last_fp = game.as_ref().map(fingerprint);
    // Gamescope starts hidden — the tick raises the window on toggle.
    // The injected helper must stay mapped (snapshots need its surface),
    // so it starts visible; mock runs have no SHM and start visible too.
    let last_visible = game.is_none() || !gamescope;
    if !last_visible {
        window.set_visible(false);
    }
    *state.borrow_mut() = Some(State {
        canvas,
        game,
        ui,
        renderer,
        window: window.clone(),
        cmd_read: 0,
        last_fp,
        last_visible,
        last_panel_size: (win_w as u32, win_h as u32),
        unmapped_game: None,
        showgeom_logged: false,
        unmap_retreat: false,
        focus_conn: None,
        overlay_xid: None,
        last_frame_render: std::time::Instant::now(),
        tail_until: std::time::Instant::now(),
        last_render_ack: 0,
        last_cursor_serial: None,
        dirty: true,
        frame_no: 0,
        gamescope,
        last_family: ira_input::ControllerFamily::Generic,
        input_rx,
    });

    glib::timeout_add_local(Duration::from_millis(4), move || {
        tick(&state);
        glib::ControlFlow::Continue
    });
}

/// Polls the live system cursor and republishes on change. Called from
/// the tick with the state borrow held.
fn poll_live_cursor(s: &mut State) {
    let Some((img, serial)) = cursor::load_live_cursor() else {
        return;
    };
    if s.last_cursor_serial == Some(serial) {
        return;
    }
    s.last_cursor_serial = Some(serial);
    if let Err(e) = s.canvas.publish_cursor(
        img.width,
        img.height,
        img.xhot as i32,
        img.yhot as i32,
        &img.pixels,
    ) {
        eprintln!("ira-overlay-ui: live cursor publish failed: {e}");
    }
}

/// Dispatches pending main-loop events without blocking, so input the
/// tick just injected is processed before the render below snapshots.
/// Bounded: each pass dispatches whatever is ready and stops on idle.
fn pump_events() {
    let ctx = glib::MainContext::default();
    for _ in 0..8 {
        if !ctx.iteration(false) {
            break;
        }
    }
}

/// The overlay window's X id, cached on first query (the surface only
/// exists once mapped; querying every tick would re-downcast for free
/// but the cache keeps the hot path to one focus roundtrip).
fn overlay_xid(s: &mut State) -> Option<u32> {
    if s.overlay_xid.is_none() {
        s.overlay_xid = gamescope::x11_id(&s.window);
    }
    s.overlay_xid
}

/// Hides the overlay from any input path (gamepad Back, mouse click):
/// canvas flag plus the game SHM mirror the freeze watcher follows.
fn apply_hide(s: &mut State) {
    s.canvas.header_mut().visible.store(0, Ordering::SeqCst);
    // Mirror into the game SHM: the freeze watcher (and a thawed layer)
    // follows overlay_visible, not the canvas flag. Idempotent with the
    // layer's own mirror.
    if let Some(shm) = &mut s.game {
        shm.header_mut().overlay_visible.store(0, Ordering::SeqCst);
        eprintln!("ira-overlay-ui: hide mirrored into game SHM");
    }
}

fn tick(state: &Rc<RefCell<Option<State>>>) {
    // The host must never die with the plane up: a dead host leaves a
    // frozen overlay composited (opacity stuck nonzero) while the daemon
    // keeps believing it is visible (pad dead to the game). A panicking
    // tick is logged and skipped instead of aborting the process.
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tick_inner(state))).is_err() {
        eprintln!("ira-overlay-ui: tick panicked; skipping this frame");
    }
}

/// Marks a repaint from the unborrowed apply path (no state borrow may
/// be alive there: the UI callbacks borrow this same state).
fn mark_dirty(state: &Rc<RefCell<Option<State>>>) {
    if let Some(s) = state.borrow_mut().as_mut() {
        s.dirty = true;
    }
}

fn drain_pointer_events(s: &mut State) {
    // Drain first: the receiver borrow must end before Hide handling
    // reborrows the state below.
    let events: Vec<grab::PointerEvent> = s
        .input_rx
        .as_ref()
        .map(|rx| {
            let mut drained = Vec::new();
            while let Ok(event) = rx.try_recv() {
                drained.push(event);
            }
            drained
        })
        .unwrap_or_default();
    for event in events {
        match event {
            grab::PointerEvent::Move(x, y) => {
                s.ui.borrow_mut().pointer_move(x, y);
            }
            grab::PointerEvent::Press(x, y, button) => {
                s.ui.borrow_mut().pointer_press(x, y, button);
            }
            grab::PointerEvent::Release(x, y, button) => {
                if s.ui.borrow_mut().pointer_release(x, y, button) == UiEffect::Hide {
                    apply_hide(s);
                }
            }
        }
    }
}

/// Rebuilds widgets when the game fingerprint changed and repaints the
/// footer when the controller family changed. Returns fresh (non-motion
/// work happened: render now and open the transition window).
fn refresh_game_data(s: &mut State) -> bool {
    let Some(shm) = &s.game else {
        return false;
    };
    let mut fresh = false;
    let fp = fingerprint(shm);
    if Some(fp) != s.last_fp {
        s.ui.borrow_mut().rebuild(&game::load(shm));
        s.last_fp = Some(fp);
        s.dirty = true;
        fresh = true;
    }
    // Footer glyphs follow the physical pad's family, published by
    // the daemon; repainted only on change.
    let family = ira_input::ControllerFamily::from_u32(
        shm.header().controller_family.load(Ordering::SeqCst),
    );
    if family != s.last_family {
        s.last_family = family;
        s.ui.borrow_mut().rebuild_footer(family);
    }
    fresh
}

fn gamescope_show(s: &mut State) {
    s.window.present();
    // Opacity back up after the map: gamescope keys the
    // plane off nonzero opacity, ignoring map state.
    if let Some(xid) = gamescope::x11_id(&s.window)
        && let Err(e) = gamescope::set_window_opacity(xid, gamescope::OPAQUE)
    {
        eprintln!("ira-overlay-ui: opacity restore failed: {e}");
    }
    eprintln!(
        "ira-overlay-ui: showing overlay (mapped={} {}x{})",
        s.window.is_mapped(),
        s.window.width(),
        s.window.height()
    );
    s.ui.borrow_mut().log_showgeom();
    // Unmap the game window while the overlay is up: an unmapped window
    // drops out of the compositor's focus list, which is what
    // focus-pausing games actually pause on. (No XSetInputFocus steal:
    // gamescope re-asserts its own focus choice every commit, so steals
    // never stick — input already arrives out-of-band via the grab
    // threads, and pausing is the freeze feature's job.)
    // Guards: never the root/pointer-root ids, never
    // our own window; errors just skip.
    let xid = overlay_xid(s);
    if s.focus_conn.is_none() {
        s.focus_conn = gamescope::FocusConn::connect();
    }
    if let Some(conn) = s.focus_conn.as_ref() {
        let prev = conn.current_focus();
        if let Some(game) = prev
            && game > 1
            && Some(game) != xid
            && !s.unmap_retreat
        {
            let (name, pid) = conn.window_info(game);
            eprintln!("ira-overlay-ui: unmap target {game} name={name:?} pid={pid:?}");
            match conn.unmap_window(game) {
                Ok(()) => {
                    eprintln!("ira-overlay-ui: unmapped game window {game} (unfocused)");
                    s.unmapped_game = Some(game);
                }
                Err(e) => {
                    eprintln!("ira-overlay-ui: game unmap skipped: {e}")
                }
            }
        }
    }
}

fn gamescope_hide(s: &mut State) {
    // Opacity to zero first: this is what drops the
    // gamescope plane (unmap alone leaves a frozen
    // frame composited). Then remap the game window we unmapped.
    if let Some(xid) = gamescope::x11_id(&s.window)
        && let Err(e) = gamescope::set_window_opacity(xid, 0)
    {
        eprintln!("ira-overlay-ui: opacity zero failed: {e}");
    } else {
        eprintln!("ira-overlay-ui: hiding overlay");
    }
    s.window.set_visible(false);
    s.showgeom_logged = false;
    // Bring the game window back first, but only one we
    // actually unmapped: remapping anything else (or a
    // window gamescope already remapped) is pointless.
    if let Some(game) = s.unmapped_game.take()
        && let Some(conn) = s.focus_conn.as_ref()
    {
        match conn.map_window(game) {
            Ok(()) => {
                eprintln!("ira-overlay-ui: remapped game window {game}")
            }
            Err(e) => {
                eprintln!("ira-overlay-ui: game remap failed: {e}")
            }
        }
    }
}

fn gamescope_hold(s: &mut State) {
    // Settled geometry, once per showing: the show edge
    // fires pre-layout (zeros), so wait for a real size.
    if !s.showgeom_logged && s.window.width() > 0 {
        s.showgeom_logged = true;
        s.ui.borrow().log_showgeom();
    }
    // Retreat check: if a window we unmapped is viewable
    // again, something remapped it behind our back — stop
    // unmapping instead of fighting it forever. Only the
    // actually-unmapped window counts.
    if s.frame_no.is_multiple_of(30)
        && !s.unmap_retreat
        && let Some(game) = s.unmapped_game
        && let Some(conn) = s.focus_conn.as_ref()
        && conn.is_viewable(game).is_some_and(|v| v)
    {
        s.unmapped_game = None;
        s.unmap_retreat = true;
        eprintln!(
            "ira-overlay-ui: game window remapped externally; \
             unmap retired for this session (enable Pause game \
             on overlay in the game settings for a guaranteed pause)"
        );
    }
}

fn handle_gamescope_visibility(s: &mut State) {
    // Gamescope shows the window itself: follow the toggle the layer
    // mirrors into the game SHM. Raising on show gives it input focus.
    let Some(shm) = &s.game else {
        return;
    };
    if !s.gamescope {
        return;
    }
    let visible = shm.header().overlay_visible.load(Ordering::SeqCst) != 0;
    if visible != s.last_visible {
        s.window.set_visible(visible);
        if visible {
            gamescope_show(s);
        } else {
            gamescope_hide(s);
        }
        s.last_visible = visible;
    } else if visible {
        gamescope_hold(s);
    }
    // The pointer belongs to the overlay window itself now (real
    // system cursor on real widgets): nothing to forward here.
}

fn drain_canvas(s: &mut State) -> (Rc<RefCell<OverlayUi>>, Vec<CanvasCommand>) {
    let ui = s.ui.clone();
    let (next, cmds) = s.canvas.drain_commands(s.cmd_read);
    s.cmd_read = next;
    // The layer publishes its swapchain extent; size the window from
    // it so the panel grows and shrinks with the game. Unknown (0)
    // keeps the startup default.
    let sw = s.canvas.header().screen_w.load(Ordering::SeqCst);
    let sh = s.canvas.header().screen_h.load(Ordering::SeqCst);
    if sw > 0 && sh > 0 {
        let want = fit_panel_size(sw, sh);
        if want != s.last_panel_size {
            s.last_panel_size = want;
            s.window.set_default_size(want.0 as i32, want.1 as i32);
        }
    }
    (ui, cmds)
}

fn tick_inner(state: &Rc<RefCell<Option<State>>>) {
    // Phase 0 (borrowed): drain desktop pointer reports into the UI.
    // Button releases that close mirror like a gamepad Back.
    {
        let mut guard = state.borrow_mut();
        let Some(s) = guard.as_mut() else { return };
        drain_pointer_events(s);
    }
    // Phase 1 (borrowed): data refresh, visibility handling, command
    // drain. The borrow ends before commands are applied below: applying
    // runs UI callbacks (screenshot/record pushes, click closes) that
    // borrow this same state, and a nested borrow would abort the host.
    let (ui, cmds, fresh) = {
        let mut guard = state.borrow_mut();
        let Some(s) = guard.as_mut() else { return };

        // Fresh (non-motion) work below marks `fresh`: it renders now and
        // opens the transition window for continuation frames.
        let fresh = refresh_game_data(s);
        handle_gamescope_visibility(s);
        let (ui, cmds) = drain_canvas(s);
        // Pointer injects straight into the compositor and dispatches
        // below in this same tick. Non-motion work renders immediately;
        // motion is noted for the repaint rule below.
        (ui, cmds, fresh)
    };

    // Phase 1b (unborrowed): classify and apply each command. No state
    // borrow is alive here, so the UI callbacks below borrow freely.
    let (fresh, pointer_input, moved) = apply_commands(state, &ui, cmds, fresh);

    // Phase 2 (unborrowed): dispatch the injected events synchronously so
    // the render below snapshots post-input state in the same tick.
    if pointer_input {
        pump_events();
    }

    // Phase 3 (borrowed): render decision + render.
    maybe_render(state, fresh, moved);
}

/// Applies drained layer commands to the widgets. Returns the updated
/// (fresh, pointer_input, moved) flags for the render rule below.
fn apply_commands(
    state: &Rc<RefCell<Option<State>>>,
    ui: &Rc<RefCell<OverlayUi>>,
    cmds: Vec<CanvasCommand>,
    mut fresh: bool,
) -> (bool, bool, bool) {
    use ira_overlay_ipc::{CMD_MOUSE_DOWN, CMD_MOUSE_MOVE, CMD_MOUSE_UP};
    let mut pointer_input = false;
    let mut moved = false;
    for cmd in cmds {
        match cmd.kind {
            CMD_MOUSE_MOVE => {
                pointer_input = true;
                moved = true;
            }
            CMD_MOUSE_DOWN | CMD_MOUSE_UP => {
                pointer_input = true;
                fresh = true;
                mark_dirty(state);
            }
            _ => {
                fresh = true;
                mark_dirty(state);
            }
        }
        let effect = ui.borrow_mut().apply(cmd);
        match effect {
            UiEffect::None => {}
            UiEffect::Show => {
                if let Some(s) = state.borrow_mut().as_mut() {
                    s.canvas.header_mut().visible.store(1, Ordering::SeqCst);
                }
            }
            UiEffect::Hide => {
                if let Some(s) = state.borrow_mut().as_mut() {
                    apply_hide(s);
                }
            }
        }
    }
    (fresh, pointer_input, moved)
}

fn maybe_render(state: &Rc<RefCell<Option<State>>>, fresh: bool, moved: bool) {
    let mut guard = state.borrow_mut();
    let Some(s) = guard.as_mut() else { return };

    // Backpressure: skip this tick if the layer is >2 frames behind.
    let hdr = s.canvas.header();
    if hdr
        .write_seq
        .load(Ordering::SeqCst)
        .wrapping_sub(hdr.ack_seq.load(Ordering::SeqCst))
        > 2
    {
        return;
    }

    // Fresh visual changes open a transition window; motion joins it.
    // Repaint rule: fresh work renders now; motion and continuation
    // repaint at most every 8ms and only once the layer consumed a frame,
    // so repaints couple to the game's present rate instead of
    // free-running past it. Without a live layer (mock, or a paused game
    // holding acks) the rate cap alone applies.
    let now = std::time::Instant::now();
    let ack = hdr.ack_seq.load(Ordering::SeqCst);
    if fresh || moved {
        s.tail_until = now + Duration::from_millis(250);
    }
    let tail = now < s.tail_until;
    let ack_fresh = ack != s.last_render_ack;
    let live_layer = ack > 0;
    if fresh
        || ((moved || tail)
            && s.last_frame_render.elapsed() >= Duration::from_millis(8)
            && (!live_layer || ack_fresh))
    {
        s.last_frame_render = now;
        s.last_render_ack = ack;
        s.dirty = true;
    }

    if !s.gamescope && (s.dirty || s.frame_no == 0) {
        snapshot_frame(s);
        s.dirty = false;
    }
    // Post-map layout truth (rebuild runs pre-layout): logs button
    // bounds once real values exist. Change-gated inside.
    if s.frame_no == 2 {
        s.ui.borrow_mut().log_layout();
        if std::env::var_os("IRA_OVERLAY_DEBUG").is_some() {
            use gtk4::prelude::NativeExt as _;
            let (nx, ny) = s.window.surface_transform();
            eprintln!(
                "ira-overlay-ui: surface-transform {nx},{ny} winsize={}x{}",
                s.window.width(),
                s.window.height()
            );
        }
    }
    // Live system cursor (~2Hz): theme, size, hotspot and game customs
    // come from the server instead of the static file. Serial-gated, so
    // a static cursor costs one query per interval and nothing more.
    if s.frame_no.is_multiple_of(125) {
        poll_live_cursor(s);
    }
    s.frame_no += 1;
}

fn snapshot_frame(s: &mut State) {
    let Some(renderer) = &s.renderer else {
        return;
    };
    // Snapshot the window itself at its allocation: a fixed-size
    // snapshot of the smaller root box would stretch pixels away
    // from layout (and input) coordinates.
    match renderer.render(s.window.upcast_ref(), s.frame_no) {
        Some((w, h, px)) => {
            if std::env::var_os("IRA_OVERLAY_DEBUG").is_some() {
                eprintln!("ira-overlay-ui: frame {} {w}x{h}", s.frame_no);
            }
            if let Err(e) = s.canvas.write_frame(w, h, &px) {
                eprintln!("ira-overlay-ui: write_frame failed: {e}");
            }
        }
        None => {
            if std::env::var_os("IRA_OVERLAY_DEBUG").is_some() {
                eprintln!("ira-overlay-ui: frame {} no pixels", s.frame_no);
            }
        }
    }
}
