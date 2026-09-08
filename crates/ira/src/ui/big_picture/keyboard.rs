//! The big-picture virtual keyboard, modeled on the Switch's: a key grid
//! with a right-hand action column (backspace, return, OK), a shift and a
//! page toggle on the bottom row, and a wide space bar. It types from the
//! gamepad/keyboard cursor, from mouse clicks, and from the physical
//! keyboard, and hands the finished text to the caller's callback. Return
//! is a dead key unless the caller's input is multiline: shown, but
//! dimmed and skipped by every cursor move.

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
        Key::Char('1'), Key::Char('2'), Key::Char('3'), Key::Char('4'), Key::Char('5'),
        Key::Char('6'), Key::Char('7'), Key::Char('8'), Key::Char('9'), Key::Char('0'),
        Key::Char('-'), Key::Backspace,
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
    &[Key::Shift, Key::Page, Key::Space, Key::Ok],
];

/// The symbol page: digits and punctuation, same shape as the letter page.
const SYMBOL_ROWS: &[&[Key]] = &[
    &[
        Key::Char('~'), Key::Char('`'), Key::Char('|'), Key::Char('_'), Key::Char('{'),
        Key::Char('}'), Key::Char('['), Key::Char(']'), Key::Char('\\'), Key::Char('<'),
        Key::Char('>'), Key::Backspace,
    ],
    &[
        Key::Char('/'), Key::Char(':'), Key::Char(';'), Key::Char('('), Key::Char(')'),
        Key::Char('$'), Key::Char('&'), Key::Char('@'), Key::Char('"'), Key::Char('\''),
        Key::Char('*'), Key::Return,
    ],
    &[
        Key::Char('<'), Key::Char('>'), Key::Char('('), Key::Char(')'), Key::Char('['),
        Key::Char(']'), Key::Char('{'), Key::Char('}'), Key::Char('+'), Key::Char('='),
        Key::Char('*'), Key::Return,
    ],
    &[
        Key::Char('['), Key::Char(']'), Key::Char('{'), Key::Char('}'), Key::Char('('),
        Key::Char(')'), Key::Char('-'), Key::Char('_'), Key::Char('/'), Key::Char(':'),
        Key::Char('"'), Key::Ok,
    ],
    &[Key::Shift, Key::Page, Key::Space, Key::Ok],
];

/// A letter key's shifted self (the top row's digits become symbols).
fn shifted(c: char) -> char {
    match c {
        '1' => '!', '2' => '@', '3' => '#', '4' => '$', '5' => '%', '6' => '^',
        '7' => '&', '8' => '*', '9' => '(', '0' => ')', '-' => '_', c => c,
    }
}

/// The cursor's next key. Horizontal moves wrap around the row and skip
/// keys the cursor can't rest on. Vertical moves from an action key ride
/// the action column — the last key of each row — landing on the next
/// restable one (up from OK reaches Backspace, skipping a single-line
/// input's dead Return); elsewhere the move keeps the column, and the
/// bottom row's four keys answer the twelve columns above them: shift,
/// page, the space bar's span, OK.
/// Whether two cursor addresses rest on the same key widget: Return and
/// OK span several rows, so each lives at two addresses (Return rows 1-2,
/// OK rows 3 and bottom). The action column's ride must not count its own
/// key's twin as progress.
fn same_key(rows: &[&[Key]], a: (usize, usize), b: (usize, usize)) -> bool {
    let key = |(row, col): (usize, usize)| {
        rows.get(row).and_then(|keys| keys.get(col)).copied()
    };
    match (key(a), key(b)) {
        (Some(Key::Ok), Some(Key::Ok)) | (Some(Key::Return), Some(Key::Return)) => true,
        _ => a == b,
    }
}

