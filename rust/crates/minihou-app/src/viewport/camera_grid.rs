use super::*;

pub(super) fn inverse_projection(m:[[f32;4];4])->[[f32;4];4] {
    let mut rows=[[0f32;8];4];
    for r in 0..4 {for c in 0..4 {rows[r][c]=m[c][r];}rows[r][r+4]=1.;}
    for c in 0..4 {
        let pivot=(c..4).max_by(|&a,&b|rows[a][c].abs().total_cmp(&rows[b][c].abs())).unwrap();
        rows.swap(c,pivot);
        let scale=rows[c][c];
        for j in 0..8 {rows[c][j]/=scale;}
        for r in 0..4 {if r!=c {let f=rows[r][c];for j in 0..8 {rows[r][j]-=f*rows[c][j];}}}
    }
    std::array::from_fn(|c|std::array::from_fn(|r|rows[r][c+4]))
}


pub(super) const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

fn make_depth(device: &wgpu::Device, w: u32, h: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default())
}


pub struct Target {
    pub color: wgpu::TextureView,
    pub depth: wgpu::TextureView,
    pub mask: wgpu::TextureView,
    pub id: egui::TextureId,
    pub size: [u32; 2],
}
impl Target {
    pub fn new(state: &egui_wgpu::RenderState, size: [u32;2]) -> Self {
        let color = state.device.create_texture(&wgpu::TextureDescriptor {
            label:Some("MiniHou color"),size:wgpu::Extent3d { width:size[0].max(1),height:size[1].max(1),depth_or_array_layers:1 },
            mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8Unorm,
            usage:wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,view_formats:&[],
        }).create_view(&Default::default());
        let depth = make_depth(&state.device,size[0],size[1]);
        let mask = state.device.create_texture(&wgpu::TextureDescriptor {
            label:Some("MiniHou mask"),size:wgpu::Extent3d { width:size[0].max(1),height:size[1].max(1),depth_or_array_layers:1 },
            mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::R8Unorm,
            usage:wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[],
        }).create_view(&Default::default());
        let id = state.renderer.write().register_native_texture(&state.device,&color,wgpu::FilterMode::Linear);
        Self { color,depth,mask,id,size }
    }

    #[cfg(test)]
    pub fn new_headless(device: &wgpu::Device, size: [u32;2]) -> (Self, wgpu::Texture) {
        let color_tex = device.create_texture(&wgpu::TextureDescriptor {
            label:Some("MiniHou color headless"),size:wgpu::Extent3d { width:size[0].max(1),height:size[1].max(1),depth_or_array_layers:1 },
            mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8Unorm,
            usage:wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,view_formats:&[],
        });
        let color = color_tex.create_view(&Default::default());
        let depth = make_depth(device,size[0],size[1]);
        let mask = device.create_texture(&wgpu::TextureDescriptor {
            label:Some("MiniHou mask headless"),size:wgpu::Extent3d { width:size[0].max(1),height:size[1].max(1),depth_or_array_layers:1 },
            mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::R8Unorm,
            usage:wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[],
        }).create_view(&Default::default());
        (Self { color,depth,mask,id:egui::TextureId::User(0),size }, color_tex)
    }
}

