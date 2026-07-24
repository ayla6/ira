use super::widget::{DrawCtx, Event, EventCtx, LayoutCtx, Rect, Size, Widget};
use super::text;

pub struct Stepper {
    label: super::label::Label,
    options: Vec<String>,
    selected: usize,
    font_size: f32,
    bounds: Rect,
    focus_index: Option<usize>,
    on_change: Box<dyn Fn(usize) + Send + Sync>,
}

impl Stepper {
    pub fn new(
        label: &str,
        options: Vec<String>,
        font_size: f32,
        on_change: impl Fn(usize) + Send + Sync + 'static,
    ) -> Self {
        Self {
            label: super::label::Label::new(label, font_size, [180, 180, 180, 255]),
            options,
            selected: 0,
            font_size,
            bounds: Rect::default(),
            focus_index: None,
            on_change: Box::new(on_change),
        }
    }
}

impl Widget for Stepper {
    fn measure(&self, ctx: &LayoutCtx) -> Size {
        let label_size = self.label.measure(ctx);
        let max_value_w = self.options.iter()
            .map(|o| text::measure_text(o, self.font_size).width)
            .fold(0.0f32, f32::max);
        Size {
            width: label_size.width + max_value_w + 60.0,
            height: label_size.height.max(self.font_size + 8.0),
        }
    }

    fn layout(&mut self, ctx: &LayoutCtx, bounds: Rect) {
        self.bounds = bounds;
        let label_h = self.label.measure(ctx).height;
        self.label.layout(ctx, Rect {
            x: bounds.x + 8.0,
            y: bounds.y + (bounds.height - label_h) / 2.0,
            width: self.label.measure(ctx).width,
            height: label_h,
        });
    }

    fn draw(&self, ctx: &mut DrawCtx) {
        let focused = self.focus_index.is_some() && ctx.focused_index == self.focus_index;
        let bg = if focused { [60, 60, 90, 200] } else { [45, 45, 45, 180] };
        ctx.push_rect(
            self.bounds.x, self.bounds.y, self.bounds.width, self.bounds.height,
            bg, 4.0,
        );

        self.label.draw(ctx);

        let value = &self.options[self.selected];
        let display = if focused { format!("< {value} >") } else { value.clone() };
        let value_w = text::measure_text(&display, self.font_size).width;
        let value_x = self.bounds.x + self.bounds.width - value_w - 12.0;
        let value_y = self.bounds.y + (self.bounds.height - self.font_size) / 2.0;
        ctx.push_text(&display, value_x, value_y, self.font_size, [255, 255, 255, 255]);
    }

    fn handle_event(&mut self, ctx: &EventCtx, event: &Event) -> bool {
        let is_focused = self.focus_index.is_some() && ctx.focused_index == self.focus_index;
        if is_focused && *event == Event::Activate {
            self.selected = (self.selected + 1) % self.options.len();
            (self.on_change)(self.selected);
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
