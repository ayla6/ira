use glib::subclass::prelude::*;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use std::cell::{Cell, RefCell};

mod imp {
    use super::*;

    pub struct RecentRow {
        pub covers: RefCell<Vec<gtk4::Widget>>,
        pub hovered: RefCell<Option<gtk4::Widget>>,
        pub cover_h: Cell<i32>,
        pub spacing: Cell<i32>,
        pub adj: RefCell<Option<gtk4::Adjustment>>,
        pub freeze: Cell<bool>,
    }

    impl Default for RecentRow {
        fn default() -> Self {
            Self {
                covers: RefCell::new(Vec::new()),
                hovered: RefCell::new(None),
                cover_h: Cell::new(0),
                spacing: Cell::new(8),
                adj: RefCell::new(None),
                freeze: Cell::new(false),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RecentRow {
        const NAME: &'static str = "IraRecentRow";
        type Type = super::RecentRow;
        type ParentType = gtk4::Widget;
        type Interfaces = (gtk4::Scrollable,);
    }

    impl ObjectImpl for RecentRow {
        fn properties() -> &'static [glib::ParamSpec] {
            use std::sync::OnceLock;
            static PROPERTIES: OnceLock<Vec<glib::ParamSpec>> = OnceLock::new();
            PROPERTIES.get_or_init(|| {
                vec![
                    glib::ParamSpecObject::builder::<gtk4::Adjustment>("vadjustment").build(),
                    glib::ParamSpecObject::builder::<gtk4::Adjustment>("hadjustment").build(),
                    glib::ParamSpecEnum::builder_with_default::<gtk4::ScrollablePolicy>(
                        "hscroll-policy",
                        gtk4::ScrollablePolicy::Minimum,
                    )
                    .build(),
                    glib::ParamSpecEnum::builder_with_default::<gtk4::ScrollablePolicy>(
                        "vscroll-policy",
                        gtk4::ScrollablePolicy::Minimum,
                    )
                    .build(),
                ]
            })
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "hadjustment" => {
                    let adj = value.get::<Option<gtk4::Adjustment>>().unwrap();
                    if let Some(ref new_adj) = adj {
                        let obj = self.obj().downgrade();
                        let freeze = self.freeze.clone();
                        new_adj.connect_value_changed(move |_| {
                            if !freeze.get() {
                                if let Some(obj) = obj.upgrade() {
                                    obj.queue_allocate();
                                    obj.queue_draw();
                                }
                            }
                        });
                    }
                    self.adj.replace(adj);
                    self.obj().queue_allocate();
                }
                "vadjustment" => {
                    value.get::<Option<gtk4::Adjustment>>().unwrap();
                }
                "hscroll-policy" | "vscroll-policy" => {}
                _ => {}
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "hadjustment" => self.adj.borrow().to_value(),
                "vadjustment" => self.adj.borrow().to_value(),
                "hscroll-policy" => gtk4::ScrollablePolicy::Minimum.to_value(),
                "vscroll-policy" => gtk4::ScrollablePolicy::Minimum.to_value(),
                _ => false.to_value(),
            }
        }

        fn dispose(&self) {
            for cover in self.covers.borrow_mut().drain(..) {
                cover.unparent();
            }
            *self.hovered.borrow_mut() = None;
        }
    }

    impl ScrollableImpl for RecentRow {}

