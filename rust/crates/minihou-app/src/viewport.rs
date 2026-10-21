use eframe::{egui, egui_wgpu::{self, wgpu}};
use wgpu::util::DeviceExt;
use bytemuck::{Pod, Zeroable};
use minihou_core::{Geometry, normals, preview};
use minihou_core::lod::{ClusterLod, ClusterId};
use minihou_app::Camera;
use minihou_app::materials::{self, Registry, State};
use std::collections::{BTreeMap,HashMap,HashSet};

#[path = "display_modes.rs"]
pub mod display_modes;
#[path = "face_markers.rs"]
mod face_markers;
pub use display_modes::{Mode, ObjectVisibility, MatCap, ObjectColor};
use display_modes::split_indices;
mod point_colors;
use point_colors::point_colors;
mod scene;
use scene::element_indices;
mod elements;
use elements::{Element, Elements, points_visible, split_point_crosses};
mod gpu_setup;
mod mesh_buffers;
mod textures;
mod overlays;
mod camera_grid;
mod draw;
mod uploads;
use uploads::{MeshSource, MeshChunk, upload_chunks, box_chunks, capped_normal_vertices, chunked_scene, neutral_color_len};
pub use camera_grid::Target;
use camera_grid::{DEPTH, inverse_projection};
use textures::TextureBatch;
use mesh_buffers::{Boxes, Partition, edge_world_of, sharp_edges, index_buffer, object_mask_indices, packed_cut};

const SKETCHUP_CREASE_DEGREES: f32 = 30.0;

/// The Null's three world-space axes, centred on its origin.
pub fn null_crosshair(size: f32) -> [[[f32; 3]; 2]; 3] {
    let radius = size.max(0.0);
    std::array::from_fn(|axis| {
        let mut a = [0.0; 3];
        let mut b = [0.0; 3];
        a[axis] = -radius;
        b[axis] = radius;
        [a, b]
    })
}

#[cfg(test)]
#[test]
fn null_crosshair_scales_all_three_axes() {
    let small = null_crosshair(1.0);
    let large = null_crosshair(2.0);
    for axis in 0..3 {
        assert_eq!(small[axis][0][axis], -1.0);
        assert_eq!(large[axis][1][axis], 2.0);
        for endpoint in 0..2 {
            for other in 0..3 {
                if axis != other { assert_eq!(large[axis][endpoint][other], 0.0); }
            }
        }
    }
}

/// Renderer switches; stats and the orientation widget are painted by the UI.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Overlay { pub grid: bool, pub axes: bool, pub geometry: crate::ui::Overlays, pub light_background: bool, pub normals: bool, pub bounds: bool, pub xray: bool, pub wireframe: bool, pub visibility: ObjectVisibility, pub matcap: MatCap, pub matcap_flat: bool, pub object_color: ObjectColor, pub textures: bool }

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TexVertex { p:[f32;3], n:[f32;3], color:[f32;3], uv:[f32;2], valid:f32 }
#[derive(Clone)]
struct SurfaceTri { vertices:[TexVertex;3], points:[u32;3], prim:u32, slot:String, uv_sets:BTreeMap<String,[[f32;2];3]> }

pub struct Scene {
    p: Vec<f32>, n: Vec<f32>, point: Vec<u32>, line: Vec<u32>, tri: Vec<u32>, edge: Vec<u32>,
    pub pick_bvh: std::sync::Arc<crate::picking::Bvh>,
    /// Normal overlay: segment end points (positions) and the matching normals; two points per segment.
    nlp: Vec<f32>, nln: Vec<f32>, normal_points: Vec<u32>,
    /// The eight corners of the bounding box.
    bbp: [f32; 24],
    pub center: [f32;3], pub radius: f32,
    textured:Vec<SurfaceTri>,
    /// Per-point display colour from the `Cd` attribute (point, vertex or primitive domain, averaged onto points); None without Cd.
    cd:Option<Vec<f32>>,
}
/// Corner order of `Scene::bbp`; the 12 edges as index pairs.
const BOX_EDGES: [u32; 24] = [0,1, 1,3, 3,2, 2,0, 4,5, 5,7, 7,6, 6,4, 0,4, 1,5, 2,6, 3,7];
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    base: [f32; 4],
    wire: [f32; 4],
    flat_shading: u32,
    wire_fade: f32,
    _pad: [u32; 2],
    object_color: u32,
    _pad_color: [u32; 3],
    floor:[f32;4], element_size:[f32;4], element_flags:[u32;4], element_color:[f32;4],
}

