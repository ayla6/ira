use adw::prelude::*;
use std::rc::Rc;

const XKB_EVDEV_OFFSET: u32 = 8;

pub(super) fn show_keyboard_output_capture(
    parent: &impl IsA<gtk4::Widget>,
    on_capture: impl Fn(u16) + 'static,
) {
    let (dialog, status, content) = capture_dialog(
        &crate::tr!("Capture keyboard output"),
        &crate::tr!("Press one key to assign it. Escape cancels."),
    );
    let on_capture: Rc<dyn Fn(u16)> = Rc::new(on_capture);
    let key = gtk4::EventControllerKey::new();
    let dialog_for_key = dialog.clone();
    key.connect_key_pressed(move |_, keyval, hardware_keycode, _| {
        if keyval == gdk4::Key::Escape {
            dialog_for_key.close();
            return gtk4::glib::Propagation::Stop;
        }
        if is_modifier_key(keyval) {
            return gtk4::glib::Propagation::Stop;
        }
        let Some(keycode) = evdev_keycode(hardware_keycode) else {
            status.set_text(&crate::tr!("That key is not supported."));
            return gtk4::glib::Propagation::Stop;
        };
        on_capture(keycode);
        dialog_for_key.close();
        gtk4::glib::Propagation::Stop
    });
    dialog.add_controller(key);
    present_dialog(&dialog, parent, &content);
}

pub(super) fn evdev_keycode(hardware_keycode: u32) -> Option<u16> {
    hardware_keycode
        .checked_sub(XKB_EVDEV_OFFSET)
        .and_then(|keycode| keycode.try_into().ok())
}

fn capture_dialog(
    title: &str,
    instruction: &str,
) -> (adw::Dialog, gtk4::Label, gtk4::Box) {
    let dialog = adw::Dialog::new();
    dialog.set_title(title);
    dialog.set_content_width(360);
    dialog.set_content_height(160);

    let header = adw::HeaderBar::new();
    let cancel = gtk4::Button::with_label(&crate::tr!("Cancel"));
    let dialog_for_cancel = dialog.clone();
    cancel.connect_clicked(move |_| {
        dialog_for_cancel.close();
    });
    header.pack_end(&cancel);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_top(24);
    content.set_margin_bottom(24);
    content.set_margin_start(24);
    content.set_margin_end(24);
    content.set_focusable(true);
    let instruction = gtk4::Label::new(Some(instruction));
    instruction.set_wrap(true);
    instruction.set_xalign(0.0);
    let status = gtk4::Label::new(Some(&crate::tr!("Waiting for input...")));
    status.add_css_class("title-3");
    status.set_xalign(0.0);
    content.append(&instruction);
    content.append(&status);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));
    (dialog, status, content)
}

fn present_dialog<P: IsA<gtk4::Widget>>(dialog: &adw::Dialog, parent: &P, content: &gtk4::Box) {
    dialog.present(Some(parent));
    content.grab_focus();
}

fn is_modifier_key(keyval: gdk4::Key) -> bool {
    matches!(
        keyval,
        gdk4::Key::Shift_L
            | gdk4::Key::Shift_R
            | gdk4::Key::Control_L
            | gdk4::Key::Control_R
            | gdk4::Key::Alt_L
            | gdk4::Key::Alt_R
            | gdk4::Key::Meta_L
            | gdk4::Key::Meta_R
            | gdk4::Key::Super_L
            | gdk4::Key::Super_R
    )
}

#[cfg(test)]
mod tests {
    use super::evdev_keycode;

    #[test]
    fn test_evdev_keycode_removes_xkb_offset() {
        assert_eq!(evdev_keycode(38), Some(30));
        assert_eq!(evdev_keycode(7), None);
    }
}
