//! Stub Wayland compositor for the injected host.
//!
//! Snapshots require a mapped widget, and a mapped widget requires a shown
//! surface — so the host brings its own display server instead of showing
//! anything on the user's desktop. This stub speaks just enough Wayland to
//! map one window: it ACKs surfaces, sends a single xdg configure, and
//! completes frame callbacks immediately. Pixels never travel here; frames
//! leave via snapshots. Pure Rust, no processes, no system services.
//!
//! Protocol glue lives in `stub_dispatch.rs`; this file owns the lifecycle
//! (starting the compositor), the shared send state, and input injection.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wayland_protocols::xdg::shell::server::xdg_wm_base::XdgWmBase;
use wayland_server::backend::ObjectId;
use wayland_server::protocol::{
    wl_callback::WlCallback, wl_compositor::WlCompositor,
    wl_data_device_manager::WlDataDeviceManager, wl_output::WlOutput, wl_pointer::WlPointer,
    wl_seat::WlSeat, wl_shm::WlShm, wl_surface::WlSurface,
};
use wayland_server::{Display, DisplayHandle};

pub(super) struct SurfaceData {
    pub(super) frames: Vec<WlCallback>,
    pub(super) configured: bool,
}

pub(super) struct StubState {
    pub(super) surfaces: HashMap<ObjectId, SurfaceData>,
}

/// Live pointer endpoints shared between the compositor thread (which
/// binds them on client requests) and the host tick thread (which sends
/// input on them). wayland-server resources send from any thread, so the
/// tick injects synchronously instead of queueing for the compositor
/// poll: one less thread hop and ~4ms of jitter on every input event.
pub(super) struct SendState {
    pub(super) display: Option<DisplayHandle>,
    pub(super) pointer: Option<WlPointer>,
    pub(super) focus: Option<WlSurface>,
    pub(super) entered: bool,
    pub(super) pos: Option<(f32, f32)>,
    pub(super) serial: u32,
}

pub(super) static SEND: Mutex<SendState> = Mutex::new(SendState {
    display: None,
    pointer: None,
    focus: None,
    entered: false,
    pos: None,
    serial: 0,
});

/// Input injected by the host from layer commands. Delivered immediately
/// as real Wayland pointer events, so GTK produces genuine hover,
/// prelight, press and scroll states.
pub enum InjectedInput {
    Motion { x: f32, y: f32 },
    Button { x11_button: u32, down: bool },
    Scroll { dy: f32 },
}

/// Sends pointer input on the calling thread and flushes it to the client.
/// Thread-safe; coordinates are host-window (canvas) pixels, 1:1. Drops
/// input while the window isn't mapped yet (no pointer/focus bound).
pub fn inject(input: InjectedInput) {
    use wayland_server::protocol::wl_pointer::{Axis, AxisSource, ButtonState};
    let mut st = SEND.lock().unwrap();
    let (Some(pointer), Some(focus)) = (st.pointer.clone(), st.focus.clone()) else {
        return;
    };
    // Focus follows the single window; GTK needs one enter ever.
    if !st.entered {
        st.serial += 1;
        pointer.enter(st.serial, &focus, 0.0, 0.0);
        st.entered = true;
    }
    match input {
        InjectedInput::Motion { x, y } => {
            st.pos = Some((x, y));
            pointer.motion(frame_now_ms(), x as f64, y as f64);
            pointer.frame();
        }
        InjectedInput::Button { x11_button, down } => {
            // Real compositors always move before pressing: pair the
            // button with a motion to the last known position first.
            if let Some((x, y)) = st.pos {
                pointer.motion(frame_now_ms(), x as f64, y as f64);
            }
            st.serial += 1;
            pointer.button(
                st.serial,
                frame_now_ms(),
                x11_to_evdev(x11_button),
                if down {
                    ButtonState::Pressed
                } else {
                    ButtonState::Released
                },
            );
            pointer.frame();
        }
        InjectedInput::Scroll { dy } => {
            pointer.axis_source(AxisSource::Wheel);
            pointer.axis(frame_now_ms(), Axis::VerticalScroll, f64::from(dy) * 24.0);
            pointer.frame();
        }
    }
    if let Some(dh) = st.display.as_mut() {
        let _ = dh.flush_clients();
    }
}

struct StubClient;

impl wayland_server::backend::ClientData for StubClient {}

/// Starts the stub on an auto-numbered socket and points this process at
/// it. Returns the socket name for `WAYLAND_DISPLAY`, or `None` when the
/// runtime dir is unusable.
pub fn start() -> Option<String> {
    let display: Display<StubState> = Display::new().ok()?;
    let dh = display.handle();

    dh.create_global::<StubState, WlCompositor, _>(4, ());
    dh.create_global::<StubState, WlShm, _>(1, ());
    dh.create_global::<StubState, WlSeat, _>(7, ());
    dh.create_global::<StubState, WlOutput, _>(2, ());
    dh.create_global::<StubState, WlDataDeviceManager, _>(3, ());
    dh.create_global::<StubState, XdgWmBase, _>(3, ());

    let socket = wayland_server::ListeningSocket::bind_auto("ira-overlay", 0..99).ok()?;
    let name = socket.socket_name()?.to_string_lossy().into_owned();
    if name != "ira-overlay-0" {
        eprintln!(
            "ira-overlay-ui: stub on {name} (ira-overlay-0 busy): a previous host may still \
             be running and fighting over the canvas"
        );
    }

    let state = StubState {
        surfaces: HashMap::new(),
    };
    // The tick thread sends input events directly; it flushes through
    // this handle without joining the compositor loop.
    SEND.lock().unwrap().display = Some(dh.clone());
    std::thread::spawn(move || {
        use std::sync::Arc;
        let mut display = display;
        let mut state = state;
        let socket = socket;
        let mut dh = display.handle();
        loop {
            while let Ok(Some(stream)) = socket.accept() {
                let _ = dh.insert_client(stream, Arc::new(StubClient));
            }
            if display.dispatch_clients(&mut state).is_err() {
                break;
            }
            if display.flush_clients().is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(4));
        }
    });

    unsafe {
        std::env::set_var("WAYLAND_DISPLAY", &name);
        std::env::set_var("GDK_BACKEND", "wayland");
        std::env::remove_var("DISPLAY");
    }
    eprintln!("ira-overlay-ui: stub compositor on {name} (no desktop window)");
    Some(name)
}

pub(super) fn frame_now_ms() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u32)
        .unwrap_or(0)
}

/// X11 button numbers to evdev codes for wl_pointer.button.
fn x11_to_evdev(button: u32) -> u32 {
    match button {
        1 => 0x110,
        2 => 0x112,
        3 => 0x111,
        _ => 0x110,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_clock_is_monotonic_ms() {
        let a = frame_now_ms();
        assert!(frame_now_ms() >= a);
    }

    #[test]
    fn test_x11_to_evdev_maps_buttons() {
        // Left/middle/right map to their evdev codes; anything else (wheel
        // included) falls back to left.
        assert_eq!(x11_to_evdev(1), 0x110);
        assert_eq!(x11_to_evdev(2), 0x112);
        assert_eq!(x11_to_evdev(3), 0x111);
        assert_eq!(x11_to_evdev(4), 0x110);
        assert_eq!(x11_to_evdev(8), 0x110);
    }
}
