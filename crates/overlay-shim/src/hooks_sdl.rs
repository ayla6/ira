//! SDL2 gamepad input hooking via LD_PRELOAD symbol interposition.
//!
//! When loaded via `LD_PRELOAD`, calls to `SDL_PollEvent` are resolved to our
//! version instead of the real SDL2 function. We chain through to the real
//! function via `dlsym(RTLD_NEXT, ...)`.
//!
//! Gamepad button events (`SDL_CONTROLLERBUTTONDOWN`, `SDL_JOYBUTTONDOWN`) are
//! intercepted. When the overlay is visible, they are consumed (removed from
//! the queue) so the game doesn't see them. The configured toggle chord
//! (default Guide) toggles on release; screenshot/record chords fire on press.
//!
//! On first call, sets `HAS_SDL` flag so the Vulkan layer knows to skip evdev
//! (SDL hooks can consume events; evdev can't).

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::OnceLock;

use ira_overlay_ipc::{
    gamepad_button_mask_from_evdev, InputEventRaw, ShmHeader, EVT_KEY_PRESS, EVT_SCROLL,
    EVT_SCREENSHOT, EVT_TOGGLE_RECORD,
};

use crate::state;

// SDL2 event types (same values in SDL3)
const SDL_CONTROLLERBUTTONDOWN: u32 = 0x651;
const SDL_CONTROLLERBUTTONUP: u32 = 0x652;
const SDL_JOYBUTTONDOWN: u32 = 0x603;
const SDL_JOYBUTTONUP: u32 = 0x604;

// SDL2 controller button codes (same in SDL3)
const BTN_A: u8 = 0;
const BTN_B: u8 = 1;
const BTN_X: u8 = 2;
const BTN_Y: u8 = 3;
const BTN_BACK: u8 = 4;
const BTN_GUIDE: u8 = 5;
const BTN_START: u8 = 6;
const BTN_LEFTSTICK: u8 = 7;
const BTN_RIGHTSTICK: u8 = 8;
const BTN_LEFTSHOULDER: u8 = 9;
const BTN_RIGHTSHOULDER: u8 = 10;
const BTN_DPAD_UP: u8 = 11;
const BTN_DPAD_DOWN: u8 = 12;
const BTN_DPAD_LEFT: u8 = 13;
const BTN_DPAD_RIGHT: u8 = 14;

// X11 navigation keycodes — what shim_bridge::convert_and_forward expects on
// InputEventRaw.keycode. Single source of truth: ShmHeader::NAV_KEYCODES_X11,
// order [Return, Up, Down, Left, Right].
const KC_RETURN: u32 = ShmHeader::NAV_KEYCODES_X11[0];
const KC_UP: u32 = ShmHeader::NAV_KEYCODES_X11[1];
const KC_DOWN: u32 = ShmHeader::NAV_KEYCODES_X11[2];
const KC_LEFT: u32 = ShmHeader::NAV_KEYCODES_X11[3];
const KC_RIGHT: u32 = ShmHeader::NAV_KEYCODES_X11[4];

// SDL2: SDL_ControllerButtonEvent button is at offset 12
//   Uint32 type (0), Uint32 timestamp (4), Sint32 which (8), Uint8 button (12)
// SDL3: SDL_GamepadButtonEvent button is at offset 20
//   Uint32 type (0), Uint32 reserved (4), Uint64 timestamp (8), Sint32 which (16), Uint8 button (20)
const EV_BUTTON_OFFSET_SDL2: usize = 12;
const EV_BUTTON_OFFSET_SDL3: usize = 20;

type PollEventFn = unsafe extern "C" fn(*mut c_void) -> i32;

static REAL_SDL_POLL_EVENT: OnceLock<Option<PollEventFn>> = OnceLock::new();
static BUTTON_OFFSET: OnceLock<usize> = OnceLock::new();
static PRESSED_BUTTONS: AtomicU32 = AtomicU32::new(0);
static TOGGLE_PENDING: AtomicBool = AtomicBool::new(false);

