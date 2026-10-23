use super::*;

fn viewport_shader() -> String {
    let mut shader = include_str!("../shader.wgsl").to_owned();
    shader.push_str(include_str!("group_lines.wgsl"));
    shader
}

#[cfg(test)]
#[test]
fn distant_grid_fades_when_a_pixel_covers_multiple_cells() {
    let shader = viewport_shader();
    assert!(shader.contains("smoothstep(0.08,0.35,width)"));
    assert!(shader.contains("minor_line*(1.-transition)+major_line*transition"));
}

impl Gpu {
    pub fn new_lod(device: &wgpu::Device, sc: &Scene) -> Self {
        Self::new_with_indices(device, sc, &[], &[])
    }
    pub fn new(device: &wgpu::Device, sc: &Scene) -> Self {
        Self::new_with_indices(device, sc, &sc.tri, &sc.edge)
    }
    pub fn new_proxy(device: &wgpu::Device, sc: &Scene, level: f32) -> Self {
        let (tri, edge) = sc.proxy_indices(level);
        Self::new_with_indices(device, sc, &tri, &edge)
    }
    fn new_with_indices(device: &wgpu::Device, sc: &Scene, tri: &[u32], edge: &[u32]) -> Self {
        let (tri,edge)=sc.base_indices(tri,edge);
        let (tri,edge)=(tri.as_ref(),edge.as_ref());
        let device_limits = device.limits();
        let mut limits = minihou_core::limits::Limits::default();
        limits.set_gpu_limits(device_limits.max_buffer_size, device_limits.max_storage_buffer_binding_size as u64);
        let max_bytes = device_limits.max_buffer_size.min(device_limits.max_storage_buffer_binding_size as u64);
        let oversized = chunked_scene(sc, tri, edge, max_bytes);
        let mesh_source = oversized.then(|| MeshSource { p: sc.p.clone(), n: sc.n.clone(), colors: vec![0.72; sc.p.len()], cd: sc.cd.clone() });
        let face_chunks = mesh_source.as_ref().map_or_else(Vec::new, |source| upload_chunks(device, source, tri, 3, &[], max_bytes));
        let edge_chunks = mesh_source.as_ref().map_or_else(Vec::new, |source| upload_chunks(device, source, edge, 2, &[], max_bytes));
        let fallback = [0f32; 3];
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let buf = |label, data: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: if data.is_empty() { &[0;4] } else { data }, usage })
        };
        let vp = buf("P", bytemuck::cast_slice(if oversized { &fallback[..] } else { &sc.p }), wgpu::BufferUsages::VERTEX);
        let vn = buf("N", bytemuck::cast_slice(if oversized { &fallback[..] } else { &sc.n }), wgpu::BufferUsages::VERTEX);
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
            }, wgpu::BindGroupLayoutEntry {
                binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None,
            }, wgpu::BindGroupLayoutEntry {
                binding: 2, visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None,
            }],
        });
        let matcap = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("neutral clay matcap"), size: wgpu::Extent3d { width:128, height:128, depth_or_array_layers:1 },
            mip_level_count:1, sample_count:1, dimension:wgpu::TextureDimension::D2, format:wgpu::TextureFormat::Rgba8Unorm,
            usage:wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats:&[],
        });
        let matcap_view = matcap.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor { mag_filter:wgpu::FilterMode::Linear, min_filter:wgpu::FilterMode::Linear, ..Default::default() });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding:1, resource:wgpu::BindingResource::TextureView(&matcap_view) },
                wgpu::BindGroupEntry { binding:2, resource:wgpu::BindingResource::Sampler(&sampler) }],
        });
        let tex_bgl=device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("material group"),entries:&[
            wgpu::BindGroupLayoutEntry {binding:0,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None},count:None},
            wgpu::BindGroupLayoutEntry {binding:1,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Texture {sample_type:wgpu::TextureSampleType::Float {filterable:true},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},count:None},
            wgpu::BindGroupLayoutEntry {binding:2,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),count:None},
        ]});
        let default_texture=device.create_texture(&wgpu::TextureDescriptor {label:Some("default material placeholder"),size:wgpu::Extent3d {width:1,height:1,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8Unorm,usage:wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]});
        let default_view=default_texture.create_view(&Default::default());
        let default_sampler=device.create_sampler(&Default::default());
        let default_factor=device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("default factor"),contents:bytemuck::cast_slice(&[1f32;4]),usage:wgpu::BufferUsages::UNIFORM});
        let default_tex_bind=device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("default material"),layout:&tex_bgl,entries:&[
            wgpu::BindGroupEntry {binding:0,resource:default_factor.as_entire_binding()},wgpu::BindGroupEntry {binding:1,resource:wgpu::BindingResource::TextureView(&default_view)},wgpu::BindGroupEntry {binding:2,resource:wgpu::BindingResource::Sampler(&default_sampler)}]});
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bgl,&tex_bgl],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(viewport_shader().into()),
        });
        const POS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x3];
        const NRM: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![1 => Float32x3];
        const COL: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![2 => Float32x3];
        let vb = |a: &'static [wgpu::VertexAttribute]| wgpu::VertexBufferLayout {
            array_stride: 12,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: a,
        };
        let surf_bufs = [vb(&POS), vb(&NRM), vb(&COL)];
        let wire_bufs = [vb(&POS), vb(&NRM), vb(&COL)];
        const INST_ATTRS:[wgpu::VertexAttribute;4]=wgpu::vertex_attr_array![3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Float32x4];
        let inst_bufs=[vb(&POS),vb(&NRM),vb(&COL),wgpu::VertexBufferLayout {
            array_stride:std::mem::size_of::<InstanceVertex>() as u64,
            step_mode:wgpu::VertexStepMode::Instance,attributes:&INST_ATTRS,
        }];
        const TEX_ATTRS:[wgpu::VertexAttribute;5]=wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x3,3=>Float32x2,4=>Float32];
        let tex_bufs=[wgpu::VertexBufferLayout {array_stride:std::mem::size_of::<TexVertex>() as u64,step_mode:wgpu::VertexStepMode::Vertex,attributes:&TEX_ATTRS}];

        let mk = |label, vs, fs, bufs: &[wgpu::VertexBufferLayout], topo, bias: i32, blend: bool, compare, depth_write: bool, color_write| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), compilation_options: Default::default(), buffers: bufs },
                fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(fs), compilation_options: Default::default(), targets: &if blend { [Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: color_write })] } else { [Some(wgpu::ColorTargetState { format, blend:None, write_mask:color_write })] } }),
                primitive: wgpu::PrimitiveState { topology: topo, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: depth_write,
                    depth_compare: compare,
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState { constant: bias, slope_scale: if bias != 0 { 1.0 } else { 0.0 }, clamp: 0.0 },
                }),
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        use wgpu::{CompareFunction as C, ColorWrites as W, PrimitiveTopology as T};
        const ELEMENT_ATTRS:[wgpu::VertexAttribute;4]=wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Uint32,3=>Uint32];
        let element_bufs=[wgpu::VertexBufferLayout {array_stride:32,step_mode:wgpu::VertexStepMode::Instance,attributes:&ELEMENT_ATTRS}];
        let pipe_floor=mk("infinite floor","vs_infinite_floor","fs_infinite_floor",&[],T::TriangleList,0,true,C::LessEqual,false,W::ALL);
        let pipe_z_axis=mk("z axis","vs_element","fs_element",&element_bufs,T::TriangleList,0,true,C::LessEqual,false,W::ALL);
        let z_axis=buf("z axis",bytemuck::cast_slice(&[Element {a:[0.,0.,-1e6],b:[0.,0.,1e6],kind:6,selected:0}]),wgpu::BufferUsages::VERTEX);
        let pipe_face_markers=mk("face centres","vs_face_marker","fs_face_marker",&element_bufs,T::TriangleList,-2,true,C::LessEqual,false,W::ALL);
        let pipe_face_markers_xray=mk("face centres xray","vs_face_marker","fs_face_marker",&element_bufs,T::TriangleList,-2,true,C::Always,false,W::ALL);
        // A corner shared by three faces needs enough depth bias to keep its
        // marker visible against the adjacent rasterized triangles.
        let pipe_elements=mk("points and curves","vs_element","fs_element",&element_bufs,T::TriangleList,-4,true,C::LessEqual,true,W::ALL);
        let pipe_elements_ghost=mk("ghost points and curves","vs_element","fs_element_ghost",&element_bufs,T::TriangleList,-1,true,C::LessEqual,false,W::ALL);
        let pipe_surf = mk("surf", "vs_surf", "fs_surf", &surf_bufs, T::TriangleList, 0, false, C::LessEqual, true, W::ALL);
        let pipe_inst_surf=mk("instanced surf","vs_inst_surf","fs_surf",&inst_bufs,T::TriangleList,0,false,C::LessEqual,true,W::ALL);
        let pipe_inst_wire=mk("instanced wire","vs_inst_wire","fs_wire",&inst_bufs,T::LineList,-1,true,C::LessEqual,false,W::ALL);
        let pipe_inst_wire_all=mk("instanced all wires","vs_inst_wire","fs_wire",&inst_bufs,T::LineList,0,true,C::Always,false,W::ALL);
        let pipe_inst_wire_hidden=mk("instanced hidden wires","vs_inst_wire","fs_ghost_wire",&inst_bufs,T::LineList,0,true,C::Greater,false,W::ALL);
        let pipe_inst_ghost_wire=mk("instanced ghost wires","vs_inst_wire","fs_ghost_wire",&inst_bufs,T::LineList,0,true,C::LessEqual,false,W::ALL);
        let pipe_inst_depth=mk("instanced depth","vs_inst_surf","fs_mask",&inst_bufs,T::TriangleList,2,false,C::LessEqual,true,W::empty());
        let pipe_inst_xray=mk("instanced xray","vs_inst_surf","fs_xray",&inst_bufs,T::TriangleList,0,true,C::Always,false,W::ALL);
        let pipe_inst_ghost=mk("instanced ghost","vs_inst_surf","fs_ghost",&inst_bufs,T::TriangleList,0,true,C::LessEqual,false,W::ALL);
        let pipe_tex=mk("textured surface","vs_tex","fs_tex",&tex_bufs,T::TriangleList,0,false,C::LessEqual,true,W::ALL);
        let pipe_tex_bias=mk("textured surface+bias","vs_tex","fs_tex",&tex_bufs,T::TriangleList,2,false,C::LessEqual,true,W::ALL);
        let pipe_tex_xray=mk("textured xray","vs_tex","fs_tex_xray",&tex_bufs,T::TriangleList,0,true,C::Always,false,W::ALL);
        let pipe_surf_bias = mk("surf+bias", "vs_surf", "fs_surf", &surf_bufs, T::TriangleList, 2, false, C::LessEqual, true, W::ALL);
        let pipe_wire = mk("wire", "vs_wire", "fs_wire", &wire_bufs, T::LineList, -1, true, C::LessEqual, false, W::ALL);
        let pipe_wire_all = mk("all edges", "vs_wire", "fs_wire", &wire_bufs, T::LineList, 0, true, C::Always, false, W::ALL);
        let pipe_wire_hidden = mk("occluded edges", "vs_wire", "fs_ghost_wire", &wire_bufs, T::LineList, 0, true, C::Greater, false, W::ALL);
        let pipe_depth = mk("hidden line depth", "vs_surf", "fs_mask", &surf_bufs, T::TriangleList, 2, false, C::LessEqual, true, W::empty());
        let pipe_ghost = mk("ghost surface", "vs_surf", "fs_ghost", &surf_bufs, T::TriangleList, 0, true, C::LessEqual, false, W::ALL);
        let pipe_ghost_wire = mk("ghost edges", "vs_wire", "fs_ghost_wire", &wire_bufs, T::LineList, 0, true, C::LessEqual, false, W::ALL);
        let pipe_xray = mk("xray", "vs_surf", "fs_xray", &surf_bufs, T::TriangleList, 0, true, C::Always, false, W::ALL);
        let pipe_normals = mk("normals", "vs_wire", "fs_normals", &wire_bufs, T::LineList, 0, false, C::LessEqual, true, W::ALL);
        let pipe_bounds = mk("bounds", "vs_wire", "fs_bounds", &wire_bufs, T::LineList, 0, false, C::LessEqual, true, W::ALL);
        let group_bufs=[vb(&POS)];
        let pipe_group_face=mk("group fill","vs_group","fs_group_fill",&group_bufs,T::TriangleList,-3,true,C::LessEqual,false,W::ALL);
        const THICK_ATTRS:[wgpu::VertexAttribute;2]=wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3];
        let thick_bufs=[wgpu::VertexBufferLayout {array_stride:24,step_mode:wgpu::VertexStepMode::Instance,attributes:&THICK_ATTRS}];
        let pipe_group_line=mk("group outline","vs_group_thick","fs_group_line",&thick_bufs,T::TriangleList,-4,true,C::LessEqual,false,W::ALL);
        let point_bufs=[wgpu::VertexBufferLayout {array_stride:12,step_mode:wgpu::VertexStepMode::Instance,attributes:&POS}];
        let pipe_group_point=mk("group points","vs_group_point","fs_group_point",&point_bufs,T::TriangleList,-4,true,C::LessEqual,false,W::ALL);
        let pipe_barrier_face=mk("barrier fill","vs_group","fs_barrier_fill",&group_bufs,T::TriangleList,-3,true,C::LessEqual,false,W::ALL);
        let pipe_barrier_line=mk("barrier outline","vs_group","fs_barrier_line",&group_bufs,T::LineList,-4,true,C::LessEqual,false,W::ALL);
        let pipe_target_face=mk("target fill","vs_group","fs_target_fill",&group_bufs,T::TriangleList,-3,true,C::LessEqual,false,W::ALL);
        let pipe_target_line=mk("target outline","vs_group_thick","fs_target_line",&thick_bufs,T::TriangleList,-4,true,C::LessEqual,false,W::ALL);
        let pipe_select = mk("select", "vs_surf", "fs_select", &surf_bufs, T::TriangleList, -2, true, C::LessEqual, false, W::ALL);

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
                // pulled toward the camera so the wire lines and biased surface already in the depth buffer never punch holes in
                // the mask (holes read as an outline along every edge)
                bias: wgpu::DepthBiasState { constant: -8, slope_scale: -2.0, clamp: 0.0 },
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let pipe_inst_mask=device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:Some("instanced mask"),layout:Some(&layout),
            vertex:wgpu::VertexState {module:&shader,entry_point:Some("vs_inst_surf"),compilation_options:Default::default(),buffers:&inst_bufs},
            fragment:Some(wgpu::FragmentState {module:&shader,entry_point:Some("fs_mask"),compilation_options:Default::default(),
                targets:&[Some(wgpu::ColorTargetState {format:wgpu::TextureFormat::R8Unorm,blend:None,write_mask:wgpu::ColorWrites::ALL})]}),
            primitive:wgpu::PrimitiveState {topology:wgpu::PrimitiveTopology::TriangleList,cull_mode:None,..Default::default()},
            depth_stencil:Some(wgpu::DepthStencilState {format:DEPTH,depth_write_enabled:false,depth_compare:wgpu::CompareFunction::LessEqual,
                stencil:Default::default(),bias:wgpu::DepthBiasState {constant:-8,slope_scale:-2.,clamp:0.}}),
            multisample:Default::default(),multiview:None,cache:None,
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
            source: wgpu::ShaderSource::Wgsl(include_str!("../outline.wgsl").into()),
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

        let group_face=buf("group faces",bytemuck::cast_slice(&[0f32;3]),wgpu::BufferUsages::VERTEX);
        let group_line=buf("group outline",bytemuck::cast_slice(&[0f32;3]),wgpu::BufferUsages::VERTEX);
        let group_point=buf("group points",bytemuck::cast_slice(&[0f32;3]),wgpu::BufferUsages::VERTEX);
        let barrier_face=buf("barrier faces",bytemuck::cast_slice(&[0f32;3]),wgpu::BufferUsages::VERTEX);
        let barrier_line=buf("barrier outline",bytemuck::cast_slice(&[0f32;3]),wgpu::BufferUsages::VERTEX);
        let target_face=buf("target faces",bytemuck::cast_slice(&[0f32;3]),wgpu::BufferUsages::VERTEX);
        let target_line=buf("target outline",bytemuck::cast_slice(&[0f32;3]),wgpu::BufferUsages::VERTEX);
        let sel_face = buf("sel_face", bytemuck::cast_slice(&[0f32; 3]), wgpu::BufferUsages::VERTEX);
        let sel_line = buf("sel_line", bytemuck::cast_slice(&[0f32; 3]), wgpu::BufferUsages::VERTEX);
        let zeros = buf("zeros", bytemuck::cast_slice(&vec![0f32; 3 * 6 * 4096]), wgpu::BufferUsages::VERTEX);
        let normal_floats = capped_normal_vertices(sc.nlp.len().min(sc.nln.len()), max_bytes) * 3;
        let nl_p = buf("nl_p", bytemuck::cast_slice(&sc.nlp[..normal_floats]), wgpu::BufferUsages::VERTEX);
        let nl_n = buf("nl_n", bytemuck::cast_slice(&sc.nln[..normal_floats]), wgpu::BufferUsages::VERTEX);
        let bb_chunks = box_chunks(device, sc, max_bytes);
        let bb_p = buf("bb_p", bytemuck::cast_slice(if bb_chunks.is_empty() { &sc.bbp[..] } else { &[0.;3][..] }), wgpu::BufferUsages::VERTEX);
        let bb_n = buf("bb_n", bytemuck::cast_slice(if bb_chunks.is_empty() { &[0f32; 24][..] } else { &[0f32; 3][..] }), wgpu::BufferUsages::VERTEX);
        let bb_i = buf("bb_i", bytemuck::cast_slice(if bb_chunks.is_empty() { &BOX_EDGES[..] } else { &[0u32][..] }), wgpu::BufferUsages::INDEX);
        let sharp_edge_indices=sharp_edges_indices(sc,tri,edge,SKETCHUP_CREASE_DEGREES);
        let sharp_edge_chunks = mesh_source.as_ref().map_or_else(Vec::new, |source| upload_chunks(device, source, &sharp_edge_indices, 2, &[], max_bytes));

        let mut out=Self {
            packed:Vec::new(),packed_cut_cache:std::sync::Mutex::new(None),pipe_inst_surf,pipe_inst_wire,pipe_inst_wire_all,pipe_inst_wire_hidden,
            pipe_inst_ghost_wire,pipe_inst_depth,pipe_inst_xray,pipe_inst_ghost,pipe_inst_mask,
            tex_bgl,default_tex_bind,pipe_tex,pipe_tex_bias,pipe_tex_xray,tex_batches:Vec::new(),texture_status:Vec::new(),gpu_images:HashMap::new(),gpu_tick:0,
            pipe_floor,z_axis,pipe_z_axis,
            face_markers:face_markers::FaceMarkers::new(device),pipe_face_markers,pipe_face_markers_xray,
            elements:Elements::new(device,sc,&[],&[]),element_points:Vec::new(),pipe_elements,pipe_elements_ghost,
            pipe_depth, pipe_wire_all, pipe_wire_hidden, pipe_ghost, pipe_ghost_wire,
            partition_tri: Partition::new(device, if oversized { &[] } else { tri }, 3, &[]), partition_edge: Partition::new(device, if oversized { &[] } else { edge }, 2, &[]),
            sharp_edge: Partition::new(device, if oversized { &[] } else { &sharp_edge_indices }, 2, &[]),
            current_tri:tri.to_vec(), current_edge:edge.to_vec(),
            sharp_edge_indices,
            boxes:Some(Boxes::new(device,sc,&[],&[],&[0..(sc.p.len()/3) as u32])), object_ranges:Vec::new(), object_ids:Vec::new(),
            matcap, matcap_loaded:std::sync::atomic::AtomicU8::new(0),
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
            group_face,group_face_n:0,group_line,group_line_n:0,group_point,group_point_n:0,pipe_group_face,pipe_group_line,pipe_group_point,barrier_face,barrier_face_n:0,barrier_line,barrier_line_n:0,pipe_barrier_face,pipe_barrier_line,target_face,target_face_n:0,target_line,target_line_n:0,pipe_target_face,pipe_target_line,
            sel_face,
            sel_face_n: 0,
            sel_line,
            sel_line_n: 0,
            zeros,
            nl_p,
            nl_n,
            nl_count: (normal_floats / 3) as u32, nl_selected:0,
            bb_p,
            bb_chunks,
            bb_n,
            bb_i,
            bind,
            ubuf,
            vp,
            vn,
            vc:buf("object colors",bytemuck::cast_slice(&vec![0.72f32;if oversized {3} else {sc.p.len().max(3)}]),wgpu::BufferUsages::VERTEX),
            vcd:sc.cd.as_ref().map(|c|buf("Cd colors",bytemuck::cast_slice(if oversized { &fallback[..] } else { c }),wgpu::BufferUsages::VERTEX)),
            neutral_color:buf("neutral color",bytemuck::cast_slice(&vec![0.72f32;neutral_color_len(sc,tri,oversized,max_bytes)]),wgpu::BufferUsages::VERTEX),
            limits,
            mesh_source, face_chunks, edge_chunks, sharp_edge_chunks, mask_chunks:Vec::new(),
            object_mask: None, object_mask_n: 0, object_mask_points: None,
            ntri_idx: tri.len() as u32,
            edge_world: edge_world_of(sc,edge),
        };
        out.upload_packed(device,sc);
        out
    }
}
