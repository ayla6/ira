use super::widget::{DrawCtx, Event, EventCtx, LayoutCtx, Rect, Size, Widget};
use super::text;

pub struct Label {
    text: String,
    font_size: f32,
    color: [u8; 4],
    bounds: Rect,
    measured: Size,
}

impl Label {
    pub fn new(text: impl Into<String>, font_size: f32, color: [u8; 4]) -> Self {
        let text_str = text.into();
        let measured = text::measure_text(&text_str, font_size);
        Self { text: text_str, font_size, color, bounds: Rect::default(), measured }
    }
}

impl Widget for Label {
    fn measure(&self, _ctx: &LayoutCtx) -> Size {
        self.measured
    }

    fn layout(&mut self, _ctx: &LayoutCtx, bounds: Rect) {
        self.bounds = bounds;
    }

    fn draw(&self, ctx: &mut DrawCtx) {
        ctx.push_text(&self.text, self.bounds.x, self.bounds.y, self.font_size, self.color);
    }

    fn handle_event(&mut self, _ctx: &EventCtx, _event: &Event) -> bool {
        false
    }

    fn collect_focusable(&mut self, _list: &mut Vec<Rect>) {}
}
