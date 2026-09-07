//! The couch's virtual keyboard: a QWERTY panel over a dimmed page used to
//! name and rename groups. For now it is the front end only — arrows move
//! the key cursor, Confirm types or acts, B deletes a character, Escape
//! cancels — and the finished text is handed to the caller's callback.

use crate::ui::css::*;
use crate::ui::state::SharedState;
use gtk4::prelude::*;
use gtk4::Widget;
use std::cell::{Cell, RefCell};

/// One key of the layout: a character to type, or a special action.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Key {
    Char(char),
    Space,
    Del,
    Ok,
    Cancel,
}

/// The layout rows, top to bottom. Keys are uppercase; the buffer keeps
/// them as typed.
const ROWS: &[&[Key]] = &[
    &[Key::Char('1'), Key::Char('2'), Key::Char('3'), Key::Char('4'), Key::Char('5'), Key::Char('6'), Key::Char('7'), Key::Char('8'), Key::Char('9'), Key::Char('0')],
    &[Key::Char('Q'), Key::Char('W'), Key::Char('E'), Key::Char('R'), Key::Char('T'), Key::Char('Y'), Key::Char('U'), Key::Char('I'), Key::Char('O'), Key::Char('P')],
    &[Key::Char('A'), Key::Char('S'), Key::Char('D'), Key::Char('F'), Key::Char('G'), Key::Char('H'), Key::Char('J'), Key::Char('K'), Key::Char('L')],
    &[Key::Char('Z'), Key::Char('X'), Key::Char('C'), Key::Char('V'), Key::Char('B'), Key::Char('N'), Key::Char('M')],
    &[Key::Space, Key::Del, Key::Ok, Key::Cancel],
];

/// What to do with the finished name.
type NameCallback = Box<dyn Fn(&SharedState, &str)>;

pub(super) struct Keyboard {
    /// The menu surface: dim layer with the panel floating on top.
    root: gtk4::Overlay,
    panel: gtk4::Box,
    /// The text preview the typed characters show up in.
    preview: gtk4::Label,
    /// Key cursor position: (row, column).
    cursor: Cell<(usize, usize)>,
    buffer: RefCell<String>,
    on_ok: RefCell<Option<NameCallback>>,
}

impl Keyboard {
    pub(super) fn new(state: &SharedState) -> Self {
        let dim = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        dim.add_css_class(CSS_BP_MENU_DIM);
        {
            let close_state = state.clone();
            let click = gtk4::GestureClick::new();
            click.connect_pressed(move |_, _, _, _| {
                if let Some(big) = close_state.borrow().big_picture.clone() {
                    big.keyboard.close();
                }
            });
            dim.add_controller(click);
        }
        let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
        panel.add_css_class(CSS_BP_MENU_PANEL);
        panel.set_halign(gtk4::Align::Center);
        panel.set_valign(gtk4::Align::Center);
        panel.set_size_request(720, -1);

        let root = gtk4::Overlay::new();
        root.set_child(Some(&dim));
        root.add_overlay(&panel);
        root.set_visible(false);
        Self {
            root,
            panel,
            preview: gtk4::Label::new(None),
            cursor: Cell::new((1, 0)),
            buffer: RefCell::new(String::new()),
            on_ok: RefCell::new(None),
        }
    }

    pub(super) fn root(&self) -> &Widget {
        self.root.upcast_ref()
    }

    pub(super) fn is_open(&self) -> bool {
        self.root.is_visible()
    }

    /// Open with a prompt, any starting text, and what to do with the
    /// finished name.
    pub(super) fn open(
        &self,
        state: &SharedState,
        prompt: &str,
        initial: &str,
        on_ok: NameCallback,
    ) {
        *self.buffer.borrow_mut() = initial.to_string();
        *self.on_ok.borrow_mut() = Some(on_ok);
        self.cursor.set((1, 0));
        crate::ui::helpers::clear_children(&self.panel);
        let title = gtk4::Label::new(Some(prompt));
        title.set_xalign(0.0);
        title.add_css_class(CSS_BP_PAGE_TITLE);
        crate::ui::helpers::crisp_label(&title);
        self.panel.append(&title);
        self.preview.add_css_class(CSS_BP_KEY_PREVIEW);
        self.preview.set_xalign(0.0);
        crate::ui::helpers::crisp_label(&self.preview);
        self.panel.append(&self.preview);
        for (row, keys) in ROWS.iter().enumerate() {
            let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            row_box.set_halign(gtk4::Align::Center);
            for (col, key) in keys.iter().enumerate() {
                let button = self.key_button(state, *key, (row, col));
                row_box.append(&button);
            }
            self.panel.append(&row_box);
        }
        self.refresh_preview();
        self.refresh_cursor();
        self.root.set_visible(true);
    }

