use glib::subclass::prelude::*;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use crate::Game;
use super::game_item::GameItem;

pub type SetupFn = Rc<dyn Fn() -> gtk4::Widget>;
pub type BindFn = Rc<dyn Fn(&gtk4::Widget, &Game)>;
pub type UnbindFn = Rc<dyn Fn(&gtk4::Widget)>;

mod imp {
    use super::*;

    pub struct VirtualGrid {
        pub model: RefCell<Option<gio::ListStore>>,
        pub model_handler: RefCell<Option<glib::SignalHandlerId>>,
        pub n_items: Cell<u32>,

        pub item_width: Cell<i32>,
        pub item_height: Cell<i32>,
        pub min_spacing: Cell<i32>,
        pub prev_width: Cell<i32>,

        pub header: RefCell<Option<gtk4::Widget>>,
        pub header_height: Cell<i32>,

        pub visible: RefCell<HashMap<usize, gtk4::Widget>>,
        pub recycle_pool: RefCell<Vec<gtk4::Widget>>,

        pub vadj: RefCell<Option<gtk4::Adjustment>>,
        pub hadj: RefCell<Option<gtk4::Adjustment>>,
        pub freeze: Cell<bool>,

        pub setup_fn: RefCell<Option<SetupFn>>,
        pub bind_fn: RefCell<Option<BindFn>>,
        pub unbind_fn: RefCell<Option<UnbindFn>>,
    }

