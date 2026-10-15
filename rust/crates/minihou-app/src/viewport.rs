use eframe::{egui, egui_wgpu::{self, wgpu}};
use wgpu::util::DeviceExt;
use bytemuck::{Pod, Zeroable};
use minihou_core::{Geometry, normals, preview};
use minihou_core::lod::{ClusterLod, ClusterId};
use minihou_app::Camera;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode { Solid, Shaded, Wire, ShadedWire }

/// Overlay switches the renderer draws itself (the grid, axes and stats are painted by the UI).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Overlay { pub normals: bool, pub bounds: bool, pub xray: bool, pub wireframe: bool }

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
    /// The same scene showing only `level` (0..1) of its triangles and edges (the viewport proxy); points, normals and bounds stay.
    pub fn proxied(&self, level: f32) -> Scene {
        Scene {
            p: self.p.clone(), n: self.n.clone(),
            tri: crate::ui::proxy_ui::proxy::reduced_triangles(&self.tri, level),
            edge: crate::ui::proxy_ui::proxy::reduced_edges(&self.edge, level),
            nlp: self.nlp.clone(), nln: self.nln.clone(), bbp: self.bbp, center: self.center, radius: self.radius,
        }
    }
    pub fn triangles(&self) -> usize { self.tri.len() / 3 }
    pub fn positions(&self) -> &[f32] { &self.p }
    pub fn triangle_indices(&self) -> &[u32] { &self.tri }
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
    pipe_xray: wgpu::RenderPipeline, pipe_normals: wgpu::RenderPipeline, pipe_bounds: wgpu::RenderPipeline, pipe_select: wgpu::RenderPipeline,
    pipe_mask: wgpu::RenderPipeline, pipe_outline: wgpu::RenderPipeline, bgl_outline: wgpu::BindGroupLayout,
    /// Selection highlight: face triangles and line segments (positions, with a zero normal buffer of the same length).
    sel_face: wgpu::Buffer, sel_face_n: u32, sel_line: wgpu::Buffer, sel_line_n: u32, zeros: wgpu::Buffer,
    nl_p: wgpu::Buffer, nl_n: wgpu::Buffer, nl_count: u32, bb_p: wgpu::Buffer, bb_n: wgpu::Buffer, bb_i: wgpu::Buffer,
    bind: wgpu::BindGroup, ubuf: wgpu::Buffer,
    vp: wgpu::Buffer, vn: wgpu::Buffer, ib: wgpu::Buffer, eb: wgpu::Buffer,
    ntri_idx: u32, nedge_idx: u32,
    lod_active: bool, selected_ranges: Vec<std::ops::Range<u32>>,
    /// Average edge length in world units: sizes the edge fade.
    edge_world: f32,
}

fn edge_world_of(sc: &Scene) -> f32 {
    (2.0 * sc.radius) / ((sc.edge.len().max(2) as f32) / 2.0).sqrt()
}
impl Gpu {
    pub fn new_lod(device: &wgpu::Device, sc: &Scene) -> Self {
        let mut gpu=Self::new(device,&sc.proxied(0.0));
        gpu.lod_active=true;
        gpu
    }
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
        let pipe_select = mk("select", "vs_surf", "fs_select", &surf_bufs, wgpu::PrimitiveTopology::TriangleList, 2, true);

