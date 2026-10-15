//! MiniHou native viewport spike: cook a graph (or a generated grid) with minihou-core, build the viewport layout with
//! `preview::layout`, upload the final u32/f32 arrays straight to wgpu buffers and draw Solid / Shaded / Wireframe /
//! Shaded+Wire. `--bench N` orbits N frames and prints timings as one JSON line.

use std::sync::Arc;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use minihou_core::{normals, preview, Graph};
use wgpu::util::DeviceExt;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Solid,
    Shaded,
    Wire,
    ShadedWire,
}

struct Opts {
    graph: Option<String>,
    grid: usize,
    mode: Mode,
    backend: wgpu::Backends,
    bench: Option<usize>,
    sync: bool,
    vsync: bool,
    low_power: bool,
    size: (u32, u32),
}

fn usage() -> ! {
    eprintln!(
        "minihou-view [--graph FILE.json | --grid QUADS_PER_SIDE] [--mode solid|shaded|wire|shaded-wire]\n  \
         [--backend vulkan|dx12|gl|primary] [--bench FRAMES] [--sync] [--no-vsync] [--gpu high|low] [--size WxH]\n  \
         keys: 1 solid, 2 shaded, 3 wire, 4 shaded+wire; LMB orbit, MMB/RMB/shift+LMB pan, wheel zoom, F frame, Esc quit"
    );
    std::process::exit(2)
}

fn parse_args() -> Opts {
    let mut o = Opts { graph: None, grid: 1000, mode: Mode::Shaded, backend: wgpu::Backends::VULKAN, bench: None, sync: false, vsync: true, low_power: false, size: (1280, 720) };
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < a.len() {
        let key = a[i].clone();
        let mut next = || -> String {
            i += 1;
            a.get(i).cloned().unwrap_or_else(|| usage())
        };
        match key.as_str() {
            "--graph" => o.graph = Some(next()),
            "--grid" => o.grid = next().parse().unwrap_or_else(|_| usage()),
            "--mode" => {
                o.mode = match next().as_str() {
                    "solid" => Mode::Solid,
                    "shaded" => Mode::Shaded,
                    "wire" => Mode::Wire,
                    "shaded-wire" => Mode::ShadedWire,
                    _ => usage(),
                }
            }
            "--backend" => {
                o.backend = match next().as_str() {
                    "vulkan" => wgpu::Backends::VULKAN,
                    "dx12" => wgpu::Backends::DX12,
                    "gl" => wgpu::Backends::GL,
                    "primary" => wgpu::Backends::PRIMARY,
                    _ => usage(),
                }
            }
            "--bench" => o.bench = Some(next().parse().unwrap_or_else(|_| usage())),
            "--sync" => o.sync = true,
            "--no-vsync" => o.vsync = false,
            "--gpu" => o.low_power = next() == "low",
            "--size" => {
                let s = next();
                let (w, h) = s.split_once('x').unwrap_or_else(|| usage());
                o.size = (w.parse().unwrap_or_else(|_| usage()), h.parse().unwrap_or_else(|_| usage()));
            }
            _ => usage(),
        }
        i += 1;
    }
    if o.bench.is_some() {
        o.vsync = false;
    }
    o
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    base: [f32; 4],
    wire: [f32; 4],
    flat_shading: u32,
    _pad: [u32; 3],
}

// ---- tiny column-major matrix helpers (enough for an orbit camera)
type M4 = [[f32; 4]; 4];

fn mul(a: &M4, b: &M4) -> M4 {
    let mut r = [[0.0; 4]; 4];
    for c in 0..4 {
        for row in 0..4 {
            r[c][row] = (0..4).map(|k| a[k][row] * b[c][k]).sum();
        }
    }
    r
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt().max(1e-20);
    [a[0] / l, a[1] / l, a[2] / l]
}

fn look_at(eye: [f32; 3], at: [f32; 3], up: [f32; 3]) -> M4 {
    let f = norm(sub(at, eye));
    let s = norm(cross(f, up));
    let u = cross(s, f);
    [
        [s[0], u[0], -f[0], 0.0],
        [s[1], u[1], -f[1], 0.0],
        [s[2], u[2], -f[2], 0.0],
        [-dot(s, eye), -dot(u, eye), dot(f, eye), 1.0],
    ]
}

/// Right-handed, depth 0..1 (wgpu).
fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> M4 {
    let t = 1.0 / (fovy / 2.0).tan();
    [
        [t / aspect, 0.0, 0.0, 0.0],
        [0.0, t, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, near * far / (near - far), 0.0],
    ]
}

struct Camera {
    target: [f32; 3],
    dist: f32,
    yaw: f32,
    pitch: f32,
    radius: f32,
}

impl Camera {
    fn frame(center: [f32; 3], radius: f32) -> Self {
        Camera { target: center, dist: radius * 2.6, yaw: 0.6, pitch: 0.5, radius }
    }

