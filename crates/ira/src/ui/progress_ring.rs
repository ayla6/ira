//! A 16px circular progress ring like Nautilus's operation icon
//! (nautilus-progress-paintable.c): the completed fraction of the circle
//! stroked in the foreground color, the rest in a quarter-alpha track.
//! When the operation ends, the ring crossfades into Nautilus's done icon
//! — the check for finished, the x for cancelled — exactly like
//! `nautilus_progress_paintable_animate_done`.

use gtk4::glib;
use gtk4::glib::object::IsA;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::SymbolicPaintable;
use std::cell::{Cell, RefCell};
use std::time::Duration;

mod imp {
    use super::*;
    use gtk4::gdk;
    use std::f64::consts::{FRAC_PI_2, TAU};

    #[derive(Default)]
    pub struct Ring {
        pub fraction: Cell<f64>,
        pub check_progress: Cell<f64>,
        pub icon_name: RefCell<String>,
        pub check_paintable: RefCell<Option<gtk4::IconPaintable>>,
        pub widget: RefCell<Option<gtk4::Widget>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Ring {
        const NAME: &'static str = "IraProgressRing";
        type Type = super::ProgressRing;
        type ParentType = glib::Object;
        type Interfaces = (gdk::Paintable, SymbolicPaintable);
    }

    impl ObjectImpl for Ring {}

    impl PaintableImpl for Ring {
        fn flags(&self) -> gdk::PaintableFlags {
            gdk::PaintableFlags::STATIC_SIZE
        }

        fn intrinsic_width(&self) -> i32 {
            16
        }

        fn intrinsic_height(&self) -> i32 {
            16
        }

        // Fallback for non-symbolic callers; the sidebar Image renders the
        // symbolic variant below, which follows the theme's foreground.
        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            let fg = gdk::RGBA::new(0.7, 0.7, 0.7, 1.0);
            self.draw(snapshot, width, height, fg, None);
        }
    }

    impl SymbolicPaintableImpl for Ring {
        fn snapshot_symbolic(
            &self,
            snapshot: &gdk::Snapshot,
            width: f64,
            height: f64,
            colors: &[gdk::RGBA],
        ) {
            let fg = colors
                .first()
                .copied()
                .unwrap_or(gdk::RGBA::new(0.7, 0.7, 0.7, 1.0));
            self.draw(snapshot, width, height, fg, Some(colors));
        }
    }

    impl Ring {
        fn draw(
            &self,
            snapshot: &gdk::Snapshot,
            width: f64,
            height: f64,
            fg: gdk::RGBA,
            colors: Option<&[gdk::RGBA]>,
        ) {
            let fraction = self.fraction.get().clamp(0.0, 1.0);
            let check = self.check_progress.get();
            let rect = gtk4::graphene::Rect::new(-2.0, -2.0, (width + 4.0) as f32, (height + 4.0) as f32);
            let cr = snapshot.append_cairo(&rect);
            cr.translate(width / 2.0, height / 2.0);
            // Nautilus strokes the ring at a 2px overscan so the stroke
            // never clips.
            cr.set_source_rgba(
                fg.red() as f64,
                fg.green() as f64,
                fg.blue() as f64,
                fg.alpha() as f64,
            );
            cr.arc(0.0, 0.0, width / 2.0 + 1.0, -FRAC_PI_2, fraction * TAU - FRAC_PI_2);
            let _ = cr.stroke();
            cr.set_source_rgba(
                fg.red() as f64,
                fg.green() as f64,
                fg.blue() as f64,
                fg.alpha() as f64 * 0.25,
            );
            cr.arc(0.0, 0.0, width / 2.0 + 1.0, fraction * TAU - FRAC_PI_2, 3.0 * FRAC_PI_2);
            let _ = cr.stroke();
            drop(cr);

            // The done icon fades in over the ring, scaled like
            // nautilus_progress_paintable's check crossfade.
            if check > 0.0 {
                snapshot.save();
                snapshot.translate(&gtk4::graphene::Point::new(
                    (width / 2.0) as f32,
                    (height / 2.0) as f32,
                ));
                snapshot.scale(check as f32, check as f32);
                snapshot.translate(&gtk4::graphene::Point::new(
                    (-(width / 2.0)) as f32,
                    (-(height / 2.0)) as f32,
                ));
                if let Some(icon) = self.check_paintable.borrow().as_ref() {
                    match colors {
                        Some(colors) => icon.snapshot_symbolic(
                            snapshot,
                            width,
                            height,
                            colors,
                        ),
                        None => icon.snapshot(snapshot, width, height),
                    }
                }
                snapshot.restore();
            }
        }
    }
}

glib::wrapper! {
    pub struct ProgressRing(ObjectSubclass<imp::Ring>)
        @implements gdk4::Paintable, gtk4::SymbolicPaintable;
}

impl ProgressRing {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Give the ring a widget so done icons can be looked up on the right
    /// display, scale factor and text direction.
    pub fn attach_widget(&self, widget: &impl IsA<gtk4::Widget>) {
        self.imp().widget.replace(Some(widget.clone().upcast()));
    }

    /// Set the completed fraction (0..1) and repaint the ring.
    pub fn set_fraction(&self, fraction: f64) {
        self.imp().fraction.set(fraction.clamp(0.0, 1.0));
        self.invalidate_contents();
    }

    /// Crossfade the ring into one of Nautilus's done icons over 500ms
    /// (the same duration as `nautilus_progress_paintable_animate_done`).
    pub fn animate_done(&self, icon_name: &str) {
        {
            let imp = self.imp();
            imp.icon_name.replace(icon_name.to_string());
            let display = imp
                .widget
                .borrow()
                .as_ref()
                .map(|widget| widget.display());
            if let Some(display) = display {
                let theme = gtk4::IconTheme::for_display(&display);
                let widget = imp.widget.borrow().as_ref().cloned();
                if let Some(widget) = widget {
                    let paintable = theme.lookup_icon(
                        icon_name,
                        &[],
                        16,
                        widget.scale_factor(),
                        widget.direction(),
                        gtk4::IconLookupFlags::FORCE_SYMBOLIC,
                    );
                    imp.check_paintable.replace(Some(paintable));
                }
            }
        }
        let ring = self.clone();
        let mut step = 0u32;
        let steps = 10u32;
        glib::timeout_add_local(Duration::from_millis(50), move || {
            step += 1;
            ring.imp().check_progress.set(f64::from(step) / f64::from(steps));
            ring.invalidate_contents();
            if step >= steps {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    /// Clear the done icon and start a fresh ring.
    pub fn reset(&self) {
        let imp = self.imp();
        imp.fraction.set(0.0);
        imp.check_progress.set(0.0);
        imp.check_paintable.replace(None);
        self.invalidate_contents();
    }
}

impl Default for ProgressRing {
    fn default() -> Self {
        Self::new()
    }
}
