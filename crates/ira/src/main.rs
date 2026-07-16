use ira::activate::activate;
use ira::ui::{restore_content, SharedState};
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

fn main() {
    let app = adw::Application::new(
        Some("com.github.ira"),
        gio::ApplicationFlags::empty(),
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
            let state = activate(app);
            *state_holder.borrow_mut() = Some(state);
        }
    });

    app.run();

    let db = state_holder.borrow().as_ref().map(|s| s.borrow().db.clone());
    if let Some(db) = db {
        if let Err(e) = ira_db::checkpoint(&db) {
            eprintln!("Failed to checkpoint database on shutdown: {}", e);
        }
    }
}