fn advance_cursor(
    rows: &[&[Key]],
    multiline: bool,
    cursor: (usize, usize),
    dx: i32,
    dy: i32,
) -> (usize, usize) {
    let restable = |(row, col): (usize, usize)| {
        rows.get(row)
            .and_then(|keys| keys.get(col))
            .is_some_and(|key| *key != Key::Return || multiline)
    };
    let (row, col) = cursor;
    let action_key = rows
        .get(row)
        .and_then(|keys| keys.get(col))
        .is_some_and(|key| matches!(key, Key::Backspace | Key::Return | Key::Ok));
    if dy != 0 {
        if action_key {
            let mut r = row as i64 + dy as i64;
            while (0..rows.len() as i64).contains(&r) {
                let next = (r as usize, rows[r as usize].len() - 1);
                if restable(next) && !same_key(rows, next, cursor) {
                    return next;
                }
                r += dy as i64;
            }
            (row, col)
        } else {
            let r = (row as i64 + dy as i64).clamp(0, rows.len() as i64 - 1) as usize;
            if r == rows.len() - 1 {
                let bottom = match col {
                    0 => 0,
                    1 => 1,
                    11 => 3,
                    _ => 2,
                };
                (r, bottom)
            } else {
                (r, col.min(rows[r].len() - 1))
            }
        }
    } else {
        let len = rows[row].len() as i64;
        let mut c = col as i64;
        for _ in 0..len {
            c = (c + dx as i64).rem_euclid(len);
            if restable((row, c as usize)) {
                break;
            }
        }
        (row, c as usize)
    }
}

/// The caret's blink half-period: visible half, then invisible half.
const BLINK_EVERY_MS: u64 = 500;
/// The preview text starts this far into the pill, so the caret bar has
/// room to sit left of the first glyph instead of covering it.
const PREVIEW_TEXT_INSET: i32 = 3;
/// The caret bar's width; the CSS `.bp-key-caret` min-width matches.
const CARET_WIDTH: i32 = 2;

/// What to do with the finished name.
type NameCallback = Box<dyn Fn(&SharedState, &str)>;

pub(super) struct Keyboard {
    /// The menu surface: dim layer with the panel floating on top.
    root: gtk4::Overlay,
    panel: gtk4::Box,
    /// The text preview: the buffer in one static label with the caret
    /// floating over it, Switch-style.
    preview: gtk4::Box,
    preview_label: gtk4::Label,
    preview_caret: gtk4::Box,
    /// The caret's blink phase; typing pins it visible.
    blink_on: Cell<bool>,
    /// Key cursor position: (row, column) into the showing page's rows.
    cursor: Cell<(usize, usize)>,
    page: Cell<Page>,
    shift: Cell<bool>,
    /// One-shot shift (L3): the next letter types uppercase, then it
    /// clears. The Shift key itself is a caps-lock toggle.
    once: Cell<bool>,
    /// Whether the input being typed accepts newlines. When it doesn't,
    /// Return is a dead key: drawn dimmed and skipped by the cursor.
    multiline: Cell<bool>,
    /// Cursor-addressable key widgets of the showing page, for repaints.
    keys: RefCell<Vec<Vec<gtk4::Widget>>>,
    /// The prompt line the panel was opened with, kept for page/shift
    /// rebuilds.
    prompt: RefCell<String>,
    /// The connected pad's family, for the badge and hint glyphs.
    family: Cell<ira_input::ControllerFamily>,
    buffer: RefCell<String>,
    /// The text caret as a char index into the buffer; the shoulders move
    /// it, typing and backspace work at it.
    caret: Cell<usize>,
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
        panel.add_css_class(CSS_BP_KEYBOARD);
        // The keyboard takes over the whole horizontal area and sits at
        // the bottom of the screen.
        panel.set_halign(gtk4::Align::Fill);
        panel.set_valign(gtk4::Align::End);

        let root = gtk4::Overlay::new();
        // The overlay is a sibling of the bp-root Box, so it must carry the
        // big-picture class itself to inherit its font.
        root.add_css_class(CSS_BP_ROOT);
        root.set_child(Some(&dim));
        root.add_overlay(&panel);
        root.set_visible(false);

