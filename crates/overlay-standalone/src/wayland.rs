//! Wayland connection + wlr_layer_shell surface for the standalone overlay.
//!
//! Uses raw FFI (dlopen libwayland-client.so.0) consistent with overlay-vk's wayland.rs.
//! Manually constructs the `zwlr_layer_shell_v1` and `zwlr_layer_surface_v1` interface
//! structs since they're not part of standard libwayland-client.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::sync::atomic::{AtomicI32, AtomicPtr, AtomicU32, Ordering};

use ira_overlay::ui::{push_event, Event};

// ─── Wayland constants ───

const WL_SEAT_CAP_KEYBOARD: u32 = 2;
const WL_SEAT_CAP_POINTER: u32 = 1;

const KEY_PRESSED: u32 = 1;
const SHIFT_MASK: u32 = 1;

const KC_TAB: u32 = 15;
const KC_RETURN: u32 = 28;
const KC_UP: u32 = 103;
const KC_DOWN: u32 = 108;
const KC_LEFT: u32 = 105;
const KC_RIGHT: u32 = 106;

const AXIS_VERTICAL: u32 = 0;

const LAYER_TOP: u32 = 2;
const ANCHOR_ALL: u32 = 1 | 2 | 4 | 8; // top | bottom | left | right

// ─── FFI types ───

#[repr(C)]
struct WlInterface {
    name: *const c_char,
    version: i32,
    method_count: i32,
    methods: *const WlMessage,
    event_count: i32,
    events: *const WlMessage,
}

unsafe impl Sync for WlInterface {}

#[repr(C)]
struct WlMessage {
    name: *const c_char,
    signature: *const c_char,
    types: *const *const WlInterface,
}

unsafe impl Sync for WlMessage {}

type FnConnect = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type FnDisconnect = unsafe extern "C" fn(*mut c_void);
type FnDispatchPending = unsafe extern "C" fn(*mut c_void) -> c_int;
type FnRoundtrip = unsafe extern "C" fn(*mut c_void) -> c_int;
type FnFlush = unsafe extern "C" fn(*mut c_void) -> c_int;
type FnMarshal = unsafe extern "C" fn(*mut c_void, u32, ...);
type FnMarshalCtor = unsafe extern "C" fn(*mut c_void, u32, *const WlInterface, ...) -> *mut c_void;
type FnAddListener = unsafe extern "C" fn(*mut c_void, *const *const c_void, *mut c_void) -> c_int;
type FnDestroy = unsafe extern "C" fn(*mut c_void);

#[derive(Clone, Copy)]
struct Fns {
    connect: FnConnect,
    disconnect: FnDisconnect,
    dispatch_pending: FnDispatchPending,
    roundtrip: FnRoundtrip,
    flush: FnFlush,
    marshal: FnMarshal,
    marshal_ctor: FnMarshalCtor,
    add_listener: FnAddListener,
    destroy: FnDestroy,
    compositor_iface: *const WlInterface,
    surface_iface: *const WlInterface,
    seat_iface: *const WlInterface,
    keyboard_iface: *const WlInterface,
    pointer_iface: *const WlInterface,
    registry_iface: *const WlInterface,
}

unsafe impl Send for Fns {}
unsafe impl Sync for Fns {}

// ─── wlr_layer_shell interface definitions ───

static LAYER_SHELL_METHODS: [WlMessage; 1] = [
    WlMessage {
        name: c"get_layer_surface".as_ptr(),
        signature: c"no?us".as_ptr(),
        types: std::ptr::null(), // interface passed via marshal_ctor, not needed here
    },
];

static ZWLR_LAYER_SHELL_V1_INTERFACE: WlInterface = WlInterface {
    name: c"zwlr_layer_shell_v1".as_ptr(),
    version: 1,
    method_count: 1,
    methods: LAYER_SHELL_METHODS.as_ptr(),
    event_count: 0,
    events: std::ptr::null(),
};

