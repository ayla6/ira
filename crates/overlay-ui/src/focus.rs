//! Gamepad/mouse focus for the overlay: shared cursor, pane switching,
//! activation, and the composited pointer.
//!
//! Focus is one location (`Focus::Side`/`Focus::Ach`, owned by `ui.rs`);
//! the pointer moves it by hit-testing, the gamepad by stepping. Clicks
//! pair press/release over the same row. Widget construction lives in
//! `widgets.rs`, the shell (sections, commands, logging) in `ui.rs`.

use glib::object::Cast as _;
use gtk4::prelude::{AdjustmentExt, FixedExt, WidgetExt as _};

use super::ui::{Focus, MenuAction, OverlayUi, UiEffect, debug_log};

impl OverlayUi {
    /// Reveals the composited pointer: the mouse moved, so it owns the
    /// pointer again (no-op unless a gamepad parked it, and never in
    /// injected mode where the real cursor rules).
    pub fn show_cursor(&mut self) {
        self.set_cursor_visible(true);
    }

    /// Parks the composited pointer: a gamepad (or the keyboard) is
    /// driving. The next real motion reveals it again.
    pub fn hide_cursor(&mut self) {
        self.set_cursor_visible(false);
    }

    pub(super) fn set_cursor_visible(&mut self, visible: bool) {
        if self.gamescope && self.cursor_hidden == visible {
            self.cursor_hidden = !visible;
            self.cursor_img.set_visible(visible);
        }
    }

    /// Window coordinates of a controller event relative to `source`.
    pub(super) fn to_window(&self, source: &gtk4::Widget, x: f64, y: f64) -> (f64, f64) {
        let base = self.root.upcast_ref::<gtk4::Widget>();
        source
            .compute_bounds(base)
            .map(|rect| (f64::from(rect.x()) + x, f64::from(rect.y()) + y))
            .unwrap_or((x, y))
    }

    /// Pure hit test under window coordinates, across the active pane.
    /// No side effects: callers apply the hit via `apply_hover`.
    fn hit_test(&self, x: f64, y: f64) -> Option<Focus> {
        let base = self.root.upcast_ref::<gtk4::Widget>();
        let (rows, side) = match self.focus {
            Focus::Side(_) => (&self.side_rows, true),
            Focus::Ach(_) => (&self.ach_rows, false),
        };
        rows.iter().enumerate().find_map(|(idx, row)| {
            let hit = row
                .upcast_ref::<gtk4::Widget>()
                .compute_bounds(base)
                .is_some_and(|rect| {
                    let (rx, ry, rw, rh) = (
                        f64::from(rect.x()),
                        f64::from(rect.y()),
                        f64::from(rect.width()),
                        f64::from(rect.height()),
                    );
                    x >= rx && x < rx + rw && y >= ry && y < ry + rh
                });
            hit.then_some(if side {
                Focus::Side(idx)
            } else {
                Focus::Ach(idx)
            })
        })
    }

    /// Applies a hit: moves the shared focus, switching content and
    /// repainting like the gamepad path does.
    fn apply_hover(&mut self, hit: Focus) {
        match hit {
            Focus::Side(idx) => {
                self.focus = Focus::Side(idx);
                self.update_content();
            }
            Focus::Ach(idx) => {
                if matches!(self.focus, Focus::Side(_)) {
                    self.focus_content();
                }
                if let Focus::Ach(_) = self.focus {
                    self.focus = Focus::Ach(idx);
                    self.paint_focus();
                    self.scroll_to_focused();
                }
            }
        }
    }

    /// Shared focus under window coordinates. Returns what was hit for
    /// press/release pairing.
    fn hover_at(&mut self, x: f64, y: f64) -> Option<Focus> {
        let hit = self.hit_test(x, y);
        if let Some(focus) = hit {
            self.apply_hover(focus);
        }
        hit
    }

