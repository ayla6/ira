//! Stub Wayland compositor for the injected host.
//!
//! Snapshots require a mapped widget, and a mapped widget requires a shown
//! surface — so the host brings its own display server instead of showing
//! anything on the user's desktop. This stub speaks just enough Wayland to
//! map one window: it ACKs surfaces, sends a single xdg configure, and
//! completes frame callbacks immediately. Pixels never travel here; frames
//! leave via snapshots. Pure Rust, no processes, no system services.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wayland_protocols::xdg::shell::server::{
    xdg_positioner::XdgPositioner, xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel,
    xdg_wm_base::XdgWmBase,
};
use wayland_server::backend::ObjectId;
use wayland_server::Resource;
use wayland_server::protocol::{
    wl_buffer::WlBuffer, wl_callback::WlCallback, wl_compositor::WlCompositor,
    wl_data_device::WlDataDevice, wl_data_device_manager::WlDataDeviceManager,
    wl_data_source::WlDataSource, wl_keyboard::WlKeyboard, wl_output::WlOutput,
    wl_pointer::WlPointer, wl_region::WlRegion, wl_seat::WlSeat, wl_shm::WlShm,
    wl_shm_pool::WlShmPool, wl_surface::WlSurface, wl_touch::WlTouch,
};
use wayland_server::{Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, New};

/// Virtual output size: covers the canvas max extent.
const OUTPUT_W: i32 = ira_overlay_ipc::CANVAS_MAX_W as i32;
const OUTPUT_H: i32 = ira_overlay_ipc::CANVAS_MAX_H as i32;

struct SurfaceData {
    frames: Vec<WlCallback>,
    configured: bool,
}

struct StubState {
    surfaces: HashMap<ObjectId, SurfaceData>,
}

/// Live pointer endpoints shared between the compositor thread (which
/// binds them on client requests) and the host tick thread (which sends
/// input on them). wayland-server resources send from any thread, so the
/// tick injects synchronously instead of queueing for the compositor
/// poll: one less thread hop and ~4ms of jitter on every input event.
struct SendState {
    display: Option<DisplayHandle>,
    pointer: Option<WlPointer>,
    focus: Option<WlSurface>,
    entered: bool,
    pos: Option<(f32, f32)>,
    serial: u32,
}

static SEND: Mutex<SendState> = Mutex::new(SendState {
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

    let socket =
        wayland_server::ListeningSocket::bind_auto("ira-overlay", 0..99).ok()?;
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

fn frame_now_ms() -> u32 {
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

fn complete_frames(state: &mut StubState, surface: &WlSurface) {
    let id = surface.id();
    if let Some(data) = state.surfaces.get_mut(&id) {
        for cb in data.frames.drain(..) {
            cb.done(frame_now_ms());
        }
    }
}

// ─── wl_compositor ───

impl GlobalDispatch<WlCompositor, ()> for StubState {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<WlCompositor>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init::<WlCompositor, ()>(resource, ());
    }
}

impl Dispatch<WlCompositor, ()> for StubState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &WlCompositor,
        request: <WlCompositor as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use wayland_server::protocol::wl_compositor::Request;
        match request {
            Request::CreateSurface { id } => {
                let surface = data_init.init::<WlSurface, ()>(id, ());
                SEND.lock().unwrap().focus = Some(surface.clone());
                state.surfaces.insert(
                    surface.id(),
                    SurfaceData {
                        frames: Vec::new(),
                        configured: false,
                    },
                );
            }
            Request::CreateRegion { id } => {
                data_init.init::<WlRegion, ()>(id, ());
            }
            _ => {}
        }
    }
}

// ─── wl_surface ───

impl Dispatch<WlSurface, ()> for StubState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &WlSurface,
        request: <WlSurface as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use wayland_server::protocol::wl_surface::Request;
        match request {
            Request::Attach { .. }
            | Request::Damage { .. }
            | Request::DamageBuffer { .. }
            | Request::SetOpaqueRegion { .. }
            | Request::SetInputRegion { .. }
            | Request::SetBufferScale { .. }
            | Request::SetBufferTransform { .. }
            | Request::Offset { .. } => {}
            Request::Frame { callback } => {
                let cb = data_init.init::<WlCallback, ()>(callback, ());
                if let Some(data) = state.surfaces.get_mut(&resource.id()) {
                    data.frames.push(cb);
                }
            }
            Request::Commit => {
                complete_frames(state, resource);
            }
            Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(
        state: &mut Self,
        _client_id: wayland_server::backend::ClientId,
        resource: &WlSurface,
        _data: &(),
    ) {
        state.surfaces.remove(&resource.id());
    }
}

// ─── wl_shm ───

impl GlobalDispatch<WlShm, ()> for StubState {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<WlShm>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let shm = data_init.init::<WlShm, ()>(resource, ());
        use wayland_server::protocol::wl_shm::Format;
        shm.format(Format::Argb8888);
        shm.format(Format::Xrgb8888);
    }
}

impl Dispatch<WlShm, ()> for StubState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlShm,
        request: <WlShm as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use wayland_server::protocol::wl_shm::Request;
        if let Request::CreatePool { id, .. } = request {
            data_init.init::<WlShmPool, ()>(id, ());
        }
    }
}

impl Dispatch<WlShmPool, ()> for StubState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlShmPool,
        request: <WlShmPool as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use wayland_server::protocol::wl_shm_pool::Request;
        match request {
            Request::CreateBuffer { id, .. } => {
                data_init.init::<WlBuffer, ()>(id, ());
            }
            Request::Destroy | Request::Resize { .. } => {}
            _ => {}
        }
    }
}

