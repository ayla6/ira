use super::widget::{DrawCtx, Event, EventCtx, LayoutCtx, Padding, Rect, Size, Widget};

pub struct Panel {
    child: Box<dyn Widget>,
    padding: Padding,
    bg_color: [u8; 4],
    corner_radius: f32,
    bounds: Rect,
}

impl Panel {
    pub fn new(
        padding: Padding,
        bg_color: [u8; 4],
        corner_radius: f32,
        child: Box<dyn Widget>,
    ) -> Self {
        Self { child, padding, bg_color, corner_radius, bounds: Rect::default() }
    }
}

impl Widget for Panel {
    fn measure(&self, ctx: &LayoutCtx) -> Size {
        let inner = self.child.measure(ctx);
        Size {
            width: inner.width + self.padding.left + self.padding.right,
            height: inner.height + self.padding.top + self.padding.bottom,
        }
    }

    fn layout(&mut self, ctx: &LayoutCtx, bounds: Rect) {
        self.bounds = bounds;
        let inner = Rect {
            x: bounds.x + self.padding.left,
            y: bounds.y + self.padding.top,
            width: bounds.width - self.padding.left - self.padding.right,
            height: bounds.height - self.padding.top - self.padding.bottom,
        };
        self.child.layout(ctx, inner);
    }

    fn draw(&self, ctx: &mut DrawCtx) {
        ctx.push_rect(
            self.bounds.x, self.bounds.y, self.bounds.width, self.bounds.height,
            self.bg_color, self.corner_radius,
        );
        self.child.draw(ctx);
    }

    fn handle_event(&mut self, ctx: &EventCtx, event: &Event) -> bool {
        self.child.handle_event(ctx, event)
    }

    fn collect_focusable(&mut self, list: &mut Vec<Rect>) {
        self.child.collect_focusable(list);
    }
}
