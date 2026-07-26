use super::super::widget::{DrawCtx, Event, EventCtx, LayoutCtx, Rect, Size, Widget};

/// A horizontal progress bar showing 0%–100% completion.
pub struct ProgressBar {
    progress: f32,
    width: f32,
    height: f32,
    bg_color: [u8; 4],
    fill_color: [u8; 4],
    bounds: Rect,
}

impl ProgressBar {
    pub fn new(progress: f32, width: f32) -> Self {
        Self {
            progress: progress.clamp(0.0, 1.0),
            width,
            height: 6.0,
            bg_color: [50, 50, 50, 200],
            fill_color: [80, 180, 80, 255],
            bounds: Rect::default(),
        }
    }
}

impl Widget for ProgressBar {
    fn measure(&self, _ctx: &LayoutCtx) -> Size {
        Size { width: self.width, height: self.height }
    }

    fn layout(&mut self, _ctx: &LayoutCtx, bounds: Rect) {
        self.bounds = bounds;
    }

    fn draw(&self, ctx: &mut DrawCtx) {
        let x = self.bounds.x;
        let y = self.bounds.y;
        let w = self.bounds.width.max(0.0);
        let h = self.bounds.height.max(0.0);

        ctx.push_rect(x, y, w, h, self.bg_color, h * 0.5);

        let fill_w = w * self.progress;
        if fill_w > 0.5 {
            ctx.push_rect(x, y, fill_w, h, self.fill_color, h * 0.5);
        }
    }

    fn handle_event(&mut self, _ctx: &EventCtx, _event: &Event) -> bool {
        false
    }

    fn collect_focusable(&mut self, _list: &mut Vec<Rect>) {}
}