    /// Moves the composited cursor without touching focus.
    fn move_cursor(&mut self, x: f64, y: f64) {
        if self.gamescope {
            self.set_cursor_visible(true);
            self.cursor_layer.move_(
                &self.cursor_img,
                x - self.cursor_hot.0,
                y - self.cursor_hot.1,
            );
        }
    }

    /// Pointer motion in window coordinates: reveal and move the
    /// composited cursor, and move the shared focus the gamepad drives.
    pub fn pointer_move(&mut self, x: f64, y: f64) {
        debug_motion(x, y);
        self.move_cursor(x, y);
        self.hover_at(x, y);
    }

    /// Button press in window coordinates: focus, and remember the row
    /// for release pairing. Nothing activates on press.
    pub fn pointer_press(&mut self, x: f64, y: f64, button: u8) {
        self.move_cursor(x, y);
        self.pressed = if button == 1 {
            self.hover_at(x, y)
        } else {
            None
        };
    }

    /// Button release in window coordinates: activate only when it lands
    /// on the row the press started on. A press that closed the overlay
    /// never reaches here, so the game never sees either edge.
    pub fn pointer_release(&mut self, x: f64, y: f64, button: u8) -> UiEffect {
        self.move_cursor(x, y);
        let hit = if button == 1 {
            self.hover_at(x, y)
        } else {
            None
        };
        let pressed = self.pressed.take();
        let paired = hit == pressed;
        match hit {
            Some(Focus::Side(_)) if paired => self.activate_focused(),
            _ => UiEffect::None,
        }
    }

    /// Keeps the focused achievement row on screen: the list's camera
    /// follows the cursor instead of letting it run off the edge.
    pub(super) fn scroll_to_focused(&self) {
        let Focus::Ach(idx) = self.focus else {
            return;
        };
        let Some(row) = self.ach_rows.get(idx) else {
            return;
        };
        let widget = row.upcast_ref::<gtk4::Widget>();
        let list = self.ach_list.upcast_ref::<gtk4::Widget>();
        let (y, h) = widget
            .compute_bounds(list)
            .map(|rect| (f64::from(rect.y()), f64::from(rect.height())))
            .unwrap_or((0.0, 0.0));
        let vadj = self.scrolled.vadjustment();
        let target = super::widgets::scroll_target(y, h, vadj.value(), vadj.page_size());
        let clamped = target.clamp(
            vadj.lower(),
            (vadj.upper() - vadj.page_size()).max(vadj.lower()),
        );
        vadj.set_value(clamped);
    }

    /// Parks the list back at the top: reopening the overlay always
    /// restarts from the same place instead of wherever it was left.
    pub(super) fn scroll_top(&self) {
        let vadj = self.scrolled.vadjustment();
        vadj.set_value(vadj.lower());
    }

    pub(super) fn move_focus(&mut self, delta: isize) {
        match self.focus {
            Focus::Side(i) => {
                if self.side_rows.is_empty() {
                    return;
                }
                let next =
                    (i as isize + delta).clamp(0, self.side_rows.len() as isize - 1) as usize;
                self.focus = Focus::Side(next);
                self.update_content();
            }
            Focus::Ach(i) => {
                if self.ach_rows.is_empty() {
                    return;
                }
                let next = (i as isize + delta).clamp(0, self.ach_rows.len() as isize - 1) as usize;
                self.focus = Focus::Ach(next);
                self.paint_focus();
                self.scroll_to_focused();
            }
        }
    }

    pub(super) fn focus_sidebar(&mut self) {
        if matches!(self.focus, Focus::Ach(_)) {
            self.focus = Focus::Side(self.side_index());
            self.update_content();
        }
    }

    pub(super) fn focus_content(&mut self) {
        // Only the achievements section has content to enter.
        if matches!(self.focus, Focus::Side(_))
            && self.section() == MenuAction::Trophies
            && !self.ach_rows.is_empty()
        {
            self.focus = Focus::Ach(0);
            self.paint_focus();
            self.scroll_to_focused();
        }
    }