    fn eye(&self) -> [f32; 3] {
        // +Y up
        let (cp, sp) = (self.pitch.cos(), self.pitch.sin());
        [
            self.target[0] + self.dist * cp * self.yaw.sin(),
            self.target[1] + self.dist * sp,
            self.target[2] + self.dist * cp * self.yaw.cos(),
        ]
    }

    fn matrices(&self, aspect: f32) -> (M4, M4) {
        let view = look_at(self.eye(), self.target, [0.0, 1.0, 0.0]);
        let near = (self.dist - self.radius * 1.5).max(self.radius * 0.001).max(1e-4);
        let far = self.dist + self.radius * 3.0;
        (mul(&perspective(0.8, aspect, near, far), &view), view)
    }
}

/// What the cook + layout produced, with timings (ms).
struct Scene {
    p: Vec<f32>,
    n: Vec<f32>,
    tri: Vec<u32>,
    edge: Vec<u32>,
    center: [f32; 3],
    radius: f32,
    t_cook: f64,
    t_layout: f64,
    t_normals: f64,
    npoints: usize,
    ntris: usize,
    nedges: usize,
}

fn build_scene(o: &Opts) -> Scene {
    let ms = |s: Instant| s.elapsed().as_secs_f64() * 1000.0;
    let text = match &o.graph {
        Some(f) => std::fs::read_to_string(f).unwrap_or_else(|e| panic!("{f}: {e}")),
        None => format!(
            r#"{{"nodes": {{"g": {{"type": "grid", "params": {{"columns": {c}, "rows": {c}}}}}, "n": {{"type": "noise", "inputs": {{"geo": "g"}}}}}}, "display": "n"}}"#,
            c = o.grid + 1
        ),
    };
    let s = Instant::now();
    let g = Graph::from_json(&text).and_then(|g| g.cook(None)).unwrap_or_else(|e| panic!("cook: {e}"));
    let t_cook = ms(s);
    let (p, t) = (g.p(), &g.topology);
    let s = Instant::now();
    let l = preview::layout(p, &t.vertex_point, &t.prim_offsets, t.closed.as_deref(), g.npoints, usize::MAX, usize::MAX, 100_000, 8192);
    let t_layout = ms(s);
    let s = Instant::now();
    let n = match g.point.get("N").and_then(|a| a.as_f32()) {
        Some(n) if n.len() == g.npoints * 3 => n.to_vec(),
        _ => normals::point_normals(p, t, g.npoints),
    };
    let t_normals = ms(s);
    let (lo, hi) = preview::bounds(p);
    let center = [0, 1, 2].map(|k| (lo[k] + hi[k]) * 0.5);
    let radius = (0..3).map(|k| (hi[k] - lo[k]).powi(2)).sum::<f32>().sqrt() * 0.5;
    Scene {
        p: p.to_vec(),
        n,
        ntris: l.tri.len() / 3,
        nedges: l.edge.len() / 2,
        tri: l.tri,
        edge: l.edge,
        center,
        radius: radius.max(1e-3),
        t_cook,
        t_layout,
        t_normals,
        npoints: g.npoints,
    }
}

