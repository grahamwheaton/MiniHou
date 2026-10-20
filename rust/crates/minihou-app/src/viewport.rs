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
/// Longest overlay normal list kept: more points than this are thinned evenly.
const NORMAL_LINES: usize = 60_000;
/// Corner order of `Scene::bbp`; the 12 edges as index pairs.
const BOX_EDGES: [u32; 24] = [0,1, 1,3, 3,2, 2,0, 4,5, 5,7, 7,6, 6,4, 0,4, 1,5, 2,6, 3,7];
impl Scene {
    /// Add a selected node's preview after the displayed outputs, preserving their point ids.
    pub fn append_selected(&mut self, other: &Scene) -> std::ops::Range<u32> {
        let start = (self.p.len() / 3) as u32;
        self.p.extend_from_slice(&other.p);
        self.n.extend_from_slice(&other.n);
        self.point.extend(other.point.iter().map(|id| id + start));
        self.line.extend(other.line.iter().map(|id| id + start));
        self.tri.extend(other.tri.iter().map(|id| id + start));
        self.edge.extend(other.edge.iter().map(|id| id + start));
        self.normal_points.extend(other.normal_points.iter().map(|id| id + start));
        self.nlp.extend_from_slice(&other.nlp);
        self.nln.extend_from_slice(&other.nln);
        for tri in &other.textured {
            let mut tri = tri.clone();
            tri.points = tri.points.map(|id| id + start);
            self.textured.push(tri);
        }
        if self.cd.is_some() || other.cd.is_some() {
            let colors = self.cd.get_or_insert_with(|| vec![0.72; start as usize * 3]);
            colors.extend(other.cd.as_deref().map_or_else(|| vec![0.72; other.p.len()], |cd| cd.to_vec()));
        }
        let (lo, hi) = crate::picking::position_bounds(&self.p);
        self.center = [0, 1, 2].map(|k| (lo[k] + hi[k]) * 0.5);
        self.radius = (0..3).map(|k| (hi[k] - lo[k]).powi(2)).sum::<f32>().sqrt().max(1e-3) * 0.5;
        for c in 0..8 { for k in 0..3 { self.bbp[c*3+k] = if (c >> k) & 1 == 1 { hi[k] } else { lo[k] }; } }
        self.pick_bvh = std::sync::Arc::new(crate::picking::Bvh::new(&self.p, &self.tri));
        start..(self.p.len() / 3) as u32
    }
    /// The same scene showing only `level` (0..1) of its triangles and edges (the viewport proxy); points, normals and bounds stay.
    pub fn proxied(&self, level: f32) -> Scene {
        Scene {
            p: self.p.clone(), n: self.n.clone(), point:self.point.clone(), line:self.line.clone(), pick_bvh:self.pick_bvh.clone(),
            tri: crate::ui::proxy_ui::proxy::reduced_triangles(&self.tri, level),
            edge: crate::ui::proxy_ui::proxy::reduced_edges(&self.edge, level),
            nlp: self.nlp.clone(), nln: self.nln.clone(), normal_points:self.normal_points.clone(), bbp: self.bbp, center: self.center, radius: self.radius, textured:self.textured.clone(), cd:self.cd.clone(),
        }
    }
    pub fn triangles(&self) -> usize { self.tri.len() / 3 }
    pub fn has_textured_slots(&self) -> bool { self.textured.iter().any(|t|!t.slot.is_empty()) }
    pub fn positions(&self) -> &[f32] { &self.p }
    pub fn normals(&self) -> &[f32] { &self.n }
    pub fn point_indices(&self) -> &[u32] { &self.point }
    pub fn line_primitive_indices(&self) -> &[u32] { &self.line }
    pub fn triangle_indices(&self) -> &[u32] { &self.tri }
    /// Polygon edges as point pairs (no fan diagonals): element selection rebuilds polygons from these.
    pub fn edge_indices(&self) -> &[u32] { &self.edge }
    pub fn new(g: &Geometry) -> Self {
        // Packed primitives / native instances: drawn at their transforms, CPU-batched into this scene's one buffer.
        if minihou_core::nodes::packed::instance_count(g) > 0 {
            if let Ok(shown) = minihou_core::nodes::packed::display_geometry(g) { return Self::new(&shown); }
        }
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
        let (mut nlp, mut nln, mut normal_points) = (Vec::new(), Vec::new(), Vec::new());
        for i in (0..g.npoints).step_by(step) {
            let (pp, nn) = (&p[i*3..i*3+3], &n[i*3..i*3+3]);
            normal_points.push(i as u32);
            nlp.extend_from_slice(pp);
            nlp.extend_from_slice(&[pp[0] + nn[0]*len, pp[1] + nn[1]*len, pp[2] + nn[2]*len]);
            nln.extend_from_slice(nn); nln.extend_from_slice(nn);
        }
        let mut bbp = [0f32; 24];
        if g.npoints > 0 {
            let (lo, hi) = preview::bounds(p);
            for c in 0..8 { for k in 0..3 { bbp[c*3+k] = if (c >> k) & 1 == 1 { hi[k] } else { lo[k] }; } }
        }
        let (point,line)=element_indices(g,&l.tri);
        let provenance=preview::triangulate_geometry(g);
        let names=g.detail.get("materials").and_then(|v|v.as_array());
        let bindings=g.detail.get("material_bindings").and_then(|v|v.get("slots")).and_then(|v|v.as_array());
        let corner_n=g.vertex.get("N").and_then(|a|a.as_f32());
        let mut textured=Vec::with_capacity(provenance.prim.len());
        for (ti,&slot) in provenance.material_slot.iter().enumerate() {
            let slot_index=usize::try_from(slot).ok();
            let name=slot_index.and_then(|i|bindings.and_then(|b|b.get(i)).and_then(|v|v.as_str())
                .or_else(||names.and_then(|n|n.get(i)).and_then(|v|v.as_str()))).unwrap_or("").to_owned();
            let vertices=std::array::from_fn(|k| { let i=ti*3+k; let point=provenance.point[i] as usize; let corner=provenance.corner[i] as usize;
                let uv=provenance.uv_sets.get("uv").and_then(|v|v.get(i*2..i*2+2)).map(|v|[v[0],v[1]]).unwrap_or([f32::NAN;2]);
                let valid=(uv[0].is_finite() && uv[1].is_finite()) as u8 as f32;
                let normal=corner_n.and_then(|v|v.get(corner*3..corner*3+3)).unwrap_or(&n[point*3..point*3+3]);
                TexVertex {p:[p[point*3],p[point*3+1],p[point*3+2]],n:[normal[0],normal[1],normal[2]],color:[0.72;3],uv:if valid>0. {uv} else {[0.;2]},valid}
            });
            let uv_sets=provenance.uv_sets.iter().map(|(name,values)| (name.clone(),std::array::from_fn(|k|[values[(ti*3+k)*2],values[(ti*3+k)*2+1]]))).collect();
            textured.push(SurfaceTri { vertices,points:[provenance.point[ti*3],provenance.point[ti*3+1],provenance.point[ti*3+2]],prim:provenance.prim[ti],slot:name,uv_sets });
        }
        let pick_bvh=std::sync::Arc::new(crate::picking::Bvh::new(p,&l.tri));
        Self { p:p.to_vec(),n,point,line,tri:l.tri,edge:l.edge,pick_bvh,nlp,nln,normal_points,bbp,center,radius,textured,cd:point_colors(g) }
    }
}