pub struct Gpu {
    tex_bgl:wgpu::BindGroupLayout, default_tex_bind:wgpu::BindGroup, pipe_tex:wgpu::RenderPipeline, pipe_tex_bias:wgpu::RenderPipeline, pipe_tex_xray:wgpu::RenderPipeline,
    tex_batches:Vec<TextureBatch>, texture_status:Vec<String>,
    gpu_images:HashMap<String,(wgpu::TextureView,usize,u64)>, gpu_tick:u64,
    pipe_surf: wgpu::RenderPipeline, pipe_surf_bias: wgpu::RenderPipeline, pipe_wire: wgpu::RenderPipeline,
    pipe_xray: wgpu::RenderPipeline, pipe_normals: wgpu::RenderPipeline, pipe_bounds: wgpu::RenderPipeline, pipe_select: wgpu::RenderPipeline,
    pipe_floor:wgpu::RenderPipeline, z_axis:wgpu::Buffer, pipe_z_axis:wgpu::RenderPipeline,
    face_markers:face_markers::FaceMarkers, pipe_face_markers:wgpu::RenderPipeline, pipe_face_markers_xray:wgpu::RenderPipeline,
    elements:Elements, element_points:Vec<u32>, pipe_elements:wgpu::RenderPipeline, pipe_elements_ghost:wgpu::RenderPipeline,
    pipe_depth: wgpu::RenderPipeline, pipe_wire_all: wgpu::RenderPipeline,
    pipe_wire_hidden: wgpu::RenderPipeline, pipe_ghost: wgpu::RenderPipeline, pipe_ghost_wire: wgpu::RenderPipeline,
    partition_tri: Partition, partition_edge: Partition, sharp_edge: Partition,
    current_tri: Vec<u32>, current_edge: Vec<u32>, sharp_edge_indices: Vec<u32>,
    boxes: Option<Boxes>, object_ranges: Vec<std::ops::Range<u32>>, object_ids: Vec<String>,
    matcap: wgpu::Texture, matcap_loaded: std::sync::atomic::AtomicU8,
    pipe_mask: wgpu::RenderPipeline, pipe_outline: wgpu::RenderPipeline, bgl_outline: wgpu::BindGroupLayout,
    /// Selection highlight: face triangles and line segments (positions, with a zero normal buffer of the same length).
    group_face:wgpu::Buffer,group_face_n:u32,group_line:wgpu::Buffer,group_line_n:u32,group_point:wgpu::Buffer,group_point_n:u32,
    pipe_group_face:wgpu::RenderPipeline,pipe_group_line:wgpu::RenderPipeline,pipe_group_point:wgpu::RenderPipeline,
    barrier_face:wgpu::Buffer,barrier_face_n:u32,barrier_line:wgpu::Buffer,barrier_line_n:u32,
    pipe_barrier_face:wgpu::RenderPipeline,pipe_barrier_line:wgpu::RenderPipeline,
    target_face:wgpu::Buffer,target_face_n:u32,target_line:wgpu::Buffer,target_line_n:u32,
    pipe_target_face:wgpu::RenderPipeline,pipe_target_line:wgpu::RenderPipeline,
    sel_face: wgpu::Buffer, sel_face_n: u32, sel_line: wgpu::Buffer, sel_line_n: u32, zeros: wgpu::Buffer,
    nl_p: wgpu::Buffer, nl_n: wgpu::Buffer, nl_count: u32, nl_selected: u32, bb_p: wgpu::Buffer, bb_n: wgpu::Buffer, bb_i: wgpu::Buffer, bb_chunks: Vec<MeshChunk>,
    bind: wgpu::BindGroup, ubuf: wgpu::Buffer,
    vp: wgpu::Buffer, vn: wgpu::Buffer, vc: wgpu::Buffer, vcd: Option<wgpu::Buffer>, neutral_color: wgpu::Buffer,
    limits: minihou_core::limits::Limits,
    mesh_source: Option<MeshSource>,
    face_chunks: Vec<MeshChunk>, edge_chunks: Vec<MeshChunk>, sharp_edge_chunks: Vec<MeshChunk>, mask_chunks: Vec<MeshChunk>,
    ntri_idx: u32,
    object_mask: Option<wgpu::Buffer>, object_mask_n: u32,
    object_mask_points: Option<Vec<std::ops::Range<u32>>>,
    /// Average edge length in world units: sizes the edge fade.
    edge_world: f32,
}

