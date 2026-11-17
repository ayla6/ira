use super::state::SharedState;
use crate::desktop::tray::{self, TrayAction};
use adw::prelude::*;
use std::rc::Rc;

pub fn start(state: &SharedState) {
    let (sender, receiver) = async_channel::bounded(32);
    let weak = Rc::downgrade(state);
    glib::MainContext::default().spawn_local(async move {
        while let Ok(action) = receiver.recv().await {
            let Some(state) = weak.upgrade() else { break };
            let application = state.borrow().window.application();
            match action {
                TrayAction::Show => {
                    if let Some(application) = application {
                        application.activate();
                    }
                }
                TrayAction::Hide => {
                    let visible = state.borrow().window.is_visible();
                    if visible {
                        super::background::hide_to_background(&state);
                    }
                }
                TrayAction::Quit => {
                    if let Some(application) = application {
                        application.quit();
                    }
                    break;
                }
            }
        }
    });
    let weak = Rc::downgrade(state);
    glib::MainContext::default().spawn_local(async move {
        match gio::spawn_blocking(move || tray::start(sender)).await {
            Ok(Ok(handle)) => {
                if let Some(state) = weak.upgrade() {
                    let visible = state.borrow().window.is_visible();
                    handle.set_visible(visible);
                    state.borrow_mut().tray = Some(handle);
                }
            }
            Ok(Err(error)) => eprintln!("Tray unavailable: {error}"),
            Err(_) => eprintln!("Tray initialization worker panicked"),
        }
    });
}

pub(super) fn watch_window(window: &adw::ApplicationWindow, state: &SharedState) {
    let weak = Rc::downgrade(state);
    window.connect_visible_notify(move |window| {
        let Some(state) = weak.upgrade() else { return };
        let state = state.borrow();
        if let Some(tray) = state.tray.as_ref() {
            tray.set_visible(window.is_visible());
        }
    });
}

pub fn stop(state: &SharedState) {
    state.borrow_mut().tray.take();
}