#[test]
fn cube_seams_expand_to_triangle_corners() {
    let graph=minihou_core::Graph::from_json(include_str!("test_data/textured_cube.minihou")).unwrap();
    let geo=graph.cook(Some("output1")).unwrap(); let scene=Scene::new(&geo);
    assert_eq!(scene.positions().len()/3,8);
    assert_eq!(scene.textured.len(),12);
    assert_eq!(scene.textured.len()*3,36);
    assert_eq!(scene.textured.iter().map(|t|t.prim).collect::<std::collections::BTreeSet<_>>().len(),6);
    assert!(scene.textured.iter().all(|t|t.vertices.iter().all(|v|v.valid==1.)));
    let mut registry=Registry::new(concat!(env!("CARGO_MANIFEST_DIR"),"/src/test_data"));
    let document:serde_json::Value=serde_json::from_str(include_str!("test_data/textured_cube.minihou")).unwrap();
    assert!(registry.register_graph(&document).is_empty());
    let binding=registry.resolve("SampleChecker");
    assert!(matches!(binding.state,materials::State::Ok));
    assert_eq!((binding.image.unwrap().width,binding.descriptor.unwrap().textures.base_color.unwrap().uv_set),(64,"uv".to_owned()));
}
#[test]
fn instance_slot_name_resolves_to_manifest_binding() {
    let mut geo=minihou_core::geometry::Geometry::from_arrays(
        vec![0.,0.,0.,1.,0.,0.,0.,1.,0.],vec![0,1,2],vec![0,3],Some(vec![true])).unwrap();
    geo.detail.insert("materials".into(),serde_json::json!(["imagepaste-261017-142014"]));
    geo.detail.insert("material_bindings".into(),serde_json::json!({"version":1,"slots":["peer3:blender:instance:0"]}));
    let scene=Scene::new(&geo);
    assert_eq!(scene.textured.len(),1);
    assert_eq!(scene.textured[0].slot,"peer3:blender:instance:0");
    let mut registry=Registry::new(".");
    registry.register_descriptor("peer3:blender:instance:0",materials::Descriptor::parse(
        &serde_json::json!({"version":1,"name":"imagepaste-261017-142014"})).unwrap());
    assert!(matches!(registry.resolve(&scene.textured[0].slot).state,State::Ok));
}
/// `Cd` as one RGB per point: point Cd as is; vertex and primitive Cd averaged onto the points they touch.
fn point_colors(g:&Geometry) -> Option<Vec<f32>> {
    let t=&g.topology; let n=g.npoints;
    let rgb=|a:&minihou_core::geometry::Attr,i:usize| -> Option<[f32;3]> {
        let v=a.as_f32()?; let w=a.width; if w<3 { return None; }
        let s=v.get(i*w..i*w+3)?; Some([s[0],s[1],s[2]])
    };
    if let Some(a)=g.point.get("Cd") { if a.as_f32().is_some() && a.width>=3 && a.rows()>=n {
        return Some((0..n).flat_map(|i|rgb(a,i).unwrap_or([0.72;3])).collect());
    } }
    let (mut sum,mut cnt)=(vec![0f32;n*3],vec![0u32;n]);
    let mut add=|pt:i32,c:[f32;3]| { let k=pt as usize; if k<n { for j in 0..3 { sum[k*3+j]+=c[j]; } cnt[k]+=1; } };
    if let Some(a)=g.vertex.get("Cd") { for (i,&pt) in t.vertex_point.iter().enumerate() { if let Some(c)=rgb(a,i) { add(pt,c); } } }
    else if let Some(a)=g.prim.get("Cd") { for prim in 0..t.nprims() {
        if let Some(c)=rgb(a,prim) { for &pt in &t.vertex_point[t.prim_offsets[prim] as usize..t.prim_offsets[prim+1] as usize] { add(pt,c); } }
    } }
    else { return None; }
    Some((0..n).flat_map(|i|{ let c=cnt[i].max(1) as f32; if cnt[i]==0 {[0.72;3]} else {[sum[i*3]/c,sum[i*3+1]/c,sum[i*3+2]/c]} }).collect())
}
/// Standalone points and non-surface primitive segments survive every proxy cut.
fn element_indices(g:&Geometry, triangles:&[u32]) -> (Vec<u32>,Vec<u32>) {
    let mut faced=vec![false;g.npoints];
    for &id in triangles { faced[id as usize]=true; }
    let point=(0..g.npoints).filter(|&i| !faced[i]).map(|i| i as u32).collect();
    let t=&g.topology;
    let mut line=Vec::new();
    for prim in 0..t.nprims() {
        let ids=&t.vertex_point[t.prim_offsets[prim] as usize..t.prim_offsets[prim+1] as usize];
        let curve=g.prim.get("__viewport_curve").is_some_and(|a| match &a.data {
            minihou_core::geometry::AttrData::I32(v)=>v.get(prim)==Some(&1),_=>false });
        if t.is_closed(prim) && ids.len()>=3 && !curve { continue; }
        for pair in ids.windows(2) { line.extend(pair.iter().map(|&i| i as u32)); }
        if t.is_closed(prim) && ids.len()>2 { line.extend([*ids.last().unwrap() as u32,ids[0] as u32]); }
    }
    (point,line)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Element { a:[f32;3], b:[f32;3], kind:u32, selected:u32 }
/// Instance data is expanded from stable point/segment indices, never from the LOD triangles.
/// Surface points (kind 2) draw in point selection mode, or when the overlays are on and the points overlay is on.
fn points_visible(o:&crate::ui::Overlays, mode:crate::ui::SelectionMode) -> bool { match mode { crate::ui::SelectionMode::Points=>true, crate::ui::SelectionMode::Object=>false, _=>o.enabled && o.points } }
/// Object-mode rule, in one place: points that are a face corner or an edge end are kind 2 (hidden in object mode);
/// only points used by no face and no edge are kind 1 (stray). Stray edges are the kind 0 segments.
fn point_kind(id:u32, loose:&std::collections::HashSet<u32>, edge_ends:&std::collections::HashSet<u32>) -> u32 {
    if loose.contains(&id) && !edge_ends.contains(&id) {1} else {2}
}

struct Elements { buffer:wgpu::Buffer, selected:u32, total:u32 }
/// Point previews arrive as three axis-aligned segments per point. Keep real edges
/// as lines and turn only that recognisable cross geometry into disc instances.
fn split_point_crosses(lines:&[f32]) -> (Vec<f32>,Vec<f32>) {
    let mut points=Vec::new(); let mut outlines=Vec::new();
    let mut offset=0;
    while offset+18<=lines.len() {
        let chunk=&lines[offset..offset+18];
        let centers:Vec<[f32;3]>=(0..3).map(|axis| std::array::from_fn(|k| (chunk[axis*6+k]+chunk[axis*6+3+k])*0.5)).collect();
        let cross=centers.iter().all(|c| c.iter().zip(centers[0]).all(|(a,b)| (a-b).abs()<1e-5))
            && (0..3).all(|axis| (0..3).all(|k| ((chunk[axis*6+k]-chunk[axis*6+3+k]).abs()>1e-6) == (k==axis)));
        if cross { points.extend_from_slice(&centers[0]); offset+=18; }
        else { outlines.extend_from_slice(&lines[offset..offset+6]); offset+=6; }
    }
    outlines.extend_from_slice(&lines[offset..]);
    (points,outlines)
}
#[cfg(test)]
#[test]
fn viewport_point_highlights_become_discs_without_losing_edges() {
    let edge=[0.,0.,0.,1.,0.,0.];
    let cross=[0.9,2.,3.,1.1,2.,3.,1.,1.9,3.,1.,2.1,3.,1.,2.,2.9,1.,2.,3.1];
    let mut lines=edge.to_vec(); lines.extend_from_slice(&cross);
    let (points,outlines)=split_point_crosses(&lines);
    assert_eq!(points,vec![1.,2.,3.]);
    assert_eq!(outlines,edge);
}
impl Elements {
    fn new(device:&wgpu::Device, scene:&Scene, ranges:&[std::ops::Range<u32>], selected_points:&[u32]) -> Self {
        let instances=Self::instances(scene,ranges,selected_points);
        let count=instances.1; let instances=instances.0;
        let buffer=device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some("point and curve instances"),contents:if instances.is_empty() { &[0u8;32] } else { bytemuck::cast_slice(&instances) },usage:wgpu::BufferUsages::VERTEX });
        Self {buffer,selected:count,total:instances.len() as u32}
    }
    fn instances(scene:&Scene, ranges:&[std::ops::Range<u32>], selected_points:&[u32]) -> (Vec<Element>,u32) {
        let mut instances=Vec::new(); let mut count=0;
        let edge_ends:std::collections::HashSet<u32>=scene.line_primitive_indices().iter().copied().collect();
        let loose:std::collections::HashSet<_>=scene.point_indices().iter().copied().collect();
        let picked:std::collections::HashSet<_>=selected_points.iter().copied().collect();
        let at=|id:u32| { let k=id as usize*3; [scene.p[k],scene.p[k+1],scene.p[k+2]] };
        for selected in [true,false] {
            let belongs=|ids:&[u32]| ranges.iter().any(|r| ids.iter().all(|id| r.contains(id)))==selected;
            for id in 0..(scene.p.len()/3) as u32 { if belongs(&[id]) {
                instances.push(Element { a:at(id), b:at(id), kind:point_kind(id,&loose,&edge_ends), selected:picked.contains(&id) as u32 });
            } }
            for pair in scene.line_primitive_indices().chunks_exact(2) { if belongs(pair) {
                instances.push(Element { a:at(pair[0]), b:at(pair[1]), kind:0, selected:0 });
            } }
            if selected { count=instances.len() as u32; }
        }
        (instances,count)
    }
}

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