static LAYER_SURFACE_METHODS: [WlMessage; 8] = [
    WlMessage { name: c"set_size".as_ptr(),              signature: c"uu".as_ptr(),    types: std::ptr::null() },
    WlMessage { name: c"set_anchor".as_ptr(),             signature: c"u".as_ptr(),     types: std::ptr::null() },
    WlMessage { name: c"set_exclusive_zone".as_ptr(),     signature: c"i".as_ptr(),     types: std::ptr::null() },
    WlMessage { name: c"set_margin".as_ptr(),             signature: c"iiii".as_ptr(),  types: std::ptr::null() },
    WlMessage { name: c"set_keyboard_interactivity".as_ptr(), signature: c"u".as_ptr(), types: std::ptr::null() },
    WlMessage { name: c"get_popup".as_ptr(),              signature: c"o".as_ptr(),     types: std::ptr::null() },
    WlMessage { name: c"ack_configure".as_ptr(),          signature: c"u".as_ptr(),     types: std::ptr::null() },
    WlMessage { name: c"destroy".as_ptr(),                signature: c"".as_ptr(),      types: std::ptr::null() },
];

static LAYER_SURFACE_EVENTS: [WlMessage; 2] = [
    WlMessage { name: c"configure".as_ptr(), signature: c"uuu".as_ptr(), types: std::ptr::null() },
    WlMessage { name: c"closed".as_ptr(),    signature: c"".as_ptr(),    types: std::ptr::null() },
];

static ZWLR_LAYER_SURFACE_V1_INTERFACE: WlInterface = WlInterface {
    name: c"zwlr_layer_surface_v1".as_ptr(),
    version: 1,
    method_count: 8,
    methods: LAYER_SURFACE_METHODS.as_ptr(),
    event_count: 2,
    events: LAYER_SURFACE_EVENTS.as_ptr(),
};

// ─── Listener structs (#[repr(C)] arrays of function pointers) ───

#[repr(C)]
struct RegistryListener {
    global: Option<extern "C" fn(*mut c_void, *mut c_void, u32, *const c_char, u32)>,
    global_remove: Option<extern "C" fn(*mut c_void, *mut c_void, u32)>,
}

#[repr(C)]
struct SeatListener {
    capabilities: Option<extern "C" fn(*mut c_void, *mut c_void, u32)>,
    name: Option<extern "C" fn(*mut c_void, *mut c_void, *const c_char)>,
}

#[repr(C)]
struct KeyboardListener {
    keymap: Option<extern "C" fn(*mut c_void, *mut c_void, u32, c_int, u32)>,
    enter: Option<extern "C" fn(*mut c_void, *mut c_void, u32, *mut c_void, *mut c_void)>,
    leave: Option<extern "C" fn(*mut c_void, *mut c_void, u32, *mut c_void)>,
    key: Option<extern "C" fn(*mut c_void, *mut c_void, u32, u32, u32, u32)>,
    modifiers: Option<extern "C" fn(*mut c_void, *mut c_void, u32, u32, u32, u32, u32)>,
    repeat_info: Option<extern "C" fn(*mut c_void, *mut c_void, c_int, c_int)>,
}

#[repr(C)]
struct PointerListener {
    enter: Option<extern "C" fn(*mut c_void, *mut c_void, u32, *mut c_void, i32, i32)>,
    leave: Option<extern "C" fn(*mut c_void, *mut c_void, u32, *mut c_void)>,
    motion: Option<extern "C" fn(*mut c_void, *mut c_void, u32, i32, i32)>,
    button: Option<extern "C" fn(*mut c_void, *mut c_void, u32, u32, u32, u32)>,
    axis: Option<extern "C" fn(*mut c_void, *mut c_void, u32, u32, i32)>,
    frame: Option<extern "C" fn(*mut c_void, *mut c_void)>,
    axis_source: Option<extern "C" fn(*mut c_void, *mut c_void, u32)>,
    axis_stop: Option<extern "C" fn(*mut c_void, *mut c_void, u32, u32)>,
    axis_discrete: Option<extern "C" fn(*mut c_void, *mut c_void, u32, c_int)>,
    axis_relative_direction: Option<extern "C" fn(*mut c_void, *mut c_void, u32, u32)>,
}

