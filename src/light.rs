#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightUniform {
    pub position: [f32; 3],
    pub _padding_1: u32,
    pub color: [f32; 3],
    pub _padding_2: u32,
}