fn inverse_projection(m:[[f32;4];4])->[[f32;4];4] {
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
    partition_tri: Partition, partition_edge: Partition, current_tri: Vec<u32>, current_edge: Vec<u32>,
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
    nl_p: wgpu::Buffer, nl_n: wgpu::Buffer, nl_count: u32, nl_selected: u32, bb_p: wgpu::Buffer, bb_n: wgpu::Buffer, bb_i: wgpu::Buffer,
    bind: wgpu::BindGroup, ubuf: wgpu::Buffer,
    vp: wgpu::Buffer, vn: wgpu::Buffer, vc: wgpu::Buffer, vcd: Option<wgpu::Buffer>, neutral_color: wgpu::Buffer,
    ntri_idx: u32,
    object_mask: Option<wgpu::Buffer>, object_mask_n: u32,
    object_mask_points: Option<Vec<std::ops::Range<u32>>>,
    /// Average edge length in world units: sizes the edge fade.
    edge_world: f32,
}

struct TextureBatch { vertex:wgpu::Buffer, bind:wgpu::BindGroup, selected:u32, total:u32, triangles:Vec<SurfaceTri> }
impl TextureBatch {
    fn repartition(&mut self,device:&wgpu::Device,ranges:&[std::ops::Range<u32>]) {
        let mut vertices=Vec::with_capacity(self.triangles.len()*3);
        for selected in [true,false] { for tri in &self.triangles {
            if ranges.iter().any(|r|tri.points.iter().all(|p|r.contains(p)))==selected {vertices.extend_from_slice(&tri.vertices);}
        } if selected {self.selected=vertices.len() as u32;} }
        self.total=vertices.len() as u32;
        self.vertex=device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("material vertices"),contents:if vertices.is_empty(){&[0u8;48]}else{bytemuck::cast_slice(&vertices)},usage:wgpu::BufferUsages::VERTEX});
    }
}

fn texture_address(s:&str)->wgpu::AddressMode { match s {"clamp"=>wgpu::AddressMode::ClampToEdge,"mirror"=>wgpu::AddressMode::MirrorRepeat,_=>wgpu::AddressMode::Repeat} }
fn texture_filter(s:&str)->wgpu::FilterMode {if s.starts_with("nearest") {wgpu::FilterMode::Nearest} else {wgpu::FilterMode::Linear} }

/// Downsample in linear light, then encode into the selected GPU byte format.
fn mip_chain(asset:&materials::ImageAsset,srgb:bool)->Vec<(u32,u32,Vec<u8>)> {
    let mut levels=vec![(asset.width,asset.height,asset.rgba.clone())];
    while levels.last().is_some_and(|(w,h,_)| *w>1 || *h>1) {
        let (w,h,src)=levels.last().unwrap(); let (nw,nh)=((*w/2).max(1),(*h/2).max(1));
        let mut dst=vec![0u8;nw as usize*nh as usize*4];
        for y in 0..nh {for x in 0..nw {for c in 0..4 {
            let mut sum=0.; let mut count=0.;
            for yy in 0..2 {for xx in 0..2 {let sx=(x*2+xx).min(*w-1); let sy=(y*2+yy).min(*h-1);
                let v=src[((sy*w+sx)*4+c) as usize] as f32/255.; sum+=if srgb && c<3 {materials::srgb_to_linear(v)} else {v}; count+=1.; }}
            let v=sum/count; let v=if srgb && c<3 {materials::linear_to_srgb(v)} else {v};
            dst[((y*nw+x)*4+c) as usize]=(v.clamp(0.,1.)*255.).round() as u8;
        }}}
        levels.push((nw,nh,dst));
    }
    levels
}

fn object_mask_indices(triangles: &[u32], ranges: &[std::ops::Range<u32>]) -> Vec<u32> {
    triangles.chunks_exact(3).filter(|tri| ranges.iter().any(|range| tri.iter().all(|id| range.contains(id))))
        .flatten().copied().collect()
}

#[test]
fn object_mask_excludes_unselected_outputs() {
    let triangles = [0, 1, 2, 3, 4, 5];
    assert_eq!(object_mask_indices(&triangles, &[0..3]), [0, 1, 2]);
    assert_eq!(object_mask_indices(&triangles, &[3..6]), [3, 4, 5]);
    assert_eq!(object_mask_indices(&triangles, &[0..3, 3..6]), triangles);
    assert!(object_mask_indices(&triangles, &[]).is_empty());
}

fn edge_world_of(sc: &Scene) -> f32 {
    (2.0 * sc.radius) / ((sc.edge.len().max(2) as f32) / 2.0).sqrt()
}