impl Dispatch<WlBuffer, ()> for StubState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlBuffer,
        _request: <WlBuffer as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<WlRegion, ()> for StubState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlRegion,
        _request: <WlRegion as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<WlCallback, ()> for StubState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlCallback,
        _request: <WlCallback as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

// ─── wl_seat (pointer only: the host injects pointer motion, buttons
// and scroll; keyboard stays tier-1 via widget focus calls) ───

impl GlobalDispatch<WlSeat, ()> for StubState {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<WlSeat>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let seat = data_init.init::<WlSeat, ()>(resource, ());
        use wayland_server::protocol::wl_seat::Capability;
        seat.capabilities(Capability::Pointer);
        seat.name("ira-stub".to_string());
    }
}

impl Dispatch<WlSeat, ()> for StubState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlSeat,
        request: <WlSeat as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use wayland_server::protocol::wl_seat::Request;
        match request {
            Request::GetPointer { id } => {
                SEND.lock().unwrap().pointer = Some(data_init.init::<WlPointer, ()>(id, ()));
            }
            Request::GetKeyboard { id } => {
                data_init.init::<WlKeyboard, ()>(id, ());
            }
            Request::GetTouch { id } => {
                data_init.init::<WlTouch, ()>(id, ());
            }
            Request::Release => {}
            _ => {}
        }
    }
}

impl Dispatch<WlPointer, ()> for StubState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlPointer,
        _request: <WlPointer as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<WlKeyboard, ()> for StubState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlKeyboard,
        _request: <WlKeyboard as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<WlTouch, ()> for StubState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlTouch,
        _request: <WlTouch as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

// ─── wl_output ───

impl GlobalDispatch<WlOutput, ()> for StubState {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<WlOutput>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let output = data_init.init::<WlOutput, ()>(resource, ());
        use wayland_server::protocol::wl_output::{Mode, Subpixel, Transform};
        output.geometry(
            0,
            0,
            160,
            100,
            Subpixel::Unknown,
            "ira-stub".to_string(),
            "stub-output".to_string(),
            Transform::Normal,
        );
        output.scale(1);
        output.mode(Mode::Current, OUTPUT_W, OUTPUT_H, 60000);
        output.done();
    }
}

impl Dispatch<WlOutput, ()> for StubState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlOutput,
        _request: <WlOutput as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

// ─── wl_data_device_manager (mandatory global; clipboard/dnd unused) ───

impl GlobalDispatch<WlDataDeviceManager, ()> for StubState {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<WlDataDeviceManager>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init::<WlDataDeviceManager, ()>(resource, ());
    }
}

impl Dispatch<WlDataDeviceManager, ()> for StubState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlDataDeviceManager,
        request: <WlDataDeviceManager as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use wayland_server::protocol::wl_data_device_manager::Request;
        match request {
            Request::CreateDataSource { id } => {
                data_init.init::<WlDataSource, ()>(id, ());
            }
            Request::GetDataDevice { id, .. } => {
                data_init.init::<WlDataDevice, ()>(id, ());
            }
            _ => {}
        }
    }
}

impl Dispatch<WlDataSource, ()> for StubState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlDataSource,
        _request: <WlDataSource as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<WlDataDevice, ()> for StubState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlDataDevice,
        _request: <WlDataDevice as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

// ─── xdg_shell ───

impl GlobalDispatch<XdgWmBase, ()> for StubState {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<XdgWmBase>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init::<XdgWmBase, ()>(resource, ());
    }
}

impl Dispatch<XdgWmBase, ()> for StubState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &XdgWmBase,
        request: <XdgWmBase as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use wayland_protocols::xdg::shell::server::xdg_wm_base::Request;
        match request {
            Request::GetXdgSurface { id, surface } => {
                data_init.init::<XdgSurface, ObjectId>(id, surface.id());
            }
            Request::CreatePositioner { id } => {
                data_init.init::<XdgPositioner, ()>(id, ());
            }
            Request::Pong { .. } => {}
            Request::Destroy => {}
            _ => {}
        }
    }
}

impl Dispatch<XdgSurface, ObjectId> for StubState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &XdgSurface,
        request: <XdgSurface as wayland_server::Resource>::Request,
        data: &ObjectId,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use wayland_protocols::xdg::shell::server::xdg_surface::Request;
        match request {
            Request::GetToplevel { id } => {
                let toplevel = data_init.init::<XdgToplevel, ()>(id, ());
                // Initial configure up front: legal before first commit and
                // sufficient for mapping.
                let serial = {
                    let mut send = SEND.lock().unwrap();
                    send.serial += 1;
                    send.serial
                };
                toplevel.configure(0, 0, vec![]);
                resource.configure(serial);
                if let Some(surf) = state.surfaces.get_mut(data) {
                    surf.configured = true;
                }
            }
            Request::GetPopup { .. } => {}
            Request::SetWindowGeometry { .. } => {}
            Request::AckConfigure { .. } => {}
            Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(
        state: &mut Self,
        _client_id: wayland_server::backend::ClientId,
        _resource: &XdgSurface,
        data: &ObjectId,
    ) {
        state.surfaces.remove(data);
    }
}

impl Dispatch<XdgToplevel, ()> for StubState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &XdgToplevel,
        _request: <XdgToplevel as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        // All toplevel requests (title, states, move/resize, minimize…)
        // are no-ops: there is nothing to show.
    }
}

impl Dispatch<XdgPositioner, ()> for StubState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &XdgPositioner,
        _request: <XdgPositioner as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
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
}