#[repr(C)]
struct LayerSurfaceListener {
    configure: Option<extern "C" fn(*mut c_void, *mut c_void, u32, u32, u32)>,
    closed: Option<extern "C" fn(*mut c_void, *mut c_void)>,
}

// ─── No-op callbacks ───

extern "C" fn nop_global_remove(_: *mut c_void, _: *mut c_void, _: u32) {}
extern "C" fn nop_seat_name(_: *mut c_void, _: *mut c_void, _: *const c_char) {}
extern "C" fn nop_keymap(_: *mut c_void, _: *mut c_void, _: u32, _: c_int, _: u32) {}
extern "C" fn nop_repeat(_: *mut c_void, _: *mut c_void, _: c_int, _: c_int) {}
extern "C" fn nop_pointer_leave(_: *mut c_void, _: *mut c_void, _: u32, _: *mut c_void) {}
extern "C" fn nop_frame(_: *mut c_void, _: *mut c_void) {}
extern "C" fn nop_axis_source(_: *mut c_void, _: *mut c_void, _: u32) {}
extern "C" fn nop_axis_stop(_: *mut c_void, _: *mut c_void, _: u32, _: u32) {}
extern "C" fn nop_axis_discrete(_: *mut c_void, _: *mut c_void, _: u32, _: c_int) {}
extern "C" fn nop_axis_rel_dir(_: *mut c_void, _: *mut c_void, _: u32, _: u32) {}
extern "C" fn nop_kb_enter(_: *mut c_void, _: *mut c_void, _: u32, _: *mut c_void, _: *mut c_void) {}
extern "C" fn nop_kb_leave(_: *mut c_void, _: *mut c_void, _: u32, _: *mut c_void) {}

// ─── Listener statics ───

static REGISTRY_LISTENER: RegistryListener = RegistryListener {
    global: Some(registry_global),
    global_remove: Some(nop_global_remove),
};

static SEAT_LISTENER: SeatListener = SeatListener {
    capabilities: Some(seat_capabilities),
    name: Some(nop_seat_name),
};

static KEYBOARD_LISTENER: KeyboardListener = KeyboardListener {
    keymap: Some(nop_keymap),
    enter: Some(nop_kb_enter),
    leave: Some(nop_kb_leave),
    key: Some(keyboard_key),
    modifiers: Some(keyboard_modifiers),
    repeat_info: Some(nop_repeat),
};

static POINTER_LISTENER: PointerListener = PointerListener {
    enter: Some(pointer_enter),
    leave: Some(nop_pointer_leave),
    motion: Some(pointer_motion),
    button: Some(pointer_button),
    axis: Some(pointer_axis),
    frame: Some(nop_frame),
    axis_source: Some(nop_axis_source),
    axis_stop: Some(nop_axis_stop),
    axis_discrete: Some(nop_axis_discrete),
    axis_relative_direction: Some(nop_axis_rel_dir),
};

static LAYER_SURFACE_LISTENER: LayerSurfaceListener = LayerSurfaceListener {
    configure: Some(layer_surface_configure),
    closed: Some(layer_surface_closed),
};

// ─── Statics for initial bind ───

static MODS_DEPRESSED: AtomicU32 = AtomicU32::new(0);
static MOUSE_SX: AtomicI32 = AtomicI32::new(0);
static MOUSE_SY: AtomicI32 = AtomicI32::new(0);

// Globals discovered during registry roundtrip
static COMPOSITOR: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static SEAT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static LAYER_SHELL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

// Fns pointer stored for callbacks
static FNS: std::sync::OnceLock<Fns> = std::sync::OnceLock::new();

// ─── State ───

pub struct WaylandState {
    display: *mut c_void,
    surface: *mut c_void,
    layer_surface: *mut c_void,
    pending_resize: Option<(u32, u32)>,
}

unsafe impl Send for WaylandState {}

