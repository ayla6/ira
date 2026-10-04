//! Frame-loop tick for the overlay host.
//!
//! Owns everything that happens after the window exists: draining desktop
//! pointer reports, refreshing game data, following gamescope visibility,
//! applying layer commands, and deciding when to snapshot the next frame.
//! Window construction lives in `host.rs`, startup in `main.rs`. The borrow
//! discipline matters here — see `tick_inner` — so keep the phases split.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use glib::object::Cast as _;
use gtk4::prelude::{GtkWindowExt, WidgetExt};
use ira_overlay_ipc::{CanvasCommand, fit_panel_size};

use super::ui::{OverlayUi, UiEffect};
use super::{State, cursor, game, gamescope, grab};

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
pub(super) fn apply_hide(s: &mut State) {
    s.canvas.header_mut().visible.store(0, Ordering::SeqCst);
    // Mirror into the game SHM: the freeze watcher (and a thawed layer)
    // follows overlay_visible, not the canvas flag. Idempotent with the
    // layer's own mirror.
    if let Some(shm) = &mut s.game {
        shm.header_mut().overlay_visible.store(0, Ordering::SeqCst);
        eprintln!("ira-overlay-ui: hide mirrored into game SHM");
    }
}

pub(super) fn tick(state: &Rc<RefCell<Option<State>>>) {
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
    let fp = game::fingerprint(shm);
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