    pub(super) fn close(&self) {
        self.root.set_visible(false);
        *self.on_ok.borrow_mut() = None;
        *self.buffer.borrow_mut() = String::new();
    }

    fn key_label(key: Key) -> String {
        match key {
            Key::Char(c) => c.to_string(),
            Key::Space => crate::tr!("Space"),
            Key::Del => crate::tr!("Del"),
            Key::Ok => crate::tr!("OK"),
            Key::Cancel => crate::tr!("Cancel"),
        }
    }

    fn key_button(
        &self,
        state: &SharedState,
        key: Key,
        position: (usize, usize),
    ) -> gtk4::Widget {
        let button = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        button.add_css_class(CSS_BP_KEY);
        if self.cursor.get() == position {
            button.add_css_class(CSS_BP_KEY_SELECTED);
        }
        let label = gtk4::Label::new(Some(&Self::key_label(key)));
        label.set_halign(gtk4::Align::Center);
        label.set_valign(gtk4::Align::Center);
        crate::ui::helpers::crisp_label(&label);
        button.append(&label);
        let wide = matches!(key, Key::Space | Key::Del | Key::Ok | Key::Cancel);
        if wide {
            button.set_size_request(116, 56);
        } else {
            button.set_size_request(56, 56);
        }
        let click_state = state.clone();
        let click = gtk4::GestureClick::new();
        click.connect_pressed(move |_, _, _, _| {
            if let Some(big) = click_state.borrow().big_picture.clone() {
                big.keyboard.cursor.set(position);
                big.keyboard.press(&click_state, key);
            }
        });
        button.add_controller(click);
        button.upcast()
    }

    /// Move the key cursor, clamping each row to its own length.
    pub(super) fn move_cursor(&self, dx: i32, dy: i32) {
        let (row, col) = self.cursor.get();
        let row = (row as i64 + dy as i64).clamp(0, ROWS.len() as i64 - 1) as usize;
        let col = (col as i64 + dx as i64).clamp(0, ROWS[row].len() as i64 - 1) as usize;
        self.cursor.set((row, col));
        self.refresh_cursor();
    }

    /// Type the key under the cursor (or the given one from a mouse
    /// click), updating the buffer or running the special action.
    pub(super) fn press(&self, state: &SharedState, key: Key) {
        match key {
            Key::Char(c) => self.buffer.borrow_mut().push(c),
            Key::Space => self.buffer.borrow_mut().push(' '),
            Key::Del => {
                self.buffer.borrow_mut().pop();
            }
            Key::Ok => {
                let text = self.buffer.borrow().trim().to_string();
                let callback = self.on_ok.borrow_mut().take();
                self.close();
                if let Some(callback) = callback {
                    callback(state, &text);
                }
                return;
            }
            Key::Cancel => {
                self.close();
                return;
            }
        }
        self.refresh_preview();
    }

    /// Delete the last character (the B button's job on a keyboard).
    pub(super) fn backspace(&self) {
        self.buffer.borrow_mut().pop();
        self.refresh_preview();
    }

    /// Type whichever key the cursor rests on (Confirm from the router).
    pub(super) fn press_selected(&self, state: &SharedState) {
        let (row, col) = self.cursor.get();
        let Some(keys) = ROWS.get(row) else {
            return;
        };
        let Some(key) = keys.get(col) else {
            return;
        };
        self.press(state, *key);
    }

    fn refresh_preview(&self) {
        let text = self.buffer.borrow().clone();
        let shown = if text.is_empty() {
            crate::tr!("Type a name…")
        } else {
            text
        };
        self.preview.set_text(&shown);
    }

    fn refresh_cursor(&self) {
        // Re-highlight without a rebuild: walk the panel's rows.
        let (cursor_row, cursor_col) = self.cursor.get();
        let mut row_index = 0usize;
        let mut child = self.panel.first_child();
        while let Some(row) = child {
            // Panel children: title, preview, then one box per key row.
            if row_index >= 2 && row.type_().name() == "GtkBox" {
                let mut key_index = 0usize;
                let mut key_child = row.first_child();
                while let Some(key) = key_child {
                    let selected =
                        row_index - 2 == cursor_row && key_index == cursor_col;
                    if selected {
                        key.add_css_class(CSS_BP_KEY_SELECTED);
                    } else {
                        key.remove_css_class(CSS_BP_KEY_SELECTED);
                    }
                    key_child = key.next_sibling();
                    key_index += 1;
                }
            }
            child = row.next_sibling();
            row_index += 1;
        }
    }
}
