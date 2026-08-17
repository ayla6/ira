//! Gamepad input via evdev — fallback for non-SDL games.
//!
//! Scans `/dev/input/event*` for gamepad devices and reads button/axis events.
//! Called every frame from `queue_present` when SDL hooks are not active.
//!
//! Hotplug is handled via inotify on `/dev/input/` — device connect/disconnect
//! is detected instantly, same as RetroArch's linuxraw driver.
//!
//! The Guide/Home/PS button (BTN_MODE) toggles the overlay.
//! D-pad and A/Cross provide navigation and activation.
//! L1/R1 scroll the achievement list.
//!
//! Limitation: evdev is read-only — we can't consume events, so the game also
//! receives gamepad input when the overlay is visible. This is an inherent
//! Linux limitation. For SDL2 games, the SDL hook in the shim can consume
//! events; evdev is only used when SDL2 is not detected.

use std::ffi::CString;
use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use ira_overlay::ui::{push_event, Event};
use ira_overlay_ipc::gamepad_button_mask_from_evdev;

// evdev event types
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;

// Button codes
const BTN_SOUTH: u16 = 0x130; // A / Cross  (== BTN_GAMEPAD)
const BTN_TL: u16 = 0x136; // L1 / LB
const BTN_TR: u16 = 0x137; // R1 / RB
const BTN_DPAD_UP: u16 = 0x220;
const BTN_DPAD_DOWN: u16 = 0x221;
const BTN_DPAD_LEFT: u16 = 0x222;
const BTN_DPAD_RIGHT: u16 = 0x223;

// Axis codes (for controllers that report D-pad as axes)
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;

const KEY_PRESS: i32 = 1;

// inotify masks
const IN_CREATE: u32 = 0x100;
const IN_DELETE: u32 = 0x200;

/// `struct input_event` on 64-bit Linux (24 bytes).
#[repr(C)]
struct InputEvent {
    tv_sec: i64,
    tv_usec: i64,
    type_: u16,
    code: u16,
    value: i32,
}
const _: () = assert!(std::mem::size_of::<InputEvent>() == 24);

const KEY_BUF_BYTES: u32 = 96; // 768 bits — covers all BTN_* codes
const MAX_EVENTS_PER_READ: usize = 16;

static OVERLAY_ACTIVE: OnceLock<bool> = OnceLock::new();
static GAMEPAD_FDS: Mutex<Vec<c_int>> = Mutex::new(Vec::new());
static INIT_DONE: AtomicBool = AtomicBool::new(false);
static INOTIFY_FD: AtomicI32 = AtomicI32::new(-1);
static RESCAN_TIMER: AtomicU32 = AtomicU32::new(0);
static HAT_X: AtomicI32 = AtomicI32::new(0);
static HAT_Y: AtomicI32 = AtomicI32::new(0);
static PRESSED_BUTTONS: AtomicU32 = AtomicU32::new(0);
static TOGGLE_PENDING: AtomicBool = AtomicBool::new(false);

/// Frames to wait after inotify fires before re-scanning.
/// The device node exists immediately but isn't fully initialized —
/// the kernel needs a moment to set up evdev capabilities.
const RESCAN_DELAY: u32 = 15; // ~250ms at 60fps

fn overlay_active() -> bool {
    *OVERLAY_ACTIVE.get_or_init(|| std::env::var_os("IRA_OVERLAY_SHM").is_some())
}

/// `_IOR('E', 0x20 + ev, len)` — get supported event codes bitfield.
fn eviocgbit(ev: u32, len: u32) -> u64 {
    let dir: u64 = 2; // _IOC_READ
    let size: u64 = len as u64;
    let type_: u64 = 0x45; // 'E'
    let nr: u64 = (0x20 + ev) as u64;
    (dir << 30) | (size << 16) | (type_ << 8) | nr
}

fn test_bit(buf: &[u8], bit: u32) -> bool {
    let byte = (bit / 8) as usize;
    byte < buf.len() && (buf[byte] & (1 << (bit % 8))) != 0
}