fn real_poll_event() -> Option<PollEventFn> {
    *REAL_SDL_POLL_EVENT.get_or_init(|| {
        let p = unsafe { libc::dlsym(libc::RTLD_NEXT, c"SDL_PollEvent".as_ptr()) };
        if !p.is_null() {
            state::set_has_sdl(true);
            // Detect SDL3 by checking for SDL_OpenGamepad (SDL3-only name;
            // SDL2 uses SDL_GameControllerOpen)
            let is_sdl3 = unsafe {
                let fn_ptr = libc::dlsym(libc::RTLD_DEFAULT, c"SDL_OpenGamepad".as_ptr());
                !fn_ptr.is_null()
            };
            let offset = if is_sdl3 {
                EV_BUTTON_OFFSET_SDL3
            } else {
                EV_BUTTON_OFFSET_SDL2
            };
            let _ = BUTTON_OFFSET.set(offset);
            eprintln!(
                "ira-overlay: SDL_PollEvent hooked (SDL3={}, button offset={})",
                is_sdl3, offset
            );
            Some(unsafe { std::mem::transmute::<*mut libc::c_void, PollEventFn>(p) })
        } else {
            None
        }
    })
}

fn read_button(event: *const c_void) -> u8 {
    let offset = *BUTTON_OFFSET.get().unwrap_or(&EV_BUTTON_OFFSET_SDL2);
    unsafe { *(event as *const u8).add(offset) }
}

fn handle_button(button: u8) -> bool {
    // B is Big Picture back: hide the overlay (consumed, game never sees it).
    // Checked before the mask lookup: B carries no chord bit, so it would
    // otherwise return early below and never reach the back handler.
    if button == BTN_B {
        if !state::is_visible() {
            return false;
        }
        state::set_visible(false);
        return true;
    }
    let Some(mask) = button_mask(button) else {
        return false;
    };
    let held = PRESSED_BUTTONS.fetch_or(mask, Ordering::Relaxed) | mask;
    let (toggle, screenshot, record) = state::gamepad_hotkeys();
    if held == screenshot {
        TOGGLE_PENDING.store(false, Ordering::Relaxed);
        state::push_event(capture_event(EVT_SCREENSHOT));
        return true;
    }
    if held == record {
        TOGGLE_PENDING.store(false, Ordering::Relaxed);
        state::push_event(capture_event(EVT_TOGGLE_RECORD));
        return true;
    }
    if held == toggle {
        // Gamepad toggle yields to the daemon when it owns the chord;
        // otherwise the same press would flip the flag twice.
        if state::daemon_owns_toggle() {
            return false;
        }
        TOGGLE_PENDING.store(true, Ordering::Relaxed);
        return true;
    }

    // Other buttons only when overlay is visible, in any mode: the game
    // must not react to input meant for the overlay.
    if !state::is_visible() {
        return false;
    }

    if let Some(event) = map_nav(button) {
        state::push_event(event);
        return true;
    }
    false
}

/// Maps overlay navigation buttons to input events. Returns None for
/// buttons with no overlay meaning (chords already handled above).
fn map_nav(button: u8) -> Option<InputEventRaw> {
    let keycode = match button {
        BTN_A => KC_RETURN,
        BTN_DPAD_UP => KC_UP,
        BTN_DPAD_DOWN => KC_DOWN,
        BTN_DPAD_LEFT => KC_LEFT,
        BTN_DPAD_RIGHT => KC_RIGHT,
        _ => return scroll_event(button),
    };
    Some(key_event(keycode))
}

fn scroll_event(button: u8) -> Option<InputEventRaw> {
    let delta = match button {
        BTN_LEFTSHOULDER => -1,
        BTN_RIGHTSHOULDER => 1,
        _ => return None,
    };
    Some(InputEventRaw {
        event_type: EVT_SCROLL,
        x: 0,
        y: delta,
        button: 0,
        keycode: 0,
    })
}

fn key_event(keycode: u32) -> InputEventRaw {
    InputEventRaw {
        event_type: EVT_KEY_PRESS,
        x: 0,
        y: 0,
        button: 0,
        keycode,
    }
}

