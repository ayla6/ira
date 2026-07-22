use glib::subclass::prelude::*;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use super::helpers::{esc, format_duration};
use super::play_history_chart::{DayData, color_hex, other_hex};
use super::css::*;

const Y_AXIS_W: i32 = 34;
const BAR_MARGIN: i32 = 8;
const BAR_RADIUS: f32 = 5.0;

type DayCallback = Rc<dyn Fn(usize) + 'static>;

fn fmt_axis(s: f64) -> String {
    if s < 60.0 { "0".into() }
    else if s < 3600.0 { format!("{}m",(s/60.0).round() as i64) }
    else { format!("{}h",(s/3600.0).round() as i64) }
}

fn parse_color(hex: &str) -> gtk4::gdk::RGBA {
    gtk4::gdk::RGBA::parse(hex).unwrap_or_else(|_| gtk4::gdk::RGBA::new(0.5, 0.5, 0.5, 1.0))
}

mod imp {
    use super::*;

    pub struct BarChart {
        pub bar_groups: RefCell<Vec<gtk4::Box>>,
        pub(super) days: RefCell<Vec<DayData>>,
        pub is_single_game: Cell<bool>,
        pub y_labels: RefCell<Vec<gtk4::Label>>,
        pub nmax: Cell<f64>,
        pub interval: Cell<f64>,
        pub selected: Cell<usize>,
        pub hovered: Cell<Option<usize>>,
        pub callback: RefCell<Option<DayCallback>>,
    }