// ─── Registry handler ───

extern "C" fn registry_global(
    _: *mut c_void,
    _: *mut c_void,
    name: u32,
    interface: *const c_char,
    version: u32,
) {
    let Some(fns) = FNS.get() else { return };
    let iface = unsafe { CStr::from_ptr(interface) };
    let iface_name = iface.to_bytes();

    match iface_name {
        b"wl_compositor" => {
            let compositor = unsafe {
                (fns.marshal_ctor)(
                    REGISTRY_PROXY.with(|r| *r.borrow()),
                    0,
                    fns.compositor_iface,
                    name,
                    (*fns.compositor_iface).name,
                    version,
                )
            };
            COMPOSITOR.store(compositor, Ordering::Relaxed);
        }
        b"wl_seat" => {
            let seat = unsafe {
                (fns.marshal_ctor)(
                    REGISTRY_PROXY.with(|r| *r.borrow()),
                    0,
                    fns.seat_iface,
                    name,
                    (*fns.seat_iface).name,
                    version.min(7),
                )
            };
            unsafe {
                (fns.add_listener)(
                    seat,
                    &SEAT_LISTENER as *const _ as *const *const c_void,
                    std::ptr::null_mut(),
                );
            }
            SEAT.store(seat, Ordering::Relaxed);
        }
        b"zwlr_layer_shell_v1" => {
            let shell = unsafe {
                (fns.marshal_ctor)(
                    REGISTRY_PROXY.with(|r| *r.borrow()),
                    0,
                    &ZWLR_LAYER_SHELL_V1_INTERFACE,
                    name,
                    ZWLR_LAYER_SHELL_V1_INTERFACE.name,
                    version.min(1),
                )
            };
            LAYER_SHELL.store(shell, Ordering::Relaxed);
        }
        _ => {}
    }
}

extern "C" fn seat_capabilities(_: *mut c_void, seat: *mut c_void, caps: u32) {
    let Some(fns) = FNS.get() else { return };
    if caps & WL_SEAT_CAP_KEYBOARD != 0 {
        let kb = unsafe { (fns.marshal_ctor)(seat, 1, fns.keyboard_iface, std::ptr::null::<c_void>()) };
        unsafe {
            (fns.add_listener)(
                kb,
                &KEYBOARD_LISTENER as *const _ as *const *const c_void,
                std::ptr::null_mut(),
            );
        }
        KEYBOARD_OBJ.with(|k| *k.borrow_mut() = Some(kb));
    }
    if caps & WL_SEAT_CAP_POINTER != 0 {
        let ptr = unsafe { (fns.marshal_ctor)(seat, 2, fns.pointer_iface, std::ptr::null::<c_void>()) };
        unsafe {
            (fns.add_listener)(
                ptr,
                &POINTER_LISTENER as *const _ as *const *const c_void,
                std::ptr::null_mut(),
            );
        }
        POINTER_OBJ.with(|p| *p.borrow_mut() = Some(ptr));
    }
}

// ─── Keyboard/pointer callbacks ───

extern "C" fn keyboard_key(
    _: *mut c_void,
    _: *mut c_void,
    _: u32,
    _: u32,
    key: u32,
    state: u32,
) {
    let pressed = state == KEY_PRESSED;
    let shift = (MODS_DEPRESSED.load(Ordering::Relaxed) & SHIFT_MASK) != 0;

    // Shift+Tab toggle is handled by the shim (writing to SHM).
    if pressed && shift && key == KC_TAB { return; }

    if let Some(e) = match key {
        KC_UP if pressed => Some(Event::NavUp),
        KC_DOWN if pressed => Some(Event::NavDown),
        KC_LEFT if pressed => Some(Event::NavLeft),
        KC_RIGHT if pressed => Some(Event::NavRight),
        KC_RETURN if pressed => Some(Event::Activate),
        _ => None,
    } {
        push_event(e);
    }
}

