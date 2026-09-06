use super::game_item::GameItem;
use crate::Game;
use glib::subclass::prelude::*;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

pub type SetupFn = Rc<dyn Fn() -> gtk4::Widget>;
pub type BindFn = Rc<dyn Fn(&gtk4::Widget, &Game)>;
pub type UnbindFn = Rc<dyn Fn(&gtk4::Widget)>;
pub type SizeChangedFn = Rc<dyn Fn(i32, i32)>;

const ASPECT_RATIO: f64 = 1.5;
const STEP_COLS: &[u32] = &[5, 7, 9, 11, 13, 15];
const STEP_SIZES: &[i32] = &[110, 150, 200, 250, 300, 350];
const MIN_VISIBLE_ROWS: f64 = 2.5;

fn compute_grid_layout(
    width: i32,
    min_item_w: i32,
    base_sp: i32,
    viewport_h: i32,
    aspect: f64,
    fixed_cols: u32,
) -> (u32, i32, i32, i32) {
    // Couch mode: an exact column count at every resolution, with
    // Switch-style proportions derived from the tile itself — half a tile
    // of edge margin on each side, a tenth-of-a-tile gap between tiles:
    // width = item × (n + 1 + (n−1)/10). At 1920 wide that lands on
    // 256px tiles, like the reference.
    if fixed_cols > 0 {
        let n = fixed_cols.max(1) as i32;
        let item_w = (width * 10 / (11 * n + 9)).max(1);
        let item_h = ((item_w as f64) * aspect) as i32;
        let sp = item_w / 10;
        return (fixed_cols, item_w, item_h, sp);
    }

    let avail_w = (width - 2 * base_sp).max(min_item_w);
    let raw_cols = (((avail_w + base_sp) / (min_item_w + base_sp)).max(1) as u32).min(30);

    let width_step = STEP_COLS
        .iter()
        .enumerate()
        .rev()
        .find(|(_, &c)| c <= raw_cols)
        .map(|(i, _)| i)
        .unwrap_or(0);

    let max_step = if viewport_h > 0 {
        STEP_SIZES
            .iter()
            .enumerate()
            .rev()
            .find(|(_, &w)| {
                let h = ((w as f64) * aspect) as i32;
                let row_h = h + base_sp;
                (viewport_h as f64) / (row_h as f64) >= MIN_VISIBLE_ROWS
            })
            .map(|(i, _)| i)
            .unwrap_or(0)
    } else {
        STEP_SIZES.len() - 1
    };

    let step_idx = width_step.min(max_step);
    let item_w = STEP_SIZES[step_idx].max(min_item_w);

    let sp = base_sp + step_idx as i32 * 4;
    let avail_w = (width - 2 * sp).max(item_w);
    let n_cols = (((avail_w + sp) / (item_w + sp)).max(1) as u32).min(30);

    // One spacing value everywhere: edges, column gaps and row gaps are
    // all `sp`, and the tiles grow or shrink to close the width exactly —
    // the leftover is never dumped into the gaps.
    let item_w = ((avail_w - (n_cols as i32 - 1) * sp) / n_cols as i32).max(min_item_w);
    let item_h = ((item_w as f64) * aspect) as i32;

    (n_cols, item_w, item_h, sp)
}

mod imp {
    use super::*;

    pub struct VirtualGrid {
        pub model: RefCell<Option<gio::ListStore>>,
        pub model_handler: RefCell<Option<glib::SignalHandlerId>>,
        pub n_items: Cell<u32>,

        pub min_item_width: Cell<i32>,
        pub cur_item_w: Cell<i32>,
        pub cur_item_h: Cell<i32>,
        pub square: Cell<bool>,
        pub item_size: Rc<Cell<(i32, i32)>>,
        pub min_spacing: Cell<i32>,
        pub prev_width: Cell<i32>,

        pub header: RefCell<Option<gtk4::Widget>>,
        pub header_height: Cell<i32>,

        pub visible: RefCell<HashMap<usize, gtk4::Widget>>,
        pub recycle_pool: RefCell<Vec<gtk4::Widget>>,

        pub vadj: RefCell<Option<gtk4::Adjustment>>,
        pub hadj: RefCell<Option<gtk4::Adjustment>>,
        pub freeze: Cell<bool>,
        pub dirty: Cell<bool>,
        /// The layout applied by the last allocation: (columns, item width,
        /// item height, edge spacing). Scroll-to-index math reads it instead
        /// of recomputing from the viewport size.
        pub last_layout: Cell<(u32, i32, i32, i32)>,
        /// The column gap from the last allocation; columns widen to fill
        /// the viewport, so cell geometry needs the real value.
        pub last_col_spacing: Cell<i32>,
        /// The edge margin from the last allocation (a fixed-column couch
        /// grid derives it from the tile, not from the gap).
        pub last_edge: Cell<i32>,
        /// When set, the grid always lays out exactly this many columns —
        /// the couch grid pins 6 and scales the tiles with the viewport.
        pub fixed_cols: Cell<u32>,

