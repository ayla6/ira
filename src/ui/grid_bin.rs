use glib::subclass::prelude::*;
use gtk4::prelude::*;

mod grid_imp {
    use glib::subclass::prelude::*;
    use gtk4::prelude::*;
    use gtk4::subclass::widget::WidgetImpl;
    use std::cell::{Cell, RefCell};

    pub struct GridBin {
        pub child: RefCell<Option<gtk4::GridView>>,
        pub cover_h: Cell<i32>,
        pub n_items: Cell<u32>,
        pub col_nat: Cell<i32>,
        pub prev_width: Cell<i32>,
    }

    impl Default for GridBin {
        fn default() -> Self {
            Self {
                child: RefCell::new(None),
                cover_h: Cell::new(0),
                n_items: Cell::new(0),
                col_nat: Cell::new(0),
                prev_width: Cell::new(0),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GridBin {
        const NAME: &'static str = "GseGridBin";
        type Type = super::GridBin;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for GridBin {
        fn dispose(&self) {
            if let Some(child) = self.child.take() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for GridBin {
        fn measure(&self, orientation: gtk4::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let cover_h = self.cover_h.get();
            let n_items = self.n_items.get();
            let col_nat = self.col_nat.get();

            if orientation == gtk4::Orientation::Vertical {
                let width = if for_size > 0 {
                    for_size
                } else {
                    let pw = self.prev_width.get();
                    if pw > 1 { pw } else { 800 }
                };
                let n_cols = if col_nat > 0 {
                    ((width as f64 / col_nat as f64) as u32).clamp(1, 30)
                } else {
                    1
                };
                let n_rows = ((n_items as f64 / n_cols as f64).ceil() as i32).max(1);
                let h = n_rows * cover_h;
                (h, h, -1, -1)
            } else {
                let w = col_nat * 30;
                (col_nat, w, -1, -1)
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(child) = self.child.borrow().as_ref() {
                child.allocate(width, height, baseline, None);
            }
            let prev = self.prev_width.get();
            if prev != width {
                self.obj().queue_resize();
            }
            self.prev_width.set(width);
        }
    }
}

glib::wrapper! {
    pub struct GridBin(ObjectSubclass<grid_imp::GridBin>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl GridBin {
    pub fn new(grid: &gtk4::GridView, cover_h: i32, n_items: u32, col_nat: i32) -> Self {
        let obj: Self = glib::Object::new();
        obj.imp().cover_h.set(cover_h);
        obj.imp().n_items.set(n_items);
        obj.imp().col_nat.set(col_nat);
        grid.set_parent(&obj);
        obj.imp().child.replace(Some(grid.clone()));
        obj
    }
}
