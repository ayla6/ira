//! Exported C API — the Vulkan layer calls these via `dlsym(RTLD_DEFAULT, ...)`.
//!
//! The Vulkan layer's `shim_bridge` resolves these at first present:
//! ```c
//! int ira_overlay_poll_events(struct InputEventRaw* out, int max);
//! void ira_overlay_set_visible(int visible);
//! int ira_overlay_is_visible(void);
//! void ira_overlay_mouse_pos(int* x, int* y);
//! ```

use std::ffi::c_int;

use ira_overlay_ipc::InputEventRaw;

use crate::state;

/// Drains up to `max` input events from the queue into `out`.
/// Returns the number of events written.
/// Called by the Vulkan layer every frame when the overlay is visible.
#[no_mangle]
pub extern "C" fn ira_overlay_poll_events(out: *mut InputEventRaw, max: usize) -> usize {
    if out.is_null() {
        return 0;
    }
    let buf = unsafe { std::slice::from_raw_parts_mut(out, max) };
    state::drain_events(buf)
}

/// Sets the overlay visibility. Called by the Vulkan layer when toggling.
#[no_mangle]
pub extern "C" fn ira_overlay_set_visible(visible: c_int) {
    state::set_visible(visible != 0);
}

/// Returns 1 if the overlay is visible, 0 otherwise.
#[no_mangle]
pub extern "C" fn ira_overlay_is_visible() -> c_int {
    if state::is_visible() {
        1
    } else {
        0
    }
}

/// Returns 1 if SDL2 hooks are active (SDL2 was detected via LD_PRELOAD).
/// When true, the Vulkan layer skips evdev gamepad polling since SDL hooks
/// can consume events (evdev can't).
#[no_mangle]
pub extern "C" fn ira_overlay_has_sdl() -> c_int {
    if state::has_sdl() {
        1
    } else {
        0
    }
}

/// Returns the current mouse position (set by X11 motion events).
#[no_mangle]
pub extern "C" fn ira_overlay_mouse_pos(x: *mut c_int, y: *mut c_int) {
    if !x.is_null() && !y.is_null() {
        let (mx, my) = state::mouse_pos();
        unsafe {
            *x = mx;
            *y = my;
        }
    }
}

/// Increments the present counter. Called by the Vulkan layer on every queue_present.
#[no_mangle]
pub extern "C" fn ira_overlay_increment_present_count() {
    state::increment_present_count();
}

/// Resets the present counter to zero. Called by the Vulkan layer when a new
/// swapchain is created, so the "ready" delay restarts after resolution changes.
#[no_mangle]
pub extern "C" fn ira_overlay_reset_present_count() {
    state::reset_present_count();
}

/// Returns 1 if enough frames have been presented for the overlay to be safe.
#[no_mangle]
pub extern "C" fn ira_overlay_ready_for_overlay() -> c_int {
    if state::ready_for_overlay() {
        1
    } else {
        0
    }
}
