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

fn main() {
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

    // Big-picture entry (gamescope, couch use): `--big-picture` or
    // IRA_BIG_PICTURE=1 fullscreens the main window, dropping the desktop
    // chrome. Fullscreening is what makes the UI fill Gamescope's display —
    // the window's default size otherwise stays put inside it.
    let big_picture = std::env::args().any(|arg| arg == "--big-picture")
        || std::env::var("IRA_BIG_PICTURE").is_ok_and(|value| value == "1");

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
            let state = activate(app);
            if big_picture {
                state.borrow().window.fullscreen();
            }
            *state_holder.borrow_mut() = Some(state);
        }
    });

    app.run();

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
