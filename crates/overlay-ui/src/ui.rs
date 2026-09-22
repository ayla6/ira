//! Adwaita achievement panel + tier-1 input application.
//!
//! The host applies captured input without event synthesis: directional nav
//! moves GTK focus, activate triggers the focused widget, scroll drives the
//! `ScrolledWindow` adjustment, and mouse targeting uses pure-geometry
//! [`gtk4::Widget::pick`]. Full hover/press states via synthetic `GdkEvent`s
//! is parked behind this tier (same latency either way).

use std::rc::Rc;
use std::sync::OnceLock;

use gtk4::prelude::{BoxExt, ButtonExt, ListBoxRowExt, WidgetExt};
use ira_overlay_ipc::{
    CanvasAction, CanvasCommand, ACT_SCREENSHOT, ACT_TOGGLE_RECORD,
    CMD_ACTIVATE, CMD_HIDE, CMD_MOUSE_DOWN, CMD_MOUSE_MOVE, CMD_MOUSE_UP, CMD_NAV_DOWN,
    CMD_NAV_LEFT, CMD_NAV_RIGHT, CMD_NAV_UP, CMD_SCROLL, CMD_SHOW,
};

use super::game::GameData;

/// Effect of an applied command that the frame loop must honor.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UiEffect {
    None,
    Show,
    Hide,
}

pub struct OverlayUi {
    root: gtk4::Box,
    list: gtk4::ListBox,
    title: gtk4::Label,
    progress: gtk4::Label,
    rows: Vec<gtk4::ListBoxRow>,
    shot_btn: gtk4::Button,
    rec_btn: gtk4::Button,
    focused: usize,
}

impl OverlayUi {
    pub fn new(on_action: Rc<dyn Fn(CanvasAction)>) -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root.set_margin_top(12);
        root.set_margin_bottom(12);
        root.set_margin_start(12);
        root.set_margin_end(12);

        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let titles = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        titles.set_hexpand(true);
        let title = gtk4::Label::new(None);
        title.set_xalign(0.0);
        title.add_css_class("title-2");
        let progress = gtk4::Label::new(None);
        progress.set_xalign(0.0);
        progress.add_css_class("dim-label");
        titles.append(&title);
        titles.append(&progress);

        let shot_btn = gtk4::Button::with_label("Screenshot");
        let rec_btn = gtk4::Button::with_label("Record");
        let on_shot = {
            let push = on_action.clone();
            Rc::new(move || push(CanvasAction::of(ACT_SCREENSHOT)))
        };
        let on_record = {
            let push = on_action.clone();
            Rc::new(move || push(CanvasAction::of(ACT_TOGGLE_RECORD)))
        };
        {
            let cb = on_shot.clone();
            shot_btn.connect_clicked(move |_| cb());
        }
        {
            let cb = on_record.clone();
            rec_btn.connect_clicked(move |_| cb());
        }
        header.append(&titles);
        header.append(&shot_btn);
        header.append(&rec_btn);

        let list = gtk4::ListBox::new();
        list.set_selection_mode(gtk4::SelectionMode::None);
        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_child(Some(&list));
        scrolled.set_vexpand(true);

        root.append(&header);
        root.append(&scrolled);

