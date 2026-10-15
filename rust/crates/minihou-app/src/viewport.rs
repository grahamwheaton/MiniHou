use eframe::{egui, egui_wgpu::{self, wgpu}};
use wgpu::util::DeviceExt;
use bytemuck::{Pod, Zeroable};
use minihou_core::{Geometry, normals, preview};
use minihou_app::Camera;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode { Solid, Shaded, Wire, ShadedWire }

/// Overlay switches the renderer draws itself (the grid, axes and stats are painted by the UI).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Overlay { pub normals: bool, pub bounds: bool, pub xray: bool }

pub struct Scene {
    p: Vec<f32>, n: Vec<f32>, tri: Vec<u32>, edge: Vec<u32>,
    /// Normal overlay: segment end points (positions) and the matching normals; two points per segment.
    nlp: Vec<f32>, nln: Vec<f32>,
    /// The eight corners of the bounding box.
    bbp: [f32; 24],
    pub center: [f32;3], pub radius: f32,
}
/// Longest overlay normal list kept: more points than this are thinned evenly.
const NORMAL_LINES: usize = 60_000;
/// Corner order of `Scene::bbp`; the 12 edges as index pairs.
const BOX_EDGES: [u32; 24] = [0,1, 1,3, 3,2, 2,0, 4,5, 5,7, 7,6, 6,4, 0,4, 1,5, 2,6, 3,7];
impl Scene {
    pub fn new(g: &Geometry) -> Self {
        let (p,t) = (g.p(), &g.topology);
        let l = preview::layout(p, &t.vertex_point, &t.prim_offsets, t.closed.as_deref(), g.npoints, usize::MAX, usize::MAX, 100_000,8192);
        let n = match g.point.get("N").and_then(|a| a.as_f32()) {
            Some(n) if n.len() == g.npoints * 3 => n.to_vec(),
            _ => normals::point_normals(p,t,g.npoints),
        };
        let (center,radius) = if g.npoints == 0 { ([0.;3],1.) } else {
            let (lo,hi) = preview::bounds(p);
            ([0,1,2].map(|k| (lo[k]+hi[k])*0.5), (0..3).map(|k| (hi[k]-lo[k]).powi(2)).sum::<f32>().sqrt()*0.5)
        };
        let radius = radius.max(1e-3);
        let step = (g.npoints / NORMAL_LINES).max(1);
        let len = radius * 0.04;
        let (mut nlp, mut nln) = (Vec::new(), Vec::new());
        for i in (0..g.npoints).step_by(step) {
            let (pp, nn) = (&p[i*3..i*3+3], &n[i*3..i*3+3]);
            nlp.extend_from_slice(pp);
            nlp.extend_from_slice(&[pp[0] + nn[0]*len, pp[1] + nn[1]*len, pp[2] + nn[2]*len]);
            nln.extend_from_slice(nn); nln.extend_from_slice(nn);
        }
        let mut bbp = [0f32; 24];
        if g.npoints > 0 {
            let (lo, hi) = preview::bounds(p);
            for c in 0..8 { for k in 0..3 { bbp[c*3+k] = if (c >> k) & 1 == 1 { hi[k] } else { lo[k] }; } }
        }
        Self { p:p.to_vec(),n,tri:l.tri,edge:l.edge,nlp,nln,bbp,center,radius }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    base: [f32; 4],
    wire: [f32; 4],
    flat_shading: u32,
    wire_fade: f32,
    _pad: [u32; 2],
}


const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

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


pub struct Gpu {
    pipe_surf: wgpu::RenderPipeline, pipe_surf_bias: wgpu::RenderPipeline, pipe_wire: wgpu::RenderPipeline,
    pipe_xray: wgpu::RenderPipeline, pipe_normals: wgpu::RenderPipeline, pipe_bounds: wgpu::RenderPipeline,
    nl_p: wgpu::Buffer, nl_n: wgpu::Buffer, nl_count: u32, bb_p: wgpu::Buffer, bb_n: wgpu::Buffer, bb_i: wgpu::Buffer,
    bind: wgpu::BindGroup, ubuf: wgpu::Buffer,
    vp: wgpu::Buffer, vn: wgpu::Buffer, ib: wgpu::Buffer, eb: wgpu::Buffer,
    ntri_idx: u32, nedge_idx: u32,
    /// Average edge length in world units: sizes the edge fade.
    edge_world: f32,
}

fn edge_world_of(sc: &Scene) -> f32 {
    (2.0 * sc.radius) / ((sc.edge.len().max(2) as f32) / 2.0).sqrt()
}
impl Gpu {
    pub fn new(device: &wgpu::Device, sc: &Scene) -> Self {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let buf = |label, data: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: if data.is_empty() { &[0;4] } else { data }, usage })
        };
        let vp = buf("P", bytemuck::cast_slice(&sc.p), wgpu::BufferUsages::VERTEX);
        let vn = buf("N", bytemuck::cast_slice(&sc.n), wgpu::BufferUsages::VERTEX);
        let ib = buf("tri", bytemuck::cast_slice(&sc.tri), wgpu::BufferUsages::INDEX);
        let eb = buf("edge", bytemuck::cast_slice(&sc.edge), wgpu::BufferUsages::INDEX);
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("u"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        const POS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x3];
        const NRM: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![1 => Float32x3];
        let vb = |a: &'static [wgpu::VertexAttribute]| wgpu::VertexBufferLayout {
            array_stride: 12,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: a,
        };
        let surf_bufs = [vb(&POS), vb(&NRM)];
        let wire_bufs = [vb(&POS), vb(&NRM)];
        let target = [Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })];
        let mk = |label, vs, fs, bufs: &[wgpu::VertexBufferLayout], topo, bias: i32, blend: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), compilation_options: Default::default(), buffers: bufs },
                fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(fs), compilation_options: Default::default(), targets: &if blend { [Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })] } else { target.clone() } }),
                primitive: wgpu::PrimitiveState { topology: topo, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: !blend,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState { constant: bias, slope_scale: if bias != 0 { 1.0 } else { 0.0 }, clamp: 0.0 },
                }),
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let pipe_surf = mk("surf", "vs_surf", "fs_surf", &surf_bufs, wgpu::PrimitiveTopology::TriangleList, 0, false);
        let pipe_surf_bias = mk("surf+bias", "vs_surf", "fs_surf", &surf_bufs, wgpu::PrimitiveTopology::TriangleList, 2, false);
        let pipe_wire = mk("wire", "vs_wire", "fs_wire", &wire_bufs, wgpu::PrimitiveTopology::LineList, 0, false);
        let pipe_xray = mk("xray", "vs_surf", "fs_xray", &surf_bufs, wgpu::PrimitiveTopology::TriangleList, 0, true);
        let pipe_normals = mk("normals", "vs_wire", "fs_normals", &wire_bufs, wgpu::PrimitiveTopology::LineList, 0, false);
        let pipe_bounds = mk("bounds", "vs_wire", "fs_bounds", &wire_bufs, wgpu::PrimitiveTopology::LineList, 0, false);
        let nl_p = buf("nl_p", bytemuck::cast_slice(&sc.nlp), wgpu::BufferUsages::VERTEX);
        let nl_n = buf("nl_n", bytemuck::cast_slice(&sc.nln), wgpu::BufferUsages::VERTEX);
        let bb_p = buf("bb_p", bytemuck::cast_slice(&sc.bbp), wgpu::BufferUsages::VERTEX);
        let bb_n = buf("bb_n", bytemuck::cast_slice(&[0f32; 24]), wgpu::BufferUsages::VERTEX);
        let bb_i = buf("bb_i", bytemuck::cast_slice(&BOX_EDGES), wgpu::BufferUsages::INDEX);

        Self { pipe_surf,pipe_surf_bias,pipe_wire,pipe_xray,pipe_normals,pipe_bounds,nl_p,nl_n,nl_count:(sc.nlp.len()/3) as u32,bb_p,bb_n,bb_i,bind,ubuf,vp,vn,ib,eb,ntri_idx:sc.tri.len() as u32,nedge_idx:sc.edge.len() as u32,edge_world:edge_world_of(sc) }
    }
    pub fn upload(&mut self, device: &wgpu::Device, sc: &Scene) {
        let buf = |label, data: &[u8], usage| device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some(label), contents:if data.is_empty() { &[0;4] } else { data }, usage
        });
        self.vp=buf("P",bytemuck::cast_slice(&sc.p),wgpu::BufferUsages::VERTEX);
        self.vn=buf("N",bytemuck::cast_slice(&sc.n),wgpu::BufferUsages::VERTEX);
        self.ib=buf("tri",bytemuck::cast_slice(&sc.tri),wgpu::BufferUsages::INDEX);
        self.eb=buf("edge",bytemuck::cast_slice(&sc.edge),wgpu::BufferUsages::INDEX);
        self.ntri_idx=sc.tri.len() as u32;
        self.nedge_idx=sc.edge.len() as u32;
        self.edge_world=edge_world_of(sc);
        self.nl_p=buf("nl_p",bytemuck::cast_slice(&sc.nlp),wgpu::BufferUsages::VERTEX);
        self.nl_n=buf("nl_n",bytemuck::cast_slice(&sc.nln),wgpu::BufferUsages::VERTEX);
        self.nl_count=(sc.nlp.len()/3) as u32;
        self.bb_p=buf("bb_p",bytemuck::cast_slice(&sc.bbp),wgpu::BufferUsages::VERTEX);
    }
    pub fn draw(&self, state: &egui_wgpu::RenderState, target: &Target, cam: &Camera, mode: Mode, overlay: Overlay) {
        let (view_proj,view) = cam.matrices(target.size[0] as f32/target.size[1] as f32);
        // 0 when the average edge is under about 1.5 px on screen, 1 from about 10 px (smoothstep): a dense mesh reads as shading
        let px_per_unit = target.size[1] as f32 / (2.0 * (0.8f32 / 2.0).tan() * cam.distance().max(1e-6));
        let t = ((self.edge_world * px_per_unit - 1.5) / 8.5).clamp(0.0, 1.0);
        let fade = t * t * (3.0 - 2.0 * t);
        // wire.a selects the line look in the shader: 1 = a darker version of the surface (Shaded+Wire), 0 = lit lines (Wire)
        let u = Uniforms { view_proj,view,base:[0.72,0.74,0.78,1.],wire:[0.,0.,0.,if mode==Mode::ShadedWire {1.} else {0.}],flat_shading:(mode==Mode::Solid) as u32,wire_fade:fade,_pad:[1,0] };
        state.queue.write_buffer(&self.ubuf,0,bytemuck::bytes_of(&u));
        let mut enc = state.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label:Some("MiniHou viewport"),
                color_attachments:&[Some(wgpu::RenderPassColorAttachment {
                    view:&target.color,depth_slice:None,resolve_target:None,
                    ops:wgpu::Operations { load:wgpu::LoadOp::Clear(wgpu::Color { r:0.244,g:0.256,b:0.278,a:1. }),store:wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment {
                    view:&target.depth,depth_ops:Some(wgpu::Operations { load:wgpu::LoadOp::Clear(1.),store:wgpu::StoreOp::Discard }),stencil_ops:None,
                }),
                timestamp_writes:None,occlusion_query_set:None,
            });
            pass.set_bind_group(0,&self.bind,&[]);
            if mode != Mode::Wire && self.ntri_idx > 0 {
                pass.set_pipeline(if overlay.xray { &self.pipe_xray } else if mode==Mode::ShadedWire { &self.pipe_surf_bias } else { &self.pipe_surf });
                pass.set_vertex_buffer(0,self.vp.slice(..)); pass.set_vertex_buffer(1,self.vn.slice(..));
                pass.set_index_buffer(self.ib.slice(..),wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.ntri_idx,0,0..1);
            }
            if (mode==Mode::Wire || mode==Mode::ShadedWire || overlay.xray) && self.nedge_idx > 0 {
                pass.set_pipeline(&self.pipe_wire); pass.set_vertex_buffer(0,self.vp.slice(..)); pass.set_vertex_buffer(1,self.vn.slice(..));
                pass.set_index_buffer(self.eb.slice(..),wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.nedge_idx,0,0..1);
            }
            if overlay.normals && self.nl_count > 0 {
                pass.set_pipeline(&self.pipe_normals);
                pass.set_vertex_buffer(0,self.nl_p.slice(..)); pass.set_vertex_buffer(1,self.nl_n.slice(..));
                pass.draw(0..self.nl_count,0..1);
            }
            if overlay.bounds {
                pass.set_pipeline(&self.pipe_bounds);
                pass.set_vertex_buffer(0,self.bb_p.slice(..)); pass.set_vertex_buffer(1,self.bb_n.slice(..));
                pass.set_index_buffer(self.bb_i.slice(..),wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..24,0,0..1);
            }
        }
        state.queue.submit([enc.finish()]);
    }
}
pub struct Target {
    color: wgpu::TextureView, depth: wgpu::TextureView,
    pub id: egui::TextureId, pub size: [u32;2],
}
impl Target {
    pub fn new(state: &egui_wgpu::RenderState, size: [u32;2]) -> Self {
        let color = state.device.create_texture(&wgpu::TextureDescriptor {
            label:Some("MiniHou color"),size:wgpu::Extent3d { width:size[0],height:size[1],depth_or_array_layers:1 },
            mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8Unorm,
            usage:wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[],
        }).create_view(&Default::default());
        let depth = make_depth(&state.device,size[0],size[1]);
        let id = state.renderer.write().register_native_texture(&state.device,&color,wgpu::FilterMode::Linear);
        Self { color,depth,id,size }
    }
}