/// Selected primitives first, followed by other objects. Vertex IDs never change.
struct Partition { buffer: wgpu::Buffer, selected: u32, total: u32 }
impl Partition {
    fn new(device: &wgpu::Device, indices: &[u32], width: usize, ranges: &[std::ops::Range<u32>]) -> Self {
        let (mut selected, other)=split_indices(indices,width,ranges);
        let count=selected.len() as u32;
        selected.extend(other);
        Self { buffer:index_buffer(device,"object partition",&selected), selected:count, total:selected.len() as u32 }
    }
    fn visible(&self, visibility: ObjectVisibility) -> std::ops::Range<u32> {
        0..if visibility == ObjectVisibility::ShowAll { self.total } else { self.selected }
    }
}
fn index_buffer(device: &wgpu::Device, label: &str, indices: &[u32]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label:Some(label), contents:if indices.is_empty() { &[0u8;4] } else { bytemuck::cast_slice(indices) }, usage:wgpu::BufferUsages::INDEX,
    })
}
const BOX_TRIANGLES: [u32;36]=[0,2,3,0,3,1, 4,5,7,4,7,6, 0,1,5,0,5,4, 2,6,7,2,7,3, 0,4,6,0,6,2, 1,3,7,1,7,5];
struct Boxes { p:wgpu::Buffer, n:wgpu::Buffer, c:wgpu::Buffer, tri:Partition, edge:Partition }
impl Boxes {
    fn new(device:&wgpu::Device, scene:&Scene, objects:&[std::ops::Range<u32>], ids:&[String], selected:&[std::ops::Range<u32>]) -> Self {
        let (mut points,mut colors,mut triangles,mut edges,mut selected_boxes)=(Vec::new(),Vec::new(),Vec::new(),Vec::new(),Vec::new());
        let fallback=[0..(scene.p.len()/3) as u32];
        let objects=if objects.is_empty() { &fallback[..] } else { objects };
        for (i,range) in objects.iter().enumerate() {
            let Some(p)=scene.p.get(range.start as usize*3..range.end as usize*3).filter(|p| !p.is_empty()) else { continue; };
            let (lo,hi)=preview::bounds(p);
            let start=(points.len()/3) as u32;
            for c in 0..8 { for k in 0..3 { points.push(if c>>k&1==1 { hi[k] } else { lo[k] }); } }
            let color=display_modes::random_object_color(ids.get(i).map(String::as_str).unwrap_or("object"));
            for _ in 0..8 { colors.extend(color); }
            triangles.extend(BOX_TRIANGLES.iter().map(|i| start+i));
            edges.extend(BOX_EDGES.iter().map(|i| start+i));
            if selected.iter().any(|s| s.start<=range.start && s.end>=range.end) { selected_boxes.push(start..start+8); }
        }
        let vertex=|label,data:&[f32]| device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some(label),contents:if data.is_empty() { &[0u8;12] } else { bytemuck::cast_slice(data) },usage:wgpu::BufferUsages::VERTEX,
        });
        Self { p:vertex("object bounds",&points),n:vertex("bounds normals",&vec![0.;points.len()]),c:vertex("bounds colors",&colors),
            tri:Partition::new(device,&triangles,3,&selected_boxes),edge:Partition::new(device,&edges,2,&selected_boxes) }
    }
}
impl Gpu {
    pub fn new_lod(device: &wgpu::Device, sc: &Scene) -> Self {
        Self::new(device,&sc.proxied(0.0))
    }
    pub fn new(device: &wgpu::Device, sc: &Scene) -> Self {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let buf = |label, data: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: if data.is_empty() { &[0;4] } else { data }, usage })
        };
        let vp = buf("P", bytemuck::cast_slice(&sc.p), wgpu::BufferUsages::VERTEX);
        let vn = buf("N", bytemuck::cast_slice(&sc.n), wgpu::BufferUsages::VERTEX);
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
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
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
        let pipe_elements=mk("points and curves","vs_element","fs_element",&element_bufs,T::TriangleList,-1,true,C::LessEqual,true,W::ALL);
        let pipe_elements_ghost=mk("ghost points and curves","vs_element","fs_element_ghost",&element_bufs,T::TriangleList,-1,true,C::LessEqual,false,W::ALL);
        let pipe_surf = mk("surf", "vs_surf", "fs_surf", &surf_bufs, T::TriangleList, 0, false, C::LessEqual, true, W::ALL);
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
        let pipe_group_line=mk("group outline","vs_group","fs_group_line",&group_bufs,T::LineList,-4,true,C::LessEqual,false,W::ALL);
        let point_bufs=[wgpu::VertexBufferLayout {array_stride:12,step_mode:wgpu::VertexStepMode::Instance,attributes:&POS}];
        let pipe_group_point=mk("group points","vs_group_point","fs_group_point",&point_bufs,T::TriangleList,-4,true,C::LessEqual,false,W::ALL);
        let pipe_barrier_face=mk("barrier fill","vs_group","fs_barrier_fill",&group_bufs,T::TriangleList,-3,true,C::LessEqual,false,W::ALL);
        let pipe_barrier_line=mk("barrier outline","vs_group","fs_barrier_line",&group_bufs,T::LineList,-4,true,C::LessEqual,false,W::ALL);
        let pipe_target_face=mk("target fill","vs_group","fs_target_fill",&group_bufs,T::TriangleList,-3,true,C::LessEqual,false,W::ALL);
        let pipe_target_line=mk("target outline","vs_group","fs_target_line",&group_bufs,T::LineList,-4,true,C::LessEqual,false,W::ALL);
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
        let nl_p = buf("nl_p", bytemuck::cast_slice(&sc.nlp), wgpu::BufferUsages::VERTEX);
        let nl_n = buf("nl_n", bytemuck::cast_slice(&sc.nln), wgpu::BufferUsages::VERTEX);
        let bb_p = buf("bb_p", bytemuck::cast_slice(&sc.bbp), wgpu::BufferUsages::VERTEX);
        let bb_n = buf("bb_n", bytemuck::cast_slice(&[0f32; 24]), wgpu::BufferUsages::VERTEX);
        let bb_i = buf("bb_i", bytemuck::cast_slice(&BOX_EDGES), wgpu::BufferUsages::INDEX);

        Self {
            tex_bgl,default_tex_bind,pipe_tex,pipe_tex_bias,pipe_tex_xray,tex_batches:Vec::new(),texture_status:Vec::new(),gpu_images:HashMap::new(),gpu_tick:0,
            pipe_floor,z_axis,pipe_z_axis,
            face_markers:face_markers::FaceMarkers::new(device),pipe_face_markers,pipe_face_markers_xray,
            elements:Elements::new(device,sc,&[],&[]),element_points:Vec::new(),pipe_elements,pipe_elements_ghost,
            pipe_depth, pipe_wire_all, pipe_wire_hidden, pipe_ghost, pipe_ghost_wire,
            partition_tri: Partition::new(device, &sc.tri, 3, &[]), partition_edge: Partition::new(device, &sc.edge, 2, &[]),
            current_tri:sc.tri.clone(), current_edge:sc.edge.clone(),
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
            nl_count: (sc.nlp.len() / 3) as u32, nl_selected:0,
            bb_p,
            bb_n,
            bb_i,
            bind,
            ubuf,
            vp,
            vn,
            vc:buf("object colors",bytemuck::cast_slice(&vec![0.72f32;(sc.p.len()).max(3)]),wgpu::BufferUsages::VERTEX),
            vcd:sc.cd.as_ref().map(|c|buf("Cd colors",bytemuck::cast_slice(c),wgpu::BufferUsages::VERTEX)),
            neutral_color:buf("neutral color",bytemuck::cast_slice(&vec![0.72f32;(sc.p.len()*4).max(sc.tri.len()).max(24)]),wgpu::BufferUsages::VERTEX),
            object_mask: None, object_mask_n: 0, object_mask_points: None,
            ntri_idx: sc.tri.len() as u32,
            edge_world: edge_world_of(sc),
        }
    }
    pub fn upload(&mut self, device: &wgpu::Device, sc: &Scene) {
        self.face_markers.invalidate();
        self.tex_batches.clear(); self.texture_status.clear();
        self.elements=Elements::new(device,sc,&[],&self.element_points);
        self.current_tri=sc.tri.clone(); self.current_edge=sc.edge.clone();
        self.partition_tri=Partition::new(device,&sc.tri,3,&[]);
        self.partition_edge=Partition::new(device,&sc.edge,2,&[]);
        self.boxes=Some(Boxes::new(device,sc,&[],&[],&[0..(sc.p.len()/3) as u32])); self.object_ranges.clear(); self.object_ids.clear();
        self.object_mask = None; self.object_mask_n = 0; self.object_mask_points = None;
        let buf = |label, data: &[u8], usage| device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some(label), contents:if data.is_empty() { &[0;4] } else { data }, usage
        });
        self.vp=buf("P",bytemuck::cast_slice(&sc.p),wgpu::BufferUsages::VERTEX);
        self.vn=buf("N",bytemuck::cast_slice(&sc.n),wgpu::BufferUsages::VERTEX);
        self.vc=buf("object colors",bytemuck::cast_slice(&vec![0.72f32;sc.p.len().max(3)]),wgpu::BufferUsages::VERTEX);
        self.vcd=sc.cd.as_ref().map(|c|buf("Cd colors",bytemuck::cast_slice(c),wgpu::BufferUsages::VERTEX));
        self.neutral_color=buf("neutral color",bytemuck::cast_slice(&vec![0.72f32;(sc.p.len()*4).max(sc.tri.len()).max(24)]),wgpu::BufferUsages::VERTEX);
        self.ntri_idx=sc.tri.len() as u32;
        self.edge_world=edge_world_of(sc);
        self.nl_p=buf("nl_p",bytemuck::cast_slice(&sc.nlp),wgpu::BufferUsages::VERTEX);
        self.nl_n=buf("nl_n",bytemuck::cast_slice(&sc.nln),wgpu::BufferUsages::VERTEX);
        self.nl_count=(sc.nlp.len()/3) as u32; self.nl_selected=0;
        self.bb_p=buf("bb_p",bytemuck::cast_slice(&sc.bbp),wgpu::BufferUsages::VERTEX);
    }
    pub fn upload_textures(&mut self,device:&wgpu::Device,queue:&wgpu::Queue,sc:&Scene,registry:&mut Registry) {
        self.tex_batches.clear(); self.texture_status.clear();
        let mut grouped:BTreeMap<String,Vec<SurfaceTri>>=BTreeMap::new();
        for tri in &sc.textured { grouped.entry(tri.slot.clone()).or_default().push(tri.clone()); }
        let mut pinned=HashSet::new(); let mut gpu_used=0usize;
        for (slot,mut triangles) in grouped {
            let binding=registry.resolve(&slot);
            let tex=binding.descriptor.as_ref().and_then(|d|d.textures.base_color.as_ref());
            if let Some(tex)=tex {for tri in &mut triangles {for k in 0..3 {
                let uv=tri.uv_sets.get(&tex.uv_set).map(|uv|uv[k]).unwrap_or([f32::NAN;2]);
                tri.vertices[k].valid=(uv[0].is_finite()&&uv[1].is_finite()) as u8 as f32;
                tri.vertices[k].uv=if tri.vertices[k].valid>0. {uv}else{[0.;2]};
            }}}
            else {for tri in &mut triangles {for v in &mut tri.vertices {v.valid=1.;v.uv=[0.;2];}}}
            let srgb=tex.is_some_and(|t|t.color_space=="srgb");
            let image=binding.image.clone().unwrap_or_else(|| {
                let checker=matches!(binding.state,State::CheckerFallback(_)|State::Unsupported(_));
                let rgba=if checker {[220,220,220,255,90,30,110,255,90,30,110,255,220,220,220,255].to_vec()} else {[255,255,255,255].to_vec()};
                materials::ImageAsset {width:if checker {2}else{1},height:if checker {2}else{1},rgba,hash:if checker {"checker".into()}else{"white".into()}}
            });
            let key=format!("{}:{}",image.hash,if srgb {"srgb"}else{"linear"}); pinned.insert(key.clone());
            let (view,resident,dimensions,downsampled)=if let Some((view,bytes,last))=self.gpu_images.get_mut(&key) {
                self.gpu_tick+=1; *last=self.gpu_tick; gpu_used+=*bytes; (view.clone(),*bytes,(image.width,image.height),false)
            } else {
                let mut levels=mip_chain(&image,srgb);
                let mut skipped=0;
                while levels.len()>1 && gpu_used+levels.iter().map(|(_,_,b)|b.len()).sum::<usize>()>registry.gpu_budget {levels.remove(0);skipped+=1;}
                let bytes=levels.iter().map(|(_,_,b)|b.len()).sum::<usize>(); gpu_used+=bytes;
                let dimensions=(levels[0].0,levels[0].1);
                let format=if srgb {wgpu::TextureFormat::Rgba8UnormSrgb}else{wgpu::TextureFormat::Rgba8Unorm};
                let texture=device.create_texture(&wgpu::TextureDescriptor {label:Some("base colour"),size:wgpu::Extent3d {width:dimensions.0,height:dimensions.1,depth_or_array_layers:1},mip_level_count:levels.len() as u32,sample_count:1,dimension:wgpu::TextureDimension::D2,format,usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[]});
                for (mip,(w,h,bytes)) in levels.iter().enumerate() {
                    queue.write_texture(wgpu::TexelCopyTextureInfo {texture:&texture,mip_level:mip as u32,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},bytes,wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(w*4),rows_per_image:Some(*h)},wgpu::Extent3d {width:*w,height:*h,depth_or_array_layers:1});
                }
                let view=texture.create_view(&Default::default());
                self.gpu_tick+=1; self.gpu_images.insert(key.clone(),(view.clone(),bytes,self.gpu_tick));
                (view,bytes,dimensions,skipped>0)
            };
            let sampler_desc=tex.map(|t|&t.sampler);
            let sampler=device.create_sampler(&wgpu::SamplerDescriptor {address_mode_u:sampler_desc.map_or(wgpu::AddressMode::Repeat,|s|texture_address(&s.wrap_u)),address_mode_v:sampler_desc.map_or(wgpu::AddressMode::Repeat,|s|texture_address(&s.wrap_v)),mag_filter:sampler_desc.map_or(wgpu::FilterMode::Linear,|s|texture_filter(&s.mag)),min_filter:sampler_desc.map_or(wgpu::FilterMode::Linear,|s|texture_filter(&s.min)),mipmap_filter:if sampler_desc.is_some_and(|s|s.min.ends_with("nearest")){wgpu::FilterMode::Nearest}else{wgpu::FilterMode::Linear},..Default::default()});
            let factor=binding.descriptor.as_ref().map_or([1.;4],|d|d.base_color_factor);
            let uniform=device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("base colour factor"),contents:bytemuck::cast_slice(&factor),usage:wgpu::BufferUsages::UNIFORM});
            let bind=device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("material"),layout:&self.tex_bgl,entries:&[
                wgpu::BindGroupEntry {binding:0,resource:uniform.as_entire_binding()},wgpu::BindGroupEntry {binding:1,resource:wgpu::BindingResource::TextureView(&view)},wgpu::BindGroupEntry {binding:2,resource:wgpu::BindingResource::Sampler(&sampler)}]});
            let mut warning=match &binding.state {State::Ok=>"ok".into(),State::Missing=>"neutral".into(),State::Unsupported(e)|State::CheckerFallback(e)=>format!("checker fallback: {e}")};
            if triangles.iter().any(|t|t.vertices.iter().any(|v|v.valid==0.)) {warning.push_str("; missing UV checker");}
            if downsampled {warning.push_str("; downsampled for GPU budget");}
            self.texture_status.push(format!("{}: descriptor {} · scope {} · image hash {} · state {:?} · {}x{} · UV {} · {} / {} · {} bytes · {}",if slot.is_empty(){"(default)"}else{&slot},binding.descriptor_id.as_deref().unwrap_or("none"),binding.scope.as_deref().unwrap_or("none"),tex.and_then(|t|t.image_id.as_deref()).unwrap_or("none"),binding.state,dimensions.0,dimensions.1,tex.map_or("uv",|t|&t.uv_set),tex.map_or("linear",|t|&t.sampler.min),tex.map_or("repeat",|t|&t.sampler.wrap_u),resident,warning));
            let mut batch=TextureBatch {vertex:device.create_buffer(&wgpu::BufferDescriptor {label:Some("empty material vertices"),size:48,usage:wgpu::BufferUsages::VERTEX,mapped_at_creation:false}),bind,selected:0,total:0,triangles};
            batch.repartition(device,&self.object_mask_points.clone().unwrap_or_default());
            self.tex_batches.push(batch);
        }
        while self.gpu_images.values().map(|(_,bytes,_)|*bytes).sum::<usize>()>registry.gpu_budget {
            let Some(id)=self.gpu_images.iter().filter(|(id,_)|!pinned.contains(*id)).min_by_key(|(_,(_,_,last))|last).map(|(id,_)|id.clone()) else {break};
            self.gpu_images.remove(&id);
        }
    }
    pub fn texture_status(&self)->&[String] {&self.texture_status}
    /// Use full-detail selected-object triangles for a stable silhouette at every proxy/LOD level.
    pub fn select_objects(&mut self, device: &wgpu::Device, scene: &Scene, ranges: Vec<std::ops::Range<u32>>) {
        if self.object_mask_points.as_ref() == Some(&ranges) { return; }
        self.face_markers.invalidate();
        self.elements=Elements::new(device,scene,&ranges,&self.element_points);
        let indices = object_mask_indices(scene.triangle_indices(), &ranges);
        self.object_mask_n = indices.len() as u32;
        self.object_mask = Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("selected object mask"),
            contents: if indices.is_empty() { &[0u8; 4] } else { bytemuck::cast_slice(&indices) },
            usage: wgpu::BufferUsages::INDEX,
        }));
        self.partition_tri=Partition::new(device,&self.current_tri,3,&ranges);
        for batch in &mut self.tex_batches {batch.repartition(device,&ranges);}
        self.partition_edge=Partition::new(device,&self.current_edge,2,&ranges);
        self.boxes=None;
        let (mut p,mut n)=(Vec::with_capacity(scene.nlp.len()),Vec::with_capacity(scene.nln.len()));
        self.nl_selected=0;
        for selected in [true,false] {
            for (i,id) in scene.normal_points.iter().enumerate() {
                if ranges.iter().any(|r| r.contains(id))==selected {
                    p.extend_from_slice(&scene.nlp[i*6..i*6+6]); n.extend_from_slice(&scene.nln[i*6..i*6+6]);
                    if selected { self.nl_selected+=2; }
                }
            }
        }
        let buf=|label,data:&[f32]| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label:Some(label),contents:if data.is_empty() { &[0u8;12] } else { bytemuck::cast_slice(data) },usage:wgpu::BufferUsages::VERTEX });
        self.nl_p=buf("partitioned normal lines",&p); self.nl_n=buf("partitioned normal normals",&n);
        self.object_mask_points = Some(ranges);
    }
    /// Bounds use full-resolution object ownership, independently of the current proxy/LOD cut.
    pub fn set_object_ranges(&mut self, device: &wgpu::Device, scene: &Scene, ranges: Vec<std::ops::Range<u32>>) {
        let named=ranges.into_iter().enumerate().map(|(i,r)| (i.to_string(),r)).collect();
        self.set_named_object_ranges(device,scene,named);
    }
    pub fn set_named_object_ranges(&mut self, device:&wgpu::Device, scene:&Scene, objects:Vec<(String,std::ops::Range<u32>)>) {
        let (ids,ranges):(Vec<_>,Vec<_>)=objects.into_iter().unzip();
        if self.boxes.is_some() && self.object_ranges==ranges && self.object_ids==ids { return; }
        let mut colors=vec![0.72f32;scene.p.len().max(3)];
        for (id,range) in ids.iter().zip(&ranges) {
            let color=display_modes::random_object_color(id);
            for i in range.clone() { if let Some(dst)=colors.get_mut(i as usize*3..i as usize*3+3) { dst.copy_from_slice(&color); } }
        }
        self.vc=device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("object colors"),contents:bytemuck::cast_slice(&colors),usage:wgpu::BufferUsages::VERTEX});
        self.boxes=Some(Boxes::new(device,scene,&ranges,&ids,self.object_mask_points.as_deref().unwrap_or(&[])));
        self.object_ranges=ranges; self.object_ids=ids;
    }
    /// Retain the source vertex attributes but start with no triangles; selection uploads a cut.
    pub fn upload_lod(&mut self, device: &wgpu::Device, sc: &Scene) {
        self.face_markers.invalidate();
        self.upload(device,&sc.proxied(0.0));
    }
    pub fn select_lod(&mut self, device: &wgpu::Device, lod: &ClusterLod, ids: &[ClusterId], wire: bool) {
        let (indices, _)=packed_cut(lod, ids);
        self.ntri_idx=indices.len() as u32;
        self.current_tri=indices;
        self.partition_tri=Partition::new(device,&self.current_tri,3,self.object_mask_points.as_deref().unwrap_or(&[]));
        if wire {
            let mut edges = std::collections::BTreeSet::new();
            for &id in ids { for tri in &lod.clusters[id as usize].triangles {
                for (a,b) in [(tri[0],tri[1]),(tri[1],tri[2]),(tri[2],tri[0])] { edges.insert((a.min(b),a.max(b))); }
            } }
            let mut indices=Vec::with_capacity(edges.len()*2);
            for (a,b) in edges { indices.extend_from_slice(&[a,b]); }
            self.current_edge=indices;
        } else { self.current_edge.clear(); }
        self.partition_edge=Partition::new(device,&self.current_edge,2,self.object_mask_points.as_deref().unwrap_or(&[]));
    }
    pub fn set_face_markers(&mut self, device:&wgpu::Device, scene:&Scene, topo:&crate::element_select::Topology,
        selected:&[u32], hovered:&[u32], visibility:ObjectVisibility) {
        self.face_markers.update(device,scene,topo,selected,hovered,visibility,self.object_mask_points.as_deref().unwrap_or(&[]));
    }
    pub fn select_points(&mut self, device:&wgpu::Device, scene:&Scene, points:&[u32]) {
        if self.element_points==points { return; }
        self.element_points=points.to_vec();
        self.elements=Elements::new(device,scene,self.object_mask_points.as_deref().unwrap_or(&[]),points);
    }
    /// Show the selection: `faces` = 3 positions per triangle, `lines` = 2 positions per segment (flat xyz).
    pub fn set_group_overlay(&mut self,device:&wgpu::Device,faces:&[f32],lines:&[f32]) {
        let buffer=|label,data:&[f32]|device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some(label),contents:if data.is_empty(){&[0u8;12]}else{bytemuck::cast_slice(data)},usage:wgpu::BufferUsages::VERTEX,
        });
        self.group_face=buffer("group faces",faces);self.group_face_n=(faces.len()/3) as u32;
        let (points, outlines)=split_point_crosses(lines);
        self.group_point=buffer("group points",&points);self.group_point_n=(points.len()/3) as u32;
        self.group_line=buffer("group outline",&outlines);self.group_line_n=(outlines.len()/3) as u32;
    }
    /// Barrier-group overlay (orange), drawn beside the purple group overlay.
    pub fn set_barrier_overlay(&mut self,device:&wgpu::Device,faces:&[f32],lines:&[f32]) {
        let buffer=|label,data:&[f32]|device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some(label),contents:if data.is_empty(){&[0u8;12]}else{bytemuck::cast_slice(data)},usage:wgpu::BufferUsages::VERTEX,
        });
        self.barrier_face=buffer("barrier faces",faces);self.barrier_face_n=(faces.len()/3) as u32;
        self.barrier_line=buffer("barrier outline",lines);self.barrier_line_n=(lines.len()/3) as u32;
    }
    /// Output-group target overlay (teal), shown while the target button is hovered.
    pub fn set_target_overlay(&mut self,device:&wgpu::Device,faces:&[f32],lines:&[f32]) {
        let buffer=|label,data:&[f32]|device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some(label),contents:if data.is_empty(){&[0u8;12]}else{bytemuck::cast_slice(data)},usage:wgpu::BufferUsages::VERTEX,
        });
        self.target_face=buffer("target faces",faces);self.target_face_n=(faces.len()/3) as u32;
        self.target_line=buffer("target outline",lines);self.target_line_n=(lines.len()/3) as u32;
    }
    pub fn set_selection(&mut self, device: &wgpu::Device, faces: &[f32], lines: &[f32]) {
        let buf = |label, data: &[f32]| device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label), contents: if data.is_empty() { &[0u8; 12] } else { bytemuck::cast_slice(data) }, usage: wgpu::BufferUsages::VERTEX });
        // Highlights expand indexed faces/edges into individual vertices. Every bound
        // vertex buffer must cover that expanded count, including the constant colour.
        let need = faces.len().max(lines.len()) * 4;
        if need as u64 > self.zeros.size() { self.zeros = buf("zeros", &vec![0f32; faces.len().max(lines.len()).next_power_of_two()]); }
        if need as u64 > self.neutral_color.size() {
            self.neutral_color = buf("neutral color", &vec![0.72f32; faces.len().max(lines.len()).next_power_of_two()]);
        }
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
        let editing = selection_mode != crate::ui::SelectionMode::Object;
        let sketchup_clean = mode.matcap() && overlay.matcap==MatCap::SketchUpClean;
        let is_shaded_wire = sketchup_clean || (mode.wire_shaded() && (editing || overlay.wireframe))
            || (overlay.wireframe && mode.shaded() && !mode.bounds())
            || (editing && !mode.bounds());
        let show_wire = mode.wire() || is_shaded_wire || (overlay.xray && editing);
        let use_cd=overlay.object_color==ObjectColor::Single && self.vcd.is_some() && !mode.matcap();
        let u = Uniforms { view_proj,view,inv_view_proj:inverse_projection(view_proj),base:[0.72,0.74,0.78,1.],wire:[0.,0.,0.,if is_shaded_wire {1.} else {0.}],
            flat_shading:(mode.flat() || (mode.matcap() && overlay.matcap_flat)) as u32,wire_fade:fade,_pad:[1,if sketchup_clean {2} else {mode.matcap() as u32}],
            object_color:(overlay.object_color==ObjectColor::Random || use_cd) as u32,_pad_color:[0;3],
            floor:[10f32.powf((cam.distance().max(0.01)*0.1).log10().floor()),overlay.grid as u32 as f32,overlay.axes as u32 as f32,cam.distance().max(0.01)],
            element_size:[target.size[0] as f32,target.size[1] as f32,overlay.geometry.point_size*if selection_mode==crate::ui::SelectionMode::Points {4.} else {1.},overlay.geometry.line_width],
            element_flags:[(overlay.geometry.points || selection_mode==crate::ui::SelectionMode::Points) as u32,overlay.geometry.lines as u32,points_visible(&overlay.geometry,selection_mode) as u32,(selection_mode==crate::ui::SelectionMode::Points) as u32],
            element_color:if overlay.light_background {[0.025,0.03,0.04,1.]} else {[0.72,0.75,0.8,1.]} };
        queue.write_buffer(&self.ubuf,0,bytemuck::bytes_of(&u));
        let matcap_id=overlay.matcap.texture_id();
        if self.matcap_loaded.swap(matcap_id,std::sync::atomic::Ordering::Relaxed)!=matcap_id {
            queue.write_texture(wgpu::TexelCopyTextureInfo { texture:&self.matcap,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All },
                &overlay.matcap.pixels(128),wgpu::TexelCopyBufferLayout { offset:0,bytes_per_row:Some(128*4),rows_per_image:Some(128) },
                wgpu::Extent3d { width:128,height:128,depth_or_array_layers:1 });
        }
        // Boxes are full object bounds, never the bounds of a decimated cluster.
        let boxes=if mode.bounds() { self.boxes.as_ref() } else { None };
        let (vp,vn,vc,tri,edge)=if let Some(b)=boxes { (&b.p,&b.n,&b.c,&b.tri,&b.edge) }
            else { (&self.vp,&self.vn,if use_cd { self.vcd.as_ref().unwrap() } else { &self.vc },&self.partition_tri,&self.partition_edge) };
        let visibility=overlay.visibility;
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label:Some("MiniHou viewport"),
                color_attachments:&[Some(wgpu::RenderPassColorAttachment {
                    view:&target.color,depth_slice:None,resolve_target:None,
                    ops:wgpu::Operations { load:wgpu::LoadOp::Clear(if overlay.light_background {wgpu::Color {r:0.76,g:0.78,b:0.81,a:1.}} else {wgpu::Color { r:0.244,g:0.256,b:0.278,a:1. }}),store:wgpu::StoreOp::Store },
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
            pass.set_bind_group(1,&self.default_tex_bind,&[]);
            pass.set_vertex_buffer(0,vp.slice(..)); pass.set_vertex_buffer(1,vn.slice(..)); pass.set_vertex_buffer(2,vc.slice(..));
            pass.set_index_buffer(tri.buffer.slice(..),wgpu::IndexFormat::Uint32);
            let visible=tri.visible(visibility);
            if overlay.geometry.faces && !visible.is_empty() {
                if mode.hidden() && !overlay.xray {
                    pass.set_pipeline(&self.pipe_depth); pass.draw_indexed(visible.clone(),0,0..1);
                }
                if mode.shaded() || mode == Mode::WireframeGhost {
                    let textured=overlay.textures && matches!(mode,Mode::FlatShaded|Mode::FlatWireShaded|Mode::SmoothShaded|Mode::SmoothWireShaded)
                        && overlay.object_color==ObjectColor::Single && !self.tex_batches.is_empty();
                    if textured {
                        pass.set_pipeline(if overlay.xray {&self.pipe_tex_xray}else if is_shaded_wire {&self.pipe_tex_bias}else{&self.pipe_tex});
                        for batch in &self.tex_batches {pass.set_bind_group(1,&batch.bind,&[]); pass.set_vertex_buffer(0,batch.vertex.slice(..));
                            pass.draw(0..if visibility==ObjectVisibility::ShowAll {batch.total}else{batch.selected},0..1);
                        }
                        pass.set_vertex_buffer(0,vp.slice(..));
                    } else {
                        pass.set_pipeline(if overlay.xray { &self.pipe_xray }
                            else if mode == Mode::WireframeGhost { &self.pipe_ghost }
                            else if is_shaded_wire { &self.pipe_surf_bias } else { &self.pipe_surf });
                        pass.draw_indexed(visible,0,0..1);
                    }
                }
            }
            if show_wire && overlay.geometry.lines {
                pass.set_index_buffer(edge.buffer.slice(..),wgpu::IndexFormat::Uint32);
                let visible=edge.visible(visibility);
                if mode == Mode::HiddenLineGhost && !overlay.xray {
                    pass.set_pipeline(&self.pipe_wire_hidden); pass.draw_indexed(visible.clone(),0,0..1);
                }
                pass.set_pipeline(if overlay.xray || matches!(mode,Mode::Wireframe | Mode::WireframeGhost | Mode::WireBoundingBox) { &self.pipe_wire_all } else { &self.pipe_wire });
                pass.draw_indexed(visible,0,0..1);
            }
            // Ghosts depth-test against selected surfaces, never write depth, and never enter the outline mask.
            if visibility == ObjectVisibility::GhostOthers {
                pass.set_index_buffer(tri.buffer.slice(..),wgpu::IndexFormat::Uint32);
                if overlay.geometry.faces { pass.set_pipeline(&self.pipe_ghost); pass.draw_indexed(tri.selected..tri.total,0,0..1); }
                pass.set_index_buffer(edge.buffer.slice(..),wgpu::IndexFormat::Uint32);
                if overlay.geometry.lines && show_wire { pass.set_pipeline(&self.pipe_ghost_wire); pass.draw_indexed(edge.selected..edge.total,0,0..1); }
            }
            if overlay.grid || overlay.axes {
                pass.set_pipeline(&self.pipe_floor);
                pass.draw(0..3,0..1);
            }
            if overlay.axes {
                pass.set_pipeline(&self.pipe_z_axis);pass.set_vertex_buffer(0,self.z_axis.slice(..));
                pass.draw(0..6,0..1);
            }
            if !mode.bounds() {
                pass.set_pipeline(&self.pipe_elements);
                pass.set_vertex_buffer(0,self.elements.buffer.slice(..));
                pass.draw(0..6,0..if visibility==ObjectVisibility::ShowAll {self.elements.total} else {self.elements.selected});
                if visibility==ObjectVisibility::GhostOthers {
                    pass.set_pipeline(&self.pipe_elements_ghost); pass.draw(0..6,self.elements.selected..self.elements.total);
                }
            }
            if self.sel_face_n > 0 && !mode.bounds() {
                pass.set_pipeline(&self.pipe_select);
                pass.set_vertex_buffer(0,self.sel_face.slice(..)); pass.set_vertex_buffer(1,self.zeros.slice(..)); pass.set_vertex_buffer(2,self.neutral_color.slice(..));
                pass.draw(0..self.sel_face_n,0..1);
            }
            if editing && self.sel_line_n > 0 && overlay.geometry.lines && !mode.bounds() {
                pass.set_pipeline(&self.pipe_bounds);
                pass.set_vertex_buffer(0,self.sel_line.slice(..)); pass.set_vertex_buffer(1,self.zeros.slice(..)); pass.set_vertex_buffer(2,self.neutral_color.slice(..));
                pass.draw(0..self.sel_line_n,0..1);
            }
            if !mode.bounds() && overlay.geometry.enabled {
                if self.group_face_n>0 {pass.set_pipeline(&self.pipe_group_face);pass.set_vertex_buffer(0,self.group_face.slice(..));pass.draw(0..self.group_face_n,0..1);}
                if self.group_line_n>0 {pass.set_pipeline(&self.pipe_group_line);pass.set_vertex_buffer(0,self.group_line.slice(..));pass.draw(0..self.group_line_n,0..1);}
                if self.group_point_n>0 {pass.set_pipeline(&self.pipe_group_point);pass.set_vertex_buffer(0,self.group_point.slice(..));pass.draw(0..6,0..self.group_point_n);}
                if self.barrier_face_n>0 {pass.set_pipeline(&self.pipe_barrier_face);pass.set_vertex_buffer(0,self.barrier_face.slice(..));pass.draw(0..self.barrier_face_n,0..1);}
                if self.barrier_line_n>0 {pass.set_pipeline(&self.pipe_barrier_line);pass.set_vertex_buffer(0,self.barrier_line.slice(..));pass.draw(0..self.barrier_line_n,0..1);}
                if self.target_face_n>0 {pass.set_pipeline(&self.pipe_target_face);pass.set_vertex_buffer(0,self.target_face.slice(..));pass.draw(0..self.target_face_n,0..1);}
                if self.target_line_n>0 {pass.set_pipeline(&self.pipe_target_line);pass.set_vertex_buffer(0,self.target_line.slice(..));pass.draw(0..self.target_line_n,0..1);}
            }
            if selection_mode == crate::ui::SelectionMode::Faces && !mode.bounds() && self.face_markers.count > 0 {
                pass.set_pipeline(if overlay.xray {&self.pipe_face_markers_xray}else{&self.pipe_face_markers});
                pass.set_vertex_buffer(0,self.face_markers.buffer.slice(..));
                pass.draw(0..6,0..self.face_markers.count);
                if self.face_markers.hover_count > 0 {
                    pass.set_vertex_buffer(0,self.face_markers.hover_buffer.slice(..));
                    pass.draw(0..6,0..self.face_markers.hover_count);
                }
            }
            if overlay.normals && self.nl_count > 0 && !mode.bounds() {
                pass.set_pipeline(&self.pipe_normals);
                pass.set_vertex_buffer(0,self.nl_p.slice(..)); pass.set_vertex_buffer(1,self.nl_n.slice(..)); pass.set_vertex_buffer(2,self.neutral_color.slice(..));
                pass.draw(0..if visibility==ObjectVisibility::ShowAll { self.nl_count } else { self.nl_selected },0..1);
            }
            if overlay.bounds && !mode.bounds() {
                pass.set_pipeline(&self.pipe_bounds);
                if let Some(b)=&self.boxes {
                    pass.set_vertex_buffer(0,b.p.slice(..)); pass.set_vertex_buffer(1,b.n.slice(..)); pass.set_vertex_buffer(2,b.c.slice(..));
                    pass.set_index_buffer(b.edge.buffer.slice(..),wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(b.edge.visible(visibility),0,0..1);
                } else {
                    pass.set_vertex_buffer(0,self.bb_p.slice(..)); pass.set_vertex_buffer(1,self.bb_n.slice(..)); pass.set_vertex_buffer(2,self.neutral_color.slice(..));
                    pass.set_index_buffer(self.bb_i.slice(..),wgpu::IndexFormat::Uint32); pass.draw_indexed(0..24,0,0..1);
                }
            }
        }
        if selected_object && (boxes.is_some_and(|b| b.tri.selected>0) || self.object_mask_n > 0 || (self.object_mask.is_none() && self.ntri_idx > 0)) {
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
                mask_pass.set_bind_group(1, &self.default_tex_bind, &[]);
                mask_pass.set_pipeline(&self.pipe_mask);
                mask_pass.set_vertex_buffer(0, vp.slice(..));
                mask_pass.set_vertex_buffer(1, vn.slice(..));
                mask_pass.set_vertex_buffer(2, vc.slice(..));
                let mask_buffer=if boxes.is_some() { &tri.buffer } else { self.object_mask.as_ref().unwrap_or(&self.partition_tri.buffer) };
                mask_pass.set_index_buffer(mask_buffer.slice(..), wgpu::IndexFormat::Uint32);
                let end = if boxes.is_some() { tri.selected } else if self.object_mask.is_some() { self.object_mask_n } else { self.ntri_idx };
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
        let graph = minihou_core::Graph::from_json(include_str!("default_graph.json")).unwrap();
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
