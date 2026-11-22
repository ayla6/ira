//! A horizontal row that lines its children up on one text line: the
//! first child showing text sets the line (the middle of its capital
//! letters), every other child showing text puts its own capitals on
//! that line, and every child without text centers on it. Box centering
//! aligns line boxes instead, and the bundled font's line box carries
//! tall CJK metrics, so icons beside text read as floating above it.
//! The placement happens inside GTK's own allocation, so a row is right
//! from its first frame and after every resize or rebuild.

use gtk4::glib;
use gtk4::graphene::Point;
use gtk4::pango;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

mod imp {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    pub struct IconRow {
        pub(super) spacing: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for IconRow {
        const NAME: &'static str = "IraIconRow";
        type Type = super::IconRow;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for IconRow {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for IconRow {
        fn measure(&self, orientation: gtk4::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let children = laid_out_children(self.obj().upcast_ref());
            let (min, nat) = match orientation {
                gtk4::Orientation::Horizontal => {
                    let gaps = self.spacing.get() * (children.len() as i32 - 1).max(0);
                    children.iter().fold((gaps, gaps), |(min, nat), child| {
                        let (child_min, child_nat, _, _) = child.measure(orientation, -1);
                        (min + child_min, nat + child_nat)
                    })
                }
                _ => children.iter().fold((0, 0), |(min, nat), child| {
                    let (child_min, child_nat, _, _) = child.measure(orientation, -1);
                    (min.max(child_min), nat.max(child_nat))
                }),
            };
            (min, nat, -1, -1)
        }

        fn size_allocate(&self, _width: i32, height: i32, _baseline: i32) {
            let row = self.obj();
            let row: &gtk4::Widget = row.upcast_ref();
            let children = laid_out_children(row);
            let mut x = 0;
            let slots: Vec<(i32, i32, i32)> = children
                .iter()
                .map(|child| {
                    let (_, width, _, _) = child.measure(gtk4::Orientation::Horizontal, -1);
                    let (_, child_height, _, _) = child.measure(gtk4::Orientation::Vertical, width);
                    let slot = (x, width, child_height);
                    x += width + self.spacing.get();
                    slot
                })
                .collect();
            let place = |index: usize, y: f32| {
                let (x, width, child_height) = slots[index];
                let at = gtk4::gsk::Transform::new().translate(&Point::new(x as f32, y.round()));
                children[index].allocate(width, child_height, -1, Some(at));
            };
            let centered = |index: usize| (height - slots[index].2) as f32 / 2.0;
            let anchor = children
                .iter()
                .position(|child| shown_label(child).is_some());
            let line = anchor.and_then(|index| {
                place(index, centered(index));
                cap_line(&shown_label(&children[index])?, row)
            });
            for index in (0..children.len()).filter(|index| Some(*index) != anchor) {
                let Some(line) = line else {
                    place(index, centered(index));
                    continue;
                };
                match shown_label(&children[index]) {
                    Some(label) => {
                        // Lay it out once at the top to read where its
                        // capitals land, then drop it onto the line.
                        place(index, 0.0);
                        let offset = cap_line(&label, row).unwrap_or(line);
                        place(index, line - offset);
                    }
                    None => place(index, line - slots[index].2 as f32 / 2.0),
                }
            }
        }
    }
}

glib::wrapper! {
    pub struct IconRow(ObjectSubclass<imp::IconRow>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl IconRow {
    pub(super) fn new(spacing: i32) -> Self {
        let row: Self = glib::Object::new();
        row.set_spacing(spacing);
        row
    }

    pub(super) fn append(&self, child: &impl IsA<gtk4::Widget>) {
        child.as_ref().insert_before(self, None::<&gtk4::Widget>);
    }

    pub(super) fn set_spacing(&self, spacing: i32) {
        if self.imp().spacing.replace(spacing) != spacing {
            self.queue_resize();
        }
    }
}

/// The children GTK wants laid out (visible ones), in order.
fn laid_out_children(row: &gtk4::Widget) -> Vec<gtk4::Widget> {
    let mut children = Vec::new();
    let mut child = row.first_child();
    while let Some(widget) = child {
        if widget.should_layout() {
            children.push(widget.clone());
        }
        child = widget.next_sibling();
    }
    children
}

/// The first label with text that `widget` actually shows, itself
/// included; hidden subtrees (a glyph's unused text fallback) never count.
fn shown_label(widget: &gtk4::Widget) -> Option<gtk4::Label> {
    if !widget.is_visible() {
        return None;
    }
    if let Some(label) = widget.downcast_ref::<gtk4::Label>() {
        return (!label.text().is_empty()).then(|| label.clone());
    }
    let mut child = widget.first_child();
    while let Some(node) = child {
        if let Some(label) = shown_label(&node) {
            return Some(label);
        }
        child = node.next_sibling();
    }
    None
}

/// The middle of `label`'s capital letters in `row`'s coordinates, from
/// its current allocation. "X" measures the cap band independent of the
/// word, so every label agrees whatever its descenders do.
pub(super) fn cap_line(label: &gtk4::Label, row: &gtk4::Widget) -> Option<f32> {
    let origin = label.compute_point(row, &Point::zero())?;
    let probe = pango::Layout::new(&label.pango_context());
    probe.set_text("X");
    let (cap, _) = probe.extents();
    if cap.height() <= 0 {
        return None;
    }
    let (_, layout_y) = label.layout_offsets();
    let scale = pango::SCALE as f32;
    let cap_top = (label.layout().baseline() - probe.baseline() + cap.y()) as f32 / scale;
    Some(origin.y() + layout_y as f32 + cap_top + cap.height() as f32 / scale / 2.0)
}
