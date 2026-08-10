use super::widget::{DrawCtx, Event, EventCtx, LayoutCtx, Rect, Size, Widget};

pub struct Column {
    children: Vec<Box<dyn Widget>>,
    spacing: f32,
    bounds: Rect,
}

impl Column {
    pub fn new(spacing: f32, children: Vec<Box<dyn Widget>>) -> Self {
        Self {
            children,
            spacing,
            bounds: Rect::default(),
        }
    }
}

impl Widget for Column {
    fn measure(&self, ctx: &LayoutCtx) -> Size {
        let mut width = 0.0f32;
        let mut height = 0.0f32;
        let mut first = true;
        for child in &self.children {
            let s = child.measure(ctx);
            width = width.max(s.width);
            if !first {
                height += self.spacing;
            }
            height += s.height;
            first = false;
        }
        Size { width, height }
    }

    fn layout(&mut self, ctx: &LayoutCtx, bounds: Rect) {
        self.bounds = bounds;
        let mut y = bounds.y;
        for child in &mut self.children {
            let s = child.measure(ctx);
            let child_bounds = Rect {
                x: bounds.x,
                y,
                width: bounds.width,
                height: s.height,
            };
            child.layout(ctx, child_bounds);
            y += s.height + self.spacing;
        }
    }

    fn draw(&self, ctx: &mut DrawCtx) {
        for child in &self.children {
            child.draw(ctx);
        }
    }

    fn handle_event(&mut self, ctx: &EventCtx, event: &Event) -> bool {
        for child in &mut self.children {
            if child.handle_event(ctx, event) {
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
