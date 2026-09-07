//! The big-picture virtual keyboard, modeled on the Switch's: a key grid
//! with a right-hand action column (backspace, return, OK), a shift and a
//! page toggle on the bottom row, and a wide space bar. It types from the
//! gamepad/keyboard cursor, from mouse clicks, and from the physical
//! keyboard, and hands the finished text to the caller's callback.

use crate::ui::css::*;
use crate::ui::state::SharedState;
use gtk4::prelude::*;
use gtk4::Widget;
use std::cell::{Cell, RefCell};

/// One key of the layout: a character to type, or a special action.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Key {
    Char(char),
    Shift,
    Page,
    Space,
    Backspace,
    Return,
    Ok,
    Cancel,
}

/// Which key table is showing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Page {
    Letters,
    Symbols,
}

/// The letter page: symbols on top, QWERTY below, and the right-hand
/// action column riding every row's last entry.
const LETTER_ROWS: &[&[Key]] = &[
    &[
        Key::Char('#'), Key::Char('['), Key::Char(']'), Key::Char('$'), Key::Char('%'),
        Key::Char('^'), Key::Char('&'), Key::Char('*'), Key::Char('('), Key::Char(')'),
        Key::Char('_'), Key::Backspace,
    ],
    &[
        Key::Char('q'), Key::Char('w'), Key::Char('e'), Key::Char('r'), Key::Char('t'),
        Key::Char('y'), Key::Char('u'), Key::Char('i'), Key::Char('o'), Key::Char('p'),
        Key::Char('@'), Key::Return,
    ],
    &[
        Key::Char('a'), Key::Char('s'), Key::Char('d'), Key::Char('f'), Key::Char('g'),
        Key::Char('h'), Key::Char('j'), Key::Char('k'), Key::Char('l'), Key::Char(';'),
        Key::Char('"'), Key::Return,
    ],
    &[
        Key::Char('z'), Key::Char('x'), Key::Char('c'), Key::Char('v'), Key::Char('b'),
        Key::Char('n'), Key::Char('m'), Key::Char('<'), Key::Char('>'), Key::Char('+'),
        Key::Char('='), Key::Ok,
    ],
    &[Key::Shift, Key::Page, Key::Space, Key::Cancel, Key::Ok],
];

/// The symbol page: digits and punctuation, same shape as the letter page.
const SYMBOL_ROWS: &[&[Key]] = &[
    &[
        Key::Char('1'), Key::Char('2'), Key::Char('3'), Key::Char('4'), Key::Char('5'),
        Key::Char('6'), Key::Char('7'), Key::Char('8'), Key::Char('9'), Key::Char('0'),
        Key::Char('-'), Key::Backspace,
    ],
    &[
        Key::Char('/'), Key::Char(':'), Key::Char(';'), Key::Char('('), Key::Char(')'),
        Key::Char('$'), Key::Char('&'), Key::Char('@'), Key::Char('"'), Key::Char('\''),
        Key::Char('*'), Key::Return,
    ],
    &[
        Key::Char('+'), Key::Char('='), Key::Char('<'), Key::Char('>'), Key::Char('%'),
        Key::Char('#'), Key::Char('!'), Key::Char('?'), Key::Char('~'), Key::Char('`'),
        Key::Char('^'), Key::Return,
    ],
    &[
        Key::Char(','), Key::Char('.'), Key::Char('\''), Key::Char('"'), Key::Char('_'),
        Key::Char('|'), Key::Char('\\'), Key::Char('{'), Key::Char('}'), Key::Char('['),
        Key::Char(']'), Key::Ok,
    ],
    &[Key::Shift, Key::Page, Key::Space, Key::Cancel, Key::Ok],
];

/// What to do with the finished name.
type NameCallback = Box<dyn Fn(&SharedState, &str)>;