struct Gpu {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth: wgpu::TextureView,
    pipe_surf: wgpu::RenderPipeline,
    pipe_surf_bias: wgpu::RenderPipeline,
    pipe_wire: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
    ubuf: wgpu::Buffer,
    vp: wgpu::Buffer,
    vn: wgpu::Buffer,
    ib: wgpu::Buffer,
    eb: wgpu::Buffer,
    ntri_idx: u32,
    nedge_idx: u32,
    adapter_info: wgpu::AdapterInfo,
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

impl Gpu {
    fn new(window: Arc<Window>, o: &Opts, sc: &Scene) -> Gpu {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: o.backend, ..wgpu::InstanceDescriptor::new_without_display_handle() });
        let surface = instance.create_surface(window.clone()).expect("surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: if o.low_power { wgpu::PowerPreference::LowPower } else { wgpu::PowerPreference::HighPerformance },
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("no adapter for that backend");
        let adapter_info = adapter.get_info();
        let al = adapter.limits();
        let limits = wgpu::Limits { max_buffer_size: al.max_buffer_size, ..wgpu::Limits::default() };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: None,
            required_features: wgpu::Features::empty(),
            required_limits: limits,
            ..Default::default()
        }))
        .expect("device");
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);
        let size = window.inner_size();
        let present_mode = if o.vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: Default::default(),
        };
        surface.configure(&device, &config);
        let depth = make_depth(&device, config.width, config.height);

        let buf = |label, data: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: data, usage })
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
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
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
        let surf_bufs = [Some(vb(&POS)), Some(vb(&NRM))];
        let wire_bufs = [Some(vb(&POS))];
        let target = [Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })];
        let mk = |label, vs, fs, bufs: &[Option<wgpu::VertexBufferLayout>], topo, bias: i32| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), compilation_options: Default::default(), buffers: bufs },
                fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(fs), compilation_options: Default::default(), targets: &target }),
                primitive: wgpu::PrimitiveState { topology: topo, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState { constant: bias, slope_scale: if bias != 0 { 1.0 } else { 0.0 }, clamp: 0.0 },
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipe_surf = mk("surf", "vs_surf", "fs_surf", &surf_bufs, wgpu::PrimitiveTopology::TriangleList, 0);
        let pipe_surf_bias = mk("surf+bias", "vs_surf", "fs_surf", &surf_bufs, wgpu::PrimitiveTopology::TriangleList, 2);
        let pipe_wire = mk("wire", "vs_wire", "fs_wire", &wire_bufs, wgpu::PrimitiveTopology::LineList, 0);
        Gpu {
            window,
            surface,
            device,
            queue,
            config,
            depth,
            pipe_surf,
            pipe_surf_bias,
            pipe_wire,
            bind,
            ubuf,
            vp,
            vn,
            ib,
            eb,
            ntri_idx: sc.tri.len() as u32,
            nedge_idx: sc.edge.len() as u32,
            adapter_info,
        }
    }

    fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
        self.depth = make_depth(&self.device, w, h);
    }

    fn draw(&mut self, cam: &Camera, mode: Mode) -> bool {
        use wgpu::CurrentSurfaceTexture as C;
        let frame = match self.surface.get_current_texture() {
            C::Success(f) | C::Suboptimal(f) => f,
            C::Outdated | C::Lost => {
                self.surface.configure(&self.device, &self.config);
                return false;
            }
            _ => return false,
        };
        let aspect = self.config.width as f32 / self.config.height as f32;
        let (view_proj, view) = cam.matrices(aspect);
        let u = Uniforms {
            view_proj,
            view,
            base: [0.72, 0.74, 0.78, 1.0],
            wire: if mode == Mode::Wire { [0.85, 0.87, 0.9, 1.0] } else { [0.05, 0.06, 0.08, 1.0] },
            flat_shading: (mode == Mode::Solid) as u32,
            _pad: [0; 3],
        };
        self.queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&u));
        let view_tex = frame.texture.create_view(&Default::default());
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view_tex,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.045, g: 0.05, b: 0.06, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind, &[]);
            if mode != Mode::Wire {
                pass.set_pipeline(if mode == Mode::ShadedWire { &self.pipe_surf_bias } else { &self.pipe_surf });
                pass.set_vertex_buffer(0, self.vp.slice(..));
                pass.set_vertex_buffer(1, self.vn.slice(..));
                pass.set_index_buffer(self.ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.ntri_idx, 0, 0..1);
            }
            if mode == Mode::Wire || mode == Mode::ShadedWire {
                pass.set_pipeline(&self.pipe_wire);
                pass.set_vertex_buffer(0, self.vp.slice(..));
                pass.set_index_buffer(self.eb.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.nedge_idx, 0, 0..1);
            }
        }
        self.queue.submit([enc.finish()]);
        self.window.pre_present_notify();
        self.queue.present(frame);
        true
    }
}

struct App {
    o: Opts,
    t0: Instant,
    scene: Scene,
    gpu: Option<Gpu>,
    cam: Camera,
    mode: Mode,
    drag: Option<MouseButton>,
    cursor: [f64; 2],
    shift: bool,
    first_frame_ms: Option<f64>,
    frames: Vec<f64>,
    last: Option<Instant>,
    t_gpu_ready: f64,
    mem_mb: Option<f64>,
}

