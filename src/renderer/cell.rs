//! Cell instance data for instanced rendering.

use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct CellInstance {
    pub position: [f32; 2],
    pub size: [f32; 2],
    pub fg_color: [f32; 4],
    pub bg_color: [f32; 4],
    pub uv_offset: [f32; 2],
    pub uv_size: [f32; 2],
    pub is_color: f32,
    pub _padding: [f32; 3], // padding to maintain 16-byte alignment (size becomes 80 bytes)
}

impl CellInstance {
    pub fn new(
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        fg: [f32; 4],
        bg: [f32; 4],
        uv_x: f32,
        uv_y: f32,
        uv_w: f32,
        uv_h: f32,
        is_color: f32,
    ) -> Self {
        Self {
            position: [x, y],
            size: [width, height],
            fg_color: fg,
            bg_color: bg,
            uv_offset: [uv_x, uv_y],
            uv_size: [uv_w, uv_h],
            is_color,
            _padding: [0.0; 3],
        }
    }
}
