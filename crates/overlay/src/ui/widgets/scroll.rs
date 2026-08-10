use super::super::widget::{DrawCtx, Event, EventCtx, LayoutCtx, Rect, Size, Widget};

/// A scrollable vertical container that clips children to a viewport.
/// Handles `Event::Scroll` to adjust the scroll offset.
pub struct ScrollView {
    children: Vec<Box<dyn Widget>>,
    viewport_height: f32,
    gap: f32,
    scroll_offset: f32,
    content_height: f32,
    bounds: Rect,
    /// (top_y, bottom_y) for each child, computed in `layout`.
    child_ys: Vec<(f32, f32)>,
}

impl ScrollView {
    pub fn new(children: Vec<Box<dyn Widget>>, viewport_height: f32) -> Self {
        Self {
            children,
            viewport_height,
            gap: 4.0,
            scroll_offset: 0.0,
            content_height: 0.0,
            bounds: Rect::default(),
            child_ys: Vec::new(),
        }
    }
}

impl Widget for ScrollView {
    fn measure(&self, _ctx: &LayoutCtx) -> Size {
        let mut max_w = 0.0f32;
        for child in &self.children {
            let s = child.measure(_ctx);
            max_w = max_w.max(s.width);
        }
        Size {
            width: max_w,
            height: self.viewport_height,
        }
    }

    fn layout(&mut self, ctx: &LayoutCtx, bounds: Rect) {
        self.bounds = bounds;

        let mut y = bounds.y - self.scroll_offset;
        let mut content_h = 0.0f32;
        self.child_ys.clear();
        self.child_ys.reserve(self.children.len());
        for child in &mut self.children {
            let size = child.measure(ctx);
            child.layout(
                ctx,
                Rect {
                    x: bounds.x,
                    y,
                    width: bounds.width,
                    height: size.height,
                },
            );
            self.child_ys.push((y, y + size.height));
            y += size.height + self.gap;
            content_h += size.height + self.gap;
        }
        self.content_height = content_h;

        let max_scroll = (self.content_height - self.viewport_height).max(0.0);
        self.scroll_offset = self.scroll_offset.clamp(0.0, max_scroll);
    }

    fn draw(&self, ctx: &mut DrawCtx) {
        let old = (ctx.clip_x, ctx.clip_y, ctx.clip_w, ctx.clip_h);
        ctx.clip_x = self.bounds.x.max(0.0);
        ctx.clip_y = self.bounds.y.max(0.0);
        ctx.clip_w = self.bounds.width;
        ctx.clip_h = self.bounds.height;

        let top = self.bounds.y;
        let bottom = self.bounds.y + self.bounds.height;

        for (i, child) in self.children.iter().enumerate() {
            if let Some(&(cy_top, cy_bottom)) = self.child_ys.get(i) {
                if cy_bottom < top || cy_top > bottom {
                    continue;
                }
            }
            child.draw(ctx);
        }

        ctx.clip_x = old.0;
        ctx.clip_y = old.1;
        ctx.clip_w = old.2;
        ctx.clip_h = old.3;
    }

    fn handle_event(&mut self, _ctx: &EventCtx, event: &Event) -> bool {
        if let Event::Scroll { delta_y } = event {
            self.scroll_offset += delta_y * 30.0;
            let max_scroll = (self.content_height - self.viewport_height).max(0.0);
            self.scroll_offset = self.scroll_offset.clamp(0.0, max_scroll);
            return true;
        }
        for child in &mut self.children {
            if child.handle_event(_ctx, event) {
                return true;
            }
        }
        false
    }

    fn collect_focusable(&mut self, list: &mut Vec<Rect>) {
        for child in &mut self.children {
            child.collect_focusable(list);
        }
    }
}