        // The buffer lives in one static label; the caret floats over it
        // at the glyph position Pango reports. Moving the caret must
        // never reflow the text — it slides along it.
        let preview_label = gtk4::Label::new(None);
        preview_label.set_margin_start(PREVIEW_TEXT_INSET);
        preview_label.set_hexpand(true);
        preview_label.set_halign(gtk4::Align::Fill);
        preview_label.set_xalign(0.0);
        preview_label.set_single_line_mode(true);
        preview_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        crate::ui::helpers::crisp_label(&preview_label);
        // The caret bar is measured from the label's layout, which reads
        // stale geometry until the label has its real allocation and
        // style. Re-derive it on every map — the first paint after an
        // open — so it can never park mid-name.
        let caret_map_state = state.clone();
        preview_label.connect_map(move |_| {
            if let Some(big) = caret_map_state.borrow().big_picture.clone() {
                big.keyboard.reposition_caret();
            }
        });
        let preview_caret = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        preview_caret.add_css_class(CSS_BP_KEY_CARET);
        preview_caret.set_halign(gtk4::Align::Start);
        preview_caret.set_valign(gtk4::Align::Fill);
        let preview_area = gtk4::Overlay::new();
        preview_area.set_child(Some(&preview_label));
        preview_area.add_overlay(&preview_caret);
        let preview = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        preview.add_css_class(CSS_BP_KEY_PREVIEW);
        preview.append(&preview_area);