extern "C" fn keyboard_modifiers(
    _: *mut c_void,
    _: *mut c_void,
    _: u32,
    mods_depressed: u32,
    _: u32,
    _: u32,
    _: u32,
) {
    MODS_DEPRESSED.store(mods_depressed, Ordering::Relaxed);
}

extern "C" fn pointer_enter(_: *mut c_void, _: *mut c_void, _: u32, _: *mut c_void, sx: i32, sy: i32) {
    MOUSE_SX.store(sx, Ordering::Relaxed);
    MOUSE_SY.store(sy, Ordering::Relaxed);
}

extern "C" fn pointer_motion(_: *mut c_void, _: *mut c_void, _: u32, sx: i32, sy: i32) {
    MOUSE_SX.store(sx, Ordering::Relaxed);
    MOUSE_SY.store(sy, Ordering::Relaxed);
    let (x, y) = (sx as f32 / 256.0, sy as f32 / 256.0);
    push_event(Event::MouseMove { x, y });
}

extern "C" fn pointer_button(_: *mut c_void, _: *mut c_void, _: u32, _: u32, _: u32, state: u32) {
    let (x, y) = (
        MOUSE_SX.load(Ordering::Relaxed) as f32 / 256.0,
        MOUSE_SY.load(Ordering::Relaxed) as f32 / 256.0,
    );
    if state == KEY_PRESSED {
        push_event(Event::MouseDown { x, y });
    } else {
        push_event(Event::MouseUp { x, y });
    }
}

extern "C" fn pointer_axis(_: *mut c_void, _: *mut c_void, _: u32, axis: u32, value: i32) {
    if axis != AXIS_VERTICAL { return; }
    let delta_y = if value > 0 { 1.0 } else { -1.0 };
    push_event(Event::Scroll { delta_y });
}

// ─── Layer surface callbacks ───

extern "C" fn layer_surface_configure(
    data: *mut c_void,
    _: *mut c_void,
    serial: u32,
    width: u32,
    height: u32,
) {
    let state = unsafe { &mut *(data as *mut WaylandState) };
    let Some(fns) = FNS.get() else { return };
    unsafe { (fns.marshal)(state.layer_surface, 6, serial) }; // ack_configure
    if width > 0 && height > 0 {
        state.pending_resize = Some((width, height));
    }
}

extern "C" fn layer_surface_closed(_data: *mut c_void, _proxy: *mut c_void) {
    eprintln!("ira-overlay-standalone: layer surface closed by compositor");
}

// ─── Thread-locals for registry proxy (used during init) ───

use std::cell::RefCell;

thread_local! {
    static REGISTRY_PROXY: RefCell<*mut c_void> = const { RefCell::new(std::ptr::null_mut()) };
    static KEYBOARD_OBJ: RefCell<Option<*mut c_void>> = const { RefCell::new(None) };
    static POINTER_OBJ: RefCell<Option<*mut c_void>> = const { RefCell::new(None) };
}