#[cfg(test)]
mod tests {
    #[test]
    fn points_pass_follows_mode_and_overlay() {
        use crate::ui::{Overlays,SelectionMode};
        let off=Overlays{enabled:false,..Default::default()};
        assert!(!super::points_visible(&off,SelectionMode::Object));
        assert!(super::points_visible(&off,SelectionMode::Points));
        assert!(!super::points_visible(&Overlays::default(),SelectionMode::Object));
        assert!(super::points_visible(&Overlays::default(),SelectionMode::Edges));
        assert!(!super::points_visible(&Overlays{points:false,..Default::default()},SelectionMode::Edges));
    }

fn cube(extra:bool) -> Geometry {
        let mut p=vec![0.,0.,0.,1.,0.,0.,1.,1.,0.,0.,1.,0.,0.,0.,1.,1.,0.,1.,1.,1.,1.,0.,1.,1.];
        if extra { p.extend([5.,5.,5.]); }
        let v=vec![0,3,2,1,4,5,6,7,0,1,5,4,2,3,7,6,0,4,7,3,1,2,6,5];
        Geometry::from_arrays(p,v,vec![0,4,8,12,16,20,24],Some(vec![true;6])).unwrap()
    }
    fn kinds(g:&Geometry) -> (Vec<u32>,Vec<u32>) {
        let (inst,_)=Elements::instances(&Scene::new(g),&[],&[]);
        let k=|n| inst.iter().filter(|e| e.kind==n).count() as u32;
        // stray points (kind 1) visible in object mode; kind 2 hidden there; kind 0 are stray edges
        (vec![k(0),k(1)],vec![k(2)])
    }
    #[test] fn object_mode_cube_points_all_hidden() {
        let g=cube(false);
        assert_eq!(kinds(&g),(vec![0,0],vec![8]));
    }
    #[test] fn sketchup_clean_keeps_only_edges_above_the_face_angle_threshold() {
        let box_scene=Scene::new(&cube(false));
        assert_eq!(sharp_edges(&box_scene,30.).len()/2,12,"all twelve box creases must remain");
        let bend=|degrees:f32| {
            let theta=degrees.to_radians();
            let p=vec![0.,0.,0., 1.,0.,0., 1.,1.,0., 0.,1.,0.,
                1.,0.,0., 1.+theta.cos(),0.,theta.sin(),
                1.+theta.cos(),1.,theta.sin(), 1.,1.,0.];
            let g=Geometry::from_arrays(p,vec![0,1,2,3,4,5,6,7],vec![0,4,8],Some(vec![true;2])).unwrap();
            Scene::new(&g)
        };
        assert!(sharp_edges(&bend(20.),30.).is_empty(),"smooth seam and open boundaries must be hidden");
        let sharp=sharp_edges(&bend(45.),30.);
        assert_eq!(sharp.len(),2,"only the shared crease should be drawn");
        let scene=bend(45.);
        let a=&scene.p[sharp[0] as usize*3..sharp[0] as usize*3+3];
        let b=&scene.p[sharp[1] as usize*3..sharp[1] as usize*3+3];
        assert_eq!(a[0],1.); assert_eq!(b[0],1.);
    }
    #[test] fn object_mode_shows_only_stray_point() {
        let g=cube(true);
        assert_eq!(kinds(&g),(vec![0,1],vec![8]));
    }
    #[test] fn object_mode_shows_only_stray_edge() {
        let p=vec![0.,0.,0.,1.,0.,0.,1.,1.,0.,0.,1.,0.,3.,0.,0.,4.,0.,0.];
        let g=Geometry::from_arrays(p,vec![0,1,2,3,4,5],vec![0,4,6],Some(vec![true,false])).unwrap();
        // the edge's two ends are attached to it, so they are hidden; the quad's corners too
        assert_eq!(kinds(&g),(vec![1,0],vec![6]));
    }

    use super::*;
    use minihou_core::lod::{Cluster, Sphere};

    #[test]
    fn floor_ray_inverse_handles_perspective_and_ortho() {
        let mut camera=Camera::frame([1.,2.,3.],2.);
        for ortho in [false,true] {
            if ortho {camera.set_view(minihou_app::StandardView::Top);}
            let m=camera.matrices(1.7).0;
            let inv=inverse_projection(m);
            for c in 0..4 {for r in 0..4 {
                let actual=(0..4).map(|k|m[k][r]*inv[c][k]).sum::<f32>();
                assert!((actual-if c==r {1.}else{0.}).abs()<1e-3,"{ortho} {c} {r}: {actual}");
            }}
        }
    }

    #[test]
    fn asymmetric_f_texture_uses_blender_uv_origin() {
        let instance=wgpu::Instance::default();
        let Ok(adapter)=pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {return};
        let Ok((device,queue))=pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) else {return};
        let p=vec![-1.,-1.,0., 1.,-1.,0., 1.,1.,0., -1.,1.,0.];
        let uv=[[0.,0.],[1.,0.],[1.,1.],[0.,1.]];
        let tex_vertex=|i:usize|TexVertex {p:[p[3*i],p[3*i+1],p[3*i+2]],n:[0.,0.,1.],color:[1.;3],uv:uv[i],valid:1.};
        let textured=[[0,1,2],[0,2,3]].into_iter().enumerate().map(|(prim,ids)|SurfaceTri {
            vertices:ids.map(tex_vertex),points:ids.map(|i|i as u32),prim:prim as u32,slot:"F".into(),
            uv_sets:BTreeMap::from([("uv".into(),ids.map(|i|uv[i]))]),
        }).collect();
        let scene=Scene {p,n:vec![0.,0.,1.].repeat(4),tri:vec![0,1,2,0,2,3],edge:vec![0,1,1,2,2,3,3,0],
            point:Vec::new(),line:Vec::new(),pick_bvh:std::sync::Arc::new(crate::picking::Bvh::new(&[],&[])),
            nlp:Vec::new(),nln:Vec::new(),normal_points:Vec::new(),bbp:[0.;24],center:[0.;3],radius:1.5,textured,cd:None};
        let mut gpu=Gpu::new(&device,&scene);
        let mut registry=Registry::new(".");
        // The red F has a top bar and left stem. A V mirror puts its bar at the bottom.
        let mut pixels=vec![0u8;4*4*4];
        for y in 0..4 {for x in 0..4 {let i=(y*4+x)*4; pixels[i]=if y==0 || x==0 || (y==2 && x<3) {255}else{0};pixels[i+3]=255;}}
        for bottom_up in [false,true] {
            let mut rows=pixels.clone();
            if bottom_up {for y in 0..2 {for x in 0..16 {rows.swap(y*16+x,(3-y)*16+x);}}}
            let hash=registry.register_pixels(4,4,rows,bottom_up).unwrap();
            registry.register_descriptor("F",materials::Descriptor::parse(&serde_json::json!({"version":1,
                "textures":{"base_color":{"image_id":hash,"sampler":{"min":"nearest","mag":"nearest","wrap_u":"clamp","wrap_v":"clamp"}}}})).unwrap());
            gpu.upload_textures(&device,&queue,&scene,&mut registry);
            let (target,color)=Target::new_headless(&device,[64,64]);
            let mut camera=Camera::frame([0.;3],1.5);camera.set_view(minihou_app::StandardView::Top);
            gpu.draw_raw(&device,&queue,&target,&camera,Mode::FlatShaded,Overlay {textures:true,..Default::default()},crate::ui::SelectionMode::Object,false);
            let buffer=device.create_buffer(&wgpu::BufferDescriptor {label:Some("F readback"),size:256*64,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
            let mut encoder=device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(wgpu::TexelCopyTextureInfo {texture:&color,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},
                wgpu::TexelCopyBufferInfo {buffer:&buffer,layout:wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(256),rows_per_image:Some(64)}},
                wgpu::Extent3d {width:64,height:64,depth_or_array_layers:1});
            queue.submit([encoder.finish()]);
            let (tx,rx)=std::sync::mpsc::channel();buffer.slice(..).map_async(wgpu::MapMode::Read,move |r|{tx.send(r).unwrap();});
            let _=device.poll(wgpu::PollType::wait_indefinitely());rx.recv().unwrap().unwrap();
            let bytes=buffer.slice(..).get_mapped_range();
            let red=|x:usize,y:usize|bytes[y*256+x*4];
            assert!((red(37,18) as u16)>(red(37,46) as u16)+40,"F top bar should be above its empty lower-right, bottom_up={bottom_up}");
        }
    }

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
    fn whole_mesh_face_selection_headless() {
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None, force_fallback_adapter: false,
        })) else { eprintln!("No wgpu adapter available, skipping selection render test"); return; };
        eprintln!("Selection render adapter: {:?}", adapter.get_info());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let graph = minihou_core::Graph::from_json(include_str!("test_data/demo_graph.json")).unwrap();
        let scene = Scene::new(&graph.cook(Some("output1")).unwrap());
        let selection = crate::picking::Selection {
            mode: crate::ui::SelectionMode::Faces,
            tris: (0..scene.triangles() as u32).collect(), ..Default::default()
        };
        let (faces, lines) = crate::picking::highlight(scene.positions(), scene.triangle_indices(), &selection, 0.01);
        assert_eq!(faces.len(), scene.triangle_indices().len() * 3);
        let mut gpu = Gpu::new(&device, &scene);
        let (target, _texture) = Target::new_headless(&device, [64, 64]);
        let mut camera = Camera::frame(scene.center, scene.radius);
        // Both initial upload and scene replacement must support the expanded highlight.
        for replace_scene in [false, true] {
            if replace_scene { gpu.upload(&device, &scene);camera.set_view(minihou_app::StandardView::Top); }
            gpu.set_selection(&device, &faces, &lines);
            device.push_error_scope(wgpu::ErrorFilter::Validation);
            gpu.draw_raw(&device, &queue, &target, &camera, Mode::Solid,
                Overlay {grid:true,axes:true,..Default::default()}, crate::ui::SelectionMode::Faces, false);
            let error = pollster::block_on(device.pop_error_scope());
            assert!(error.is_none(), "whole-mesh face selection must render: {error:?}");
        }
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
        // Include an internal diagonal so the test catches any orange wire overlay.
        let edge = vec![0, 1, 1, 2, 2, 3, 3, 0, 0, 2];
        let scene = Scene {
            p, n, tri, edge, point:Vec::new(), line:Vec::new(),
            pick_bvh:std::sync::Arc::new(crate::picking::Bvh::new(&[],&[])),
            nlp: Vec::new(), nln: Vec::new(), normal_points:Vec::new(), bbp: [0.0; 24],
            center: [0.0, 0.0, 0.0], radius: 1.5,
            textured:Vec::new(), cd:None,
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
            Overlay { wireframe:true, ..Default::default() },
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
        // #d97812 object outline; element selection remains #ffa028.
        let is_outline = |rgba: [u8; 4]| (212..=222).contains(&rgba[0]) && (115..=125).contains(&rgba[1]) && (13..=23).contains(&rgba[2]);

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



#[cfg(test)]
#[path="display_render_tests.rs"] mod display_render_tests;

#[cfg(test)]
#[path="geometry_render_tests.rs"] mod geometry_render_tests;

#[cfg(test)]
#[path="group_render_tests.rs"] mod group_render_tests;
#[cfg(test)]
mod cd_scene_tests {
    use super::*;
    use minihou_core::geometry::Attr;
    use std::sync::Arc;
    fn tri() -> Geometry { Geometry::from_arrays(vec![0.,0.,0.,1.,0.,0.,0.,1.,0.],vec![0,1,2],vec![0,3],Some(vec![true])).unwrap() }
    #[test] fn scene_without_cd_keeps_default_colour() { assert!(Scene::new(&tri()).cd.is_none()); }
    #[test] fn point_cd_is_the_scene_colour() {
        let mut g=tri(); let c=vec![1.,0.,0., 0.,1.,0., 0.,0.,1.];
        g.point.insert("Cd".into(),Arc::new(Attr::f32(3,c.clone())));
        assert_eq!(Scene::new(&g).cd.unwrap(),c);
    }
    #[test] fn vertex_and_face_cd_reach_the_points() {
        let mut g=tri(); g.vertex.insert("Cd".into(),Arc::new(Attr::f32(3,vec![1.,0.,0., 0.,1.,0., 0.,0.,1.])));
        assert_eq!(Scene::new(&g).cd.unwrap(),vec![1.,0.,0., 0.,1.,0., 0.,0.,1.]);
        let mut g=tri(); g.prim.insert("Cd".into(),Arc::new(Attr::f32(3,vec![0.2,0.4,0.6])));
        assert_eq!(Scene::new(&g).cd.unwrap(),vec![0.2,0.4,0.6].repeat(3));
    }
}
