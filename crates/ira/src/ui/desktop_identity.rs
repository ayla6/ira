use adw::prelude::*;
use std::sync::OnceLock;

static WINDOW_ID: OnceLock<String> = OnceLock::new();

pub fn init() {
    WINDOW_ID.get_or_init(|| {
        match std::thread::spawn(crate::desktop::resolve_window_identity).join() {
            Ok(Ok(Some(id))) => return id,
            Ok(Ok(None)) => {}
            Ok(Err(error)) => eprintln!("Resolve Ira's desktop launcher: {error}"),
            Err(_) => eprintln!("Desktop identity worker panicked"),
        }
        crate::desktop::protocol::APP_ID.to_string()
    });
}

fn window_id() -> &'static str {
    WINDOW_ID
        .get()
        .map(String::as_str)
        .unwrap_or(crate::desktop::protocol::APP_ID)
}

pub fn apply_program_class() {
    if let Some(display) = gtk4::gdk::Display::default().and_downcast::<gdk4_x11::X11Display>() {
        display.set_program_class(window_id());
    }
}

pub fn bind(window: &adw::ApplicationWindow) {
    // GDK ignores application IDs until mapping creates the native toplevel.
    window.connect_closure(
        "map",
        true,
        glib::closure_local!(|window: adw::ApplicationWindow| {
            if let Some(surface) = window
                .surface()
                .and_downcast::<gdk4_wayland::WaylandToplevel>()
            {
                // The compositor matches the installed desktop filename, while
                // GApplication keeps Ira's stable D-Bus identity for forwarding.
                surface.set_application_id(window_id());
            }
        }),
    );
}