        pub setup_fn: RefCell<Option<SetupFn>>,
        pub bind_fn: RefCell<Option<BindFn>>,
        pub unbind_fn: RefCell<Option<UnbindFn>>,
        pub size_changed_fn: RefCell<Option<SizeChangedFn>>,
        /// Fired after every allocation pass: floating overlays anchored
        /// to tiles (the selection ring, the name tooltip) re-read their
        /// anchor here, so a relayout can never leave them stale.
        pub post_layout_fn: RefCell<Option<Rc<dyn Fn()>>>,
    }

    impl Default for VirtualGrid {
        fn default() -> Self {
            let min_w = 110;
            let min_h = ((min_w as f64) * ASPECT_RATIO) as i32;
            Self {
                model: RefCell::new(None),
                model_handler: RefCell::new(None),
                n_items: Cell::new(0),
                min_item_width: Cell::new(min_w),
                cur_item_w: Cell::new(min_w),
                cur_item_h: Cell::new(min_h),
                square: Cell::new(false),
                item_size: Rc::new(Cell::new((min_w, min_h))),
                min_spacing: Cell::new(8),
                prev_width: Cell::new(0),
                header: RefCell::new(None),
                header_height: Cell::new(0),
                visible: RefCell::new(HashMap::new()),
                recycle_pool: RefCell::new(Vec::new()),
                vadj: RefCell::new(None),
                hadj: RefCell::new(None),
                freeze: Cell::new(false),
                dirty: Cell::new(false),
                last_layout: Cell::new((1, min_w, min_h, 8)),
                last_col_spacing: Cell::new(8),
                last_edge: Cell::new(8),
                fixed_cols: Cell::new(0),
                setup_fn: RefCell::new(None),
                bind_fn: RefCell::new(None),
                unbind_fn: RefCell::new(None),
                size_changed_fn: RefCell::new(None),
                post_layout_fn: RefCell::new(None),
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

    impl VirtualGrid {
        fn square_aspect(&self) -> f64 {
            if self.square.get() {
                1.0
            } else {
                ASPECT_RATIO
            }
        }
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
                        let obj = self.obj().downgrade();
                        let freeze = self.freeze.clone();
                        new_adj.connect_value_changed(move |_| {
                            if !freeze.get() {
                                if let Some(obj) = obj.upgrade() {
                                    obj.queue_allocate();
                                }
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
            let min_w = self.min_item_width.get();
            let min_sp = self.min_spacing.get();
            let n_items = self.n_items.get();

            if orientation == gtk4::Orientation::Vertical {
                let width = if for_size > 0 {
                    for_size
                } else {
                    self.prev_width.get().max(1).max(800)
                };
                let (n_cols, _item_w, item_h, sp) = compute_grid_layout(
                    width,
                    min_w,
                    min_sp,
                    0,
                    self.square_aspect(),
                    self.fixed_cols.get(),
                );
                let n_rows = if n_items == 0 {
                    0
                } else {
                    n_items.div_ceil(n_cols)
                };
                let row_h = item_h + sp;
                let h = header_h + sp + (n_rows as i32) * row_h;
                (0, h, -1, -1)
            } else {
                (min_w, min_w * 30, -1, -1)
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

            let min_w = self.min_item_width.get();
            let min_sp = self.min_spacing.get();
            let n_items = self.n_items.get();

            if n_items == 0 || width <= 0 || height <= 0 {
                if let Some(adj) = self.vadj.borrow().as_ref() {
                    let upper = (header_h as f64).max(height as f64);
                    let ps = (height as f64).min(upper);
                    self.freeze.set(true);
                    adj.configure(0.0, 0.0, upper, 1.0, ps * 0.9, ps);
                    self.freeze.set(false);
                }
                if let Some(header) = self.header.borrow().as_ref() {
                    if header_h > 0 {
                        header.allocate(width.max(1), header_h, -1, None);
                    }
                }
                return;
            }

            let (n_cols, item_w, item_h, sp) = compute_grid_layout(
                width,
                min_w,
                min_sp,
                height,
                self.square_aspect(),
                self.fixed_cols.get(),
            );
            self.last_layout.set((n_cols, item_w, item_h, sp));
            // Center the content: the rounding slack from the integer
            // tile size splits between the two edges instead of piling
            // up on the right.
            let content_w = n_cols as i32 * item_w + (n_cols as i32 - 1) * sp;
            self.last_edge.set(((width - content_w) / 2).max(0));
            let row_h = item_h + sp;
            let n_rows = n_items.div_ceil(n_cols) as i32;
            let content_h = n_rows * row_h + sp;
            let total_h = header_h + content_h;

            // The tiles were sized to close the width exactly, so every
            // gap equals the edge spacing.
            let col_spacing = sp;
            self.last_col_spacing.set(col_spacing);

            let prev_w = self.cur_item_w.get();
            let prev_h = self.cur_item_h.get();
            let size_changed = prev_w != item_w || prev_h != item_h;
            if size_changed {
                self.cur_item_w.set(item_w);
                self.cur_item_h.set(item_h);
                self.item_size.set((item_w, item_h));
                if let Some(cb) = self.size_changed_fn.borrow().as_ref() {
                    let cb = cb.clone();
                    glib::idle_add_local_once(move || cb(item_w, item_h));
                }
            }

            let scroll_pos = self
                .vadj
                .borrow()
                .as_ref()
                .map(|adj| adj.value() as i32)
                .unwrap_or(0);

            if let Some(adj) = self.vadj.borrow().as_ref() {
                let upper = total_h.max(height) as f64;
                let ps = (height as f64).min(upper);
                let max_val = (upper - ps).max(0.0);
                let cur_val = (scroll_pos as f64).min(max_val).max(0.0);
                let need_configure =
                    (adj.upper() - upper).abs() > 0.5 || (adj.page_size() - ps).abs() > 0.5;
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
            let last_item =
                (n_items as usize).min(last_row.saturating_add(1).saturating_mul(n_cols as usize));

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
                                        bind(&widget, game.as_ref());
                                    }
                                }
                            }
                        }
                    }

                    visible.insert(position, widget);
                }
            }

            if size_changed {
                self.dirty.set(false);
                let unbind = self.unbind_fn.borrow().clone();
                let bind = self.bind_fn.borrow().clone();
                let model = self.model.borrow().clone();
                let visible = self.visible.borrow();
                for (&position, widget) in visible.iter() {
                    if let Some(ref unbind) = unbind {
                        unbind(widget);
                    }
                    if let Some(ref model) = model {
                        if let Some(item) = model.item(position as u32) {
                            if let Some(game_item) = item.downcast_ref::<GameItem>() {
                                if let Some(game) = game_item.game() {
                                    if let Some(ref bind) = bind {
                                        bind(widget, game.as_ref());
                                    }
                                }
                            }
                        }
                    }
                }
            } else if self.dirty.get() {
                self.dirty.set(false);
                let bind = self.bind_fn.borrow().clone();
                let model = self.model.borrow().clone();
                let visible = self.visible.borrow();
                for (&position, widget) in visible.iter() {
                    if let Some(ref model) = model {
                        if let Some(item) = model.item(position as u32) {
                            if let Some(game_item) = item.downcast_ref::<GameItem>() {
                                if let Some(game) = game_item.game() {
                                    if let Some(ref bind) = bind {
                                        bind(widget, game.as_ref());
                                    }
                                }
                            }
                        }
                    }
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
                    let x = self.last_edge.get() + col as i32 * (item_w + col_spacing);
                    let y = header_h + sp + row as i32 * row_h - actual_scroll;

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

            if let Some(post) = self.post_layout_fn.borrow().as_ref() {
                post();
            }
        }
    }
}

glib::wrapper! {
    pub struct VirtualGrid(ObjectSubclass<imp::VirtualGrid>)
        @extends gtk4::Widget,
        @implements gtk4::Scrollable, gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl VirtualGrid {
    pub fn new(min_item_width: i32) -> Self {
        let obj: Self = glib::Object::new();
        let imp = obj.imp();
        let min_h = ((min_item_width as f64) * ASPECT_RATIO) as i32;
        imp.min_item_width.set(min_item_width);
        imp.cur_item_w.set(min_item_width);
        imp.cur_item_h.set(min_h);
        imp.item_size.set((min_item_width, min_h));
        obj
    }

    pub fn compute_item_size(
        width: i32,
        height: i32,
        min_item_width: i32,
        aspect: f64,
    ) -> (i32, i32) {
        let (_, item_w, item_h, _) =
            compute_grid_layout(width, min_item_width, 12, height, aspect, 0);
        (item_w, item_h)
    }

    /// The grid's minimum edge/column spacing (`sp`) for a given cover width,
    /// derived the same way as `compute_grid_layout`. The grid pads columns
    /// wider to fill the viewport; the recent row mirrors this base spacing.
    pub(crate) fn grid_spacing_for_item_w(item_w: i32) -> i32 {
        let step_idx = STEP_SIZES
            .iter()
            .enumerate()
            .rev()
            .find(|(_, &w)| w <= item_w)
            .map(|(i, _)| i)
            .unwrap_or(0);
        8 + step_idx as i32 * 4
    }

    /// Square capsules (1:1) instead of 2:3 portraits.
    pub fn set_square(&self, on: bool) {
        if self.imp().square.get() != on {
            self.imp().square.set(on);
            self.queue_allocate();
        }
    }

    pub fn item_size_cell(&self) -> Rc<Cell<(i32, i32)>> {
        self.imp().item_size.clone()
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

        let obj = self.downgrade();
        let handler = model.connect_items_changed(move |_, _, removed, added| {
            if let Some(obj) = obj.upgrade() {
                let imp = obj.imp();
                let new_n = imp
                    .model
                    .borrow()
                    .as_ref()
                    .map(|m| m.n_items())
                    .unwrap_or(0);
                imp.n_items.set(new_n);
                if removed != added {
                    clear_visible(imp);
                } else {
                    imp.dirty.set(true);
                }
                obj.queue_allocate();
            }
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

    pub fn set_factory(&self, setup: SetupFn, bind: BindFn, unbind: UnbindFn) {
        let imp = self.imp();
        *imp.setup_fn.borrow_mut() = Some(setup);
        *imp.bind_fn.borrow_mut() = Some(bind);
        *imp.unbind_fn.borrow_mut() = Some(unbind);
    }

    pub fn set_size_changed(&self, cb: SizeChangedFn) {
        *self.imp().size_changed_fn.borrow_mut() = Some(cb);
    }

    /// A callback fired at the end of every allocation pass, after the
    /// visible cells are placed.
    pub fn set_post_layout_fn(&self, cb: Rc<dyn Fn()>) {
        *self.imp().post_layout_fn.borrow_mut() = Some(cb);
    }

    /// The widget currently displaying `index`, if it is visible.
    pub fn widget_for_index(&self, index: usize) -> Option<gtk4::Widget> {
        self.imp()
            .visible
            .borrow()
            .get(&index)
            .cloned()
    }

    /// Pin the column count (the couch grid pins 6 so every resolution
    /// shows the same layout); 0 keeps the width-driven step system.
    pub fn set_fixed_cols(&self, cols: u32) {
        if self.imp().fixed_cols.get() != cols {
            self.imp().fixed_cols.set(cols);
            self.queue_allocate();
        }
    }

    /// The layout applied by the last allocation: (columns, item width,
    /// item height, edge spacing). Callers driving an external selection
    /// (the big-picture grid) use it for move and scroll-to-index math.
    pub fn current_layout(&self) -> (u32, i32, i32, i32) {
        self.imp().last_layout.get()
    }

    /// A cell's origin in the grid's own content coordinates, or None when
    /// `index` is out of range. The floating All Software tooltip reads it
    /// to hover above the selected tile.
    pub fn cell_geometry(&self, index: usize) -> Option<(f64, f64)> {
        let (cols, item_w, item_h, sp) = self.imp().last_layout.get();
        if index >= self.imp().n_items.get() as usize {
            return None;
        }
        let cols = cols.max(1) as usize;
        let col = (index % cols) as f64;
        let row = (index / cols) as f64;
        let x = self.imp().last_edge.get() as f64
            + col * (item_w as f64 + self.imp().last_col_spacing.get() as f64);
        let y = sp as f64 + row * (item_h + sp) as f64;
        Some((x, y))
    }

    /// Re-run the bind closure on every visible cell. Cells reflect state
    /// they can't know on their own (an external selection highlight), so
    /// after that state changes the visible range is bound again.
    pub fn rebind_visible(&self) {
        self.imp().dirty.set(true);
        self.queue_allocate();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grid_spacing_for_item_w() {
        assert_eq!(VirtualGrid::grid_spacing_for_item_w(80), 8);
        assert_eq!(VirtualGrid::grid_spacing_for_item_w(110), 8);
        assert_eq!(VirtualGrid::grid_spacing_for_item_w(125), 8);
        assert_eq!(VirtualGrid::grid_spacing_for_item_w(150), 12);
        assert_eq!(VirtualGrid::grid_spacing_for_item_w(200), 16);
        assert_eq!(VirtualGrid::grid_spacing_for_item_w(250), 20);
        assert_eq!(VirtualGrid::grid_spacing_for_item_w(300), 24);
        assert_eq!(VirtualGrid::grid_spacing_for_item_w(350), 28);
        assert_eq!(VirtualGrid::grid_spacing_for_item_w(400), 28);
    }
}