pub(super) struct Keyboard {
    /// The menu surface: dim layer with the panel floating on top.
    root: gtk4::Overlay,
    panel: gtk4::Box,
    /// The text preview the typed characters show up in.
    preview: gtk4::Label,
    /// Key cursor position: (row, column) into the showing page's rows.
    cursor: Cell<(usize, usize)>,
    page: Cell<Page>,
    shift: Cell<bool>,
    /// Cursor-addressable key widgets of the showing page, for repaints.
    keys: RefCell<Vec<Vec<gtk4::Widget>>>,
    /// The prompt line the panel was opened with, kept for page/shift
    /// rebuilds.
    prompt: RefCell<String>,
    /// The connected pad's family, for the badge and hint glyphs.
    family: Cell<ira_input::ControllerFamily>,
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
                let big = close_state.borrow().big_picture.clone();
                if let Some(big) = big {
                    big.keyboard.close(&close_state);
                }
            });
            dim.add_controller(click);
        }
        let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
        panel.add_css_class(CSS_BP_MENU_PANEL);
        panel.set_halign(gtk4::Align::Center);
        panel.set_valign(gtk4::Align::Center);
        panel.set_size_request(880, -1);

        let root = gtk4::Overlay::new();
        // The overlay is a sibling of the bp-root Box, so it must carry the
        // big-picture class itself to inherit its font.
        root.add_css_class(CSS_BP_ROOT);
        root.set_child(Some(&dim));
        root.add_overlay(&panel);
        root.set_visible(false);
        Self {
            root,
            panel,
            preview: gtk4::Label::new(None),
            cursor: Cell::new((1, 0)),
            page: Cell::new(Page::Letters),
            shift: Cell::new(false),
            keys: RefCell::new(Vec::new()),
            prompt: RefCell::new(String::new()),
            family: Cell::new(ira_input::ControllerFamily::Xbox),
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
        *self.prompt.borrow_mut() = prompt.to_string();
        *self.on_ok.borrow_mut() = Some(on_ok);
        // The keyboard takes over the screen: the panel spans most of the
        // window so the keys and the hint row read at TV distance.
        let width = (state.borrow().window.width() as f64 * 0.82).round() as i32;
        self.panel.set_size_request(width.max(880), -1);
        self.cursor.set((1, 0));
        self.rebuild(state, prompt);
        self.refresh_preview();
        self.refresh_cursor();
        self.root.set_visible(true);
        // The keyboard's controls live in the bottom rail while it shows.
        if let Some(big) = state.borrow().big_picture.clone() {
            big.set_prompts(&[
                (ira_input::GamepadButton::A, &crate::tr!("Type")),
                (ira_input::GamepadButton::B, &crate::tr!("Delete")),
                (ira_input::GamepadButton::Start, &crate::tr!("Close")),
            ]);
        }
    }

    /// Close and hand the bottom rail's prompts back to the page.
    pub(super) fn close(&self, state: &SharedState) {
        self.root.set_visible(false);
        *self.on_ok.borrow_mut() = None;
        *self.buffer.borrow_mut() = String::new();
        if let Some(big) = state.borrow().big_picture.clone() {
            big.all.apply_mode(state);
        }
    }

    fn rows(&self) -> &'static [&'static [Key]] {
        match self.page.get() {
            Page::Letters => LETTER_ROWS,
            Page::Symbols => SYMBOL_ROWS,
        }
    }

    fn key_label(&self, key: Key) -> String {
        match key {
            Key::Char(c) => {
                let c = if self.shift.get() && self.page.get() == Page::Letters {
                    c.to_ascii_uppercase()
                } else {
                    c
                };
                c.to_string()
            }
            Key::Shift => crate::tr!("Shift"),
            Key::Page => match self.page.get() {
                Page::Letters => crate::tr!("#+="),
                Page::Symbols => crate::tr!("ABC"),
            },
            Key::Space => crate::tr!("Space"),
            Key::Backspace => "⌫".to_string(),
            Key::Return => crate::tr!("Return"),
            Key::Ok => crate::tr!("OK"),
            Key::Cancel => crate::tr!("Cancel"),
        }
    }

    /// The pad button wired to an action key, shown as a badge on the key
    /// itself like the Switch keyboard does.
    fn badge(key: Key) -> Option<ira_input::GamepadButton> {
        match key {
            Key::Backspace => Some(ira_input::GamepadButton::B),
            Key::Cancel => Some(ira_input::GamepadButton::X),
            Key::Ok => Some(ira_input::GamepadButton::A),
            _ => None,
        }
    }

    /// Grid placement for a key: the action column rides column 11 with
    /// Return spanning rows 1-2 and OK spanning rows 3-4; the bottom row
    /// is shift, page toggle, and a space bar nine cells wide. None means
    /// the key shares an already-attached widget.
    fn place(row: usize, col: usize, key: Key) -> Option<(i32, i32, i32, i32)> {
        match key {
            Key::Shift => Some((0, 4, 1, 1)),
            Key::Page => Some((1, 4, 1, 1)),
            Key::Space => Some((2, 4, 8, 1)),
            Key::Cancel => Some((10, 4, 1, 1)),
            Key::Ok if row == 4 => None,
            Key::Ok => Some((11, 3, 1, 2)),
            Key::Return => match row {
                1 => Some((11, 1, 1, 2)),
                _ => None,
            },
            Key::Backspace => Some((11, 0, 1, 1)),
            Key::Char(_) => Some((col as i32, row as i32, 1, 1)),
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
        if matches!(key, Key::Shift) && self.shift.get() {
            button.add_css_class(CSS_BP_KEY_ACTIVE);
        }
        let label = gtk4::Label::new(Some(&self.key_label(key)));
        // Expand+center: a Box ignores halign along its own axis, so
        // without expanding, the letter hugs the key's left edge.
        label.set_hexpand(true);
        label.set_halign(gtk4::Align::Center);
        label.set_valign(gtk4::Align::Center);
        crate::ui::helpers::crisp_label(&label);
        button.append(&label);
        // Action keys wear their pad button in the corner, Switch-style.
        if let Some(badge) = Self::badge(key) {
            let badge_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            badge_box.set_valign(gtk4::Align::Start);
            badge_box.set_halign(gtk4::Align::End);
            badge_box.set_margin_top(4);
            badge_box.set_margin_end(4);
            let glyph = gtk4::Image::new();
            glyph.set_pixel_size(24);
            glyph.set_halign(gtk4::Align::Center);
            glyph.set_valign(gtk4::Align::Center);
            let fallback = gtk4::Label::new(Some(
                &crate::ui::input_profile_assets::source_badge(
                    ira_input::InputSource::Button(badge),
                    self.family.get(),
                ),
            ));
            fallback.add_css_class(CSS_BP_KEY_BADGE);
            fallback.set_halign(gtk4::Align::Center);
            fallback.set_valign(gtk4::Align::Center);
            badge_box.append(&glyph);
            badge_box.append(&fallback);
            crate::ui::input_profile_assets::set_source_asset(
                &glyph,
                &fallback,
                ira_input::InputSource::Button(badge),
                self.family.get(),
            );
            button.append(&badge_box);
        }
        button.set_size_request(80, 72);
        let click_state = state.clone();
        let click = gtk4::GestureClick::new();
        click.connect_pressed(move |_, _, _, _| {
            // Clone out of the borrow: confirming runs the caller's
            // callback, which mutably borrows the state.
            let big = click_state.borrow().big_picture.clone();
            if let Some(big) = big {
                big.keyboard.cursor.set(position);
                big.keyboard.refresh_cursor();
                big.keyboard.press(&click_state, key);
            }
        });
        button.add_controller(click);
        button.upcast()
    }

    /// Rebuild the panel for the current page and shift state.
    fn rebuild(&self, state: &SharedState, prompt: &str) {
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

        let grid = gtk4::Grid::new();
        grid.set_row_spacing(8);
        grid.set_column_spacing(8);
        grid.set_halign(gtk4::Align::Center);
        let mut map: Vec<Vec<gtk4::Widget>> = Vec::new();
        for (row, keys) in self.rows().iter().enumerate() {
            let mut row_map = Vec::new();
            for (col, key) in keys.iter().enumerate() {
                let widget = self.key_button(state, *key, (row, col));
                row_map.push(widget.clone());
                if let Some((left, top, width, height)) = Self::place(row, col, *key) {
                    grid.attach(&widget, left, top, width, height);
                }
            }
            map.push(row_map);
        }
        *self.keys.borrow_mut() = map;
        self.panel.append(&grid);

        // The shortcut row, Switch-style: every action with its pad
        // button beside it, in the pad's own colors.
        let hints = gtk4::Box::new(gtk4::Orientation::Horizontal, 24);
        hints.set_halign(gtk4::Align::Center);
        hints.set_margin_top(8);
        let family = self.family.get();
        for (button, label) in [
            (ira_input::GamepadButton::A, crate::tr!("Select")),
            (ira_input::GamepadButton::B, crate::tr!("Delete")),
            (ira_input::GamepadButton::X, crate::tr!("Cancel")),
        ] {
            let label = label.as_str();
            let item = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            let glyph = gtk4::Image::new();
            glyph.set_pixel_size(28);
            let fallback = gtk4::Label::new(Some(
                &crate::ui::input_profile_assets::source_badge(
                    ira_input::InputSource::Button(button),
                    family,
                ),
            ));
            fallback.add_css_class(CSS_BP_KEY_BADGE);
            fallback.set_halign(gtk4::Align::Center);
            fallback.set_valign(gtk4::Align::Center);
            crate::ui::helpers::crisp_label(&fallback);
            item.append(&glyph);
            item.append(&fallback);
            crate::ui::input_profile_assets::set_source_asset(
                &glyph,
                &fallback,
                ira_input::InputSource::Button(button),
                family,
            );
            let text = gtk4::Label::new(Some(label));
            text.add_css_class(CSS_BP_PROMPT);
            crate::ui::helpers::crisp_label(&text);
            item.append(&text);
            hints.append(&item);
        }
        self.panel.append(&hints);
    }

    /// The connected pad's family changed; the next open draws its glyphs.
    pub(super) fn set_pad_family(&self, family: ira_input::ControllerFamily) {
        self.family.set(family);
    }

    /// Move the key cursor, clamping each row to its own length.
    pub(super) fn move_cursor(&self, dx: i32, dy: i32) {
        let rows = self.rows();
        let (row, col) = self.cursor.get();
        let row = (row as i64 + dy as i64).clamp(0, rows.len() as i64 - 1) as usize;
        let col = (col as i64 + dx as i64).clamp(0, rows[row].len() as i64 - 1) as usize;
        self.cursor.set((row, col));
        self.refresh_cursor();
    }

    /// Type a character straight from the physical keyboard.
    pub(super) fn type_char(&self, ch: char) {
        self.buffer.borrow_mut().push(ch);
        self.refresh_preview();
    }

    /// Type the key under the cursor (or the given one from a mouse
    /// click), updating the buffer or running the special action.
    pub(super) fn press(&self, state: &SharedState, key: Key) {
        match key {
            Key::Char(c) => {
                let c = if self.shift.get() && self.page.get() == Page::Letters {
                    c.to_ascii_uppercase()
                } else {
                    c
                };
                self.buffer.borrow_mut().push(c);
            }
            Key::Space => self.buffer.borrow_mut().push(' '),
            Key::Backspace => {
                self.buffer.borrow_mut().pop();
            }
            Key::Cancel => {
                self.close(state);
                return;
            }
            Key::Page => {
                self.page
                    .set(match self.page.get() {
                        Page::Letters => Page::Symbols,
                        Page::Symbols => Page::Letters,
                    });
                let (row, col) = self.cursor.get();
                let rows = self.rows();
                let row = row.min(rows.len().saturating_sub(1));
                let col = col.min(rows[row].len().saturating_sub(1));
                self.cursor.set((row, col));
                self.rebuild(state, &self.prompt.borrow().clone());
                self.refresh_cursor();
                self.refresh_preview();
                return;
            }
            Key::Shift => {
                self.shift.set(!self.shift.get());
                let cursor = self.cursor.get();
                self.rebuild(state, &self.prompt.borrow().clone());
                self.cursor.set(cursor);
                self.refresh_cursor();
                self.refresh_preview();
                return;
            }
            Key::Return | Key::Ok => {
                let text = self.buffer.borrow().trim().to_string();
                // An unnamed group helps nobody: refuse to commit and let
                // the name keep being typed.
                if text.is_empty() {
                    return;
                }
                let callback = self.on_ok.borrow_mut().take();
                self.close(state);
                if let Some(callback) = callback {
                    callback(state, &text);
                }
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
        let rows = self.rows();
        let Some(keys) = rows.get(row) else {
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
        let (cursor_row, cursor_col) = self.cursor.get();
        // Return and OK sit at two cursor addresses (their column spans
        // two rows); compare widgets so the shared one is highlighted
        // exactly once.
        let selected = self
            .keys
            .borrow()
            .get(cursor_row)
            .and_then(|row| row.get(cursor_col))
            .cloned();
        for keys in self.keys.borrow().iter() {
            for widget in keys.iter() {
                let is_selected = selected
                    .as_ref()
                    .is_some_and(|selected| selected.as_ptr() == widget.as_ptr());
                if is_selected {
                    widget.add_css_class(CSS_BP_KEY_SELECTED);
                } else {
                    widget.remove_css_class(CSS_BP_KEY_SELECTED);
                }
            }
        }
    }
}
