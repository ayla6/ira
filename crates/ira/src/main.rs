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

/// Run GTK on Gamescope's XWayland (Wayland segfaults on menus) without
/// server decorations (no GTK_CSD paints a fake titlebar fullscreen).
/// Explicit env wins; this only fills in the defaults.
fn apply_gamescope_env_defaults() {
    if !ira::ui::big_picture::running_in_gamescope() {
        return;
    }
    if std::env::var("GDK_BACKEND").is_err() {
        std::env::set_var("GDK_BACKEND", "x11");
    }
    if std::env::var("GTK_CSD").is_err() {
        std::env::set_var("GTK_CSD", "0");
    }
}
/// Silence the benign groups-tile measure warning: the tiles carry a fixed
/// square size request — that request IS their squareness, removing it
/// stretches the collage — and square requests measure transitional widths
/// during slides. Must be a writer func — measure warnings bypass classic
/// handlers entirely.
fn init_layout_warning_filter() {
    gtk4::glib::log_set_writer_func(|level, fields| {
        use gtk4::glib::{LogLevel, LogWriterOutput};
        if level == LogLevel::Warning {
            let mut domain = None;
            let mut message = None;
            for field in fields {
                match field.key() {
                    "GLIB_DOMAIN" => domain = field.value_str(),
                    "MESSAGE" => message = field.value_str(),
                    _ => {}
                }
            }
            if domain == Some("Gtk")
                && message.is_some_and(|m| {
                    m.starts_with("Trying to measure GtkBox")
                        && m.contains("but it needs at least")
                })
            {
                return LogWriterOutput::Handled;
            }
        }
        LogWriterOutput::Unhandled
    });
}

/// When IRA_CRITICAL_BACKTRACE is set, every GTK critical prints a Rust
/// backtrace after it, pinpointing which code path passed the stale pointer.
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

/// Publish display size for overlay hosts so the first frame is already
/// right (largest monitor wins). Absent headless; host stays default-sized.
/// Inherited through spawn.
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
    eprintln!("ira: starting build {}", env!("IRA_BUILD_HASH"));
    apply_gamescope_env_defaults();

    // Big-picture entry: `--big-picture` or running under Gamescope.
    // Gamescope never maximizes the window, so fullscreen explicitly.
    let big_picture = ira::ui::big_picture::is_big_picture();
    if big_picture {
        eprintln!("ira: big picture mode");
        // Before GTK initializes fontconfig, so the app font is usable in
        // this same session.
        ira_widgets::font::install();
    }

    let _flush_guard = init_tracing();
    init_critical_backtrace();
    init_layout_warning_filter();
    // Keep large image buffers mmap-backed so freeing returns memory to the OS
    // instead of fragmenting the arena; stop glibc raising the threshold.
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

    // Input daemon outlives callers by design; tell it Ira is done so it does
    // not linger (or forever, if wedged). A running game keeps it until it ends.
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
