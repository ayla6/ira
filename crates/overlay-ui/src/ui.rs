//! Big Picture-style overlay: left sidebar sections, content submenu,
//! bottom button bar with real controller glyphs, fullscreen dim.
//!
//! The sidebar (game title, section menu) sits in a full-height panel
//! flush left; the content pane swaps between a hint panel and the
//! achievements list, so achievements live inside their submenu instead
//! of beside the menu. Rows are plain boxes like big picture's own menus
//! (no list widget, no list background banding). Dpad up/down moves
//! within the active pane, left/right switches panes, A activates, B
//! backs out (content → sidebar, sidebar → close). Gamepad focus is
//! tracked manually; achievement rows are informational.
//!
//! Split by responsibility: this file owns the shell (sections, command
//! dispatch, logging) plus shared types; `focus.rs` owns focus/pointer
//! state; `widgets.rs` builds rows and wires their controllers.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use glib::object::Cast as _;
use gtk4::prelude::{AdjustmentExt, BoxExt, FixedExt, WidgetExt};
use ira_input::ControllerFamily;
use ira_overlay_ipc::CanvasAction;
use ira_widgets::{menu, style as bp};

use super::game::GameData;
use super::widgets;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UiEffect {
    None,
    Show,
    Hide,
}

/// Shared focus location: exactly one across both panes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Focus {
    Side(usize),
    Ach(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum MenuAction {
    Resume,
    Trophies,
    Screenshot,
    Record,
    ExitGame,
}

impl MenuAction {
    pub(super) const ALL: [Self; 5] = [
        Self::Resume,
        Self::Trophies,
        Self::Screenshot,
        Self::Record,
        Self::ExitGame,
    ];

    /// Translated section label for the sidebar row (sentence case).
    fn label(&self) -> String {
        match self {
            Self::Resume => crate::tr!("Continue"),
            Self::Trophies => crate::tr!("Trophies"),
            Self::Screenshot => crate::tr!("Take screenshot"),
            Self::Record => crate::tr!("Record video"),
            Self::ExitGame => crate::tr!("Exit game"),
        }
    }
}

/// Glyph family default before the daemon publishes the physical pad's:
/// Xbox, the XInput default most pads present.
const DEFAULT_FAMILY: ControllerFamily = ControllerFamily::Xbox;

pub struct OverlayUi {
    pub(super) root: gtk4::Overlay,
    /// Weak self for the row motion/click controllers: GTK owns those
    /// closures, so a strong cycle would leak the whole UI.
    pub(super) self_weak: Weak<RefCell<OverlayUi>>,
    pub(super) stack: gtk4::Stack,
    pub(super) scrolled: gtk4::ScrolledWindow,
    pub(super) content: gtk4::Box,
    pub(super) sidebar: gtk4::Box,
    pub(super) side_title: gtk4::Label,
    pub(super) side_progress: gtk4::Label,
    pub(super) prompts: gtk4::Box,
    pub(super) side_rows: Vec<gtk4::Box>,
    pub(super) ach_rows: Vec<gtk4::Box>,
    pub(super) ach_list: gtk4::Box,
    /// Composited pointer: the real theme cursor image following real
    /// pointer motion (gamescope draws no hardware cursor over external
    /// planes, so the overlay draws the system's own). Parked while a
    /// gamepad or the keyboard drives.
    pub(super) cursor_layer: gtk4::Fixed,
    pub(super) cursor_img: gtk4::Image,
    pub(super) cursor_hot: (f64, f64),
    /// Pressed row awaiting its release: clicks activate on release over
    /// the same row, never on press (press may close the overlay, and the
    /// release must not leak to the game).
    pub(super) pressed: Option<Focus>,
    /// Gamescope mode draws the composited cursor and parks the system
    /// one; injected mode keeps the real Wayland cursor (and must never
    /// burn a fake into snapshots).
    pub(super) gamescope: bool,
    pub(super) on_action: Rc<dyn Fn(CanvasAction)>,
    /// Hides the overlay from a mouse click: closing needs the same
    /// SHM mirror the tick's Hide path performs, and controllers fire
    /// outside the tick.
    pub(super) on_hide: Rc<dyn Fn()>,
    pub(super) focus: Focus,
    /// The pointer is parked (a gamepad or the keyboard is driving): the
    /// composited cursor stays hidden until the mouse moves again.
    pub(super) cursor_hidden: bool,
    /// Last logged window geometry (change-gated layout tracing).
    pub(super) last_geom: String,
}

/// One-shot env check for input-path diagnostics (`IRA_OVERLAY_DEBUG=1`).
pub(super) fn debug_log() -> bool {
    static DEBUG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DEBUG.get_or_init(|| std::env::var_os("IRA_OVERLAY_DEBUG").is_some())
}

/// Sidebar panel: full-height, flush left, one row per section in order
/// plus title/progress headers. The header is the shared menu header, so
/// its text starts exactly where row text starts (same side padding).
fn build_sidebar() -> (gtk4::Box, gtk4::Label, gtk4::Label, Vec<gtk4::Box>) {
    let sidebar = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    sidebar.set_halign(gtk4::Align::Start);
    sidebar.set_valign(gtk4::Align::Fill);
    // Big picture's own menu width at the 1080p reference.
    sidebar.set_size_request(560, -1);
    sidebar.add_css_class(bp::CSS_BP_MENU_PANEL);
    let side_title = menu::menu_header("");
    let side_progress = menu::menu_header("");
    side_progress.add_css_class(bp::CSS_BP_PAGE_SUBTITLE);
    sidebar.append(&side_title);
    sidebar.append(&side_progress);
    let mut side_rows = Vec::new();
    for action in MenuAction::ALL {
        // The same row big picture itself builds.
        let row = menu::menu_row(&action.label());
        sidebar.append(&row);
        side_rows.push(row);
    }
    let spacer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    spacer.set_vexpand(true);
    sidebar.append(&spacer);
    (sidebar, side_title, side_progress, side_rows)
}

/// Content pane: hint panel and trophies list in a stack, swapped per
/// section. An extension of the sidebar: same surface flush against it,
/// full height, square — the list starts at its edge.
fn build_content() -> (gtk4::Box, gtk4::Stack, gtk4::ScrolledWindow, gtk4::Box) {
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content.set_hexpand(true);
    content.set_valign(gtk4::Align::Fill);
    // The surface class is managed per section in update_content
    // (trophies only); the box itself starts bare.
    content.set_margin_top(0);
    content.set_margin_bottom(0);
    content.set_margin_start(0);
    content.set_margin_end(32);
    let stack = gtk4::Stack::new();
    stack.set_vexpand(true);
    stack.set_hexpand(true);
    let hint_box = widgets::build_hint_panel();
    stack.add_named(&hint_box, Some("hint"));
    let ach_list = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    ach_list.set_valign(gtk4::Align::Fill);
    ach_list.set_vexpand(true);
    // Fills the viewport: without this the rows size to their
    // natural text width and park wherever the viewport puts them.
    ach_list.set_hexpand(true);
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_child(Some(&ach_list));
    scrolled.set_vexpand(true);
    // Rows wrap; nothing scrolls sideways. A horizontal offset is
    // how the list ended up parked on the right edge, so the axis
    // stays off by construction, not by hope.
    scrolled.set_hscrollbar_policy(gtk4::PolicyType::Never);
    stack.add_named(&scrolled, Some("list"));
    stack.set_visible_child_name("hint");
    content.append(&stack);
    (content, stack, scrolled, ach_list)
}

/// Bottom button bar, prompts flush right (glyphs painted later by
/// `rebuild_footer`).
fn build_footer() -> (gtk4::Box, gtk4::Box) {
    let bottom = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    bottom.set_halign(gtk4::Align::Fill);
    bottom.add_css_class(bp::CSS_BP_BOTTOM);
    let prompts = gtk4::Box::new(gtk4::Orientation::Horizontal, 28);
    prompts.set_halign(gtk4::Align::End);
    prompts.set_hexpand(true);
    bottom.append(&prompts);
    (bottom, prompts)
}

impl OverlayUi {
    /// Builds the UI shared behind `Rc<RefCell>`: the sidebar and
    /// achievement rows carry real GTK motion/click controllers that borrow
    /// back through the weak self, so the mouse is the system pointer
    /// driving real widgets — never a drawn fake.
    pub fn new(
        on_action: Rc<dyn Fn(CanvasAction)>,
        on_hide: Rc<dyn Fn()>,
        gamescope: bool,
    ) -> Rc<RefCell<Self>> {
        let rc = Rc::new_cyclic(|weak: &Weak<RefCell<Self>>| {
            // Fullscreen column: main row above a bottom button bar.
            let root = gtk4::Overlay::new();
            root.set_halign(gtk4::Align::Fill);
            root.set_valign(gtk4::Align::Fill);
            // The bundled font only applies below a bp-root ancestor.
            root.add_css_class(bp::CSS_BP_ROOT);
            let column = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            column.set_halign(gtk4::Align::Fill);
            column.set_valign(gtk4::Align::Fill);
            root.set_child(Some(&column));

            let main = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            main.set_hexpand(true);
            main.set_vexpand(true);

            let (sidebar, side_title, side_progress, side_rows) = build_sidebar();
            main.append(&sidebar);

            let (content, stack, scrolled, ach_list) = build_content();
            main.append(&content);
            column.append(&main);

            let (bottom, prompts) = build_footer();
            column.append(&bottom);

            // Motion over empty areas still belongs to the mouse: reveal the
            // parked cursor even when no row is under it.
            {
                let hover = weak.clone();
                let motion = gtk4::EventControllerMotion::new();
                motion.connect_motion(move |_, _, _| {
                    if let Some(ui) = hover.upgrade() {
                        ui.borrow_mut().show_cursor();
                    }
                });
                content.add_controller(motion);
            }

            let blank_cursor = gdk4::Cursor::from_name("none", None);
            if gamescope {
                // No hardware cursor over external planes: park the system
                // one now, the composited image below carries the pointer.
                root.set_cursor(blank_cursor.as_ref());
            }
            let (cursor_img, cursor_hot) = widgets::cursor_image();
            cursor_img.set_visible(false);
            let cursor_layer = gtk4::Fixed::new();
            cursor_layer.set_halign(gtk4::Align::Fill);
            cursor_layer.set_valign(gtk4::Align::Fill);
            cursor_layer.put(&cursor_img, 0.0, 0.0);
            root.add_overlay(&cursor_layer);
            let mut ui = Self {
                root,
                self_weak: Weak::new(),
                stack,
                scrolled,
                content: content.clone(),
                sidebar: sidebar.clone(),
                side_title,
                side_progress,
                prompts,
                side_rows,
                ach_rows: Vec::new(),
                ach_list,
                cursor_layer,
                cursor_img,
                cursor_hot,
                pressed: None,
                gamescope,
                on_action,
                on_hide,
                focus: Focus::Side(0),
                cursor_hidden: false,
                last_geom: String::new(),
            };
            for row in ui.side_rows.iter() {
                widgets::watch_row(row, weak, true);
            }
            ui.rebuild_footer(DEFAULT_FAMILY);
            ui.update_content();
            RefCell::new(ui)
        });
        rc.borrow_mut().self_weak = Rc::downgrade(&rc);
        rc
    }

    /// Repaints the footer prompts for a controller family. Called when
    /// the daemon publishes a new physical identity.
    pub fn rebuild_footer(&mut self, family: ControllerFamily) {
        use ira_input::GamepadButton;
        widgets::clear_children(&self.prompts);
        self.prompts.append(&widgets::build_prompt(
            GamepadButton::A,
            &crate::tr!("Select"),
            family,
        ));
        self.prompts.append(&widgets::build_prompt(
            GamepadButton::B,
            &crate::tr!("Back"),
            family,
        ));
        self.prompts.append(&widgets::build_prompt(
            GamepadButton::Guide,
            &crate::tr!("Close"),
            family,
        ));
    }

    pub fn root(&self) -> &gtk4::Overlay {
        &self.root
    }

    pub fn rebuild(&mut self, data: &GameData) {
        self.side_title.set_text(&data.name);
        self.side_progress.set_text(&data.progress_line());
        widgets::clear_children(&self.ach_list);
        self.ach_rows.clear();
        let weak = self.self_weak.clone();
        for ach in &data.achievements {
            // Hidden achievements stay secret until earned (Steam behavior).
            if ach.hidden && !ach.earned {
                continue;
            }
            let row = widgets::build_row(ach);
            widgets::watch_row(&row, &weak, false);
            self.ach_list.append(&row);
            self.ach_rows.push(row);
        }
        if matches!(self.focus, Focus::Ach(_)) {
            self.focus = Focus::Ach(0);
        }
        if debug_log() {
            // One-line data truth per rebuild: distinguishes empty strings
            // at runtime from text that fails to render.
            let first = data.achievements.first().map(|ach| {
                format!(
                    "name_len={} desc_len={} icon_file={} earned={}",
                    ach.name.len(),
                    ach.desc.len(),
                    std::path::Path::new(if ach.earned {
                        &ach.icon
                    } else {
                        &ach.icon_gray
                    })
                    .is_file(),
                    ach.earned
                )
            });
            eprintln!(
                "ira-overlay-ui: rebuild rows={} first={:?}",
                data.achievements.len(),
                first
            );
        }
        self.update_content();
    }

    /// Production layout truth on every show: window, sidebar/content
    /// adjacency and fill, list/viewport/row widths. Same assertions as
    /// the stub geometry test, measured where it matters.
    pub fn log_showgeom(&self) {
        if !debug_log() {
            return;
        }
        let base = self.root.upcast_ref::<gtk4::Widget>();
        let geom = |w: &gtk4::Widget| {
            w.compute_bounds(base)
                .map(|r| (r.x() as i32, r.width() as i32))
        };
        eprintln!(
            "ira-overlay-ui: showgeom win={}x{} sidebar={:?} content={:?} \
             scrolled_w={} list_w={} row0_w={} hadj={}/{}",
            base.width(),
            base.height(),
            geom(self.sidebar.upcast_ref()),
            geom(self.content.upcast_ref()),
            self.scrolled.width(),
            self.ach_list.width(),
            self.ach_rows.first().map(|r| r.width()).unwrap_or(-1),
            self.scrolled.hadjustment().value(),
            self.scrolled.hadjustment().upper(),
        );
    }

    /// Window size for close-path tracing. Logs on change only.
    pub fn log_layout(&mut self) {
        if !debug_log() || self.root.width() <= 0 {
            return;
        }
        let geom = format!("win={}x{}", self.root.width(), self.root.height());
        if self.last_geom != geom {
            self.last_geom = geom.clone();
            eprintln!("ira-overlay-ui: layout {geom}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk4::gio::prelude::ListModelExt as _;

    fn test_ui() -> Option<Rc<RefCell<OverlayUi>>> {
        // Widget construction needs a display connection; headless CI
        // skips instead of failing.
        if gtk4::init().is_err() {
            return None;
        }
        Some(OverlayUi::new(Rc::new(|_| {}), Rc::new(|| {}), true))
    }

    #[test]
    fn test_menu_structure_and_layout() {
        // Single GTK init per process: initializing twice (even on the
        // same thread) aborts, so every widget assertion lives in this
        // one test.
        let Some(ui) = test_ui() else {
            eprintln!("skipping: no display for widget construction");
            return;
        };
        let u = ui.borrow();
        // Five sections, Resume first and focused; content shows the hint.
        assert_eq!(u.side_rows.len(), MenuAction::ALL.len());
        assert_eq!(u.focus, Focus::Side(0));
        assert_eq!(u.section(), MenuAction::Resume);
        assert_eq!(u.stack.visible_child_name().as_deref(), Some("hint"));
        // Footer prompts for the default family.
        assert_eq!(
            u.prompts.observe_children().n_items(),
            3,
            "footer holds Select/Back/Close prompts"
        );
        // Structural adjacency: the main row holds [sidebar, content] in
        // order, and the content pane fills beside the fixed sidebar.
        let sidebar_w = u.sidebar.upcast_ref::<gtk4::Widget>();
        let content_w = u.content.upcast_ref::<gtk4::Widget>();
        let main = content_w.parent().expect("content sits in the main row");
        assert_eq!(main.first_child().as_ref(), Some(sidebar_w));
        assert_eq!(sidebar_w.next_sibling().as_ref(), Some(content_w));
        assert!(u.content.hexpands());
        assert_eq!(u.sidebar.width_request(), 560);
        // Trophies rows fill the viewport width once rebuilt.
        drop(u);
        ui.borrow_mut().rebuild(&GameData {
            name: String::new(),
            total: 0,
            unlocked: 0,
            playtime_seconds: 0,
            achievements: Vec::new(),
        });
        let u = ui.borrow();
        assert!(u.ach_list.hexpands());
    }
}
