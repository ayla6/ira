//! Standalone overlay binary — renders an overlay under gamescope.
//!
//! Used when running under gamescope. The overlay creates its own X11 window
//! and Vulkan instance, and runs under the Gamescope WSI layer: the layer
//! intercepts `vkCreateXcbSurfaceKHR` and presents the overlay's frames to
//! gamescope via Wayland (bypassing XWayland), blending with pre-multiplied
//! alpha over the game.
//!
//! The window is marked as `GAMESCOPE_EXTERNAL_OVERLAY` so gamescope composites
//! it on top of the game as a separate plane (like mangoapp).
//! Visibility is toggled via the `_NET_WM_WINDOW_OPACITY` and
//! `GAMESCOPE_EXTERNAL_OVERLAY` properties.
//! When visible, the keyboard is grabbed so all key events go to the overlay.
//! The visibility toggle is read from shared memory (written by overlay-shim
//! when the overlay is hidden, or by the overlay itself when visible).

use std::sync::atomic::Ordering;

mod capture;
mod capture_frame;
mod capture_types;
mod capture_writer;
mod ffmpeg;
mod gamepad;
mod vulkan;
mod x11;

use ira_overlay::ui;
use ira_overlay_ipc::MappedShm;

fn main() {
    ira_overlay::i18n::init();

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

    let capture_settings = capture::RecordingSettings::from_shm(&shm);
    let capture = match capture::CaptureController::new(capture_settings) {
        Ok(capture) => capture,
        Err(error) => {
            eprintln!("ira-overlay-standalone: capture initialization failed: {error}");
            return;
        }
    };

    // Read hotkey config from SHM (written by Ira app before launch).
    // X11 grabs X11 keycodes (evdev + X11_KEYCODE_OFFSET); the canonical
    // decoder fills in the default chord when the header carries zeros.
    {
        let hdr = shm.header();
        let (tog_kc, tog_mods, ..) = hdr.hotkeys();
        x11::TOGGLE_KEYCODE.store(
            tog_kc + ira_overlay_ipc::X11_KEYCODE_OFFSET,
            Ordering::Relaxed,
        );
        x11::TOGGLE_MODS.store(tog_mods, Ordering::Relaxed);
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

    let ui_renderer = unsafe {
        ui::UiRenderer::new(
            vk.fns,
            vk.device,
            vk.physical_device,
            vk.cmd_pool,
            vk.render_pass,
        )
    };
    let ui_renderer = match ui_renderer {
        Some(r) => r,
        None => {
            eprintln!("ira-overlay-standalone: UI renderer init failed");
            return;
        }
    };
    eprintln!("ira-overlay-standalone: UI renderer initialized");

    let initial_visible = shm.header().overlay_visible.load(Ordering::SeqCst) != 0;
    x11_state.set_visible(initial_visible);
    let mut prev_visible = initial_visible;
    let mut direct_capture_ready = false;
    let mut gamepad = gamepad::GamepadInput::new();

    loop {
        // Poll XCB events (keyboard input when visible via keyboard grab).
        // When the keyboard is grabbed, the shim doesn't see key events,
        // so the overlay detects the toggle hotkey itself.
        let toggle_pressed = x11_state.poll_events();

        if toggle_pressed {
            // Outcome is picked up below via the SHM visibility load.
            let _ = shm.header().toggle_visible();
        }

        let ready = shm.header().direct_capture_ready.load(Ordering::SeqCst) != 0;
        if ready != direct_capture_ready {
            capture.set_direct_capture_ready(ready);
            direct_capture_ready = ready;
        }

        let (toggle_gamepad, screenshot_gamepad, record_gamepad) = shm.header().gamepad_hotkeys();
        let overlay_visible = shm.header().overlay_visible.load(Ordering::SeqCst) != 0;
        for event in gamepad.poll(
            toggle_gamepad,
            screenshot_gamepad,
            record_gamepad,
            overlay_visible,
        ) {
            handle_gamepad_event(&shm, &capture, event);
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

fn handle_gamepad_event(
    shm: &MappedShm,
    capture: &capture::CaptureController,
    event: gamepad::GamepadEvent,
) {
    match event {
        gamepad::GamepadEvent::Toggle => {
            let _ = shm.header().toggle_visible();
        }
        gamepad::GamepadEvent::Ui(event) => ui::push_event(event),
        gamepad::GamepadEvent::Screenshot => capture.request_screenshot(),
        gamepad::GamepadEvent::Record => {
            capture.toggle_recording(capture::RecordingSettings::from_shm(shm));
        }
    }
}