impl WaylandState {
    pub fn new() -> Result<Self, String> {
        let fns = load_library().ok_or("failed to load libwayland-client.so.0")?;
        FNS.set(fns).ok();

        // Connect to display
        let display_name = std::env::var("GAMESCOPE_WAYLAND_DISPLAY")
            .or_else(|_| std::env::var("WAYLAND_DISPLAY"))
            .unwrap_or_else(|_| "wayland-0".to_string());
        let display_c = CString::new(display_name.as_str())
            .map_err(|e| format!("invalid display name: {e}"))?;
        let display = unsafe { (fns.connect)(display_c.as_ptr()) };
        if display.is_null() {
            return Err(format!("failed to connect to Wayland display '{display_name}'"));
        }

        // Get registry
        let registry = unsafe { (fns.marshal_ctor)(display, 1, fns.registry_iface, std::ptr::null::<c_void>()) };
        if registry.is_null() {
            unsafe { (fns.disconnect)(display) };
            return Err("failed to get registry".to_string());
        }
        REGISTRY_PROXY.with(|r| *r.borrow_mut() = registry);
        unsafe {
            (fns.add_listener)(
                registry,
                &REGISTRY_LISTENER as *const _ as *const *const c_void,
                std::ptr::null_mut(),
            );
        }

        // Roundtrip to bind globals
        unsafe { (fns.roundtrip)(display) };

        // Set up seat listener
        let seat = SEAT.load(Ordering::Relaxed);
        if seat.is_null() { return Err("no wl_seat found".to_string()); }
        unsafe {
            (fns.add_listener)(
                seat,
                &SEAT_LISTENER as *const _ as *const *const c_void,
                std::ptr::null_mut(),
            );
        }
        unsafe { (fns.roundtrip)(display) }; // seat_capabilities fires → keyboard/pointer created

        // Set keyboard/pointer listeners
        let keyboard = KEYBOARD_OBJ.with(|k| k.borrow().ok_or("no wl_keyboard from seat"))?;
        unsafe {
            (fns.add_listener)(
                keyboard,
                &KEYBOARD_LISTENER as *const _ as *const *const c_void,
                std::ptr::null_mut(),
            );
        }
        if let Some(ptr) = POINTER_OBJ.with(|p| *p.borrow()) {
            unsafe {
                (fns.add_listener)(
                    ptr,
                    &POINTER_LISTENER as *const _ as *const *const c_void,
                    std::ptr::null_mut(),
                );
            }
        }
        unsafe { (fns.roundtrip)(display) };

        // Get compositor + layer_shell
        let compositor = COMPOSITOR.load(Ordering::Relaxed);
        if compositor.is_null() { return Err("no wl_compositor found".to_string()); }
        let layer_shell = LAYER_SHELL.load(Ordering::Relaxed);
        if layer_shell.is_null() { return Err("no zwlr_layer_shell_v1 found".to_string()); }

        // Create wl_surface
        let surface = unsafe { (fns.marshal_ctor)(compositor, 0, fns.surface_iface, std::ptr::null::<c_void>()) };
        if surface.is_null() {
            unsafe { (fns.disconnect)(display) };
            return Err("failed to create wl_surface".to_string());
        }

        // Create zwlr_layer_surface_v1: get_layer_surface(id, surface, output, layer, namespace)
        let layer_surface = unsafe {
            (fns.marshal_ctor)(
                layer_shell,
                0, // get_layer_surface
                &ZWLR_LAYER_SURFACE_V1_INTERFACE,
                surface,
                std::ptr::null::<c_void>(), // output = null (default)
                LAYER_TOP,
                c"ira-overlay".as_ptr(),
            )
        };
        if layer_surface.is_null() {
            unsafe { (fns.disconnect)(display) };
            return Err("failed to create zwlr_layer_surface_v1".to_string());
        }

        // Set anchor (full screen) and initial size
        unsafe {
            (fns.marshal)(layer_surface, 1, ANCHOR_ALL); // set_anchor
            (fns.marshal)(layer_surface, 0, 0u32, 0u32); // set_size
        }

        let mut state = WaylandState {
            display,
            surface,
            layer_surface,
            pending_resize: None,
        };

        // Add layer surface listener with data pointer
        unsafe {
            (fns.add_listener)(
                layer_surface,
                &LAYER_SURFACE_LISTENER as *const _ as *const *const c_void,
                &mut state as *mut _ as *mut c_void,
            );
        }

        // Commit + roundtrip to get initial configure
        unsafe {
            (fns.marshal)(surface, 6); // wl_surface::commit
            (fns.flush)(display);
            (fns.roundtrip)(display);
        }

        Ok(state)
    }

    pub fn dispatch(&mut self) {
        let Some(fns) = FNS.get() else { return };
        unsafe {
            (fns.dispatch_pending)(self.display);
            (fns.flush)(self.display);
        }
    }

    pub fn set_keyboard_interactivity(&self, enabled: bool) {
        let Some(fns) = FNS.get() else { return };
        unsafe {
            (fns.marshal)(self.layer_surface, 4, if enabled { 1u32 } else { 0u32 });
            (fns.marshal)(self.surface, 6); // commit
            (fns.flush)(self.display);
        }
    }