fn handle_button_release(button: u8) -> bool {
    let Some(mask) = button_mask(button) else {
        return false;
    };
    let held = PRESSED_BUTTONS.fetch_and(!mask, Ordering::Relaxed) & !mask;
    if !state::injected_ui_disabled()
        && TOGGLE_PENDING.swap(false, Ordering::Relaxed)
        && held == 0
        && state::ready_for_overlay()
    {
        state::set_visible(!state::is_visible());
        return true;
    }
    false
}

fn button_mask(button: u8) -> Option<u32> {
    // B never reaches here (hardcoded Big Picture back above) and triggers
    // have no SDL button codes, so neither enters the chord table (the
    // evdev fallback covers them in non-SDL games).
    //
    // Evdev codes (linux/input-event-codes.h) for the shared chord table.
    const EVDEV_SOUTH: u16 = 0x130;
    const EVDEV_WEST: u16 = 0x132;
    const EVDEV_NORTH: u16 = 0x133;
    const EVDEV_BACK: u16 = 0x13a;
    const EVDEV_GUIDE: u16 = 0x13c;
    const EVDEV_START: u16 = 0x13b;
    const EVDEV_THUMBL: u16 = 0x13d;
    const EVDEV_THUMBR: u16 = 0x13e;
    const EVDEV_TL: u16 = 0x136;
    const EVDEV_TR: u16 = 0x137;
    const EVDEV_DPAD_UP: u16 = 0x220;
    const EVDEV_DPAD_DOWN: u16 = 0x221;
    const EVDEV_DPAD_LEFT: u16 = 0x222;
    const EVDEV_DPAD_RIGHT: u16 = 0x223;
    let evdev = match button {
        BTN_A => EVDEV_SOUTH,
        BTN_X => EVDEV_WEST,
        BTN_Y => EVDEV_NORTH,
        BTN_BACK => EVDEV_BACK,
        BTN_GUIDE => EVDEV_GUIDE,
        BTN_START => EVDEV_START,
        BTN_LEFTSTICK => EVDEV_THUMBL,
        BTN_RIGHTSTICK => EVDEV_THUMBR,
        BTN_LEFTSHOULDER => EVDEV_TL,
        BTN_RIGHTSHOULDER => EVDEV_TR,
        BTN_DPAD_UP => EVDEV_DPAD_UP,
        BTN_DPAD_DOWN => EVDEV_DPAD_DOWN,
        BTN_DPAD_LEFT => EVDEV_DPAD_LEFT,
        BTN_DPAD_RIGHT => EVDEV_DPAD_RIGHT,
        _ => return None,
    };
    gamepad_button_mask_from_evdev(evdev)
}

fn capture_event(event_type: u32) -> InputEventRaw {
    InputEventRaw {
        event_type,
        ..Default::default()
    }
}

/// LD_PRELOAD hook for `SDL_PollEvent`.
#[no_mangle]
pub unsafe extern "C" fn SDL_PollEvent(event: *mut c_void) -> i32 {
    let Some(real_fn) = real_poll_event() else {
        return 0;
    };

    let mut consumed = 0;
    loop {
        let result = real_fn(event);
        if result == 0 {
            return 0;
        }

        if !state::overlay_active() {
            return result;
        }

        let event_type = *(event as *const u32);
        if event_type == SDL_CONTROLLERBUTTONDOWN || event_type == SDL_JOYBUTTONDOWN {
            let button = read_button(event);
            let was_visible = state::is_visible();
            let handled = handle_button(button);

            // Also consume a handled Guide press after it closes the overlay.
            if (handled || was_visible) && consumed < 64 {
                consumed += 1;
                continue;
            }
        } else if event_type == SDL_CONTROLLERBUTTONUP || event_type == SDL_JOYBUTTONUP {
            let was_visible = state::is_visible();
            let handled = handle_button_release(read_button(event));
            if (handled || was_visible) && consumed < 64 {
                consumed += 1;
                continue;
            }
        }

        return result;
    }
}
