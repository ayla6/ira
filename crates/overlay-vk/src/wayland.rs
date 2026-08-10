//! Wayland input capture — keyboard and pointer events.
//!
//! On Wayland, the X11 LD_PRELOAD hooks don't fire because games use
//! the Wayland protocol directly. This module binds to `wl_seat` and
//! creates its own `wl_keyboard` and `wl_pointer` objects on a private
//! event queue, receiving events alongside the game.
//!
//! Unlike X11 (where events can be consumed/intercepted), Wayland only
//! notifies us of events — the game still receives them. This means
//! keyboard/mouse input may also reach the game while the overlay is
//! visible. This is an inherent Wayland limitation.
//!
//! The toggle hotkey (Shift+Tab), screenshot (F12), and recording (F11)
//! hotkeys are always detected. Navigation keys (arrows, Enter) and
//! mouse events are only forwarded when the overlay is visible.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU32, Ordering};
use std::sync::OnceLock;

use ira_overlay::ui::{capture, push_event, Event};

pub static HAS_FOCUS: AtomicBool = AtomicBool::new(false);

// --- Tracked input state (all accessed from dispatch(), single-threaded) ---

static MODS_DEPRESSED: AtomicU32 = AtomicU32::new(0);
static MOUSE_SX: AtomicI32 = AtomicI32::new(0);
static MOUSE_SY: AtomicI32 = AtomicI32::new(0);
static POINTER: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

// --- Evdev keycodes (Wayland uses evdev directly, no +8 offset like X11) ---

const KC_RETURN: u32 = 28;
#[cfg(debug_assertions)]
const KC_F10: u32 = 68;
const KC_UP: u32 = 103;
const KC_DOWN: u32 = 108;
const KC_LEFT: u32 = 105;
const KC_RIGHT: u32 = 106;

// Wayland key/button state values.
const KEY_PRESSED: u32 = 1;
const AXIS_VERTICAL: u32 = 0;

type FnCreateQueue = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
type FnDispatchPending = unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int;
type FnRoundtrip = unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int;
type FnMarshalCtor =
    unsafe extern "C" fn(*mut c_void, u32, *const c_void, *mut c_void) -> *mut c_void;
type FnMarshalCtorBind = unsafe extern "C" fn(
    *mut c_void,
    u32,
    *const c_void,
    u32,
    *const c_char,
    u32,
    *mut c_void,
) -> *mut c_void;
type FnAddListener = unsafe extern "C" fn(*mut c_void, *const *const c_void, *mut c_void) -> c_int;
type FnSetQueue = unsafe extern "C" fn(*mut c_void, *mut c_void);

struct Fns {
    create_queue: FnCreateQueue,
    dispatch_pending: FnDispatchPending,
    roundtrip: FnRoundtrip,
    marshal_ctor: FnMarshalCtor,
    marshal_ctor_bind: FnMarshalCtorBind,
    add_listener: FnAddListener,
    set_queue: FnSetQueue,
    seat_iface: *const c_void,
    keyboard_iface: *const c_void,
    pointer_iface: *const c_void,
    registry_iface: *const c_void,
}

unsafe impl Send for Fns {}
unsafe impl Sync for Fns {}

static FNS: OnceLock<Option<Fns>> = OnceLock::new();
static DISPLAY: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static QUEUE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static SEAT_NAME: AtomicU32 = AtomicU32::new(0);
static SEAT_VERSION: AtomicU32 = AtomicU32::new(0);
static KEYBOARD: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

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

/// wl_pointer listener — must include all v5+ callbacks to avoid
/// `wl_proxy_add_listener` reading past the end of the struct.
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

// --- No-op callbacks ---

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

// --- Listener statics ---

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
    enter: Some(keyboard_enter),
    leave: Some(keyboard_leave),
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

// --- Registry / seat callbacks ---

extern "C" fn registry_global(
    _: *mut c_void,
    _: *mut c_void,
    name: u32,
    interface: *const c_char,
    version: u32,
) {
    if unsafe { CStr::from_ptr(interface) }.to_bytes() == b"wl_seat" {
        SEAT_NAME.store(name, Ordering::Relaxed);
        SEAT_VERSION.store(version.min(7), Ordering::Relaxed);
    }
}

extern "C" fn seat_capabilities(_: *mut c_void, seat: *mut c_void, caps: u32) {
    let Some(fns) = FNS.get().and_then(|f| f.as_ref()) else {
        return;
    };
    // caps: 1 = pointer, 2 = keyboard, 4 = touch
    if caps & 1 != 0 && POINTER.load(Ordering::Relaxed).is_null() {
        let ptr = unsafe { (fns.marshal_ctor)(seat, 0, fns.pointer_iface, std::ptr::null_mut()) };
        POINTER.store(ptr, Ordering::Relaxed);
    }
    if caps & 2 != 0 && KEYBOARD.load(Ordering::Relaxed).is_null() {
        let kb = unsafe { (fns.marshal_ctor)(seat, 1, fns.keyboard_iface, std::ptr::null_mut()) };
        KEYBOARD.store(kb, Ordering::Relaxed);
    }
}