        let pipe_mask = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mask"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_surf"),
                compilation_options: Default::default(),
                buffers: &surf_bufs,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_mask"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::R8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let bgl_outline = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bgl_outline"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let layout_outline = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("layout_outline"),
            bind_group_layouts: &[&bgl_outline],
            push_constant_ranges: &[],
        });
        let shader_outline = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shader_outline"),
            source: wgpu::ShaderSource::Wgsl(include_str!("outline.wgsl").into()),
        });
        let pipe_outline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("outline"),
            layout: Some(&layout_outline),
            vertex: wgpu::VertexState {
                module: &shader_outline,
                entry_point: Some("vs_outline"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader_outline,
                entry_point: Some("fs_outline"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let sel_face = buf("sel_face", bytemuck::cast_slice(&[0f32; 3]), wgpu::BufferUsages::VERTEX);
        let sel_line = buf("sel_line", bytemuck::cast_slice(&[0f32; 3]), wgpu::BufferUsages::VERTEX);
        let zeros = buf("zeros", bytemuck::cast_slice(&vec![0f32; 3 * 6 * 4096]), wgpu::BufferUsages::VERTEX);
        let nl_p = buf("nl_p", bytemuck::cast_slice(&sc.nlp), wgpu::BufferUsages::VERTEX);
        let nl_n = buf("nl_n", bytemuck::cast_slice(&sc.nln), wgpu::BufferUsages::VERTEX);
        let bb_p = buf("bb_p", bytemuck::cast_slice(&sc.bbp), wgpu::BufferUsages::VERTEX);
        let bb_n = buf("bb_n", bytemuck::cast_slice(&[0f32; 24]), wgpu::BufferUsages::VERTEX);
        let bb_i = buf("bb_i", bytemuck::cast_slice(&BOX_EDGES), wgpu::BufferUsages::INDEX);

        Self {
            pipe_surf,
            pipe_surf_bias,
            pipe_wire,
            pipe_xray,
            pipe_normals,
            pipe_bounds,
            pipe_select,
            pipe_mask,
            pipe_outline,
            bgl_outline,
            sel_face,
            sel_face_n: 0,
            sel_line,
            sel_line_n: 0,
            zeros,
            nl_p,
            nl_n,
            nl_count: (sc.nlp.len() / 3) as u32,
            bb_p,
            bb_n,
            bb_i,
            bind,
            ubuf,
            vp,
            vn,
            ib,
            eb,
            ntri_idx: sc.tri.len() as u32,
            nedge_idx: sc.edge.len() as u32,
            lod_active: false,
            selected_ranges: Vec::new(),
            edge_world: edge_world_of(sc),
        }
    }
    pub fn upload(&mut self, device: &wgpu::Device, sc: &Scene) {
        self.lod_active=false; self.selected_ranges.clear();
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
    /// Retain the source vertex attributes but start with no triangles; selection uploads a cut.
    pub fn upload_lod(&mut self, device: &wgpu::Device, sc: &Scene) {
        self.upload(device,&sc.proxied(0.0));
        self.lod_active=true;
    }
    pub fn select_lod(&mut self, device: &wgpu::Device, lod: &ClusterLod, ids: &[ClusterId], wire: bool) {
        let (indices, ranges)=packed_cut(lod, ids);
        self.selected_ranges=ranges;
        self.ib=device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label:Some("selected LOD triangles"), contents:if indices.is_empty() { &[0u8;4] } else { bytemuck::cast_slice(&indices) }, usage:wgpu::BufferUsages::INDEX });
        self.ntri_idx=indices.len() as u32;
        if wire {
            let mut edges = std::collections::BTreeSet::new();
            for &id in ids { for tri in &lod.clusters[id as usize].triangles {
                for (a,b) in [(tri[0],tri[1]),(tri[1],tri[2]),(tri[2],tri[0])] { edges.insert((a.min(b),a.max(b))); }
            } }
            let mut indices=Vec::with_capacity(edges.len()*2);
            for (a,b) in edges { indices.extend_from_slice(&[a,b]); }
            self.eb=device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label:Some("LOD edges"), contents:if indices.is_empty() { &[0u8;4] } else { bytemuck::cast_slice(&indices) }, usage:wgpu::BufferUsages::INDEX });
            self.nedge_idx=indices.len() as u32;
        } else { self.nedge_idx=0; }
    }
    /// Show the selection: `faces` = 3 positions per triangle, `lines` = 2 positions per segment (flat xyz).
    pub fn set_selection(&mut self, device: &wgpu::Device, faces: &[f32], lines: &[f32]) {
        let buf = |label, data: &[f32]| device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label), contents: if data.is_empty() { &[0u8; 12] } else { bytemuck::cast_slice(data) }, usage: wgpu::BufferUsages::VERTEX });
        // the normal buffer must cover every vertex: grow the zero buffer when a selection is larger than it
        let need = faces.len().max(lines.len()) * 4;
        if need as u64 > self.zeros.size() { self.zeros = buf("zeros", &vec![0f32; faces.len().max(lines.len()).next_power_of_two()]); }
        self.sel_face = buf("sel_face", faces);
        self.sel_face_n = (faces.len() / 3) as u32;
        self.sel_line = buf("sel_line", lines);
        self.sel_line_n = (lines.len() / 3) as u32;
    }
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        state: &egui_wgpu::RenderState,
        target: &Target,
        cam: &Camera,
        mode: Mode,
        overlay: Overlay,
        selection_mode: crate::ui::SelectionMode,
        selected_object: bool,
    ) {
        self.draw_raw(
            &state.device,
            &state.queue,
            target,
            cam,
            mode,
            overlay,
            selection_mode,
            selected_object,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_raw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &Target,
        cam: &Camera,
        mode: Mode,
        overlay: Overlay,
        selection_mode: crate::ui::SelectionMode,
        selected_object: bool,
    ) {
        let (view_proj,view) = cam.matrices(target.size[0] as f32/target.size[1] as f32);
        // 0 when the average edge is under about 1.5 px on screen, 1 from about 10 px (smoothstep): a dense mesh reads as shading
        let px_per_unit = target.size[1] as f32 / (2.0 * (0.8f32 / 2.0).tan() * cam.distance().max(1e-6));
        let t = ((self.edge_world * px_per_unit - 1.5) / 8.5).clamp(0.0, 1.0);
        let fade = t * t * (3.0 - 2.0 * t);
        let is_shaded_wire = mode == Mode::ShadedWire
            || (overlay.wireframe && (mode == Mode::Solid || mode == Mode::Shaded))
            || selection_mode == crate::ui::SelectionMode::Edges;
        let show_wire = mode == Mode::Wire || is_shaded_wire || overlay.xray;
        // wire.a selects the line look in the shader: 1 = a darker version of the surface (Shaded+Wire), 0 = lit lines (Wire)
        let u = Uniforms { view_proj,view,base:[0.72,0.74,0.78,1.],wire:[0.,0.,0.,if is_shaded_wire {1.} else {0.}],flat_shading:(mode==Mode::Solid) as u32,wire_fade:fade,_pad:[1,0] };
        queue.write_buffer(&self.ubuf,0,bytemuck::bytes_of(&u));
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label:Some("MiniHou viewport"),
                color_attachments:&[Some(wgpu::RenderPassColorAttachment {
                    view:&target.color,depth_slice:None,resolve_target:None,
                    ops:wgpu::Operations { load:wgpu::LoadOp::Clear(wgpu::Color { r:0.244,g:0.256,b:0.278,a:1. }),store:wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment {
                    view:&target.depth,depth_ops:Some(wgpu::Operations {
                        load:wgpu::LoadOp::Clear(1.),
                        store:if selected_object { wgpu::StoreOp::Store } else { wgpu::StoreOp::Discard }
                    }),stencil_ops:None,
                }),
                timestamp_writes:None,occlusion_query_set:None,
            });
            pass.set_bind_group(0,&self.bind,&[]);
            if mode != Mode::Wire && self.ntri_idx > 0 {
                pass.set_pipeline(if overlay.xray { &self.pipe_xray } else if is_shaded_wire { &self.pipe_surf_bias } else { &self.pipe_surf });
                pass.set_vertex_buffer(0,self.vp.slice(..)); pass.set_vertex_buffer(1,self.vn.slice(..));
                pass.set_index_buffer(self.ib.slice(..),wgpu::IndexFormat::Uint32);
                // All selected ranges are adjacent and share the same material for now.
                let end=if self.lod_active { self.selected_ranges.last().map_or(0, |r| r.end) } else { self.ntri_idx };
                if end > 0 { pass.draw_indexed(0..end,0,0..1); }
            }
            if show_wire && self.nedge_idx > 0 {
                pass.set_pipeline(&self.pipe_wire); pass.set_vertex_buffer(0,self.vp.slice(..)); pass.set_vertex_buffer(1,self.vn.slice(..));
                pass.set_index_buffer(self.eb.slice(..),wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.nedge_idx,0,0..1);
            }
            if self.sel_face_n > 0 {
                pass.set_pipeline(&self.pipe_select);
                pass.set_vertex_buffer(0,self.sel_face.slice(..)); pass.set_vertex_buffer(1,self.zeros.slice(..));
                pass.draw(0..self.sel_face_n,0..1);
            }
            if self.sel_line_n > 0 {
                pass.set_pipeline(&self.pipe_bounds);
                pass.set_vertex_buffer(0,self.sel_line.slice(..)); pass.set_vertex_buffer(1,self.zeros.slice(..));
                pass.draw(0..self.sel_line_n,0..1);
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
        if selected_object && self.ntri_idx > 0 {
            {
                let mut mask_pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("MiniHou object mask"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.mask,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &target.depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                mask_pass.set_bind_group(0, &self.bind, &[]);
                mask_pass.set_pipeline(&self.pipe_mask);
                mask_pass.set_vertex_buffer(0, self.vp.slice(..));
                mask_pass.set_vertex_buffer(1, self.vn.slice(..));
                mask_pass.set_index_buffer(self.ib.slice(..), wgpu::IndexFormat::Uint32);
                let end = if self.lod_active {
                    self.selected_ranges.last().map_or(0, |r| r.end)
                } else {
                    self.ntri_idx
                };
                if end > 0 {
                    mask_pass.draw_indexed(0..end, 0, 0..1);
                }
            }
            {
                let outline_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("outline_bind"),
                    layout: &self.bgl_outline,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&target.mask),
                    }],
                });
                let mut outline_pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("MiniHou outline composite"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.color,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                outline_pass.set_pipeline(&self.pipe_outline);
                outline_pass.set_bind_group(0, &outline_bind, &[]);
                outline_pass.draw(0..3, 0..1);
            }
        }
        queue.submit([enc.finish()]);
    }
}
/// Only the selected clusters enter the GPU index buffer; ranges stay contiguous.
fn packed_cut(lod: &ClusterLod, ids: &[ClusterId]) -> (Vec<u32>, Vec<std::ops::Range<u32>>) {
    let mut indices=Vec::with_capacity(lod.selected_triangles(ids)*3);
    let mut ranges=Vec::with_capacity(ids.len());
    for &id in ids {
        let start=indices.len() as u32;
        for tri in &lod.clusters[id as usize].triangles { indices.extend_from_slice(tri); }
        ranges.push(start..indices.len() as u32);
    }
    (indices,ranges)
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

#[cfg(test)]
mod tests {
    use super::*;
    use minihou_core::lod::{Cluster, Sphere};

    #[test]
    fn gpu_cut_contains_only_selected_clusters_and_contiguous_ranges() {
        let cluster = |triangles| Cluster { triangles, bounds:Sphere::default(), error:0., level:0 };
        let lod = ClusterLod { clusters:vec![cluster(vec![[0,1,2]]),cluster(vec![[3,4,5],[5,6,7]])], ..Default::default() };
        let (indices,ranges)=packed_cut(&lod,&[1]);
        assert_eq!(indices,vec![3,4,5,5,6,7]);
        assert_eq!(ranges,vec![0..6]);
        let (indices,ranges)=packed_cut(&lod,&[0,1]);
        assert_eq!(indices,vec![0,1,2,3,4,5,5,6,7]);
        assert_eq!(ranges,vec![0..3,3..9]);
    }

    #[test]
    fn outline_silhouette_headless() {
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
        })) else {
            eprintln!("No wgpu adapter available, skipping headless render test");
            return;
        };

        let (device, queue) = match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) {
            Ok(dq) => dq,
            Err(e) => {
                eprintln!("Failed to create wgpu device ({e:?}), skipping headless render test");
                return;
            }
        };

        // Flat quad in XY plane at z=0, from -1 to 1 in x and y
        let p = vec![
            -1.0, -1.0, 0.0,
             1.0, -1.0, 0.0,
             1.0,  1.0, 0.0,
            -1.0,  1.0, 0.0,
        ];
        let tri = vec![0, 1, 2, 0, 2, 3];
        let n = vec![
            0.0, 0.0, 1.0,
            0.0, 0.0, 1.0,
            0.0, 0.0, 1.0,
            0.0, 0.0, 1.0,
        ];
        let edge = vec![0, 1, 1, 2, 2, 3, 3, 0];
        let scene = Scene {
            p, n, tri, edge,
            nlp: Vec::new(), nln: Vec::new(), bbp: [0.0; 24],
            center: [0.0, 0.0, 0.0], radius: 1.5,
        };

        let gpu = Gpu::new(&device, &scene);
        let size = [64, 64];
        let (target, color_tex) = Target::new_headless(&device, size);

        // Frame camera looking directly at the flat quad
        let cam = Camera::frame([0.0, 0.0, 0.0], 1.5);

        // Render with selected_object = true
        gpu.draw_raw(
            &device,
            &queue,
            &target,
            &cam,
            Mode::Solid,
            Overlay::default(),
            crate::ui::SelectionMode::Object,
            true,
        );

        // Read back color buffer
        let w = size[0];
        let h = size[1];
        let bytes_per_row = (w * 4 + 255) & !255;
        let readback_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (bytes_per_row * h) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &color_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback_buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        queue.submit([enc.finish()]);

        let slice = readback_buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().unwrap().unwrap();

        let data = slice.get_mapped_range();

        let pixel_at = |x: u32, y: u32| -> [u8; 4] {
            let offset = (y * bytes_per_row + x * 4) as usize;
            [data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]
        };

        // Check center pixel (32, 32): inside the mesh silhouette
        let center = pixel_at(32, 32);
        // #ffa028 in sRGB: (255, 160, 40)
        let is_outline = |rgba: [u8; 4]| rgba[0] >= 250 && rgba[1] >= 155 && rgba[1] <= 165 && rgba[2] >= 35 && rgba[2] <= 45;

        // Inside the silhouette must NEVER be outline colored!
        assert!(!is_outline(center), "Center of silhouette must not be outline colored, got {:?}", center);

        // Count outline pixels across the image: there must be a silhouette ring around the mesh
        let mut outline_count = 0;
        for y in 0..h {
            for x in 0..w {
                if is_outline(pixel_at(x, y)) {
                    outline_count += 1;
                }
            }
        }
        assert!(outline_count > 0, "Expected silhouette outline pixels around the mesh, found none");

        // Corner pixel (0, 0) is background, far outside outline
        let corner = pixel_at(0, 0);
        assert!(!is_outline(corner), "Corner must be background, got {:?}", corner);
    }
}