    pub fn take_pending_resize(&mut self) -> Option<(u32, u32)> {
        self.pending_resize.take()
    }

    pub fn display_ptr(&self) -> *mut c_void { self.display }
    pub fn surface_ptr(&self) -> *mut c_void { self.surface }
}

impl Drop for WaylandState {
    fn drop(&mut self) {
        let Some(fns) = FNS.get() else { return };
        unsafe {
            if !self.layer_surface.is_null() {
                (fns.marshal)(self.layer_surface, 7); // destroy
            }
            if !self.surface.is_null() {
                (fns.destroy)(self.surface);
            }
            (fns.disconnect)(self.display);
        }
    }
}

// ─── Library loading ───

fn dlsym_ptr(lib: *mut c_void, name: &str) -> *mut c_void {
    let c_name = CString::new(name).unwrap();
    unsafe { libc::dlsym(lib, c_name.as_ptr()) }
}

fn load_library() -> Option<Fns> {
    let lib_name = CString::new("libwayland-client.so.0").unwrap();
    let lib = unsafe { libc::dlopen(lib_name.as_ptr(), libc::RTLD_LAZY) };
    if lib.is_null() { return None; }

    let p = |n: &str| dlsym_ptr(lib, n);
    let connect = p("wl_display_connect");
    let disconnect = p("wl_display_disconnect");
    let dispatch_pending = p("wl_display_dispatch_pending");
    let roundtrip = p("wl_display_roundtrip");
    let flush = p("wl_display_flush");
    let marshal = p("wl_proxy_marshal");
    let marshal_ctor = p("wl_proxy_marshal_constructor");
    let add_listener = p("wl_proxy_add_listener");
    let destroy = p("wl_proxy_destroy");
    let compositor_iface = p("wl_compositor_interface");
    let surface_iface = p("wl_surface_interface");
    let seat_iface = p("wl_seat_interface");
    let keyboard_iface = p("wl_keyboard_interface");
    let pointer_iface = p("wl_pointer_interface");
    let registry_iface = p("wl_registry_interface");

    if connect.is_null() || disconnect.is_null() || dispatch_pending.is_null()
        || roundtrip.is_null() || flush.is_null() || marshal.is_null()
        || marshal_ctor.is_null() || add_listener.is_null() || destroy.is_null()
        || compositor_iface.is_null() || surface_iface.is_null()
        || seat_iface.is_null() || keyboard_iface.is_null() || pointer_iface.is_null()
        || registry_iface.is_null()
    {
        return None;
    }

    Some(Fns {
        connect: unsafe { std::mem::transmute::<*mut c_void, FnConnect>(connect) },
        disconnect: unsafe { std::mem::transmute::<*mut c_void, FnDisconnect>(disconnect) },
        dispatch_pending: unsafe { std::mem::transmute::<*mut c_void, FnDispatchPending>(dispatch_pending) },
        roundtrip: unsafe { std::mem::transmute::<*mut c_void, FnRoundtrip>(roundtrip) },
        flush: unsafe { std::mem::transmute::<*mut c_void, FnFlush>(flush) },
        marshal: unsafe { std::mem::transmute::<*mut c_void, FnMarshal>(marshal) },
        marshal_ctor: unsafe { std::mem::transmute::<*mut c_void, FnMarshalCtor>(marshal_ctor) },
        add_listener: unsafe { std::mem::transmute::<*mut c_void, FnAddListener>(add_listener) },
        destroy: unsafe { std::mem::transmute::<*mut c_void, FnDestroy>(destroy) },
        compositor_iface: compositor_iface as *const WlInterface,
        surface_iface: surface_iface as *const WlInterface,
        seat_iface: seat_iface as *const WlInterface,
        keyboard_iface: keyboard_iface as *const WlInterface,
        pointer_iface: pointer_iface as *const WlInterface,
        registry_iface: registry_iface as *const WlInterface,
    })
}