    impl WidgetImpl for RecentRow {
        fn measure(&self, orientation: gtk4::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let spacing = self.spacing.get();
            if orientation == gtk4::Orientation::Vertical {
                // A scrollable child isn't wrapped in a viewport, so the
                // scrolled window can only size itself from our minimum here.
                let h = self.cover_h.get() + 2 * spacing;
                (h, h, -1, -1)
            } else {
                let covers = self.covers.borrow();
                let content: i32 = covers.iter().map(|c| c.width_request().max(1)).sum::<i32>()
                    + spacing * (covers.len() as i32 + 1);
                (0, content.max(1), -1, -1)
            }
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let covers = self.covers.borrow();
            let spacing = self.spacing.get();
            let cover_h = self.cover_h.get().max(1);

            let widths: Vec<i32> = covers.iter().map(|c| c.width_request().max(1)).collect();
            let content: i32 = widths.iter().sum::<i32>() + spacing * (covers.len() as i32 + 1);

            let page = (width as f64).max(1.0);
            let upper = (content as f64).max(page);
            if let Some(adj) = self.adj.borrow().as_ref() {
                let max_val = (upper - page).max(0.0);
                let cur = adj.value().clamp(0.0, max_val);
                if !self.freeze.get() {
                    self.freeze.set(true);
                    adj.configure(cur.min(upper), 0.0, upper, 1.0, page * 0.5, page);
                    self.freeze.set(false);
                }
            }

            let off = if upper > page + 0.5 {
                self.adj.borrow().as_ref().map(|a| a.value()).unwrap_or(0.0)
            } else {
                0.0
            };

            for c in covers.iter() {
                c.set_child_visible(false);
            }

            let mut x: f64 = spacing as f64 - off;
            for (i, c) in covers.iter().enumerate() {
                let w = widths[i] as f64;
                if x + w >= -0.5 && x <= width as f64 + 0.5 {
                    c.set_child_visible(true);
                    let tx = gtk4::gsk::Transform::new()
                        .translate(&gtk4::graphene::Point::new(x as f32, spacing as f32));
                    c.allocate(w as i32, cover_h, -1, Some(tx));
                }
                x += w + spacing as f64;
            }
        }

        /// Paint covers in child order but draw the hovered cover last so its
        /// hover scale sits on top of the overlapping neighbor to its right
        /// (which would otherwise be painted over it).
        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            let covers = self.covers.borrow();
            let hovered = self.hovered.borrow();
            for c in covers.iter() {
                if !c.is_child_visible() {
                    continue;
                }
                if hovered.as_ref().is_none_or(|h| h.as_ptr() != c.as_ptr()) {
                    self.obj().snapshot_child(c, snapshot);
                }
            }
            if let Some(h) = hovered.as_ref() {
                if h.is_child_visible() {
                    self.obj().snapshot_child(h, snapshot);
                }
            }
        }
    }
}

glib::wrapper! {
    pub struct RecentRow(ObjectSubclass<imp::RecentRow>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget, gtk4::Scrollable;
}

impl RecentRow {
    /// `spacing` mirrors the grid's `sp` (edge pads and cover gap). Covers are
    /// laid out with that gap on every side, so hover shadows and the hover
    /// scale stay inside the widget's own vertical allocation and paint above
    /// and below it (the widget's overflow is visible). `cover_h` is the fixed
    /// cover height every appended cover is allocated.
    pub fn new(spacing: i32, cover_h: i32) -> Self {
        let obj: Self = glib::Object::new();
        obj.imp().spacing.set(spacing);
        obj.imp().cover_h.set(cover_h);
        obj.set_overflow(gtk4::Overflow::Visible);
        obj
    }

    pub fn append_cover(&self, cover: &impl IsA<gtk4::Widget>) {
        let cover = cover.upcast_ref::<gtk4::Widget>().clone();
        cover.set_parent(self);
        self.imp().covers.borrow_mut().push(cover.clone());

        // Keep only weak refs in the closures: the row owns the covers and
        // adjusting stores the callback, so strong captures would leak the
        // whole carousel on every rebuild.
        let row = self.downgrade();
        let c = cover.downgrade();
        let motion = gtk4::EventControllerMotion::new();
        motion.connect_enter(move |_, _, _| {
            if let (Some(row), Some(c)) = (row.upgrade(), c.upgrade()) {
                row.set_hovered(Some(&c));
            }
        });
        let row = self.downgrade();
        motion.connect_leave(move |_| {
            if let Some(row) = row.upgrade() {
                row.set_hovered(None);
            }
        });
        cover.add_controller(motion);

        self.queue_resize();
    }

    fn set_hovered(&self, cover: Option<&gtk4::Widget>) {
        let cur = self.imp().hovered.borrow().as_ref().map(|h| h.as_ptr());
        let new = cover.map(|c| c.as_ptr());
        if cur != new {
            *self.imp().hovered.borrow_mut() = cover.map(ToOwned::to_owned);
            self.queue_draw();
        }
    }
}
