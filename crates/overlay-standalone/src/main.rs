//! Standalone overlay binary — renders to an X11 window via XCB.
//!
//! Used when running under gamescope. The overlay creates its own X11 window,
//! Vulkan instance, and swapchain. The Gamescope WSI layer handles swapchain
//! creation and buffer management with pre-multiplied alpha support.
//!
//! The window is marked as `GAMESCOPE_EXTERNAL_OVERLAY` so gamescope composites
//! it on top of the game as a separate plane (like mangoapp).
//! Visibility is toggled via the `_NET_WM_WINDOW_OPACITY` property.
//! When visible, the keyboard is grabbed so all key events go to the overlay.
//! The visibility toggle is read from shared memory (written by overlay-shim
//! when the overlay is hidden, or by the overlay itself when visible).

use std::sync::atomic::Ordering;

mod x11;
mod vulkan;

use ira_overlay::ui;
use ira_overlay_ipc::MappedShm;

fn main() {
    if std::env::var_os("IRA_OVERLAY_TEXT_BACKEND").is_none() {
        std::env::set_var("IRA_OVERLAY_TEXT_BACKEND", "pango");
    }

    let shm_path = match std::env::var("IRA_OVERLAY_SHM") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("ira-overlay-standalone: IRA_OVERLAY_SHM not set, exiting");
            return;
        }
    };

    let shm = match MappedShm::open_rw(&shm_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ira-overlay-standalone: failed to open SHM '{shm_path}': {e}");
            return;
        }
    };
    eprintln!("ira-overlay-standalone: connected to SHM '{shm_path}'");

    // Read hotkey config from SHM (written by Ira app before launch).
    {
        let hdr = shm.header();
        if hdr.toggle_keysym != 0 {
            x11::TOGGLE_KEYCODE.store(hdr.toggle_keysym + ira_overlay_ipc::X11_KEYCODE_OFFSET, Ordering::Relaxed);
            x11::TOGGLE_MODS.store(hdr.toggle_mods, Ordering::Relaxed);
        }
    }

    let x11_state = match x11::X11State::new() {
        Ok(x) => x,
        Err(e) => {
            eprintln!("ira-overlay-standalone: X11 init failed: {e}");
            return;
        }
    };

    eprintln!("ira-overlay-standalone: starting Vulkan init...");
    let mut vk = match vulkan::VulkanState::new(x11_state.connection_ptr(), x11_state.window_id()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("ira-overlay-standalone: vulkan init failed: {e}");
            return;
        }
    };
    eprintln!("ira-overlay-standalone: vulkan initialized, swapchain created");

    ui::text::init_fonts();
    ui::set_screen_size(vk.extent.width, vk.extent.height);

    let ui_renderer = unsafe { ui::UiRenderer::new(vk.fns, vk.device, vk.physical_device, vk.cmd_pool, vk.render_pass) };
    let ui_renderer = match ui_renderer {
        Some(r) => r,
        None => {
            eprintln!("ira-overlay-standalone: UI renderer init failed");
            return;
        }
    };
    eprintln!("ira-overlay-standalone: UI renderer initialized");

    let mut prev_visible = false;

    loop {
        // Poll XCB events (keyboard input when visible via keyboard grab).
        // When the keyboard is grabbed, the shim doesn't see key events,
        // so the overlay detects the toggle hotkey itself.
        let toggle_pressed = x11_state.poll_events();

        if toggle_pressed {
            let current = shm.header().overlay_visible.load(Ordering::SeqCst);
            let new_val: u32 = if current != 0 { 0 } else { 1 };
            shm.header().overlay_visible.store(new_val, Ordering::SeqCst);
        }

        // Read visibility from SHM (written by shim or by ourselves above).
        let visible = shm.header().overlay_visible.load(Ordering::SeqCst) != 0;

        if visible != prev_visible {
            x11_state.set_visible(visible);
            if visible {
                ui::mark_ui_dirty();
            }
            prev_visible = visible;
        }

        if !visible {
            std::thread::sleep(std::time::Duration::from_millis(16));
            continue;
        }

        // Render frame
        unsafe {
            ui_renderer.prepare(vk.extent);
            if !vk.render_frame(&ui_renderer) {
                let (w, h) = (vk.extent.width, vk.extent.height);
                vk.recreate_swapchain(w, h);
            }
        }
    }
}