impl App {
    fn finish_bench(&mut self, el: &ActiveEventLoop) {
        let mut v = self.frames.clone();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let pct = |q: f64| v[((v.len() - 1) as f64 * q) as usize];
        let info = &self.gpu.as_ref().unwrap().adapter_info;
        println!(
            "{{\"adapter\":\"{}\",\"backend\":\"{:?}\",\"mode\":\"{:?}\",\"npoints\":{},\"tris\":{},\"edges\":{},\"cook_ms\":{:.1},\"layout_ms\":{:.1},\"normals_ms\":{:.1},\"gpu_ready_ms\":{:.1},\"first_frame_ms\":{:.1},\"frames\":{},\"sync\":{},\"median_frame_ms\":{:.2},\"p95_frame_ms\":{:.2},\"min_frame_ms\":{:.2},\"allocator_mb\":{}}}",
            info.name,
            info.backend,
            self.mode,
            self.scene.npoints,
            self.scene.ntris,
            self.scene.nedges,
            self.scene.t_cook,
            self.scene.t_layout,
            self.scene.t_normals,
            self.t_gpu_ready,
            self.first_frame_ms.unwrap_or(0.0),
            v.len(),
            self.o.sync,
            pct(0.5),
            pct(0.95),
            v[0],
            self.mem_mb.map(|m| format!("{m:.0}")).unwrap_or("null".into())
        );
        el.exit();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }
        let w = el
            .create_window(Window::default_attributes().with_title("MiniHou view (wgpu spike)").with_window_level(winit::window::WindowLevel::AlwaysOnTop).with_position(winit::dpi::PhysicalPosition::new(100, 100)).with_inner_size(winit::dpi::PhysicalSize::new(self.o.size.0, self.o.size.1)))
            .unwrap();
        let g = Gpu::new(Arc::new(w), &self.o, &self.scene);
        self.t_gpu_ready = self.t0.elapsed().as_secs_f64() * 1000.0;
        g.window.request_redraw();
        self.gpu = Some(g);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        let Some(gpu) = self.gpu.as_mut() else { return };
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => gpu.resize(s.width, s.height),
            WindowEvent::ModifiersChanged(m) => self.shift = m.state().shift_key(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => match event.logical_key.as_ref() {
                Key::Named(NamedKey::Escape) => el.exit(),
                Key::Character("1") => self.mode = Mode::Solid,
                Key::Character("2") => self.mode = Mode::Shaded,
                Key::Character("3") => self.mode = Mode::Wire,
                Key::Character("4") => self.mode = Mode::ShadedWire,
                Key::Character("f") | Key::Character("F") => self.cam = Camera::frame(self.scene.center, self.scene.radius),
                _ => {}
            },
            WindowEvent::MouseInput { state, button, .. } => {
                self.drag = (state == ElementState::Pressed).then_some(button);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let now = [position.x, position.y];
                if let Some(b) = self.drag {
                    let (dx, dy) = ((now[0] - self.cursor[0]) as f32, (now[1] - self.cursor[1]) as f32);
                    if b == MouseButton::Left && !self.shift {
                        self.cam.yaw -= dx * 0.008;
                        self.cam.pitch = (self.cam.pitch + dy * 0.008).clamp(-1.55, 1.55);
                    } else {
                        // pan in the view plane
                        let f = norm(sub(self.cam.target, self.cam.eye()));
                        let s = norm(cross(f, [0.0, 1.0, 0.0]));
                        let u = cross(s, f);
                        let k = self.cam.dist * 0.0011;
                        for i in 0..3 {
                            self.cam.target[i] += (-s[i] * dx + u[i] * dy) * k;
                        }
                    }
                }
                self.cursor = now;
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                self.cam.dist = (self.cam.dist * (1.0 - y * 0.1)).max(self.scene.radius * 0.002);
            }
            WindowEvent::RedrawRequested => {
                if self.o.bench.is_some() {
                    self.cam.yaw += 0.01;
                }
                let drew = gpu.draw(&self.cam, self.mode);
                if self.first_frame_ms.is_none() && drew {
                    // wait for the GPU so the first frame is really done
                    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
                    self.first_frame_ms = Some(self.t0.elapsed().as_secs_f64() * 1000.0);
                    if let Some(r) = gpu.device.generate_allocator_report() {
                        self.mem_mb = Some(r.total_allocated_bytes as f64 / 1048576.0);
                    }
                    self.last = Some(Instant::now());
                } else if drew {
                    if self.o.sync {
                        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
                    }
                    let now = Instant::now();
                    if let Some(l) = self.last {
                        self.frames.push((now - l).as_secs_f64() * 1000.0);
                    }
                    self.last = Some(now);
                    if let Some(n) = self.o.bench {
                        // the first few frames after the first are warm-up
                        if self.frames.len() >= n + 10 {
                            self.frames.drain(..10);
                            return self.finish_bench(el);
                        }
                    }
                }
                gpu.window.request_redraw();
            }
            _ => {}
        }
    }
}

fn main() {
    let t0 = Instant::now();
    let o = parse_args();
    let scene = build_scene(&o);
    eprintln!(
        "scene: {} points, {} tris, {} edges | cook {:.0} ms, layout {:.0} ms, normals {:.0} ms",
        scene.npoints, scene.ntris, scene.nedges, scene.t_cook, scene.t_layout, scene.t_normals
    );
    let cam = Camera::frame(scene.center, scene.radius);
    let mode = o.mode;
    let el = EventLoop::new().unwrap();
    el.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        o,
        t0,
        scene,
        gpu: None,
        cam,
        mode,
        drag: None,
        cursor: [0.0; 2],
        shift: false,
        first_frame_ms: None,
        frames: Vec::new(),
        last: None,
        t_gpu_ready: 0.0,
        mem_mb: None,
    };
    el.run_app(&mut app).unwrap();
}
