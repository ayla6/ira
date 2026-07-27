//! Standalone overlay binary — renders to a wlr_layer_shell surface.
//!
//! Used when running under gamescope (or any wlroots compositor).
//! The overlay creates its own Wayland surface, Vulkan instance, and swapchain.
//! Input is handled by the compositor via keyboard interactivity on the layer surface.
//!
//! The visibility toggle is read from shared memory (written by overlay-shim).

use std::sync::atomic::Ordering;

mod wayland;
mod vulkan;

use ira_overlay::ui;
use ira_overlay_ipc::MappedShm;

fn main() {
    // Default to pango backend for the standalone overlay.
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

    let mut wl = match wayland::WaylandState::new() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("ira-overlay-standalone: wayland init failed: {e}");
            return;
        }
    };
    eprintln!("ira-overlay-standalone: wayland connected, surface created");

    let mut vk = match vulkan::VulkanState::new(wl.display_ptr(), wl.surface_ptr()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("ira-overlay-standalone: vulkan init failed: {e}");
            return;
        }
    };
    eprintln!("ira-overlay-standalone: vulkan initialized, swapchain created");

    // Initialize fonts (reads IRA_OVERLAY_TEXT_BACKEND env var)
    ui::text::init_fonts();

    // Set screen size for the UI
    ui::set_screen_size(vk.extent.width, vk.extent.height);

    // Create the UI renderer
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
        // Dispatch Wayland events
        wl.dispatch();

        // Read visibility from SHM (written by overlay-shim)
        let visible = shm.header().overlay_visible.load(Ordering::SeqCst) != 0;

        // Toggle keyboard interactivity when visibility changes
        if visible != prev_visible {
            wl.set_keyboard_interactivity(visible);
            if visible {
                ui::mark_ui_dirty();
            }
            prev_visible = visible;
        }

        if !visible {
            // Sleep briefly to avoid spinning when hidden
            std::thread::sleep(std::time::Duration::from_millis(16));
            continue;
        }

        // Check for resize
        if let Some((w, h)) = wl.take_pending_resize() {
            if w > 0 && h > 0 && (w != vk.extent.width || h != vk.extent.height) {
                unsafe { vk.recreate_swapchain(w, h) };
                ui::set_screen_size(vk.extent.width, vk.extent.height);
                ui::mark_ui_dirty();
            }
        }

        // Render frame
        unsafe {
            ui_renderer.prepare(vk.extent);
            ui_renderer.update_atlas(vk.fns, vk.cmd, vk.fence);
            if !vk.render_frame(&ui_renderer) {
                // Swapchain suboptimal or out of date — recreate on next frame
                let (w, h) = (vk.extent.width, vk.extent.height);
                vk.recreate_swapchain(w, h);
            }
        }
    }
}
