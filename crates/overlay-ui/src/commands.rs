//! Layer command dispatch for the overlay.
//!
//! Applies [`CanvasCommand`]s to the widgets: gamepad navigation and
//! activation, pointer injection into the stub compositor, and show/hide
//! effects the frame loop honors. Shell construction and shared types stay
//! in `ui.rs`, focus movement in `focus.rs`.

use gtk4::prelude::WidgetExt;
use ira_overlay_ipc::{
    CMD_ACTIVATE, CMD_BACK, CMD_EXIT_GAME, CMD_HIDE, CMD_MOUSE_DOWN, CMD_MOUSE_MOVE, CMD_MOUSE_UP,
    CMD_NAV_DOWN, CMD_NAV_LEFT, CMD_NAV_RIGHT, CMD_NAV_UP, CMD_SCROLL, CMD_SHOW, CanvasCommand,
};
use ira_widgets::style as bp;

use super::ui::{Focus, MenuAction, OverlayUi, UiEffect, debug_log};

impl OverlayUi {
    pub fn apply(&mut self, cmd: CanvasCommand) -> UiEffect {
        if debug_log() && cmd.kind != CMD_MOUSE_MOVE {
            eprintln!(
                "ira-overlay-ui: apply kind={} a={} b={} c={}",
                cmd.kind, cmd.a, cmd.b, cmd.c
            );
        }
        // Every non-pointer command parks the composited cursor: a gamepad
        // or the keyboard is driving until the mouse actually moves. Pointer
        // commands drive the real Wayland cursor instead and never park it.
        if matches!(
            cmd.kind,
            CMD_NAV_UP
                | CMD_NAV_DOWN
                | CMD_NAV_LEFT
                | CMD_NAV_RIGHT
                | CMD_ACTIVATE
                | CMD_BACK
                | CMD_EXIT_GAME
                | CMD_SCROLL
                | CMD_SHOW
        ) {
            self.hide_cursor();
        }
        match cmd.kind {
            CMD_NAV_UP => self.move_focus(-1),
            CMD_NAV_DOWN => self.move_focus(1),
            CMD_NAV_LEFT => self.focus_sidebar(),
            CMD_NAV_RIGHT => self.focus_content(),
            CMD_ACTIVATE => return self.activate_focused(),
            CMD_BACK => return self.go_back(),
            CMD_EXIT_GAME => return self.activate_exit(),
            CMD_SCROLL => {
                super::stub::inject(super::stub::InjectedInput::Scroll { dy: cmd.a as f32 });
            }
            // Pointer goes straight into the compositor as real Wayland
            // input in injected mode; in gamescope mode there is no stub
            // compositor and these are no-ops (pad navigates instead).
            CMD_MOUSE_MOVE => {
                super::stub::inject(super::stub::InjectedInput::Motion {
                    x: cmd.a as f32,
                    y: cmd.b as f32,
                });
            }
            CMD_MOUSE_DOWN => {
                super::stub::inject(super::stub::InjectedInput::Button {
                    x11_button: cmd.c,
                    down: true,
                });
            }
            CMD_MOUSE_UP => {
                super::stub::inject(super::stub::InjectedInput::Button {
                    x11_button: cmd.c,
                    down: false,
                });
            }
            CMD_SHOW => {
                self.focus = Focus::Side(0);
                self.update_content();
                self.scroll_top();
                self.log_showgeom();
                return UiEffect::Show;
            }
            CMD_HIDE => return UiEffect::Hide,
            _ => {}
        }
        UiEffect::None
    }

    pub(super) fn section(&self) -> MenuAction {
        match self.focus {
            Focus::Side(i) => MenuAction::ALL
                .get(i)
                .copied()
                .unwrap_or(MenuAction::Resume),
            // Content only exists under Trophies (`focus_content`
            // refuses every other section).
            Focus::Ach(_) => MenuAction::Trophies,
        }
    }

    /// Sidebar index behind the current focus: sticky while browsing
    /// content, so going back lands where you left.
    pub(super) fn side_index(&self) -> usize {
        match self.focus {
            Focus::Side(i) => i,
            Focus::Ach(_) => MenuAction::ALL
                .iter()
                .position(|a| *a == MenuAction::Trophies)
                .unwrap_or(0),
        }
    }

    /// Swaps the content pane (hint vs trophies) to the cursor's
    /// section and repaints focus. Trophies live inside their submenu:
    /// the list shows only while its section is up, and only then does
    /// the area wear the sidebar's surface — every other section is bare
    /// dim with nothing on it.
    pub(super) fn update_content(&mut self) {
        let show_list =
            self.section() == MenuAction::Trophies || matches!(self.focus, Focus::Ach(_));
        self.stack
            .set_visible_child_name(if show_list { "list" } else { "hint" });
        if show_list {
            self.content.add_css_class(bp::CSS_BP_EXTENSION);
        } else {
            self.content.remove_css_class(bp::CSS_BP_EXTENSION);
        }
        self.paint_focus();
    }

    /// Click release that paired with its press: activates, and closes
    /// through the shared on_hide mirror (same as a gamepad Back).
    pub(super) fn click_release(&mut self, x: f64, y: f64) {
        if self.pointer_release(x, y, 1) == UiEffect::Hide {
            (self.on_hide)();
        }
    }
}
