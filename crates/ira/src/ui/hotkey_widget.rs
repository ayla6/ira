use adw::prelude::*;
use gtk4::glib;
use std::cell::RefCell;
use std::rc::Rc;

const GAMEPAD_BUTTON_NAMES: &[(u16, &str)] = &[
    (0x130, "A"),
    (0x131, "B"),
    (0x132, "X"),
    (0x133, "Y"),
    (0x136, "L1"),
    (0x137, "R1"),
    (0x138, "L2"),
    (0x139, "R2"),
    (0x13a, "Select"),
    (0x13b, "Start"),
    (0x13c, "Guide"),
    (0x13d, "L3"),
    (0x13e, "R3"),
    (0x220, "DpadUp"),
    (0x221, "DpadDown"),
    (0x222, "DpadLeft"),
    (0x223, "DpadRight"),
];

fn button_code_to_name(code: u16) -> Option<&'static str> {
    GAMEPAD_BUTTON_NAMES
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, n)| *n)
}

pub struct HotkeyWidgets {
    pub kb_value: Rc<RefCell<String>>,
    pub gp_value: Rc<RefCell<String>>,
}

pub fn build_hotkey_row(
    group: &adw::PreferencesGroup,
    title: &str,
    keyboard: &str,
    gamepad: &str,
    default_kb: &str,
    default_gp: &str,
) -> HotkeyWidgets {
    let row = adw::ActionRow::new();
    row.set_title(title);

    let kb_value = Rc::new(RefCell::new(keyboard.to_string()));
    let gp_value = Rc::new(RefCell::new(gamepad.to_string()));

    // Keyboard binding button
    let kb_btn = gtk4::Button::new();
    kb_btn.set_valign(gtk4::Align::Center);
    kb_btn.add_css_class("flat");
    kb_btn.set_size_request(100, -1);
    update_kb_label(&kb_btn, &kb_value.borrow());

    // Reset keyboard to default (only visible when not default)
    let kb_reset = gtk4::Button::from_icon_name("edit-undo-symbolic");
    kb_reset.set_valign(gtk4::Align::Center);
    kb_reset.set_tooltip_text(Some(&crate::tr!("Reset to default")));
    kb_reset.add_css_class("flat");
    kb_reset.set_visible(keyboard != default_kb);

    // Gamepad binding button
    let gp_btn = gtk4::Button::new();
    gp_btn.set_valign(gtk4::Align::Center);
    gp_btn.add_css_class("flat");
    gp_btn.set_size_request(100, -1);
    update_gp_label(&gp_btn, &gp_value.borrow());

    // Reset gamepad to default
    let gp_reset = gtk4::Button::from_icon_name("edit-undo-symbolic");
    gp_reset.set_valign(gtk4::Align::Center);
    gp_reset.set_tooltip_text(Some(&crate::tr!("Reset to default")));
    gp_reset.add_css_class("flat");
    gp_reset.set_visible(gamepad != default_gp);

    row.add_suffix(&kb_btn);
    row.add_suffix(&kb_reset);
    row.add_suffix(&gtk4::Separator::new(gtk4::Orientation::Vertical));
    row.add_suffix(&gp_btn);
    row.add_suffix(&gp_reset);
    group.add(&row);

    setup_keyboard(&kb_btn, &kb_value, &kb_reset, default_kb);
    setup_gamepad(&gp_btn, &gp_value, &gp_reset, default_gp);

    HotkeyWidgets { kb_value, gp_value }
}

fn update_kb_label(btn: &gtk4::Button, value: &str) {
    let label = if value.is_empty() {
        crate::tr!("Not set")
    } else {
        value.to_string()
    };
    btn.set_label(&label);
}

fn update_gp_label(btn: &gtk4::Button, value: &str) {
    let label = if value.is_empty() {
        crate::tr!("Not set")
    } else {
        value.to_string()
    };
    btn.set_label(&label);
}

