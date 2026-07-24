use super::widget::{DrawCtx, Event, EventCtx, LayoutCtx, Rect, Size, Widget};

pub struct Row {
    children: Vec<Box<dyn Widget>>,
    spacing: f32,
    bounds: Rect,
}

impl Row {
    pub fn new(spacing: f32, children: Vec<Box<dyn Widget>>) -> Self {
        Self { children, spacing, bounds: Rect::default() }
    }
}

impl Widget for Row {
    fn measure(&self, ctx: &LayoutCtx) -> Size {
        let mut width = 0.0f32;
        let mut height = 0.0f32;
        let mut first = true;
        for child in &self.children {
            let s = child.measure(ctx);
            height = height.max(s.height);
            if !first { width += self.spacing; }
            width += s.width;
            first = false;
        }
        Size { width, height }
    }

    fn layout(&mut self, ctx: &LayoutCtx, bounds: Rect) {
        self.bounds = bounds;
        let mut x = bounds.x;
        for child in &mut self.children {
            let s = child.measure(ctx);
            child.layout(ctx, Rect { x, y: bounds.y, width: s.width, height: bounds.height });
            x += s.width + self.spacing;
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
