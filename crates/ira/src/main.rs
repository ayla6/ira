use gtk4::prelude::*;
use ira::activate::{activate, remove_source};
use ira::ui::{restore_content, SharedState};
use std::cell::RefCell;
use std::rc::Rc;
#[cfg(feature = "trace")]
use tracing_subscriber::prelude::*;

#[cfg(feature = "trace")]
fn init_tracing() -> Option<tracing_chrome::FlushGuard> {
    if std::env::var("IRA_TRACE").is_err() {
        return None;
    }
    let (chrome_layer, guard) = tracing_chrome::ChromeLayerBuilder::new()
        .file("ira-trace.json")
        .include_args(true)
        .build();
    tracing_subscriber::registry().with(chrome_layer).init();
    Some(guard)
}

#[cfg(not(feature = "trace"))]
fn init_tracing() -> Option<()> {
    None
}

/// When IRA_CRITICAL_BACKTRACE is set, every GTK critical warning prints a
/// Rust backtrace after it, so a single run pinpoints which code path passed
/// the stale widget pointer.
fn init_critical_backtrace() {
    if std::env::var_os("IRA_CRITICAL_BACKTRACE").is_none() {
        return;
    }
    gtk4::glib::log_set_handler(
        Some("Gtk"),
        gtk4::glib::LogLevels::LEVEL_CRITICAL,
        false,
        false,
        |_domain, _level, message| {
            eprintln!(
                "ira: critical: {message}\n{}",
                std::backtrace::Backtrace::force_capture()
            );
        },
    );
}

/// Publishes the display size for overlay processes: the overlay host
/// sizes its window from this at startup so the first frame is already
/// right (no flash-then-resize). Largest monitor wins; absent without a
/// display (headless/tests), in which case the host stays default-sized
/// and follows the game extent instead. Inherited through spawn.
fn publish_overlay_screen_size() {
    use gtk4::gdk::prelude::{DisplayExt, MonitorExt};
    let size = gtk4::gdk::Display::default().and_then(|d| {
        let monitors = d.monitors();
        let mut best: Option<(i32, i32)> = None;
        for i in 0..monitors.n_items() {
            let Some(m) = monitors
                .item(i)
                .and_then(|o| o.downcast::<gtk4::gdk::Monitor>().ok())
            else {
                continue;
            };
            let g = m.geometry();
            let area = i64::from(g.width()) * i64::from(g.height());
            let best_area =
                best.map_or(-1, |(bw, bh)| i64::from(bw) * i64::from(bh));
            if area > best_area {
                best = Some((g.width(), g.height()));
            }
        }
        best
    });
    if let Some((w, h)) = size {
        std::env::set_var("IRA_OVERLAY_SCREEN_W", w.to_string());
        std::env::set_var("IRA_OVERLAY_SCREEN_H", h.to_string());
    }
}

fn main() {
    // Under Gamescope's nested compositor, run GTK against its XWayland:
    // the Wayland-native path segfaults the compositor on menus (xdg_popup).
    // Respect an explicit GDK_BACKEND; this only fills in the default.
    let in_gamescope = ira::ui::big_picture::running_in_gamescope();
    if in_gamescope && std::env::var("GDK_BACKEND").is_err() {
        std::env::set_var("GDK_BACKEND", "x11");
    }
    // Gamescope never draws server-side decorations, and the fullscreen
    // shell has no headerbar of its own — without this GTK paints a fake
    // titlebar inside the fullscreen window. Respect an explicit GTK_CSD.
    if in_gamescope && std::env::var("GTK_CSD").is_err() {
        std::env::set_var("GTK_CSD", "0");
    }

    // Big-picture entry: `--big-picture` or running under Gamescope.
    // Fullscreens the main window, dropping the desktop chrome — nothing
    // maximizes the window for the app inside Gamescope, so its default
    // size otherwise stays put with the header bars on.
    let big_picture = ira::ui::big_picture::is_big_picture();
    if big_picture {
        eprintln!("ira: big picture mode");
        // Before GTK initializes fontconfig, so the app font is usable in
        // this same session.
        ira::ui::big_picture_font::install();
    }

    let _flush_guard = init_tracing();
    init_critical_backtrace();
    // Keep large image buffers (covers, heroes, logos) mmap-backed so freeing
    // them returns the memory to the OS instead of leaving fragmented holes in
    // the sbrk arena that malloc_trim can't reclaim. Pinning the threshold
    // also stops glibc from dynamically raising it.
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 256 * 1024);
    }
    ira::i18n::init();

    gio::resources_register_include!("ira.gresource")
        .expect("failed to register application resources");

    let app = adw::Application::new(Some("com.github.ira"), gio::ApplicationFlags::empty());
    // Register the flag so glib's option parser accepts it; main() below
    // reads it back out of argv.
    app.add_main_option(
        "big-picture",
        glib::Char::from(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Start in big picture mode",
        None,
    );

    let state_holder: Rc<RefCell<Option<SharedState>>> = Rc::new(RefCell::new(None));

    app.connect_activate({
        let state_holder = state_holder.clone();
        move |app| {
            if let Some(state) = state_holder.borrow().as_ref() {
                let win = state.borrow().window.clone();
                win.present();
                restore_content(state);
                return;
            }
            publish_overlay_screen_size();
            let state = activate(app);
            *state_holder.borrow_mut() = Some(state);
        }
    });

    app.run();

    // The input daemon outlives short-lived callers by design; tell it Ira
    // is done so it does not linger on its idle timer (or forever, if a
    // session wedged). A running game makes it stay until the game ends.
    ira_launcher::input_daemon::shutdown_daemon();

    if let Some(state) = state_holder.borrow().as_ref() {
        remove_source(state);
    }
    let state = state_holder.borrow_mut().take();
    let db = state.as_ref().map(|s| s.borrow().db.clone());
    drop(state);
    if let Some(db) = db {
        if let Err(e) = ira_db::checkpoint(&db) {
            eprintln!("Failed to checkpoint database on shutdown: {}", e);
        }
    }
}
