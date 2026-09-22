//! The big-picture "game running" screen: a full-window black surface
//! with the game's name and a status line. It fades in when a launch
//! lands (the dark transition out of the library) and lifts when the
//! session ends.

use crate::ui::css::*;
use gtk4::prelude::*;
use std::time::Instant;

/// Fade length for the dark transition into the running screen.
const FADE_MILLIS: u128 = 250;

pub(super) struct RunningOverlay {
    root: gtk4::Box,
    title: gtk4::Label,
}

impl RunningOverlay {
    pub(super) fn new() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root.add_css_class(CSS_BP_RUNNING);
        root.add_css_class(CSS_BP_ROOT);
        root.set_halign(gtk4::Align::Fill);
        root.set_valign(gtk4::Align::Fill);
        root.set_hexpand(true);
        root.set_vexpand(true);
        root.set_visible(false);

        let center = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        center.set_halign(gtk4::Align::Center);
        center.set_valign(gtk4::Align::Center);
        center.set_hexpand(true);
        center.set_vexpand(true);

        let title = gtk4::Label::new(None);
        title.add_css_class(CSS_BP_RUNNING_TITLE);
        title.set_wrap(true);
        title.set_justify(gtk4::Justification::Center);
        crate::ui::helpers::crisp_label(&title);
        center.append(&title);

        let status = gtk4::Label::new(Some(&crate::tr!("This game is running")));
        status.add_css_class(CSS_BP_RUNNING_STATUS);
        crate::ui::helpers::crisp_label(&status);
        center.append(&status);

        root.append(&center);
        Self { root, title }
    }

    pub(super) fn root(&self) -> &gtk4::Widget {
        self.root.upcast_ref()
    }

    pub(super) fn is_showing(&self) -> bool {
        self.root.is_visible()
    }

    /// Show the black screen for `game_name`, fading in from transparent
    /// so the library drops into darkness instead of cutting to it.
    pub(super) fn show(&self, game_name: &str) {
        self.title.set_text(game_name);
        self.root.set_opacity(0.0);
        self.root.set_visible(true);
        let root = self.root.clone();
        let started = Instant::now();
        // A frame-clock fade: vsync-stepped like the carousel glides.
        self.root.add_tick_callback(move |_, _| {
            let t = (started.elapsed().as_millis() as f64 / FADE_MILLIS as f64).min(1.0);
            root.set_opacity(t);
            if t >= 1.0 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    pub(super) fn hide(&self) {
        self.root.set_visible(false);
        self.root.set_opacity(1.0);
    }
}

impl super::view::BigPictureUi {
    /// Fade into the black running screen for `game_name`.
    pub(super) fn show_running(&self, game_name: &str) {
        self.game_menu.close();
        self.running.show(game_name);
    }

    /// Lift the black running screen.
    pub(super) fn hide_running(&self) {
        self.running.hide();
    }

    /// True while the black running screen covers the shell.
    pub(super) fn is_running_showing(&self) -> bool {
        self.running.is_showing()
    }
}