// --- Keyboard callbacks ---

extern "C" fn keyboard_enter(
    _: *mut c_void,
    _: *mut c_void,
    _: u32,
    _: *mut c_void,
    _: *mut c_void,
) {
    HAS_FOCUS.store(true, Ordering::Relaxed);
    eprintln!("ira-overlay: keyboard enter (focus=true)");
}

extern "C" fn keyboard_leave(_: *mut c_void, _: *mut c_void, _: u32, _: *mut c_void) {
    HAS_FOCUS.store(false, Ordering::Relaxed);
    eprintln!("ira-overlay: keyboard leave (focus=false)");
}

extern "C" fn keyboard_key(_: *mut c_void, _: *mut c_void, _: u32, _: u32, key: u32, state: u32) {
    if !overlay_active() {
        return;
    }
    let pressed = state == KEY_PRESSED;
    let mods = MODS_DEPRESSED.load(Ordering::Relaxed);

    // Hotkeys work even when overlay is hidden.
    let (tog_kc, tog_mods, ss_kc, ss_mods, rec_kc, rec_mods) = crate::shim_bridge::hotkeys();
    if pressed && (mods & tog_mods) == tog_mods && key == tog_kc {
        if !crate::shim_bridge::ready_for_overlay() {
            return;
        }
        let visible = crate::shim_bridge::is_visible();
        crate::shim_bridge::set_visible(!visible);
        return;
    }
    if pressed && (mods & ss_mods) == ss_mods && key == ss_kc {
        capture::request_screenshot();
        return;
    }
    if pressed && (mods & rec_mods) == rec_mods && key == rec_kc {
        capture::toggle_recording();
        return;
    }

    // Navigation keys and debug toggles only when overlay is visible.
    if !crate::shim_bridge::is_visible() {
        return;
    }
    #[cfg(debug_assertions)]
    if pressed && key == KC_F10 {
        ira_overlay::ui::toggle_backend();
        return;
    }
    if pressed {
        let event = match key {
            KC_UP => Some(Event::NavUp),
            KC_DOWN => Some(Event::NavDown),
            KC_LEFT => Some(Event::NavLeft),
            KC_RIGHT => Some(Event::NavRight),
            KC_RETURN => Some(Event::Activate),
            _ => None,
        };
        if let Some(e) = event {
            push_event(e);
        }
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

// --- Pointer callbacks ---

extern "C" fn pointer_enter(
    _: *mut c_void,
    _: *mut c_void,
    _: u32,
    _: *mut c_void,
    sx: i32,
    sy: i32,
) {
    MOUSE_SX.store(sx, Ordering::Relaxed);
    MOUSE_SY.store(sy, Ordering::Relaxed);
}

extern "C" fn pointer_motion(_: *mut c_void, _: *mut c_void, _: u32, sx: i32, sy: i32) {
    MOUSE_SX.store(sx, Ordering::Relaxed);
    MOUSE_SY.store(sy, Ordering::Relaxed);
    if crate::shim_bridge::is_visible() {
        let (x, y) = fixed_to_f32(sx, sy);
        push_event(Event::MouseMove { x, y });
    }
}

extern "C" fn pointer_button(
    _: *mut c_void,
    _: *mut c_void,
    _: u32,
    _: u32,
    _button: u32,
    state: u32,
) {
    if !crate::shim_bridge::is_visible() {
        return;
    }
    let (x, y) = fixed_to_f32(
        MOUSE_SX.load(Ordering::Relaxed),
        MOUSE_SY.load(Ordering::Relaxed),
    );
    if state == KEY_PRESSED {
        push_event(Event::MouseDown { x, y });
    } else {
        push_event(Event::MouseUp { x, y });
    }
}

extern "C" fn pointer_axis(_: *mut c_void, _: *mut c_void, _: u32, axis: u32, value: i32) {
    if !crate::shim_bridge::is_visible() {
        return;
    }
    if axis != AXIS_VERTICAL {
        return;
    }
    let delta_y = if value > 0 { 1.0 } else { -1.0 };
    push_event(Event::Scroll { delta_y });
}

// --- Init / dispatch ---

pub fn init(display: *mut c_void) {
    let Some(fns) = load_library() else { return };
    let _ = FNS.set(Some(fns));
    let fns = FNS.get().and_then(|f| f.as_ref()).unwrap();

    let queue = unsafe { (fns.create_queue)(display) };
    if queue.is_null() {
        return;
    }

    let registry =
        unsafe { (fns.marshal_ctor)(display, 1, fns.registry_iface, std::ptr::null_mut()) };
    if registry.is_null() {
        return;
    }
    unsafe { (fns.set_queue)(registry, queue) };
    unsafe {
        (fns.add_listener)(
            registry,
            &REGISTRY_LISTENER as *const _ as *const *const c_void,
            std::ptr::null_mut(),
        )
    };

    DISPLAY.store(display, Ordering::Relaxed);
    QUEUE.store(queue, Ordering::Relaxed);

    unsafe { (fns.roundtrip)(display, queue) };

    let name = SEAT_NAME.load(Ordering::Relaxed);
    if name == 0 {
        return;
    }

    let version = SEAT_VERSION.load(Ordering::Relaxed);
    let seat = unsafe {
        (fns.marshal_ctor_bind)(
            registry,
            0,
            fns.seat_iface,
            name,
            c"wl_seat".as_ptr(),
            version,
            std::ptr::null_mut(),
        )
    };
    if seat.is_null() {
        return;
    }
    unsafe { (fns.set_queue)(seat, queue) };
    unsafe {
        (fns.add_listener)(
            seat,
            &SEAT_LISTENER as *const _ as *const *const c_void,
            std::ptr::null_mut(),
        )
    };

    unsafe { (fns.roundtrip)(display, queue) };

    // seat_capabilities has now fired — keyboard and pointer objects are created.
    let kb = KEYBOARD.load(Ordering::Relaxed);
    if kb.is_null() {
        return;
    }
    unsafe { (fns.set_queue)(kb, queue) };
    unsafe {
        (fns.add_listener)(
            kb,
            &KEYBOARD_LISTENER as *const _ as *const *const c_void,
            std::ptr::null_mut(),
        )
    };

    let ptr = POINTER.load(Ordering::Relaxed);
    if !ptr.is_null() {
        unsafe { (fns.set_queue)(ptr, queue) };
        unsafe {
            (fns.add_listener)(
                ptr,
                &POINTER_LISTENER as *const _ as *const *const c_void,
                std::ptr::null_mut(),
            )
        };
    }

    unsafe { (fns.roundtrip)(display, queue) };
}

pub fn dispatch() {
    let Some(fns) = FNS.get().and_then(|f| f.as_ref()) else {
        return;
    };
    let display = DISPLAY.load(Ordering::Relaxed);
    let queue = QUEUE.load(Ordering::Relaxed);
    if display.is_null() || queue.is_null() {
        return;
    }
    unsafe { (fns.dispatch_pending)(display, queue) };
}

// --- Helpers ---

fn overlay_active() -> bool {
    std::env::var_os("IRA_OVERLAY_SHM").is_some()
}

/// Convert wl_fixed_t (24.8 fixed-point) to f32.
fn fixed_to_f32(sx: i32, sy: i32) -> (f32, f32) {
    (sx as f32 / 256.0, sy as f32 / 256.0)
}

fn dlsym_ptr(lib: *mut c_void, name: &str) -> *mut c_void {
    let c_name = CString::new(name).unwrap();
    unsafe { libc::dlsym(lib, c_name.as_ptr()) }
}

fn load_library() -> Option<Fns> {
    let lib_name = CString::new("libwayland-client.so.0").unwrap();
    let lib = unsafe { libc::dlopen(lib_name.as_ptr(), libc::RTLD_LAZY) };
    if lib.is_null() {
        return None;
    }

    let p = |n: &str| dlsym_ptr(lib, n);
    let create_queue = p("wl_display_create_queue");
    let dispatch_pending = p("wl_display_dispatch_queue_pending");
    let roundtrip = p("wl_display_roundtrip_queue");
    let marshal_ctor_raw = p("wl_proxy_marshal_constructor");
    let add_listener = p("wl_proxy_add_listener");
    let set_queue = p("wl_proxy_set_queue");
    let seat_iface = p("wl_seat_interface");
    let keyboard_iface = p("wl_keyboard_interface");
    let pointer_iface = p("wl_pointer_interface");
    let registry_iface = p("wl_registry_interface");

    if create_queue.is_null()
        || dispatch_pending.is_null()
        || roundtrip.is_null()
        || marshal_ctor_raw.is_null()
        || add_listener.is_null()
        || set_queue.is_null()
        || seat_iface.is_null()
        || keyboard_iface.is_null()
        || pointer_iface.is_null()
        || registry_iface.is_null()
    {
        return None;
    }

    Some(Fns {
        create_queue: unsafe { std::mem::transmute::<*mut c_void, FnCreateQueue>(create_queue) },
        dispatch_pending: unsafe {
            std::mem::transmute::<*mut c_void, FnDispatchPending>(dispatch_pending)
        },
        roundtrip: unsafe { std::mem::transmute::<*mut c_void, FnRoundtrip>(roundtrip) },
        marshal_ctor: unsafe {
            std::mem::transmute::<*mut c_void, FnMarshalCtor>(marshal_ctor_raw)
        },
        marshal_ctor_bind: unsafe {
            std::mem::transmute::<*mut c_void, FnMarshalCtorBind>(marshal_ctor_raw)
        },
        add_listener: unsafe { std::mem::transmute::<*mut c_void, FnAddListener>(add_listener) },
        set_queue: unsafe { std::mem::transmute::<*mut c_void, FnSetQueue>(set_queue) },
        seat_iface: seat_iface as *const c_void,
        keyboard_iface: keyboard_iface as *const c_void,
        pointer_iface: pointer_iface as *const c_void,
        registry_iface: registry_iface as *const c_void,
    })
}
