//! Wayland protocol glue for the stub compositor.
//!
//! Every `Dispatch`/`GlobalDispatch` impl lives here so `stub.rs` stays the
//! lifecycle/input story: starting the compositor, injecting pointer input,
//! and the shared send state. All of this runs on the compositor thread,
//! except the tick thread's synchronous sends through `SEND`.

use wayland_protocols::xdg::shell::server::{
    xdg_positioner::XdgPositioner, xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel,
    xdg_wm_base::XdgWmBase,
};
use wayland_server::Resource;
use wayland_server::backend::ObjectId;
use wayland_server::protocol::{
    wl_buffer::WlBuffer, wl_callback::WlCallback, wl_compositor::WlCompositor,
    wl_data_device::WlDataDevice, wl_data_device_manager::WlDataDeviceManager,
    wl_data_source::WlDataSource, wl_keyboard::WlKeyboard, wl_output::WlOutput,
    wl_pointer::WlPointer, wl_region::WlRegion, wl_seat::WlSeat, wl_shm::WlShm,
    wl_shm_pool::WlShmPool, wl_surface::WlSurface, wl_touch::WlTouch,
};
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New};

use super::stub::{SEND, StubState, SurfaceData, frame_now_ms};

macro_rules! empty_dispatch {
    ($($ty:ty),*) => {
        $(
            impl Dispatch<$ty, ()> for StubState {
                fn request(
                    _state: &mut Self,
                    _client: &Client,
                    _resource: &$ty,
                    _request: <$ty as wayland_server::Resource>::Request,
                    _data: &(),
                    _dh: &DisplayHandle,
                    _data_init: &mut DataInit<'_, Self>,
                ) {
                }
            }
        )*
    };
}

empty_dispatch!(
    WlBuffer,
    WlRegion,
    WlCallback,
    WlPointer,
    WlKeyboard,
    WlTouch,
    WlOutput,
    WlDataSource,
    WlDataDevice,
    XdgToplevel,
    XdgPositioner
);

/// Virtual output size: covers the canvas max extent.
const OUTPUT_W: i32 = ira_overlay_ipc::CANVAS_MAX_W as i32;
const OUTPUT_H: i32 = ira_overlay_ipc::CANVAS_MAX_H as i32;

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
