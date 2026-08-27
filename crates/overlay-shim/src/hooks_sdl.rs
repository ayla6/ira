//! SDL2 gamepad input hooking via LD_PRELOAD symbol interposition.
//!
//! When loaded via `LD_PRELOAD`, calls to `SDL_PollEvent` are resolved to our
//! version instead of the real SDL2 function. We chain through to the real
//! function via `dlsym(RTLD_NEXT, ...)`.
//!
//! Gamepad button events (`SDL_CONTROLLERBUTTONDOWN`, `SDL_JOYBUTTONDOWN`) are
//! intercepted. When the overlay is visible, they are consumed (removed from
//! the queue) so the game doesn't see them. The Guide button always toggles
//! the overlay, even when hidden.
//!
//! On first call, sets `HAS_SDL` flag so the Vulkan layer knows to skip evdev
//! (SDL hooks can consume events; evdev can't).

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::OnceLock;

use ira_overlay_ipc::{gamepad_button_mask_from_evdev, InputEventRaw, ShmHeader};

use crate::state;

// SDL2 event types (same values in SDL3)
const SDL_CONTROLLERBUTTONDOWN: u32 = 0x651;
const SDL_CONTROLLERBUTTONUP: u32 = 0x652;
const SDL_JOYBUTTONDOWN: u32 = 0x603;
const SDL_JOYBUTTONUP: u32 = 0x604;

// SDL2 controller button codes (same in SDL3)
const BTN_A: u8 = 0;
const BTN_GUIDE: u8 = 5;
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
    let Some(mask) = button_mask(button) else {
        return false;
    };
    let held = PRESSED_BUTTONS.fetch_or(mask, Ordering::Relaxed) | mask;
    let (toggle, screenshot, record) = state::gamepad_hotkeys();
    if held == screenshot {
        TOGGLE_PENDING.store(false, Ordering::Relaxed);
        state::push_event(capture_event(5));
        return true;
    }
    if held == record {
        TOGGLE_PENDING.store(false, Ordering::Relaxed);
        state::push_event(capture_event(6));
        return true;
    }
    if held == toggle {
        if state::injected_ui_disabled() {
            return false;
        }
        TOGGLE_PENDING.store(true, Ordering::Relaxed);
        return true;
    }

    // Other buttons only when overlay is visible.
    if state::injected_ui_disabled() || !state::is_visible() {
        return false;
    }

    let event = match button {
        BTN_A => InputEventRaw {
            event_type: 3,
            x: 0,
            y: 0,
            button: 0,
            keycode: KC_RETURN,
        },
        BTN_DPAD_UP => InputEventRaw {
            event_type: 3,
            x: 0,
            y: 0,
            button: 0,
            keycode: KC_UP,
        },
        BTN_DPAD_DOWN => InputEventRaw {
            event_type: 3,
            x: 0,
            y: 0,
            button: 0,
            keycode: KC_DOWN,
        },
        BTN_DPAD_LEFT => InputEventRaw {
            event_type: 3,
            x: 0,
            y: 0,
            button: 0,
            keycode: KC_LEFT,
        },
        BTN_DPAD_RIGHT => InputEventRaw {
            event_type: 3,
            x: 0,
            y: 0,
            button: 0,
            keycode: KC_RIGHT,
        },
        BTN_LEFTSHOULDER => InputEventRaw {
            event_type: 7,
            x: 0,
            y: -1,
            button: 0,
            keycode: 0,
        },
        BTN_RIGHTSHOULDER => InputEventRaw {
            event_type: 7,
            x: 0,
            y: 1,
            button: 0,
            keycode: 0,
        },
        _ => return false,
    };
    state::push_event(event);
    true
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
    let evdev = match button {
        BTN_A => 0x130,
        BTN_GUIDE => 0x13c,
        BTN_LEFTSHOULDER => 0x136,
        BTN_RIGHTSHOULDER => 0x137,
        BTN_DPAD_UP => 0x220,
        BTN_DPAD_DOWN => 0x221,
        BTN_DPAD_LEFT => 0x222,
        BTN_DPAD_RIGHT => 0x223,
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

fn should_consume_gamepad_event(was_visible: bool, handled: bool, ui_disabled: bool) -> bool {
    handled || (was_visible && !ui_disabled)
}

/// LD_PRELOAD hook for `SDL_PollEvent`.
///
/// Intercepts gamepad button events. When the overlay is visible, gamepad
/// events are consumed (the game never sees them). The Guide button always
/// toggles the overlay. Non-gamepad events always pass through.
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
            if should_consume_gamepad_event(was_visible, handled, state::injected_ui_disabled())
                && consumed < 64
            {
                consumed += 1;
                continue;
            }
        } else if event_type == SDL_CONTROLLERBUTTONUP || event_type == SDL_JOYBUTTONUP {
            let was_visible = state::is_visible();
            let handled = handle_button_release(read_button(event));
            if should_consume_gamepad_event(was_visible, handled, state::injected_ui_disabled())
                && consumed < 64
            {
                consumed += 1;
                continue;
            }
        }

        return result;
    }
}

#[cfg(test)]
mod tests {
    use super::should_consume_gamepad_event;

    #[test]
    fn test_should_consume_gamepad_event_after_guide_hides_overlay() {
        assert!(should_consume_gamepad_event(false, true, true));
    }

    #[test]
    fn test_should_not_consume_visible_ui_event_when_injected_ui_disabled() {
        assert!(!should_consume_gamepad_event(true, false, true));
    }
}
