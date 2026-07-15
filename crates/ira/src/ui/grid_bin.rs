use glib::subclass::prelude::*;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

mod grid_imp {
    use super::*;

    pub struct GridBin {
        pub grid: RefCell<Option<gtk4::GridView>>,
        pub header: RefCell<Option<gtk4::Widget>>,
        pub header_h: Cell<i32>,
        pub cover_h: Cell<i32>,
        pub n_items: Cell<u32>,
        pub col_nat: Cell<i32>,
        pub prev_width: Cell<i32>,
        pub vadj: RefCell<Option<gtk4::Adjustment>>,
        pub hadj: RefCell<Option<gtk4::Adjustment>>,
        pub grid_vadj: RefCell<Option<gtk4::Adjustment>>,
        pub freeze: Rc<Cell<bool>>,
    }

    impl Default for GridBin {
        fn default() -> Self {
            Self {
                grid: RefCell::new(None),
                header: RefCell::new(None),
                header_h: Cell::new(0),
                cover_h: Cell::new(0),
                n_items: Cell::new(0),
                col_nat: Cell::new(0),
                prev_width: Cell::new(0),
                vadj: RefCell::new(None),
                hadj: RefCell::new(None),
                grid_vadj: RefCell::new(None),
                freeze: Rc::new(Cell::new(false)),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GridBin {
        const NAME: &'static str = "IraGridBin";
        type Type = super::GridBin;
        type ParentType = gtk4::Widget;
        type Interfaces = (gtk4::Scrollable,);
    }

    impl ObjectImpl for GridBin {
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
                "vadjustment" => {
                    let adj = value.get::<Option<gtk4::Adjustment>>().unwrap();
                    if let Some(ref new_adj) = adj {
                        let obj = self.obj().clone();
                        let freeze = self.freeze.clone();
                        new_adj.connect_value_changed(move |_| {
                            if !freeze.get() {
                                obj.queue_allocate();
                            }
                        });
                    }
                    self.vadj.replace(adj);
                    self.obj().queue_allocate();
                }
                "hadjustment" => {
                    let adj = value.get::<Option<gtk4::Adjustment>>().unwrap();
                    self.hadj.replace(adj);
                }
                "hscroll-policy" | "vscroll-policy" => {}
                _ => {}
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "vadjustment" => self.vadj.borrow().to_value(),
                "hadjustment" => self.hadj.borrow().to_value(),
                "hscroll-policy" => gtk4::ScrollablePolicy::Minimum.to_value(),
                "vscroll-policy" => gtk4::ScrollablePolicy::Minimum.to_value(),
                _ => false.to_value(),
            }
        }

        fn dispose(&self) {
            if let Some(grid) = self.grid.take() {
                grid.unparent();
            }
            if let Some(header) = self.header.take() {
                header.unparent();
            }
        }
    }

    impl ScrollableImpl for GridBin {}

    impl GridBin {
        fn grid_content_height(&self, width: i32) -> i32 {
            let col_nat = self.col_nat.get();
            let n_cols = if col_nat > 0 {
                ((width as f64 / col_nat as f64) as u32).clamp(1, 30)
            } else {
                1
            };
            let n_rows = ((self.n_items.get() as f64 / n_cols as f64).ceil() as i32).max(1);
            n_rows * self.cover_h.get()
        }
    }

    impl WidgetImpl for GridBin {
        fn measure(&self, orientation: gtk4::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let header_h = self
                .header
                .borrow()
                .as_ref()
                .map(|h| {
                    let (_, nat, _, _) = h.measure(orientation, for_size);
                    nat
                })
                .unwrap_or(0);

            if orientation == gtk4::Orientation::Vertical {
                self.header_h.set(header_h);
                let width = if for_size > 0 {
                    for_size
                } else {
                    self.prev_width.get().max(1).max(800)
                };
                let h = header_h + self.grid_content_height(width);
                (h, h, -1, -1)
            } else {
                let w = self.col_nat.get() * 30;
                (self.col_nat.get(), w, -1, -1)
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            let header_h = self.header_h.get();
            let cover_h = self.cover_h.get();

            let scroll_pos = self
                .vadj
                .borrow()
                .as_ref()
                .map(|adj| adj.value() as i32)
                .unwrap_or(0);

            let header_visible_h = (header_h - scroll_pos).max(0);

            if let Some(header) = self.header.borrow().as_ref() {
                if header_h > 0 {
                    let tx = gtk4::gsk::Transform::new()
                        .translate(&gtk4::graphene::Point::new(0.0, (-scroll_pos) as f32));
                    header.allocate(width, header_h, -1, Some(tx));
                }
            }

            let grid_y = header_visible_h;
            let grid_h = (height - grid_y).max(0);
            if grid_h > 0 {
                if let Some(g) = self.grid.borrow().as_ref() {
                    let tx = gtk4::gsk::Transform::new()
                        .translate(&gtk4::graphene::Point::new(0.0, grid_y as f32));
                    g.allocate(width, grid_h, baseline, Some(tx));
                }
            }

            let grid_scroll = (scroll_pos - header_h).max(0);
            let grid_content_h = self.grid_content_height(width);

            if let Some(adj) = self.grid_vadj.borrow().as_ref() {
                let grid_ps = grid_h.min(grid_content_h);
                let grid_max = (grid_content_h - grid_ps).max(0);
                let gv = grid_scroll.min(grid_max).max(0) as f64;
                adj.set_value(gv);
                let prev_upper = adj.upper();
                let prev_ps = adj.page_size();
                if (prev_upper - grid_content_h as f64).abs() > 0.5
                    || (prev_ps - grid_ps as f64).abs() > 0.5
                {
                    adj.configure(
                        gv,
                        0.0,
                        grid_content_h as f64,
                        cover_h as f64,
                        grid_ps as f64,
                        grid_ps as f64,
                    );
                }
            }

            let prev = self.prev_width.get();
            if prev != width {
                self.obj().queue_resize();
            }
            self.prev_width.set(width);

            if let Some(vadj) = self.vadj.borrow().as_ref() {
                let upper = header_h + grid_content_h;
                let ps = height.min(upper);
                let max_val = (upper - ps).max(0) as f64;
                if (vadj.upper() - upper as f64).abs() > 0.5 || (vadj.page_size() - ps as f64).abs() > 0.5 {
                    let cur_val = vadj.value().min(max_val).max(0.0);
                    self.freeze.set(true);
                    vadj.configure(cur_val, 0.0, upper as f64, cover_h as f64, ps as f64, ps as f64);
                    self.freeze.set(false);
                }
            }

        }
    }

}

glib::wrapper! {
    pub struct GridBin(ObjectSubclass<grid_imp::GridBin>)
        @extends gtk4::Widget,
        @implements gtk4::Scrollable, gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl GridBin {
    pub fn new(
        grid: &gtk4::GridView,
        header: &gtk4::Widget,
        cover_h: i32,
        n_items: u32,
        col_nat: i32,
    ) -> Self {
        let obj: Self = glib::Object::new();
        let imp = obj.imp();
        imp.cover_h.set(cover_h);
        imp.n_items.set(n_items);
        imp.col_nat.set(col_nat);

        grid.set_parent(&obj);
        imp.grid.replace(Some(grid.clone()));

        header.set_parent(&obj);
        imp.header.replace(Some(header.clone()));

        let grid_adj = gtk4::Adjustment::new(0.0, 0.0, 0.0, 1.0, 0.0, 0.0);
        grid.set_vadjustment(Some(&grid_adj));
        imp.grid_vadj.replace(Some(grid_adj));

        obj
    }
}
