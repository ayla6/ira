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

mod cursor;
mod game;
mod gamescope;
mod render;
mod stub;
mod ui;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use gtk4::prelude::{ApplicationExt, ApplicationExtManual, GtkWindowExt, WidgetExt};
use glib::object::Cast as _;
use ira_overlay_ipc::{
    canvas_shm_path, fit_panel_size, CanvasAction, CanvasShm, MappedShm, CANVAS_VERSION,
};

use game::{fingerprint, Fingerprint};
use render::{FrameRenderer, CANVAS_H, CANVAS_W};
use ui::{OverlayUi, UiEffect};

struct State {
    canvas: CanvasShm,
    game: Option<MappedShm>,
    ui: OverlayUi,
    renderer: Option<FrameRenderer>,
    window: gtk4::Window,
    cmd_read: u32,
    last_fp: Fingerprint,
    last_visible: bool,
    /// Last applied window size: the window follows the game extent.
    last_panel_size: (u32, u32),
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
    // Injected mode renders on our own stub compositor: zero visible
    // output anywhere (snapshots require a mapped window).
    // Gamescope mode keeps its real window — the compositor is its display.
    let stub_name = stub_or_none();
    // Mock flag is env, not argv: GTK/Adwaita claim unknown CLI flags.
    let mock_mode = std::env::var_os("IRA_OVERLAY_UI_MOCK").is_some();
    let (game, mock, db_id) = match std::env::var("IRA_OVERLAY_SHM") {
        Ok(path) => match MappedShm::open(&path) {
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

    let app = adw::Application::new(
        Some("org.ira.OverlayUi"),
        gio::ApplicationFlags::FLAGS_NONE,
    );
    // connect_activate takes Fn: hand owned state over via take-once cells.
    let game = RefCell::new(Some(game));
    let mock = RefCell::new(mock);
    let canvas = RefCell::new(Some(canvas));
    app.connect_activate(move |app| {
        let (Some(game), mock, Some(canvas)) = (game.take(), mock.take(), canvas.take()) else {
            return;
        };
        activate(app, game, mock, canvas, stub_name.is_none());
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

/// Display size published by Ira (`IRA_OVERLAY_SCREEN_W/H`), if any.
fn screen_size_from_env() -> Option<(u32, u32)> {
    let w: u32 = std::env::var("IRA_OVERLAY_SCREEN_W").ok()?.parse().ok()?;
    let h: u32 = std::env::var("IRA_OVERLAY_SCREEN_H").ok()?.parse().ok()?;
    (w > 0 && h > 0).then_some((w, h))
}

/// Builds the helper window + UI and starts the 30Hz frame loop.
/// Takes `game`/`canvas` by value — the activate closure runs once.
fn activate(
    app: &adw::Application,
    game: Option<MappedShm>,
    mock: Option<game::GameData>,
    mut canvas: CanvasShm,
    no_stub: bool,
) {
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::PreferDark);
    publish_theme_cursor(&mut canvas);
    // Transparent window: the compositor (gamescope plane or Vulkan layer)
    // blends the panel over the game; outside the widgets is see-through.
    if let Some(display) = gdk4::Display::default() {
        let css = gtk4::CssProvider::new();
        css.load_from_string(
            "window { background-color: transparent; } \
             .ira-focused-row { background-color: alpha(@theme_fg_color, 0.10); border-radius: 9px; }",
        );
        gtk4::style_context_add_provider_for_display(
            &display,
            &css,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    let window = gtk4::Window::new();
    window.set_application(Some(app));
    window.set_title(Some("Ira Overlay"));
    window.set_decorated(false);
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
    let gamescope = gamescope::is_gamescope_mode();
    if !gamescope {
        // The injected helper is never user-facing: bury it below other
        // windows (a fullscreen game covers it regardless).
        gamescope::background_helper(&window);
    }

    let state = Rc::new(RefCell::new(None::<State>));
    let push_action = {
        let state = state.clone();
        Rc::new(move |act: CanvasAction| {
            if let Some(s) = state.borrow_mut().as_mut() {
                s.canvas.push_action(act);
            }
        })
    };
    let mut ui = OverlayUi::new(push_action);
    let initial = match (&game, &mock) {
        (Some(shm), _) => game::load(shm),
        (None, Some(m)) => m.clone(),
        (None, None) => game::GameData::mock(),
    };
    ui.rebuild(&initial);
    window.set_child(Some(ui.root()));
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

    let last_fp = game.as_ref().map(fingerprint).unwrap_or((0, 0, 0, 0));
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
        last_frame_render: std::time::Instant::now(),
        tail_until: std::time::Instant::now(),
        last_render_ack: 0,
        last_cursor_serial: None,
        dirty: true,
        frame_no: 0,
        gamescope,
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

fn tick(state: &Rc<RefCell<Option<State>>>) {
    // Phase 1 (borrowed): drain commands and inject input synchronously.
    // The borrow ends before the event pump below: pumped handlers
    // (button clicks) push actions through this same state borrow.
    let (pointer_input, moved, fresh) = {
        let mut guard = state.borrow_mut();
        let Some(s) = guard.as_mut() else { return };

        // Fresh (non-motion) work below marks `fresh`: it renders now and
        // opens the transition window for continuation frames.
        let mut fresh = false;
        if let Some(shm) = &s.game {
            let fp = fingerprint(shm);
            if fp != s.last_fp {
                s.ui.rebuild(&game::load(shm));
                s.last_fp = fp;
                s.dirty = true;
                fresh = true;
            }
            // Gamescope shows the window itself: follow the toggle the layer
            // mirrors into the game SHM. Raising on show gives it input focus.
            if s.gamescope {
                let visible = shm.header().overlay_visible.load(Ordering::SeqCst) != 0;
                if visible != s.last_visible {
                    s.window.set_visible(visible);
                    if visible {
                        s.window.present();
                    }
                    s.last_visible = visible;
                }
            }
        }

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
        // Pointer injects straight into the compositor and dispatches
        // below in this same tick. Non-motion work renders immediately;
        // motion is noted for the repaint rule below.
        let mut pointer_input = false;
        let mut moved = false;
        for cmd in cmds {
            use ira_overlay_ipc::{CMD_MOUSE_DOWN, CMD_MOUSE_MOVE, CMD_MOUSE_UP};
            match cmd.kind {
                CMD_MOUSE_MOVE => {
                    pointer_input = true;
                    moved = true;
                }
                CMD_MOUSE_DOWN | CMD_MOUSE_UP => {
                    pointer_input = true;
                    s.dirty = true;
                    fresh = true;
                }
                _ => {
                    s.dirty = true;
                    fresh = true;
                }
            }
            match s.ui.apply(cmd) {
                UiEffect::None => {}
                UiEffect::Show => {
                    s.canvas.header_mut().visible.store(1, Ordering::SeqCst);
                }
                UiEffect::Hide => {
                    s.canvas.header_mut().visible.store(0, Ordering::SeqCst);
                }
            }
        }
        (pointer_input, moved, fresh)
    };

    // Phase 2 (unborrowed): dispatch the injected events synchronously so
    // the render below snapshots post-input state in the same tick.
    if pointer_input {
        pump_events();
    }

    // Phase 3 (borrowed): render decision + render.
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
        if let Some(renderer) = &s.renderer {
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
        s.dirty = false;
    }
    // Post-map layout truth (rebuild runs pre-layout): logs button
    // bounds once real values exist. Change-gated inside.
    if s.frame_no == 2 {
        s.ui.log_layout();
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
    if s.frame_no % 125 == 0 {
        poll_live_cursor(s);
    }
    s.frame_no += 1;
}
