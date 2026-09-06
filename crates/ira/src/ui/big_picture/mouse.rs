//! Mouse and touchpad behavior for the big picture shell. Clicking works
//! through the covers' own gesture handlers, and both pages scroll natively
//! through GTK scrolled windows (wheel, touchpad pan, kinetic), so what is
//! left lives here: the cursor hides while a gamepad or the keyboard drives
//! the UI and only returns when the pointer physically moves — GTK also
//! delivers motion when content scrolls under a stationary pointer, and
//! that must not wake it.

use crate::ui::state::SharedState;
use gtk4::prelude::*;
use gtk4::Widget;
use std::cell::Cell;
use std::rc::Rc;

/// The gamepad or keyboard took over: hide the pointer until the mouse
/// physically moves again.
pub(super) fn note_controller_use(state: &SharedState) {
    let Some(big) = state.borrow().big_picture.clone() else {
        return;
    };
    if big.cursor_hidden.get() {
        return;
    }
    big.cursor_hidden.set(true);
    if let Some(cursor) = gdk4::Cursor::from_name("none", None) {
        state.borrow().window.set_cursor(Some(&cursor));
    }
}

/// Watch for mouse motion on the big picture root and bring the pointer
/// back when it actually changes position.
pub(super) fn attach(state: &SharedState, root: &Widget) {
    let motion_state = state.clone();
    // NaN start: every real position differs from it, so the first event
    // after attach shows the pointer.
    let last_position = Rc::new(Cell::new((f64::NAN, f64::NAN)));
    let motion = gtk4::EventControllerMotion::new();
    motion.connect_motion(move |_, x, y| {
        if last_position.get() == (x, y) {
            return;
        }
        last_position.set((x, y));
        show_cursor(&motion_state);
    });
    root.add_controller(motion);
}

fn show_cursor(state: &SharedState) {
    let hidden = state
        .borrow()
        .big_picture
        .as_ref()
        .is_some_and(|big| big.cursor_hidden.get());
    if !hidden {
        return;
    }
    if let Some(big) = state.borrow().big_picture.clone() {
        big.cursor_hidden.set(false);
    }
    state.borrow().window.set_cursor(None);
}