    impl Default for VirtualGrid {
        fn default() -> Self {
            Self {
                model: RefCell::new(None),
                model_handler: RefCell::new(None),
                n_items: Cell::new(0),
                item_width: Cell::new(200),
                item_height: Cell::new(300),
                min_spacing: Cell::new(12),
                prev_width: Cell::new(0),
                header: RefCell::new(None),
                header_height: Cell::new(0),
                visible: RefCell::new(HashMap::new()),
                recycle_pool: RefCell::new(Vec::new()),
                vadj: RefCell::new(None),
                hadj: RefCell::new(None),
                freeze: Cell::new(false),
                setup_fn: RefCell::new(None),
                bind_fn: RefCell::new(None),
                unbind_fn: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VirtualGrid {
        const NAME: &'static str = "IraVirtualGrid";
        type Type = super::VirtualGrid;
        type ParentType = gtk4::Widget;
        type Interfaces = (gtk4::Scrollable,);
    }

    impl ObjectImpl for VirtualGrid {
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
            let unbind = self.unbind_fn.borrow().clone();
            let mut visible = self.visible.borrow_mut();
            for (_, widget) in visible.drain() {
                if let Some(ref unbind) = unbind {
                    unbind(&widget);
                }
                widget.unparent();
            }
            let mut pool = self.recycle_pool.borrow_mut();
            for widget in pool.drain(..) {
                widget.unparent();
            }
            if let Some(header) = self.header.borrow().as_ref() {
                header.unparent();
            }
            if let Some(handler) = self.model_handler.borrow_mut().take() {
                if let Some(model) = self.model.borrow().as_ref() {
                    model.disconnect(handler);
                }
            }
        }
    }

    impl ScrollableImpl for VirtualGrid {}

    impl WidgetImpl for VirtualGrid {
        fn measure(&self, orientation: gtk4::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let header_h = self.header_height.get();
            let item_w = self.item_width.get();
            let item_h = self.item_height.get();
            let min_sp = self.min_spacing.get();
            let n_items = self.n_items.get();

            if orientation == gtk4::Orientation::Vertical {
                let width = if for_size > 0 { for_size } else { self.prev_width.get().max(1).max(800) };
                let avail = (width - 2 * min_sp).max(item_w);
                let n_cols = compute_n_cols(avail, item_w, min_sp);
                let n_rows = if n_items == 0 { 0 } else { n_items.div_ceil(n_cols) };
                let row_h = item_h + min_sp;
                let h = header_h + min_sp + (n_rows as i32) * row_h;
                (0, h, -1, -1)
            } else {
                (item_w, item_w * 30, -1, -1)
            }
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let header_h = self
                .header
                .borrow()
                .as_ref()
                .map(|h| {
                    let (_, nat, _, _) = h.measure(gtk4::Orientation::Vertical, width);
                    nat
                })
                .unwrap_or(0);
            self.header_height.set(header_h);

            let item_w = self.item_width.get();
            let item_h = self.item_height.get();
            let min_sp = self.min_spacing.get();
            let n_items = self.n_items.get();
            let row_h = item_h + min_sp;

            if n_items == 0 || width <= 0 || height <= 0 || row_h <= 0 {
                if let Some(adj) = self.vadj.borrow().as_ref() {
                    let upper = header_h as f64;
                    let ps = height.max(0) as f64;
                    self.freeze.set(true);
                    adj.configure(0.0, 0.0, upper, 1.0, ps, ps);
                    self.freeze.set(false);
                }
                if let Some(header) = self.header.borrow().as_ref() {
                    if header_h > 0 {
                        header.allocate(width.max(1), header_h, -1, None);
                    }
                }
                return;
            }

            let avail_width = (width - 2 * min_sp).max(item_w);
            let n_cols = compute_n_cols(avail_width, item_w, min_sp);
            let n_rows = n_items.div_ceil(n_cols) as i32;
            let content_h = n_rows * row_h + min_sp;
            let total_h = header_h + content_h;

            let col_spacing = if n_cols > 1 {
                ((avail_width - n_cols as i32 * item_w) / (n_cols as i32 - 1)).max(0)
            } else {
                0
            };

            let scroll_pos = self
                .vadj
                .borrow()
                .as_ref()
                .map(|adj| adj.value() as i32)
                .unwrap_or(0);

            if let Some(adj) = self.vadj.borrow().as_ref() {
                let ps = (height as f64).min(total_h as f64);
                let upper = total_h as f64;
                let max_val = (upper - ps).max(0.0);
                let cur_val = (scroll_pos as f64).min(max_val).max(0.0);
                let need_configure = (adj.upper() - upper).abs() > 0.5
                    || (adj.page_size() - ps).abs() > 0.5;
                if need_configure {
                    self.freeze.set(true);
                    adj.configure(cur_val, 0.0, upper, row_h as f64, ps * 0.9, ps);
                    self.freeze.set(false);
                } else if (adj.value() - cur_val).abs() > 0.5 {
                    self.freeze.set(true);
                    adj.set_value(cur_val);
                    self.freeze.set(false);
                }
            }

            let actual_scroll = self
                .vadj
                .borrow()
                .as_ref()
                .map(|adj| adj.value() as i32)
                .unwrap_or(0);

            let header_visible_h = (header_h - actual_scroll).max(0);
            let grid_y = header_visible_h;
            let grid_scroll = (actual_scroll - header_h).max(0);
            let first_row = (grid_scroll / row_h).saturating_sub(1).max(0) as usize;
            let visible_h = (height - grid_y).max(0);
            let last_row = ((grid_scroll + visible_h) / row_h + 1) as usize;
            let first_item = first_row.saturating_mul(n_cols as usize);
            let last_item = (n_items as usize).min(last_row.saturating_add(1).saturating_mul(n_cols as usize));

            let setup = self.setup_fn.borrow().clone();
            let bind = self.bind_fn.borrow().clone();
            let unbind = self.unbind_fn.borrow().clone();

            let mut to_recycle: Vec<usize> = self
                .visible
                .borrow()
                .iter()
                .filter(|(&pos, _)| pos < first_item || pos >= last_item)
                .map(|(&pos, _)| pos)
                .collect();

            {
                let mut visible = self.visible.borrow_mut();
                let mut pool = self.recycle_pool.borrow_mut();
                for pos in to_recycle.drain(..) {
                    if let Some(widget) = visible.remove(&pos) {
                        if let Some(ref unbind) = unbind {
                            unbind(&widget);
                        }
                        widget.set_child_visible(false);
                        pool.push(widget);
                    }
                }
            }

            {
                let mut visible = self.visible.borrow_mut();
                let mut pool = self.recycle_pool.borrow_mut();
                for position in first_item..last_item {
                    if visible.contains_key(&position) {
                        continue;
                    }

                    let widget = if let Some(w) = pool.pop() {
                        w.set_child_visible(true);
                        w
                    } else if let Some(ref setup) = setup {
                        let w = setup();
                        w.set_parent(&*self.obj());
                        w
                    } else {
                        continue;
                    };

                    if let Some(ref model) = *self.model.borrow() {
                        if let Some(item) = model.item(position as u32) {
                            if let Some(game_item) = item.downcast_ref::<GameItem>() {
                                if let Some(game) = game_item.game() {
                                    if let Some(ref bind) = bind {
                                        bind(&widget, &game);
                                    }
                                }
                            }
                        }
                    }

                    visible.insert(position, widget);
                }
            }

            if let Some(header) = self.header.borrow().as_ref() {
                if header_h > 0 {
                    let tx = gtk4::gsk::Transform::new()
                        .translate(&gtk4::graphene::Point::new(0.0, (-actual_scroll) as f32));
                    header.allocate(width, header_h, -1, Some(tx));
                }
            }

            {
                let visible = self.visible.borrow();
                for (&position, widget) in visible.iter() {
                    let row = position / n_cols as usize;
                    let col = position % n_cols as usize;
                    let x = min_sp + col as i32 * (item_w + col_spacing);
                    let y = header_h + min_sp + row as i32 * row_h - actual_scroll;

                    widget.set_child_visible(true);
                    let tx = gtk4::gsk::Transform::new()
                        .translate(&gtk4::graphene::Point::new(x as f32, y as f32));
                    widget.allocate(item_w, item_h, -1, Some(tx));
                }
            }

            let prev = self.prev_width.get();
            if prev != width {
                self.obj().queue_resize();
            }
            self.prev_width.set(width);
        }
    }
}

fn compute_n_cols(width: i32, item_w: i32, min_sp: i32) -> u32 {
    (((width + min_sp) / (item_w + min_sp)).max(1) as u32).min(30)
}

glib::wrapper! {
    pub struct VirtualGrid(ObjectSubclass<imp::VirtualGrid>)
        @extends gtk4::Widget,
        @implements gtk4::Scrollable, gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl VirtualGrid {
    pub fn new(item_width: i32, item_height: i32) -> Self {
        let obj: Self = glib::Object::new();
        let imp = obj.imp();
        imp.item_width.set(item_width);
        imp.item_height.set(item_height);
        obj
    }

    pub fn set_model(&self, model: &gio::ListStore) {
        let imp = self.imp();

        if let Some(handler) = imp.model_handler.borrow_mut().take() {
            if let Some(old_model) = imp.model.borrow().as_ref() {
                old_model.disconnect(handler);
            }
        }

        clear_visible(imp);

        let n_items = model.n_items();
        imp.n_items.set(n_items);
        *imp.model.borrow_mut() = Some(model.clone());

        let obj = self.clone();
        let handler = model.connect_items_changed(move |_, _, _, _| {
            let imp = obj.imp();
            let new_n = imp.model.borrow().as_ref().map(|m| m.n_items()).unwrap_or(0);
            imp.n_items.set(new_n);
            clear_visible(imp);
            obj.queue_allocate();
        });
        *imp.model_handler.borrow_mut() = Some(handler);

        self.queue_allocate();
    }

    pub fn set_header(&self, header: Option<&impl IsA<gtk4::Widget>>) {
        let imp = self.imp();
        if let Some(old) = imp.header.borrow().as_ref() {
            old.unparent();
        }
        let header = header.map(|h| h.upcast_ref::<gtk4::Widget>().clone());
        if let Some(ref h) = header {
            h.set_parent(self);
        }
        *imp.header.borrow_mut() = header;
        self.queue_allocate();
    }

    pub fn set_factory(
        &self,
        setup: SetupFn,
        bind: BindFn,
        unbind: UnbindFn,
    ) {
        let imp = self.imp();
        *imp.setup_fn.borrow_mut() = Some(setup);
        *imp.bind_fn.borrow_mut() = Some(bind);
        *imp.unbind_fn.borrow_mut() = Some(unbind);
    }

    pub fn clear_recycle_pool(&self) {
        let imp = self.imp();
        let mut pool = imp.recycle_pool.borrow_mut();
        for widget in pool.drain(..) {
            widget.unparent();
        }
    }
}

fn clear_visible(imp: &imp::VirtualGrid) {
    let unbind = imp.unbind_fn.borrow().clone();
    let mut visible = imp.visible.borrow_mut();
    let mut pool = imp.recycle_pool.borrow_mut();
    for (_, widget) in visible.drain() {
        if let Some(ref unbind) = unbind {
            unbind(&widget);
        }
        widget.set_child_visible(false);
        pool.push(widget);
    }
}
