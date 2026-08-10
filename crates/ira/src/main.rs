use gtk4::prelude::*;
use ira::activate::activate;
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

fn main() {
    let _flush_guard = init_tracing();

    let app = adw::Application::new(Some("com.github.ira"), gio::ApplicationFlags::empty());

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
            *state_holder.borrow_mut() = Some(state);
        }
    });

    app.run();

    let db = state_holder
        .borrow()
        .as_ref()
        .map(|s| s.borrow().db.clone());
    if let Some(db) = db {
        if let Err(e) = ira_db::checkpoint(&db) {
            eprintln!("Failed to checkpoint database on shutdown: {}", e);
        }
    }
}
