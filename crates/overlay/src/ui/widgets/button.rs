use super::widget::{DrawCtx, Event, EventCtx, LayoutCtx, Padding, Rect, Size, Widget};

pub struct Button {
    label: super::label::Label,
    padding: Padding,
    bg_color: [u8; 4],
    bg_color_focused: [u8; 4],
    corner_radius: f32,
    bounds: Rect,
    focus_index: Option<usize>,
    on_activate: Box<dyn Fn() + Send + Sync>,
}

impl Button {
    pub fn new(
        text: impl Into<String>,
        font_size: f32,
        on_activate: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self {
            label: super::label::Label::new(text, font_size, [255, 255, 255, 255]),
            padding: Padding::horizontal(8.0),
            bg_color: [60, 60, 60, 200],
            bg_color_focused: [90, 90, 140, 220],
            corner_radius: 4.0,
            bounds: Rect::default(),
            focus_index: None,
            on_activate: Box::new(on_activate),
        }
    }
}

impl Widget for Button {
    fn measure(&self, ctx: &LayoutCtx) -> Size {
        let inner = self.label.measure(ctx);
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
        self.label.layout(ctx, inner);
    }

    fn draw(&self, ctx: &mut DrawCtx) {
        let focused = self.focus_index.is_some() && ctx.focused_index == self.focus_index;
        let color = if focused {
            self.bg_color_focused
        } else {
            self.bg_color
        };
        ctx.push_rect(
            self.bounds.x,
            self.bounds.y,
            self.bounds.width,
            self.bounds.height,
            color,
            self.corner_radius,
        );
        self.label.draw(ctx);
    }

    fn handle_event(&mut self, ctx: &EventCtx, event: &Event) -> bool {
        let is_focused = self.focus_index.is_some() && ctx.focused_index == self.focus_index;
        if is_focused && *event == Event::Activate {
            (self.on_activate)();
            true
        } else {
            false
        }
    }

    fn collect_focusable(&mut self, list: &mut Vec<Rect>) {
        self.focus_index = Some(list.len());
        list.push(self.bounds);
    }
}