    impl Default for BarChart {
        fn default() -> Self {
            Self {
                bar_groups: RefCell::new(vec![]),
                days: RefCell::new(vec![]),
                is_single_game: Cell::new(false),
                y_labels: RefCell::new(vec![]),
                nmax: Cell::new(3600.0),
                interval: Cell::new(1800.0),
                selected: Cell::new(6),
                hovered: Cell::new(None),
                callback: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BarChart {
        const NAME: &'static str = "IraBarChart";
        type Type = super::BarChart;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for BarChart {
        fn dispose(&self) {
            for w in self.bar_groups.borrow().iter() { w.unparent(); }
            for w in self.y_labels.borrow().iter() { w.unparent(); }
        }
    }

    impl WidgetImpl for BarChart {
        fn measure(&self, orientation: gtk4::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk4::Orientation::Horizontal => (70, 350, -1, -1),
                _ => (120, 400, -1, -1),
            }
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let nmax = self.nmax.get();
            let interval = self.interval.get();
            let chart_w = (width - Y_AXIS_W).max(1);
            let bar_w = chart_w / 7;

            // Y-axis labels (right side, centered on gridlines, with gap from chart)
            for (i, lbl) in self.y_labels.borrow().iter().enumerate() {
                let val = i as f64 * interval;
                let pct = val / nmax;
                let y = height - (height as f64 * pct).round() as i32;
                let tx = gtk4::gsk::Transform::new()
                    .translate(&gtk4::graphene::Point::new(
                        (chart_w + 6) as f32,
                        (y - 9) as f32,
                    ));
                lbl.allocate(Y_AXIS_W - 6, 18, -1, Some(tx));
            }

            // Bar groups (full column, for click/tooltips)
            for (i, grp) in self.bar_groups.borrow().iter().enumerate() {
                let x = i as i32 * bar_w;
                let tx = gtk4::gsk::Transform::new()
                    .translate(&gtk4::graphene::Point::new(x as f32, 0.0));
                grp.allocate(bar_w, height, -1, Some(tx));
            }
        }

        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            let width = self.obj().width();
            let height = self.obj().height();
            if width <= 0 || height <= 0 { return; }

            let nmax = self.nmax.get();
            let interval = self.interval.get();
            let scale = height as f64 / nmax;
            let chart_w = (width - Y_AXIS_W).max(1);
            let bar_w = chart_w / 7;
            let inner_w = (bar_w - 2 * BAR_MARGIN).max(1) as f32;
            let is_sg = self.is_single_game.get();
            let selected = self.selected.get();
            let hovered = self.hovered.get();

            let accent = adw::StyleManager::default()
                .accent_color()
                .to_rgba();
            let fg = self.obj().color();

            let days = self.days.borrow();

            // Hover background (rounded, full column)
            if let Some(hov) = hovered {
                let hov_x = (hov as i32 * bar_w) as f32;
                let hov_w = bar_w as f32;
                let rect = gtk4::gsk::RoundedRect::from_rect(
                    gtk4::graphene::Rect::new(hov_x, 0.0, hov_w, height as f32),
                    6.0,
                );
                snapshot.push_rounded_clip(&rect);
                snapshot.append_color(
                    &fg.with_alpha(0.07),
                    &gtk4::graphene::Rect::new(hov_x, 0.0, hov_w, height as f32),
                );
                snapshot.pop();
            }

            // Selection background (rounded, full column)
            {
                let sel_x = (selected as i32 * bar_w) as f32;
                let sel_w = bar_w as f32;
                let rect = gtk4::gsk::RoundedRect::from_rect(
                    gtk4::graphene::Rect::new(sel_x, 0.0, sel_w, height as f32),
                    6.0,
                );
                snapshot.push_rounded_clip(&rect);
                snapshot.append_color(
                    &fg.with_alpha(0.10),
                    &gtk4::graphene::Rect::new(sel_x, 0.0, sel_w, height as f32),
                );
                snapshot.pop();
            }

            // Bars
            for (day_idx, day) in days.iter().enumerate() {
                if day.total <= 0.0 { continue; }

                let x = (day_idx as i32 * bar_w + BAR_MARGIN) as f32;
                let total_h = (day.total * scale).max(1.0) as f32;
                let y_top = height as f32 - total_h;

                let rounded = gtk4::gsk::RoundedRect::new(
                    gtk4::graphene::Rect::new(x, y_top, inner_w, total_h),
                    gtk4::graphene::Size::new(BAR_RADIUS, BAR_RADIUS),
                    gtk4::graphene::Size::new(BAR_RADIUS, BAR_RADIUS),
                    gtk4::graphene::Size::new(0.0, 0.0),
                    gtk4::graphene::Size::new(0.0, 0.0),
                );
                snapshot.push_rounded_clip(&rounded);

                if is_sg {
                    let color = if day_idx == selected {
                        accent
                    } else {
                        accent.with_alpha(0.65)
                    };
                    snapshot.append_color(
                        &color,
                        &gtk4::graphene::Rect::new(x, y_top, inner_w, total_h),
                    );
                } else {
                    let seg_alpha = if day_idx == selected { 1.0 } else { 0.65 };
                    let mut y_bottom = height as f32;
                    for seg in &day.segments {
                        let seg_h = (seg.value * scale).max(1.0) as f32;
                        let seg_y = y_bottom - seg_h;
                        let color = match seg.color_index {
                            Some(idx) => parse_color(color_hex(idx)),
                            None => parse_color(other_hex()),
                        }.with_alpha(seg_alpha);
                        snapshot.append_color(
                            &color,
                            &gtk4::graphene::Rect::new(x, seg_y, inner_w, seg_h),
                        );
                        y_bottom = seg_y;
                    }
                }

                snapshot.pop();
            }

            // Gridlines on top of bars
            let num_lines = (nmax / interval) as usize + 1;
            let grid_color = fg.with_alpha(0.08);
            for i in 0..num_lines {
                let val = i as f64 * interval;
                let pct = val / nmax;
                let y = height as f32 - (height as f32 * pct as f32).round();
                snapshot.append_color(
                    &grid_color,
                    &gtk4::graphene::Rect::new(0.0, y, chart_w as f32, 1.0),
                );
            }

            // Draw child widgets (y-axis labels, bar_groups for tooltips)
            self.parent_snapshot(snapshot);
        }
    }
}

glib::wrapper! {
    pub struct BarChart(ObjectSubclass<imp::BarChart>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl BarChart {
    pub fn new() -> Self {
        let obj: Self = glib::Object::new();

        let motion = gtk4::EventControllerMotion::new();
        {
            let obj_clone = obj.clone();
            motion.connect_motion(move |_, x, _| {
                let width = obj_clone.width();
                let chart_w = (width - Y_AXIS_W).max(1);
                let bar_w = chart_w / 7;
                let idx = if bar_w > 0 {
                    ((x as i32) / bar_w) as usize
                } else {
                    0
                };
                let idx = idx.min(6);
                let imp = obj_clone.imp();
                if imp.hovered.get() != Some(idx) {
                    imp.hovered.set(Some(idx));
                    obj_clone.queue_draw();
                }
            });
        }
        {
            let obj_clone = obj.clone();
            motion.connect_leave(move |_| {
                if obj_clone.imp().hovered.replace(None).is_some() {
                    obj_clone.queue_draw();
                }
            });
        }
        obj.add_controller(motion);
        obj
    }

    pub(super) fn connect_day_activated<F: Fn(usize) + 'static>(&self, f: F) {
        self.imp().callback.replace(Some(Rc::new(f)));
    }

    pub(super) fn set_data(&self, days: &[DayData], nmax: f64, interval: f64, is_single_game: bool) {
        let imp = self.imp();

        // Unparent old children
        for w in imp.bar_groups.borrow().iter() { w.unparent(); }
        for w in imp.y_labels.borrow().iter() { w.unparent(); }

        imp.nmax.set(nmax);
        imp.interval.set(interval);
        imp.is_single_game.set(is_single_game);

        // Y-axis labels
        let num_lines = (nmax / interval) as usize + 1;
        let mut y_labels = Vec::with_capacity(num_lines);
        for i in 0..num_lines {
            let val = i as f64 * interval;
            let lbl = gtk4::Label::new(Some(&fmt_axis(val)));
            lbl.add_css_class(CSS_DIM_LABEL);
            lbl.set_halign(gtk4::Align::Start);
            lbl.set_parent(self);
            y_labels.push(lbl);
        }
        imp.y_labels.replace(y_labels);

        // Bar groups (for click handling and combined tooltips)
        let mut bar_groups = Vec::with_capacity(7);
        for (i, day) in days.iter().enumerate() {
            let grp = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            grp.set_parent(self);

            if day.total > 0.0 {
                let tooltip: String = if is_single_game {
                    day.segments.first().map(|seg| {
                        format!("<b>{}</b>\n{}", esc(&seg.label), format_duration(seg.value as i64))
                    }).unwrap_or_default()
                } else {
                    day.segments.iter()
                        .map(|seg| format!("<b>{}</b>\n{}", esc(&seg.label), format_duration(seg.value as i64)))
                        .collect::<Vec<_>>()
                        .join("\n\n")
                };
                if !tooltip.is_empty() {
                    grp.set_tooltip_markup(Some(&tooltip));
                }
            }

            let cb = imp.callback.borrow().clone();
            let click = gtk4::GestureClick::new();
            click.connect_pressed(move |_, _, _, _| {
                if let Some(f) = &cb { f(i); }
            });
            grp.add_controller(click);
            bar_groups.push(grp);
        }
        imp.bar_groups.replace(bar_groups);
        imp.days.replace(days.to_vec());

        self.queue_resize();
        self.queue_draw();
    }

    pub(super) fn set_selected_day(&self, idx: usize, _is_single_game: bool) {
        self.imp().selected.set(idx);
        self.queue_draw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn test_fmt_axis() {
        assert_eq!(fmt_axis(0.0), "0");
        assert_eq!(fmt_axis(1800.0), "30m");
        assert_eq!(fmt_axis(3600.0), "1h");
    }
}