        Self {
            root,
            list,
            title,
            progress,
            rows: Vec::new(),
            shot_btn,
            rec_btn,
            focused: 0,
        }
    }

    pub fn root(&self) -> &gtk4::Box {
        &self.root
    }

    pub fn rebuild(&mut self, data: &GameData) {
        self.title.set_text(&data.name);
        self.progress.set_text(&data.progress_line());
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        self.rows.clear();
        for ach in &data.achievements {
            // Hidden achievements stay secret until earned (Steam behavior).
            if ach.hidden && !ach.earned {
                continue;
            }
            let row = build_row(ach);
            self.list.append(&row);
            self.rows.push(row);
        }
        self.focused = 0;
        if let Some(first) = self.rows.first() {
            first.add_css_class("ira-focused-row");
            first.grab_focus();
        }
        self.log_layout();
    }

    /// Layout truth for coordinate-bug tracing: where the header buttons
    /// actually are in window pixels. Logs on change only; skipped before
    /// first layout (all zeros).
    pub fn log_layout(&self) {
        if !debug_log() || self.root.width() <= 0 {
            return;
        }
        use glib::object::Cast as _;
        use gtk4::prelude::WidgetExt as _;
        let bounds = |w: &gtk4::Widget| {
            w.compute_bounds(self.root.upcast_ref::<gtk4::Widget>())
                .map(|r| {
                    format!("{},{},{},{}",
                        r.x() as i32, r.y() as i32,
                        r.width() as i32, r.height() as i32)
                })
                .unwrap_or_else(|| "?".to_string())
        };
        static LAST_GEOM: std::sync::OnceLock<std::sync::Mutex<String>> =
            std::sync::OnceLock::new();
        let geom = format!(
            "win={}x{} shot=[{}] rec=[{}]",
            self.root.width(),
            self.root.height(),
            bounds(self.shot_btn.upcast_ref::<gtk4::Widget>()),
            bounds(self.rec_btn.upcast_ref::<gtk4::Widget>())
        );
        let lock = LAST_GEOM.get_or_init(|| std::sync::Mutex::new(String::new()));
        if let Ok(mut last) = lock.lock()
            && *last != geom
        {
            *last = geom.clone();
            eprintln!("ira-overlay-ui: layout {geom}");
        }
    }

    pub fn apply(&mut self, cmd: CanvasCommand) -> UiEffect {
        if debug_log() && cmd.kind != CMD_MOUSE_MOVE {
            eprintln!(
                "ira-overlay-ui: apply kind={} a={} b={} c={}",
                cmd.kind, cmd.a, cmd.b, cmd.c
            );
        }
        match cmd.kind {
            CMD_NAV_UP => self.move_focus(-1),
            CMD_NAV_DOWN => self.move_focus(1),
            CMD_NAV_LEFT => self.move_focus(-1),
            CMD_NAV_RIGHT => self.move_focus(1),
            CMD_ACTIVATE => self.activate_focused(),
            CMD_SCROLL => {
                super::stub::inject(super::stub::InjectedInput::Scroll {
                    dy: cmd.a as f32,
                });
            }
            // Pointer goes straight into the compositor as real Wayland
            // input: GTK produces genuine hover, prelight, press, scroll
            // and focus states itself. No manual picking.
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
            CMD_SHOW => return UiEffect::Show,
            CMD_HIDE => return UiEffect::Hide,
            _ => {}
        }
        UiEffect::None
    }

    fn move_focus(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let next = (self.focused as isize + delta).clamp(0, self.rows.len() as isize - 1);
        self.set_focused(next as usize);
    }

    /// Keyboard focus with a visible highlight. Pointer hover needs no
    /// manual state: injected motion events make GTK paint genuine
    /// `:hover` itself.
    fn set_focused(&mut self, idx: usize) {
        if debug_log() {
            eprintln!("ira-overlay-ui: focus row {idx}");
        }
        if let Some(old) = self.rows.get(self.focused) {
            old.remove_css_class("ira-focused-row");
        }
        self.focused = idx;
        if let Some(row) = self.rows.get(idx) {
            row.add_css_class("ira-focused-row");
            row.grab_focus();
        }
    }

    fn activate_focused(&mut self) {
        // Rows are informational; keyboard activate is a no-op. Pointer
        // clicks land as real button events through the compositor.
        if let Some(row) = self.rows.get(self.focused) {
            row.activate();
        }
    }
}

/// One-shot env check for input-path diagnostics (`IRA_OVERLAY_DEBUG=1`).
fn debug_log() -> bool {
    static DEBUG: OnceLock<bool> = OnceLock::new();
    *DEBUG.get_or_init(|| std::env::var_os("IRA_OVERLAY_DEBUG").is_some())
}

fn build_row(ach: &super::game::AchData) -> gtk4::ListBoxRow {
    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    hbox.set_margin_top(6);
    hbox.set_margin_bottom(6);

    let icon = match icon_image(ach) {
        Some(img) => img,
        None => gtk4::Image::from_icon_name("emblem-default"),
    };
    icon.set_pixel_size(48);

    let texts = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    texts.set_hexpand(true);
    let title = gtk4::Label::new(Some(&ach.name));
    title.set_xalign(0.0);
    let desc = gtk4::Label::new(Some(&ach.desc));
    desc.set_xalign(0.0);
    desc.set_wrap(true);
    desc.add_css_class("dim-label");
    texts.append(&title);
    texts.append(&desc);

    hbox.append(&icon);
    hbox.append(&texts);
    if ach.earned {
        let check = gtk4::Image::from_icon_name("emblem-ok-symbolic");
        hbox.append(&check);
    }

    let row = gtk4::ListBoxRow::new();
    row.set_child(Some(&hbox));
    row
}

fn icon_image(ach: &super::game::AchData) -> Option<gtk4::Image> {
    let path = if ach.earned { &ach.icon } else { &ach.icon_gray };
    if path.is_empty() || !std::path::Path::new(path).is_file() {
        return None;
    }
    let img = gtk4::Image::from_file(path);
    if !ach.earned {
        img.set_opacity(0.45);
    }
    Some(img)
}