    pub(super) fn activate_focused(&mut self) -> UiEffect {
        if matches!(self.focus, Focus::Ach(_)) {
            // Rows are informational; nothing to activate (Steam parity).
            return UiEffect::None;
        }
        match self.section() {
            MenuAction::Resume => UiEffect::Hide,
            MenuAction::Trophies => {
                self.focus_content();
                UiEffect::None
            }
            MenuAction::Screenshot => {
                (self.on_action)(ira_overlay_ipc::CanvasAction::of(
                    ira_overlay_ipc::ACT_SCREENSHOT,
                ));
                UiEffect::None
            }
            MenuAction::Record => {
                (self.on_action)(ira_overlay_ipc::CanvasAction::of(
                    ira_overlay_ipc::ACT_TOGGLE_RECORD,
                ));
                UiEffect::None
            }
            MenuAction::ExitGame => self.activate_exit(),
        }
    }

    /// Exit Game from any path (menu row, Alt+F4): same action + close.
    pub(super) fn activate_exit(&mut self) -> UiEffect {
        (self.on_action)(ira_overlay_ipc::CanvasAction::of(
            ira_overlay_ipc::ACT_EXIT_GAME,
        ));
        UiEffect::Hide
    }

    pub(super) fn go_back(&mut self) -> UiEffect {
        if matches!(self.focus, Focus::Ach(_)) {
            self.focus = Focus::Side(self.side_index());
            self.update_content();
            UiEffect::None
        } else {
            UiEffect::Hide
        }
    }

    /// Cursor highlight: exactly one row across both panes, ring plus
    /// accent — the same selection big picture itself uses.
    pub(super) fn paint_focus(&mut self) {
        for (idx, row) in self.side_rows.iter().enumerate() {
            let selected = matches!(self.focus, Focus::Side(i) if i == idx);
            set_selected(row, selected);
        }
        for (idx, row) in self.ach_rows.iter().enumerate() {
            let selected = matches!(self.focus, Focus::Ach(i) if i == idx);
            set_selected(row, selected);
        }
        if debug_log() {
            let (name, idx, len) = match self.focus {
                Focus::Side(i) => ("side", i, self.side_rows.len()),
                Focus::Ach(i) => ("ach", i, self.ach_rows.len()),
            };
            eprintln!("ira-overlay-ui: focus {name} {idx}/{len}");
        }
        let current = match self.focus {
            Focus::Side(i) => self.side_rows.get(i),
            Focus::Ach(i) => self.ach_rows.get(i),
        };
        if let Some(row) = current {
            // Row geometry for centering diagnostics: row height vs the
            // CSS padding + line height it should equal.
            if debug_log() {
                let widget = row.upcast_ref::<gtk4::Widget>();
                eprintln!(
                    "ira-overlay-ui: focusgeom row_h={} row_w={}",
                    widget.height(),
                    widget.width()
                );
            }
            row.grab_focus();
        }
    }
}

fn set_selected(row: &gtk4::Box, selected: bool) {
    use gtk4::prelude::WidgetExt as _;
    use ira_widgets::style as bp;
    if selected {
        row.add_css_class(bp::CSS_BP_MENU_ROW_SELECTED);
        row.add_css_class(bp::CSS_BP_MENU_ROW_ACTIVE);
    } else {
        row.remove_css_class(bp::CSS_BP_MENU_ROW_SELECTED);
        row.remove_css_class(bp::CSS_BP_MENU_ROW_ACTIVE);
    }
}

/// Rate-limited debug logging for the motion hot path: at most one line
/// per 2s, proving real pointer events arrive without flooding.
fn debug_motion(x: f64, y: f64) {
    if !debug_log() {
        return;
    }
    static LAST_MS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u32)
        .unwrap_or(0);
    if now.wrapping_sub(LAST_MS.load(std::sync::atomic::Ordering::Relaxed)) > 2000 {
        LAST_MS.store(now, std::sync::atomic::Ordering::Relaxed);
        eprintln!("ira-overlay-ui: pointer motion at {x:.0},{y:.0} (real events arrive)");
    }
}
