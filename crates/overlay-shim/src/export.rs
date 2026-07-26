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