        let keyboard = Self {
            root,
            panel,
            preview,
            preview_label,
            preview_caret,
            blink_on: Cell::new(true),
            cursor: Cell::new((1, 0)),
            page: Cell::new(Page::Letters),
            shift: Cell::new(false),
            once: Cell::new(false),
            multiline: Cell::new(false),
            keys: RefCell::new(Vec::new()),
            prompt: RefCell::new(String::new()),
            family: Cell::new(ira_input::ControllerFamily::Xbox),
            buffer: RefCell::new(String::new()),
            caret: Cell::new(0),
            on_ok: RefCell::new(None),
        };
        // The caret blinks while the keyboard shows; typing re-pins it
        // visible (see refresh_preview).
        let blink_state = state.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(BLINK_EVERY_MS), move || {
            if let Some(big) = blink_state.borrow().big_picture.clone() {
                let shown = &big.keyboard;
                if shown.is_open() {
                    let on = !shown.blink_on.get();
                    shown.blink_on.set(on);
                    shown.preview_caret.set_opacity(if on { 1.0 } else { 0.0 });
                }
            }
            glib::ControlFlow::Continue
        });
        keyboard
    }

    pub(super) fn root(&self) -> &Widget {
        self.root.upcast_ref()
    }

    pub(super) fn is_open(&self) -> bool {
        self.root.is_visible()
    }

    /// Open with a prompt, any starting text, whether the input accepts
    /// newlines (a group name doesn't — Return stays a dead key), and what
    /// to do with the finished name.
    pub(super) fn open(
        &self,
        state: &SharedState,
        prompt: &str,
        initial: &str,
        multiline: bool,
        on_ok: NameCallback,
    ) {
        *self.buffer.borrow_mut() = initial.to_string();
        self.caret.set(initial.chars().count());
        *self.prompt.borrow_mut() = prompt.to_string();
        *self.on_ok.borrow_mut() = Some(on_ok);
        self.multiline.set(multiline);
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
        self.caret.set(0);
        self.shift.set(false);
        self.once.set(false);
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

    /// Caps lock or the one-shot shift raises the keys.
    fn shifted_now(&self) -> bool {
        self.shift.get() || self.once.get()
    }

    fn key_label(&self, key: Key) -> String {
        match key {
            Key::Char(c) => {
                if self.shifted_now() && self.page.get() == Page::Letters {
                    let c = shifted(c);
                    if c.is_ascii_lowercase() {
                        c.to_ascii_uppercase().to_string()
                    } else {
                        c.to_string()
                    }
                } else {
                    c.to_string()
                }
            }
            Key::Page => match self.page.get() {
                Page::Letters => crate::tr!("#+="),
                Page::Symbols => crate::tr!("ABC"),
            },
            Key::Space => crate::tr!("Space"),
            Key::Return => crate::tr!("Return"),
            Key::Ok => crate::tr!("OK"),
            // Shift and Backspace render as icons (see key_button),
            // never as text.
            Key::Shift | Key::Backspace => String::new(),
        }
    }

    /// The pad button wired to an action key, shown as a badge on the key
    /// itself like the Switch keyboard does.
    fn badge(key: Key) -> Option<ira_input::GamepadButton> {
        match key {
            Key::Backspace => Some(ira_input::GamepadButton::B),
            Key::Ok => Some(ira_input::GamepadButton::Start),
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
            Key::Space => Some((2, 4, 9, 1)),
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
        // A single-line input's Return: shown for Switch-keyboard
        // fidelity, but dimmed and deaf — the cursor skips it and clicks
        // fall through nowhere.
        let dead = matches!(key, Key::Return) && !self.multiline.get();
        if dead {
            button.add_css_class(CSS_BP_KEY_DISABLED);
        }
        // OK is the keyboard's suggested action, wearing libadwaita's
        // opaque accent the way a suggested-action button does.
        if matches!(key, Key::Ok) {
            button.add_css_class(CSS_BP_KEY_OK);
        }
        // The badge rides an overlay above the label, so its presence
        // never shifts the letter's centering.
        let key_surface = gtk4::Overlay::new();
        let icon_name = match key {
            // Caps lock wears its state: outlined at rest, filled when
            // raised.
            Key::Shift => Some(if self.shifted_now() {
                "shift-filled-symbolic"
            } else {
                "shift-symbolic"
            }),
            Key::Backspace => Some("entry-clear-symbolic"),
            _ => None,
        };
        if let Some(icon_name) = icon_name {
            let icon = gtk4::Image::from_icon_name(icon_name);
            icon.set_pixel_size(28);
            icon.set_hexpand(true);
            icon.set_halign(gtk4::Align::Center);
            icon.set_valign(gtk4::Align::Center);
            key_surface.set_child(Some(&icon));
        } else {
            let label = gtk4::Label::new(Some(&self.key_label(key)));
            label.set_hexpand(true);
            label.set_halign(gtk4::Align::Center);
            label.set_valign(gtk4::Align::Center);
            crate::ui::helpers::crisp_label(&label);
            key_surface.set_child(Some(&label));
        }
        button.append(&key_surface);
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
            key_surface.add_overlay(&badge_box);
        }
        // The keys stretch with the panel and take the entire width.
        button.set_hexpand(true);
        button.set_size_request(80, 72);
        if !dead {
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
        }
        button.upcast()
    }

    /// Rebuild the panel for the current page and shift state.
    fn rebuild(&self, state: &SharedState, prompt: &str) {
        crate::ui::helpers::clear_children(&self.panel);
        let title = gtk4::Label::new(Some(prompt));
        title.set_xalign(0.0);
        title.set_wrap(true);
        title.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        title.add_css_class(CSS_BP_PAGE_TITLE);
        crate::ui::helpers::crisp_label(&title);
        self.panel.append(&title);
        self.panel.append(&self.preview);

        let grid = gtk4::Grid::new();
        grid.set_row_spacing(8);
        grid.set_column_spacing(8);
        grid.set_hexpand(true);
        let mut map: Vec<Vec<gtk4::Widget>> = Vec::new();
        // place() returns None for a key that continues an earlier row's
        // widget (Return spans rows 1-2, OK rows 3 and the bottom row).
        // Those must resolve to the attached widget itself: a fresh
        // never-attached widget here is an invisible key the cursor can
        // land on — the ghost "OK" to the right of the space bar.
        let mut attached: Vec<(Key, gtk4::Widget)> = Vec::new();
        for (row, keys) in self.rows().iter().enumerate() {
            let mut row_map = Vec::new();
            for (col, key) in keys.iter().enumerate() {
                let widget = match Self::place(row, col, *key) {
                    Some((left, top, width, height)) => {
                        let widget = self.key_button(state, *key, (row, col));
                        grid.attach(&widget, left, top, width, height);
                        attached.push((*key, widget.clone()));
                        widget
                    }
                    None => attached
                        .iter()
                        .rev()
                        .find(|(shared, _)| shared == key)
                        .map(|(_, widget)| widget.clone())
                        .expect("shared key's first copy is always attached"),
                };
                row_map.push(widget);
            }
            map.push(row_map);
        }
        *self.keys.borrow_mut() = map;
        self.panel.append(&grid);

        // The shortcut row, Switch-style: every action with its pad
        // button beside it, in the pad's own colors. The shoulders'
        // caret move is a directional icon, not a word.
        enum Hint {
            Icon(&'static str),
            Text(String),
        }
        let hints = gtk4::Box::new(gtk4::Orientation::Horizontal, 24);
        hints.set_halign(gtk4::Align::End);
        hints.set_margin_top(8);
        let family = self.family.get();
        for (button, hint) in [
            (
                ira_input::GamepadButton::LeftShoulder,
                Hint::Icon("left-large-symbolic"),
            ),
            (
                ira_input::GamepadButton::RightShoulder,
                Hint::Icon("right-large-symbolic"),
            ),
            (ira_input::GamepadButton::LeftStick, Hint::Text(crate::tr!("Shift"))),
            (ira_input::GamepadButton::A, Hint::Text(crate::tr!("Select"))),
            (ira_input::GamepadButton::B, Hint::Text(crate::tr!("Delete"))),
            (ira_input::GamepadButton::X, Hint::Text(crate::tr!("Cancel"))),
        ] {
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
            match hint {
                Hint::Icon(name) => {
                    let icon = gtk4::Image::from_icon_name(name);
                    icon.set_pixel_size(22);
                    icon.add_css_class(CSS_BP_PROMPT);
                    item.append(&icon);
                }
                Hint::Text(label) => {
                    let text = gtk4::Label::new(Some(&label));
                    text.add_css_class(CSS_BP_PROMPT);
                    crate::ui::helpers::crisp_label(&text);
                    item.append(&text);
                }
            }
            hints.append(&item);
        }
        self.panel.append(&hints);
    }

    /// The connected pad's family changed; the next open draws its glyphs.
    pub(super) fn set_pad_family(&self, family: ira_input::ControllerFamily) {
        self.family.set(family);
    }

    /// Move the key cursor. The rules live in `advance_cursor` so they
    /// can be tested without GTK.
    pub(super) fn move_cursor(&self, dx: i32, dy: i32) {
        let (row, col) = self.cursor.get();
        self.cursor.set(advance_cursor(
            self.rows(),
            self.multiline.get(),
            (row, col),
            dx,
            dy,
        ));
        self.refresh_cursor();
    }

    /// Type a character straight from the physical keyboard.
    pub(super) fn type_char(&self, ch: char) {
        self.insert(ch);
        self.refresh_preview();
    }

    /// Insert a character at the text caret.
    fn insert(&self, ch: char) {
        let mut buffer = self.buffer.borrow_mut();
        let caret = self.caret.get().min(buffer.chars().count());
        let byte = buffer
            .char_indices()
            .nth(caret)
            .map(|(i, _)| i)
            .unwrap_or(buffer.len());
        buffer.insert(byte, ch);
        drop(buffer);
        self.caret.set(caret + 1);
    }

    /// Move the text caret one character left or right (the shoulders).
    pub(super) fn move_caret(&self, dx: i32) {
        let count = self.buffer.borrow().chars().count();
        let caret = (self.caret.get() as i64 + dx as i64).clamp(0, count as i64) as usize;
        self.caret.set(caret);
        self.refresh_preview();
    }

    /// Type the key under the cursor (or the given one from a mouse
    /// click), updating the buffer or running the special action.
    pub(super) fn press(&self, state: &SharedState, key: Key) {
        match key {
            Key::Char(c) => {
                let raised = self.shifted_now() && self.page.get() == Page::Letters;
                let c = shifted(c);
                let c = if raised && c.is_ascii_lowercase() {
                    c.to_ascii_uppercase()
                } else {
                    c
                };
                self.insert(c);
                if self.once.take() {
                    let cursor = self.cursor.get();
                    self.rebuild(state, &self.prompt.borrow().clone());
                    self.cursor.set(cursor);
                    self.refresh_cursor();
                }
            }
            Key::Space => self.insert(' '),
            Key::Backspace => self.backspace(),
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

    /// Start: commit the text, same as the OK key.
    pub(super) fn press_ok(&self, state: &SharedState) {
        self.press(state, Key::Ok);
    }

    /// L3: raise the very next letter only.
    pub(super) fn bump_shift(&self, state: &SharedState) {
        self.once.set(!self.once.get());
        let cursor = self.cursor.get();
        self.rebuild(state, &self.prompt.borrow().clone());
        self.cursor.set(cursor);
        self.refresh_cursor();
    }

    /// Delete the character before the text caret (the B button's job).
    pub(super) fn backspace(&self) {
        let mut buffer = self.buffer.borrow_mut();
        let caret = self.caret.get().min(buffer.chars().count());
        if caret == 0 {
            return;
        }
        let start = buffer
            .char_indices()
            .nth(caret - 1)
            .map(|(i, _)| i)
            .unwrap_or(0);
        let end = buffer
            .char_indices()
            .nth(caret)
            .map(|(i, _)| i)
            .unwrap_or(buffer.len());
        buffer.replace_range(start..end, "");
        drop(buffer);
        self.caret.set(caret - 1);
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
        // Typing pins the caret visible; the blink resumes from there.
        self.blink_on.set(true);
        self.preview_caret.set_opacity(1.0);
        if text.is_empty() {
            // The caret leads a dimmed placeholder, like an empty entry.
            self.preview_label.set_opacity(0.45);
            self.preview_label.set_text(&crate::tr!("Type a name…"));
            self.place_caret(0);
            return;
        }
        self.preview_label.set_opacity(1.0);
        self.preview_label.set_text(&text);
        self.place_caret(self.caret.get().min(text.chars().count()));
    }

    /// Re-derive the caret bar's position from the current buffer and
    /// caret index, without changing either (the label-map hook).
    fn reposition_caret(&self) {
        let text = self.buffer.borrow().clone();
        if text.is_empty() {
            self.place_caret(0);
            return;
        }
        self.place_caret(self.caret.get().min(text.chars().count()));
    }

    /// Float the caret over the glyph slot the text caret occupies.
    /// Pango reports the index's rect inside the label's layout, and the
    /// overlay shares the label's coordinate origin, so the rect is the
    /// floating bar's margin — the text itself never moves. The bar's
    /// right edge meets the glyph's left edge, so it sits in the space
    /// before that glyph instead of covering it.
    fn place_caret(&self, caret: usize) {
        let text = self.preview_label.text();
        let byte = text
            .char_indices()
            .nth(caret)
            .map(|(i, _)| i)
            .unwrap_or(text.len());
        let layout = self.preview_label.layout();
        // The label's layout is measured at the last allocated width;
        // right after a rebuild that width is stale or zero, which
        // ellipsizes the layout and drops the caret somewhere mid-name
        // instead of after the last letter. A natural-width layout maps
        // the caret index to its true glyph slot.
        layout.set_width(-1);
        let rect = layout.index_to_pos(byte as i32);
        let scale = gtk4::pango::SCALE as f64;
        let inset = ((rect.height() as f64 / scale) * 0.14).round() as i32;
        let x = PREVIEW_TEXT_INSET + (rect.x() as f64 / scale).round() as i32 - CARET_WIDTH;
        self.preview_caret.set_margin_start(x.max(0));
        self.preview_caret
            .set_margin_top((rect.y() as f64 / scale).round() as i32 + inset);
        self.preview_caret.set_margin_bottom(inset);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_advance_down_from_backspace_lands_on_ok() {
        // The dead Return between them is skipped on a single-line input.
        assert_eq!(advance_cursor(LETTER_ROWS, false, (0, 11), 0, 1), (3, 11));
    }

    #[test]
    fn test_advance_up_from_ok_lands_on_backspace() {
        assert_eq!(advance_cursor(LETTER_ROWS, false, (3, 11), 0, -1), (0, 11));
        // The OK key under the bottom row's last cell is the same widget;
        // riding up from it reaches Backspace too.
        assert_eq!(advance_cursor(LETTER_ROWS, false, (4, 3), 0, -1), (0, 11));
    }

    #[test]
    fn test_advance_return_restable_only_when_multiline() {
        assert_eq!(advance_cursor(LETTER_ROWS, true, (0, 11), 0, 1), (1, 11));
        // Return's second row is the same widget — not a second stop.
        assert_eq!(advance_cursor(LETTER_ROWS, true, (1, 11), 0, 1), (3, 11));
        assert_eq!(advance_cursor(LETTER_ROWS, true, (2, 11), 0, 1), (3, 11));
        assert_eq!(advance_cursor(LETTER_ROWS, true, (2, 11), 0, -1), (0, 11));
    }

    #[test]
    fn test_advance_horizontal_wraps_past_dead_return() {
        // Right from the last letter skips Return and wraps to the row's
        // first key; left from the first letter skips it the other way.
        assert_eq!(advance_cursor(LETTER_ROWS, false, (1, 10), 1, 0), (1, 0));
        assert_eq!(advance_cursor(LETTER_ROWS, false, (1, 0), -1, 0), (1, 10));
        // With multiline the wrap stops on Return.
        assert_eq!(advance_cursor(LETTER_ROWS, true, (1, 10), 1, 0), (1, 11));
    }

    #[test]
    fn test_advance_down_maps_letter_columns_to_bottom_row_keys() {
        // The space bar answers the columns above it; the edges map to
        // shift and page. Down from OK stays: the bottom row's OK cell is
        // the same widget, not a further step.
        assert_eq!(advance_cursor(LETTER_ROWS, false, (3, 5), 0, 1), (4, 2));
        assert_eq!(advance_cursor(LETTER_ROWS, false, (3, 0), 0, 1), (4, 0));
        assert_eq!(advance_cursor(LETTER_ROWS, false, (3, 1), 0, 1), (4, 1));
        assert_eq!(advance_cursor(LETTER_ROWS, false, (3, 11), 0, 1), (3, 11));
    }

    #[test]
    fn test_advance_up_from_bottom_row_keeps_column() {
        assert_eq!(advance_cursor(LETTER_ROWS, false, (4, 2), 0, -1), (3, 2));
        assert_eq!(advance_cursor(LETTER_ROWS, false, (4, 0), 0, -1), (3, 0));
    }

    #[test]
    fn test_advance_horizontal_wraps_on_bottom_row() {
        assert_eq!(advance_cursor(LETTER_ROWS, false, (4, 3), 1, 0), (4, 0));
    }

    #[test]
    fn test_advance_uses_same_shape_on_both_pages() {
        // The symbol page carries the same action column, so the rules
        // hold there verbatim.
        assert_eq!(advance_cursor(SYMBOL_ROWS, false, (0, 11), 0, 1), (3, 11));
        assert_eq!(advance_cursor(SYMBOL_ROWS, false, (2, 11), 0, -1), (0, 11));
    }
}
