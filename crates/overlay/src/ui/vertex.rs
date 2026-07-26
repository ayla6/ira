#[repr(C)]
#[derive(Clone, Copy)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [u8; 4],
}

const _: () = assert!(std::mem::size_of::<Vertex>() == 20);

#[repr(C)]
pub struct PushConstants {
    pub screen_size: [f32; 2],
    pub shape_size: [f32; 2],
    pub corner_radius: f32,
    pub is_shape: u32,
}

const _: () = assert!(std::mem::size_of::<PushConstants>() == 24);

pub const MODE_TEXT: u32 = 0;
pub const MODE_SHAPE: u32 = 1;

pub struct DrawCmd {
    pub index_count: u32,
    pub index_offset: u32,
    pub vertex_offset: i32,
    pub draw_mode: u32,
    pub shape_size: [f32; 2],
    pub corner_radius: f32,
    pub clip_x: f32,
    pub clip_y: f32,
    pub clip_w: f32,
    pub clip_h: f32,
}
