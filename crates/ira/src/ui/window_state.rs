use super::state::SharedState;
use gtk4::prelude::*;
use std::rc::Rc;

pub(super) fn restore(window: &adw::ApplicationWindow, state: &SharedState) {
    let saved = state.borrow().desktop_window;
    let (width, height) = saved.normal_size();
    window.set_default_size(width, height);
    window.set_maximized(saved.maximized);
    for property in ["default-width", "default-height", "maximized"] {
        let weak = Rc::downgrade(state);
        window.connect_notify_local(Some(property), move |window, _| {
            // Unmapping/destroying can clear maximized; preserve the last live state.
            if !window.is_mapped() || window.is_fullscreen() {
                return;
            }
            if let Some(state) = weak.upgrade() {
                state.borrow_mut().desktop_window = ira_config::WindowState {
                    width: window.default_width(),
                    height: window.default_height(),
                    maximized: window.is_maximized(),
                };
            }
        });
    }
}