fn is_gamepad(fd: c_int) -> bool {
    let mut buf = [0u8; KEY_BUF_BYTES as usize];
    let ret = unsafe {
        libc::ioctl(
            fd,
            eviocgbit(EV_KEY as u32, KEY_BUF_BYTES),
            buf.as_mut_ptr(),
        )
    };
    if ret < 0 {
        return false;
    }
    test_bit(&buf, BTN_SOUTH as u32)
}

/// Set up inotify watch on `/dev/input/` for instant hotplug detection.
fn init_inotify() {
    let fd = unsafe { libc::inotify_init1(libc::O_NONBLOCK) };
    if fd < 0 {
        return;
    }

    let path = CString::new("/dev/input").unwrap();
    let wd = unsafe { libc::inotify_add_watch(fd, path.as_ptr(), IN_CREATE | IN_DELETE) };
    if wd < 0 {
        unsafe { libc::close(fd) };
        return;
    }

    INOTIFY_FD.store(fd, Ordering::Relaxed);
}

/// Returns true if a device was added or removed since last check.
fn check_hotplug() -> bool {
    let fd = INOTIFY_FD.load(Ordering::Relaxed);
    if fd < 0 {
        return false;
    }
    let mut buf = [0u8; 4096];
    let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) };
    n > 0
}

/// Scan `/dev/input/event*` for gamepad devices. Closes old fds and opens new ones.
fn scan_devices() {
    let mut fds = GAMEPAD_FDS.lock().unwrap();
    for &fd in &*fds {
        unsafe { libc::close(fd) };
    }
    fds.clear();

    let Ok(entries) = std::fs::read_dir("/dev/input") else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if !name.starts_with("event") {
            continue;
        }

        let path = CString::new(format!("/dev/input/{name}")).unwrap();
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
        if fd < 0 {
            continue;
        }

        if is_gamepad(fd) {
            eprintln!("ira-overlay: gamepad found at /dev/input/{name}");
            // Drain stale events from the kernel buffer so we don't
            // misinterpret old events as button presses.
            let drain_buf = [0u8; 24 * 64];
            loop {
                let n = unsafe { libc::read(fd, drain_buf.as_ptr() as *mut _, drain_buf.len()) };
                if n <= 0 {
                    break;
                }
            }
            fds.push(fd);
        } else {
            unsafe { libc::close(fd) };
        }
    }
    if fds.is_empty() {
        eprintln!("ira-overlay: no gamepad devices found");
    }
}

/// Initial scan + inotify setup. Called from `present.rs` every frame but
/// only runs once.
pub fn init() {
    if !INIT_DONE.swap(true, Ordering::Relaxed) {
        init_inotify();
        scan_devices();
    }
}

/// Poll all gamepad devices and dispatch events. Call every frame.
/// Caller skips this when SDL hooks are active.
pub fn poll() {
    if !overlay_active() {
        return;
    }

    // Instant hotplug detection via inotify
    if check_hotplug() {
        RESCAN_TIMER.store(RESCAN_DELAY, Ordering::Relaxed);
    }

    // Delayed re-scan — wait a few frames for the kernel to finish
    // initializing the newly created device node.
    if RESCAN_TIMER.load(Ordering::Relaxed) > 0 {
        let prev = RESCAN_TIMER.fetch_sub(1, Ordering::Relaxed);
        if prev == 1 {
            scan_devices();
        }
    }

    let fds = GAMEPAD_FDS.lock().unwrap();
    if fds.is_empty() {
        return;
    }

    let mut buf = [0u8; 24 * MAX_EVENTS_PER_READ];
    for &fd in &*fds {
        loop {
            let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n <= 0 {
                break;
            }

            let count = (n as usize) / 24;
            for i in 0..count {
                let ev = unsafe { &*(buf.as_ptr().add(i * 24) as *const InputEvent) };
                handle_event(ev);
            }
        }
    }
}

