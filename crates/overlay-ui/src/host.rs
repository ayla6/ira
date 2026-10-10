//! Helper window + UI construction for the overlay host.
//!
//! Takes over from `main.rs` once the canvas exists: builds the window and
//! widgets, waits for the surface to map, and starts the `tick.rs` frame
//! loop. Takes `game`/`canvas` by value — the activate closure runs once.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk4::prelude::{GtkWindowExt, WidgetExt};
use ira_overlay_ipc::{CanvasAction, CanvasShm, MappedShm, fit_panel_size};

use super::render::{CANVAS_H, CANVAS_W, FrameRenderer};
use super::tick::{apply_hide, tick};
use super::ui::OverlayUi;
use super::{State, game, gamescope, grab, proc};

/// Startup size from the display (published by Ira; inherited through
/// spawn) so the first frame is already right: fit the screen the same
/// way the per-tick resize does, defaulting to 512x800 without it.
fn startup_window_size() -> (i32, i32) {
    match super::screen_size_from_env() {
        Some((sw, sh)) => {
            let (w, h) = fit_panel_size(sw, sh);
            (w as i32, h as i32)
        }
        None => (CANVAS_W as i32, CANVAS_H as i32),
    }
}

/// Pumps the main context until the window maps (spike precedent).
/// Non-blocking iterations only: iteration(true) would wait forever on
/// a dead compositor thread, hanging startup with zero further output.
/// False when the surface never appears.
fn wait_window_mapped(window: &gtk4::Window) -> bool {
    // The renderer needs the mapped surface; the window was just presented.
    let ctx = glib::MainContext::default();
    let start = std::time::Instant::now();
    while !window.is_mapped() {
        if start.elapsed() > Duration::from_secs(5) {
            eprintln!("ira-overlay-ui: helper window never mapped");
            return false;
        }
        ctx.iteration(false);
        std::thread::sleep(Duration::from_millis(10));
    }
    true
}

/// Binds the snapshot renderer for injected mode (`None` in gamescope
/// mode, where the window itself is the overlay plane). `None` in
/// injected mode is fatal: without a renderer no frame ever publishes.
fn make_renderer(window: &gtk4::Window, gamescope: bool, no_stub: bool) -> Option<FrameRenderer> {
    if gamescope {
        // Gamescope shows the window itself; only the injected path snapshots it.
        return None;
    }
    let renderer = match FrameRenderer::for_window(window) {
        Some(r) => Some(r),
        None => {
            eprintln!("ira-overlay-ui: no GSK renderer for helper surface");
            return None;
        }
    };
    // The window maps on the stub compositor (invisible anywhere), so only
    // the real-desktop fallback minimizes the helper.
    if no_stub {
        window.minimize();
    }
    renderer
}

/// Forwards widget actions to the canvas. Exit Game also terminates the
/// session leader; its cleanup reaps the game and this host with it.
/// Best-effort: without a session ancestor there is nothing to exit
/// here — but the action is still forwarded below, and the in-process
/// layer terminates the game itself on receipt.
fn action_sink(state: &Rc<RefCell<Option<State>>>) -> Rc<dyn Fn(CanvasAction)> {
    let state = state.clone();
    Rc::new(move |act: CanvasAction| {
        if act.kind == ira_overlay_ipc::ACT_EXIT_GAME
            && let Some(pid) = proc::session_leader_pid()
        {
            eprintln!("ira-overlay-ui: exiting game (session leader pid={pid})");
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
        } else if act.kind == ira_overlay_ipc::ACT_EXIT_GAME {
            eprintln!("ira-overlay-ui: exit game requested, no session leader found");
        }
        if let Some(s) = state.borrow_mut().as_mut() {
            s.canvas.push_action(act);
        }
    })
}

fn hide_sink(state: &Rc<RefCell<Option<State>>>) -> Rc<dyn Fn()> {
    let state = state.clone();
    Rc::new(move || {
        if let Some(s) = state.borrow_mut().as_mut() {
            apply_hide(s);
        }
    })
}

pub(super) fn activate(
    app: &adw::Application,
    game: Option<MappedShm>,
    mock: Option<game::GameData>,
    mut canvas: CanvasShm,
    no_stub: bool,
    input_rx: Option<std::sync::mpsc::Receiver<grab::PointerEvent>>,
) {
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::PreferDark);
    super::publish_theme_cursor(&mut canvas);
    // Same stylesheet as big picture (1.0 = the 1080p reference): the
    // overlay speaks the same visual language instead of a hand-rolled
    // clone. Injected mode renders on our own stub compositor and stays
    // un-dimmed: the renderer snapshots the window and the layer blends
    // it over the game.
    let gamescope = gamescope::is_gamescope_mode();
    ira_widgets::style::install_bp_css(1.0);
    // Controller glyphs ride the binary's GResource like the app's own
    // icons, so prompts resolve by name on every display.
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::IconTheme::for_display(&display).add_resource_path("/com/github/ira-overlay/icons");
    }

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
    let (win_w, win_h) = startup_window_size();
    window.set_default_size(win_w, win_h);
    if !gamescope {
        // The injected helper is never user-facing: bury it below other
        // windows (a fullscreen game covers it regardless).
        gamescope::background_helper(&window);
    }

    let state = Rc::new(RefCell::new(None::<State>));
    let ui = OverlayUi::new(action_sink(&state), hide_sink(&state), gamescope);
    let initial = match (&game, &mock) {
        (Some(shm), _) => game::load(shm),
        (None, Some(m)) => m.clone(),
        (None, None) => game::GameData::mock(),
    };
    ui.borrow_mut().rebuild(&initial);
    window.set_child(Some(ui.borrow().root()));
    window.present();

    if !wait_window_mapped(&window) {
        return;
    }
    if gamescope {
        gamescope::setup_external_overlay(&window);
    }
    let renderer = make_renderer(&window, gamescope, no_stub);
    if !gamescope && renderer.is_none() {
        return;
    }

    let last_fp = game.as_ref().map(game::fingerprint);
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
