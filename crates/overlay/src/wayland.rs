use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};
use std::sync::OnceLock;

pub static HAS_FOCUS: AtomicBool = AtomicBool::new(false);

type FnCreateQueue = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
type FnDispatchPending = unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int;
type FnRoundtrip = unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int;
type FnMarshalCtor = unsafe extern "C" fn(*mut c_void, u32, *const c_void, *mut c_void) -> *mut c_void;
type FnMarshalCtorBind = unsafe extern "C" fn(*mut c_void, u32, *const c_void, u32, *const c_char, u32, *mut c_void) -> *mut c_void;
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

extern "C" fn nop_global_remove(_: *mut c_void, _: *mut c_void, _: u32) {}
extern "C" fn nop_seat_name(_: *mut c_void, _: *mut c_void, _: *const c_char) {}
extern "C" fn nop_keymap(_: *mut c_void, _: *mut c_void, _: u32, _: c_int, _: u32) {}
extern "C" fn nop_key(_: *mut c_void, _: *mut c_void, _: u32, _: u32, _: u32, _: u32) {}
extern "C" fn nop_modifiers(_: *mut c_void, _: *mut c_void, _: u32, _: u32, _: u32, _: u32, _: u32) {}
extern "C" fn nop_repeat(_: *mut c_void, _: *mut c_void, _: c_int, _: c_int) {}

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
    key: Some(nop_key),
    modifiers: Some(nop_modifiers),
    repeat_info: Some(nop_repeat),
};

extern "C" fn registry_global(
    _: *mut c_void, _: *mut c_void,
    name: u32, interface: *const c_char, version: u32,
) {
    if unsafe { CStr::from_ptr(interface) }.to_bytes() == b"wl_seat" {
        SEAT_NAME.store(name, Ordering::Relaxed);
        SEAT_VERSION.store(version.min(7), Ordering::Relaxed);
    }
}

extern "C" fn seat_capabilities(_: *mut c_void, seat: *mut c_void, caps: u32) {
    if caps & 1 == 0 { return }
    let Some(fns) = FNS.get().and_then(|f| f.as_ref()) else { return };
    let kb = unsafe { (fns.marshal_ctor)(seat, 1, fns.keyboard_iface, std::ptr::null_mut()) };
    KEYBOARD.store(kb, Ordering::Relaxed);
}

extern "C" fn keyboard_enter(_: *mut c_void, _: *mut c_void, _: u32, _: *mut c_void, _: *mut c_void) {
    HAS_FOCUS.store(true, Ordering::Relaxed);
    eprintln!("ira-overlay: keyboard enter (focus=true)");
}

extern "C" fn keyboard_leave(_: *mut c_void, _: *mut c_void, _: u32, _: *mut c_void) {
    HAS_FOCUS.store(false, Ordering::Relaxed);
    eprintln!("ira-overlay: keyboard leave (focus=false)");
}

pub fn init(display: *mut c_void) {
    let Some(fns) = load_library() else { return };
    let _ = FNS.set(Some(fns));
    let fns = FNS.get().and_then(|f| f.as_ref()).unwrap();

    let queue = unsafe { (fns.create_queue)(display) };
    if queue.is_null() { return }

    let registry = unsafe { (fns.marshal_ctor)(display, 1, fns.registry_iface, std::ptr::null_mut()) };
    if registry.is_null() { return }
    unsafe { (fns.set_queue)(registry, queue) };
    unsafe { (fns.add_listener)(registry, &REGISTRY_LISTENER as *const _ as *const *const c_void, std::ptr::null_mut()) };

    DISPLAY.store(display, Ordering::Relaxed);
    QUEUE.store(queue, Ordering::Relaxed);

    unsafe { (fns.roundtrip)(display, queue) };

    let name = SEAT_NAME.load(Ordering::Relaxed);
    if name == 0 { return }

    let version = SEAT_VERSION.load(Ordering::Relaxed);
    let seat = unsafe { (fns.marshal_ctor_bind)(
        registry, 0, fns.seat_iface,
        name, c"wl_seat".as_ptr(), version,
        std::ptr::null_mut(),
    ) };
    if seat.is_null() { return }
    unsafe { (fns.set_queue)(seat, queue) };
    unsafe { (fns.add_listener)(seat, &SEAT_LISTENER as *const _ as *const *const c_void, std::ptr::null_mut()) };

    unsafe { (fns.roundtrip)(display, queue) };

    let kb = KEYBOARD.load(Ordering::Relaxed);
    if kb.is_null() { return }
    unsafe { (fns.set_queue)(kb, queue) };
    unsafe { (fns.add_listener)(kb, &KEYBOARD_LISTENER as *const _ as *const *const c_void, std::ptr::null_mut()) };
    unsafe { (fns.roundtrip)(display, queue) };
}

pub fn dispatch() {
    let Some(fns) = FNS.get().and_then(|f| f.as_ref()) else { return };
    let display = DISPLAY.load(Ordering::Relaxed);
    let queue = QUEUE.load(Ordering::Relaxed);
    if display.is_null() || queue.is_null() { return }
    unsafe { (fns.dispatch_pending)(display, queue) };
}

fn dlsym_ptr(lib: *mut c_void, name: &str) -> *mut c_void {
    let c_name = CString::new(name).unwrap();
    unsafe { libc::dlsym(lib, c_name.as_ptr()) }
}

fn load_library() -> Option<Fns> {
    let lib_name = CString::new("libwayland-client.so.0").unwrap();
    let lib = unsafe { libc::dlopen(lib_name.as_ptr(), libc::RTLD_LAZY) };
    if lib.is_null() { return None }

    let p = |n: &str| dlsym_ptr(lib, n);
    let create_queue = p("wl_display_create_queue");
    let dispatch_pending = p("wl_display_dispatch_queue_pending");
    let roundtrip = p("wl_display_roundtrip_queue");
    let marshal_ctor_raw = p("wl_proxy_marshal_constructor");
    let add_listener = p("wl_proxy_add_listener");
    let set_queue = p("wl_proxy_set_queue");
    let seat_iface = p("wl_seat_interface");
    let keyboard_iface = p("wl_keyboard_interface");
    let registry_iface = p("wl_registry_interface");

    if create_queue.is_null() || dispatch_pending.is_null() || roundtrip.is_null()
        || marshal_ctor_raw.is_null() || add_listener.is_null() || set_queue.is_null()
        || seat_iface.is_null() || keyboard_iface.is_null() || registry_iface.is_null()
    {
        return None;
    }

    Some(Fns {
        create_queue: unsafe { std::mem::transmute::<*mut c_void, FnCreateQueue>(create_queue) },
        dispatch_pending: unsafe { std::mem::transmute::<*mut c_void, FnDispatchPending>(dispatch_pending) },
        roundtrip: unsafe { std::mem::transmute::<*mut c_void, FnRoundtrip>(roundtrip) },
        marshal_ctor: unsafe { std::mem::transmute::<*mut c_void, FnMarshalCtor>(marshal_ctor_raw) },
        marshal_ctor_bind: unsafe { std::mem::transmute::<*mut c_void, FnMarshalCtorBind>(marshal_ctor_raw) },
        add_listener: unsafe { std::mem::transmute::<*mut c_void, FnAddListener>(add_listener) },
        set_queue: unsafe { std::mem::transmute::<*mut c_void, FnSetQueue>(set_queue) },
        seat_iface: seat_iface as *const c_void,
        keyboard_iface: keyboard_iface as *const c_void,
        registry_iface: registry_iface as *const c_void,
    })
}
