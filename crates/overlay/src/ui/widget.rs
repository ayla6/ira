use super::vertex::{DrawCmd, Vertex, MODE_SHAPE, MODE_TEXT, MODE_FILLED};

#[derive(Clone, Copy, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Default)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy)]
pub struct Padding {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Padding {
    pub const fn all(v: f32) -> Self {
        Self { left: v, top: v, right: v, bottom: v }
    }
    pub const fn horizontal(v: f32) -> Self {
        Self { left: v, top: 0.0, right: v, bottom: 0.0 }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Event {
    NavUp,
    NavDown,
    NavLeft,
    NavRight,
    Activate,
    MouseMove { x: f32, y: f32 },
    MouseDown { x: f32, y: f32 },
    MouseUp { x: f32, y: f32 },
}

pub struct LayoutCtx;

pub struct DrawCtx<'a> {
    pub vertices: &'a mut Vec<Vertex>,
    pub indices: &'a mut Vec<u32>,
    pub draw_cmds: &'a mut Vec<DrawCmd>,
    pub screen_w: f32,
    pub screen_h: f32,
    pub focused_index: Option<usize>,
}

pub struct EventCtx {
    pub focused_index: Option<usize>,
}

impl DrawCtx<'_> {
    pub fn push_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [u8; 4], corner_radius: f32) {
        let v_off = self.vertices.len() as i32;
        let i_off = self.indices.len() as u32;
        self.vertices.push(Vertex { pos: [x, y], uv: [0.0, 0.0], color });
        self.vertices.push(Vertex { pos: [x + w, y], uv: [1.0, 0.0], color });
        self.vertices.push(Vertex { pos: [x, y + h], uv: [0.0, 1.0], color });
        self.vertices.push(Vertex { pos: [x + w, y + h], uv: [1.0, 1.0], color });
        self.indices.extend_from_slice(&[0, 1, 2, 1, 3, 2]);
        self.draw_cmds.push(DrawCmd {
            index_count: 6, index_offset: i_off, vertex_offset: v_off,
            draw_mode: MODE_SHAPE, shape_size: [w, h], corner_radius,
            clip_x: 0.0, clip_y: 0.0, clip_w: self.screen_w, clip_h: self.screen_h,
        });
    }

    pub fn push_triangle(&mut self, p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], color: [u8; 4]) {
        let v_off = self.vertices.len() as i32;
        let i_off = self.indices.len() as u32;
        self.vertices.push(Vertex { pos: p0, uv: [0.0, 0.0], color });
        self.vertices.push(Vertex { pos: p1, uv: [0.0, 0.0], color });
        self.vertices.push(Vertex { pos: p2, uv: [0.0, 0.0], color });
        self.indices.extend_from_slice(&[0, 1, 2]);
        self.draw_cmds.push(DrawCmd {
            index_count: 3, index_offset: i_off, vertex_offset: v_off,
            draw_mode: MODE_FILLED, shape_size: [0.0, 0.0], corner_radius: 0.0,
            clip_x: 0.0, clip_y: 0.0, clip_w: self.screen_w, clip_h: self.screen_h,
        });
    }

    pub fn push_text(&mut self, text: &str, x: f32, y: f32, font_size: f32, color: [u8; 4]) {
        let v_off = self.vertices.len() as i32;
        let i_off = self.indices.len() as u32;
        let (tv, ti) = super::text::shape_text(text, x, y, font_size, color);
        let ti_count = ti.len() as u32;
        self.vertices.extend(tv);
        self.indices.extend(ti);
        if ti_count > 0 {
            self.draw_cmds.push(DrawCmd {
                index_count: ti_count, index_offset: i_off, vertex_offset: v_off,
                draw_mode: MODE_TEXT, shape_size: [0.0, 0.0], corner_radius: 0.0,
                clip_x: 0.0, clip_y: 0.0, clip_w: self.screen_w, clip_h: self.screen_h,
            });
        }
    }
}

pub trait Widget: Send {
    fn measure(&self, ctx: &LayoutCtx) -> Size;
    fn layout(&mut self, ctx: &LayoutCtx, bounds: Rect);
    fn draw(&self, ctx: &mut DrawCtx);
    fn handle_event(&mut self, ctx: &EventCtx, event: &Event) -> bool;
    fn collect_focusable(&mut self, list: &mut Vec<Rect>);
}