fn setup_keyboard(
    btn: &gtk4::Button,
    value: &Rc<RefCell<String>>,
    reset: &gtk4::Button,
    default: &str,
) {
    let default = default.to_string();
    let capturing = Rc::new(RefCell::new(false));

    let ec = gtk4::EventControllerKey::new();
    btn.add_controller(ec.clone());

    let btn_c = btn.clone();
    let capturing_c = capturing.clone();
    let value_c = value.clone();
    let reset_c = reset.clone();
    let default_c = default.clone();
    ec.connect_key_pressed(move |_, keyval, _keycode, state| {
        if !*capturing_c.borrow() {
            return glib::Propagation::Proceed;
        }

        let key_name = keyval_to_name(keyval);
        if key_name == "Escape" {
            *capturing_c.borrow_mut() = false;
            update_kb_label(&btn_c, &value_c.borrow());
            return glib::Propagation::Stop;
        }

        if is_modifier(keyval) {
            return glib::Propagation::Proceed;
        }

        let mods = modifier_names(state);
        let full = if mods.is_empty() {
            key_name
        } else {
            format!("{}+{}", mods, key_name)
        };

        *value_c.borrow_mut() = full.clone();
        *capturing_c.borrow_mut() = false;
        update_kb_label(&btn_c, &full);
        reset_c.set_visible(full != default_c);
        glib::Propagation::Stop
    });

    let btn_c = btn.clone();
    let capturing_c = capturing;
    let value_c = value.clone();
    let reset_c = reset.clone();
    btn.connect_clicked(move |_| {
        if *capturing_c.borrow() {
            *capturing_c.borrow_mut() = false;
            update_kb_label(&btn_c, &value_c.borrow());
        } else {
            *capturing_c.borrow_mut() = true;
            btn_c.set_label("…");
            btn_c.set_tooltip_text(Some(&crate::tr!("Press key — Esc to cancel")));
            reset_c.set_visible(false);
        }
    });

    let right_click = gtk4::GestureClick::new();
    right_click.set_button(3);
    btn.add_controller(right_click.clone());
    let btn_c = btn.clone();
    let value_c = value.clone();
    let reset_c = reset.clone();
    let default_c = default.clone();
    right_click.connect_pressed(move |_, _, _, _| {
        *value_c.borrow_mut() = String::new();
        update_kb_label(&btn_c, "");
        reset_c.set_visible(default_c.is_empty());
    });

    let btn_c = btn.clone();
    let value_c = value.clone();
    let default_c = default;
    reset.connect_clicked(move |_| {
        *value_c.borrow_mut() = default_c.clone();
        update_kb_label(&btn_c, &default_c);
    });
}

fn setup_gamepad(
    btn: &gtk4::Button,
    value: &Rc<RefCell<String>>,
    reset: &gtk4::Button,
    default: &str,
) {
    let default = default.to_string();
    let capturing = Rc::new(RefCell::new(false));
    let pressed = Rc::new(RefCell::new(Vec::<String>::new()));
    let timeout_id = Rc::new(RefCell::new(None::<glib::SourceId>));

    let btn_c = btn.clone();
    let capturing_c = capturing.clone();
    let pressed_c = pressed;
    let timeout_c = timeout_id.clone();
    let value_c = value.clone();
    let reset_c = reset.clone();
    let default_c = default.clone();
    btn.connect_clicked(move |_| {
        if *capturing_c.borrow() {
            stop_gamepad_capture(&capturing_c, &timeout_c);
            update_gp_label(&btn_c, &value_c.borrow());
        } else {
            *capturing_c.borrow_mut() = true;
            pressed_c.borrow_mut().clear();
            btn_c.set_label("…");
            btn_c.set_tooltip_text(Some(&crate::tr!(
                "Press gamepad button — right-click to cancel"
            )));
            reset_c.set_visible(false);

            let btn_cc = btn_c.clone();
            let capturing_cc = capturing_c.clone();
            let pressed_cc = pressed_c.clone();
            let timeout_cc = timeout_c.clone();
            let value_cc = value_c.clone();
            let reset_cc = reset_c.clone();
            let default_cc = default_c.clone();
            let id = glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                if !*capturing_cc.borrow() {
                    return glib::ControlFlow::Break;
                }
                if let Some(name) = poll_gamepad_buttons() {
                    let mut p = pressed_cc.borrow_mut();
                    if !p.contains(&name) {
                        p.push(name);
                    }
                    let pressed_cc2 = pressed_cc.clone();
                    let btn_cc2 = btn_cc.clone();
                    let capturing_cc2 = capturing_cc.clone();
                    let value_cc2 = value_cc.clone();
                    let reset_cc2 = reset_cc.clone();
                    let default_cc2 = default_cc.clone();
                    let id2 =
                        glib::timeout_add_local(std::time::Duration::from_millis(300), move || {
                            let captured = pressed_cc2.borrow().clone();
                            let combo = captured.join("+");
                            *value_cc2.borrow_mut() = combo.clone();
                            update_gp_label(&btn_cc2, &combo);
                            reset_cc2.set_visible(combo != default_cc2);
                            *capturing_cc2.borrow_mut() = false;
                            glib::ControlFlow::Break
                        });
                    *timeout_cc.borrow_mut() = Some(id2);
                }
                glib::ControlFlow::Continue
            });
            *timeout_c.borrow_mut() = Some(id);
        }
    });

    let right_click = gtk4::GestureClick::new();
    right_click.set_button(3);
    btn.add_controller(right_click.clone());
    let btn_c = btn.clone();
    let capturing_c = capturing;
    let timeout_c = timeout_id;
    let value_c = value.clone();
    let reset_c = reset.clone();
    let default_c = default.clone();
    right_click.connect_pressed(move |_, _, _, _| {
        stop_gamepad_capture(&capturing_c, &timeout_c);
        *value_c.borrow_mut() = String::new();
        update_gp_label(&btn_c, "");
        reset_c.set_visible(default_c.is_empty());
    });

    let btn_c = btn.clone();
    let value_c = value.clone();
    let default_c = default;
    reset.connect_clicked(move |_| {
        *value_c.borrow_mut() = default_c.clone();
        update_gp_label(&btn_c, &default_c);
    });
}