fn handle_event(ev: &InputEvent) {
    match ev.type_ {
        EV_KEY => {
            let Some(button) = gamepad_button_mask_from_evdev(ev.code) else {
                return;
            };
            if ev.value == 0 {
                let held = PRESSED_BUTTONS.fetch_and(!button, Ordering::Relaxed) & !button;
                if TOGGLE_PENDING.swap(false, Ordering::Relaxed) && held == 0 {
                    toggle_overlay();
                }
                return;
            }
            if ev.value != KEY_PRESS {
                return;
            }
            let held = PRESSED_BUTTONS.fetch_or(button, Ordering::Relaxed) | button;
            match hotkey_action(held) {
                HotkeyAction::Screenshot => {
                    TOGGLE_PENDING.store(false, Ordering::Relaxed);
                    ira_overlay::ui::capture::request_screenshot();
                    return;
                }
                HotkeyAction::Record => {
                    TOGGLE_PENDING.store(false, Ordering::Relaxed);
                    ira_overlay::ui::capture::toggle_recording();
                    return;
                }
                HotkeyAction::Toggle => {
                    if injected_ui_disabled() {
                        return;
                    }
                    TOGGLE_PENDING.store(true, Ordering::Relaxed);
                    return;
                }
                HotkeyAction::None => {}
            }
            // Other buttons only when overlay is visible.
            if injected_ui_disabled() || !crate::shim_bridge::is_visible() {
                return;
            }
            let event = match ev.code {
                BTN_SOUTH => Some(Event::Activate),
                BTN_DPAD_UP => Some(Event::NavUp),
                BTN_DPAD_DOWN => Some(Event::NavDown),
                BTN_DPAD_LEFT => Some(Event::NavLeft),
                BTN_DPAD_RIGHT => Some(Event::NavRight),
                BTN_TL => Some(Event::Scroll { delta_y: -1.0 }),
                BTN_TR => Some(Event::Scroll { delta_y: 1.0 }),
                _ => None,
            };
            if let Some(e) = event {
                push_event(e);
            }
        }
        EV_ABS => {
            if injected_ui_disabled() || !crate::shim_bridge::is_visible() {
                return;
            }
            match ev.code {
                ABS_HAT0X => {
                    let prev = HAT_X.swap(ev.value, Ordering::Relaxed);
                    if prev == 0 && ev.value != 0 {
                        push_event(if ev.value < 0 {
                            Event::NavLeft
                        } else {
                            Event::NavRight
                        });
                    }
                }
                ABS_HAT0Y => {
                    let prev = HAT_Y.swap(ev.value, Ordering::Relaxed);
                    if prev == 0 && ev.value != 0 {
                        push_event(if ev.value < 0 {
                            Event::NavUp
                        } else {
                            Event::NavDown
                        });
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

#[derive(Debug, PartialEq, Eq)]
enum HotkeyAction {
    None,
    Toggle,
    Screenshot,
    Record,
}

fn hotkey_action(held: u32) -> HotkeyAction {
    let (toggle, screenshot, record) = crate::shim_bridge::gamepad_hotkeys();
    hotkey_action_for(held, toggle, screenshot, record)
}

fn hotkey_action_for(held: u32, toggle: u32, screenshot: u32, record: u32) -> HotkeyAction {
    if held == screenshot {
        HotkeyAction::Screenshot
    } else if held == record {
        HotkeyAction::Record
    } else if held == toggle {
        HotkeyAction::Toggle
    } else {
        HotkeyAction::None
    }
}

fn toggle_overlay() {
    if crate::shim_bridge::ready_for_overlay() {
        crate::shim_bridge::set_visible(!crate::shim_bridge::is_visible());
    }
}

fn injected_ui_disabled() -> bool {
    std::env::var_os("IRA_OVERLAY_DISABLE_UI").is_some()
}

#[cfg(test)]
mod tests {
    use super::{hotkey_action_for, HotkeyAction};
    use ira_overlay_ipc::{
        DEFAULT_RECORD_GAMEPAD_HOTKEY, DEFAULT_SCREENSHOT_GAMEPAD_HOTKEY,
        DEFAULT_TOGGLE_GAMEPAD_HOTKEY,
    };

    #[test]
    fn test_hotkey_action_prefers_guide_chord_over_toggle() {
        assert_eq!(
            hotkey_action_for(
                DEFAULT_SCREENSHOT_GAMEPAD_HOTKEY,
                DEFAULT_TOGGLE_GAMEPAD_HOTKEY,
                DEFAULT_SCREENSHOT_GAMEPAD_HOTKEY,
                DEFAULT_RECORD_GAMEPAD_HOTKEY,
            ),
            HotkeyAction::Screenshot
        );
    }
}