fn stop_gamepad_capture(
    capturing: &Rc<RefCell<bool>>,
    timeout: &Rc<RefCell<Option<glib::SourceId>>>,
) {
    *capturing.borrow_mut() = false;
    if let Some(id) = timeout.borrow_mut().take() {
        id.remove();
    }
}

#[repr(C)]
struct InputEvent {
    tv_sec: i64,
    tv_usec: i64,
    type_: u16,
    code: u16,
    value: i32,
}

fn poll_gamepad_buttons() -> Option<String> {
    let entries = std::fs::read_dir("/dev/input").ok()?;
    let mut found: Option<String> = None;

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("event") {
            continue;
        }
        let path = std::ffi::CString::new(entry.path().to_string_lossy().as_bytes()).ok()?;
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
        if fd < 0 {
            continue;
        }

        let mut buf = [0u8; 24 * 16]; // up to 16 events
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) };
        unsafe {
            libc::close(fd);
        }

        if n <= 0 {
            continue;
        }

        let count = (n as usize) / 24;
        for i in 0..count {
            let ptr = buf.as_ptr() as *const InputEvent;
            let event = unsafe { &*ptr.add(i) };
            if event.type_ == 0x01 && event.value == 1 {
                if let Some(name) = button_code_to_name(event.code) {
                    found = Some(name.to_string());
                    break;
                }
            }
        }
        if found.is_some() {
            break;
        }
    }
    found
}

fn is_modifier(keyval: gdk4::Key) -> bool {
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

fn modifier_names(state: gdk4::ModifierType) -> String {
    let mut parts = Vec::new();
    if state.contains(gdk4::ModifierType::CONTROL_MASK) {
        parts.push("Ctrl");
    }
    if state.contains(gdk4::ModifierType::SHIFT_MASK) {
        parts.push("Shift");
    }
    if state.contains(gdk4::ModifierType::ALT_MASK) {
        parts.push("Alt");
    }
    if state.contains(gdk4::ModifierType::SUPER_MASK) {
        parts.push("Super");
    }
    parts.join("+")
}

fn keyval_to_name(keyval: gdk4::Key) -> String {
    let name = keyval.name().unwrap_or_default();
    let name = name.as_str();
    match name {
        "Tab" | "ISO_Left_Tab" => "Tab".to_string(),
        "Return" => "Return".to_string(),
        "Escape" => "Escape".to_string(),
        "space" => "Space".to_string(),
        n if n.starts_with('F') && n.len() <= 3 => n.to_string(),
        n if n.len() == 1 => n.to_string(),
        _ => name.to_string(),
    }
}
