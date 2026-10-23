use super::*;
use std::time::Instant;

/// Longest overlay normal list kept: more points than this are thinned evenly.
const NORMAL_LINES: usize = 60_000;
/// Keep CPU expansion below the measured 120k-triangle / 80k-point scene's
/// roughly one-second scene-plus-upload cost; see finding3-measure.md.
const MAX_EXPANDED_VIEWPORT_POINTS: usize = 50_000;
const MAX_EXPANDED_VIEWPORT_TRIANGLES: usize = 50_000;

fn viewport_geometry(g: &Geometry) -> Option<(std::borrow::Cow<'_, Geometry>, bool)> {
    use minihou_core::nodes::packed;
    if packed::instance_count(g) == 0 { return None; }
    let fallback = packed::estimated_display_counts(g).is_none_or(|(points, triangles)|
        points > MAX_EXPANDED_VIEWPORT_POINTS || triangles > MAX_EXPANDED_VIEWPORT_TRIANGLES);
    let shown = if fallback { packed::bounds_proxy_geometry(g).map(std::borrow::Cow::Owned) }
        else { packed::display_geometry(g) };
    shown.ok().map(|shown| (shown, fallback))
}

fn build_lod(scene:&Scene)->Option<std::sync::Arc<ClusterLod>> {
    if scene.tri.is_empty() || scene.has_textured_slots() {return None;}
    let positions=scene.p.chunks_exact(3).map(|p|[p[0],p[1],p[2]]).collect();
    let triangles=scene.tri.chunks_exact(3).map(|t|[t[0],t[1],t[2]]).collect();
    ClusterLod::build(positions,triangles).ok().map(std::sync::Arc::new)
}
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
    /// Only index cuts change with proxy level; vertex attributes and picking stay at full detail.
    pub fn proxy_indices(&self, level: f32) -> (Vec<u32>, Vec<u32>) {
        (crate::ui::proxy_ui::proxy::reduced_triangles(&self.tri, level),
         crate::ui::proxy_ui::proxy::reduced_edges(&self.edge, level))
    }
    pub(super) fn base_indices<'a>(&self, tri:&'a [u32], edge:&'a [u32]) -> (std::borrow::Cow<'a,[u32]>,std::borrow::Cow<'a,[u32]>) {
        if self.packed_sources.is_empty() {return (std::borrow::Cow::Borrowed(tri),std::borrow::Cow::Borrowed(edge));}
        let packed=|id:&u32| self.packed_sources.iter().any(|source|source.lod.is_some() && source.instances.iter().any(|instance|instance.range.contains(id)));
        let tri=tri.chunks_exact(3).filter(|t|!packed(&t[0])).flatten().copied().collect();
        let edge=edge.chunks_exact(2).filter(|e|!packed(&e[0])).flatten().copied().collect();
        (std::borrow::Cow::Owned(tri),std::borrow::Cow::Owned(edge))
    }
    pub fn packed_point(&self,id:u32)->bool {
        self.packed_sources.iter().any(|source|source.instances.iter().any(|instance|instance.range.contains(&id)))
    }
    pub fn pick_packed(&self,o:[f32;3],d:[f32;3],allowed:impl Fn(&std::ops::Range<u32>)->bool)->Option<crate::picking::Hit> {
        fn cross(a:[f32;3],b:[f32;3])->[f32;3] {[a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]]}
        fn dot(a:[f32;3],b:[f32;3])->f32 {(0..3).map(|i|a[i]*b[i]).sum()}
        let mut best:Option<crate::picking::Hit>=None;
        for source in &self.packed_sources {for instance in &source.instances {
            if !allowed(&instance.range) {continue;}
            let rows=&instance.rows;
            let cols:[[f32;3];3]=std::array::from_fn(|c|std::array::from_fn(|r|rows[r][c]));
            let cof=[cross(cols[1],cols[2]),cross(cols[2],cols[0]),cross(cols[0],cols[1])];
            let det=dot(cols[0],cof[0]);
            if det.abs()<1e-20 {continue;}
            let local_o=std::array::from_fn(|k|dot(cof[k],[0,1,2].map(|r|o[r]-rows[r][3]))/det);
            let local_d=std::array::from_fn(|k|dot(cof[k],d)/det);
            let Some(mut hit)=source.scene.pick_bvh.pick(&source.scene.p,&source.scene.tri,local_o,local_d,|_|true) else {continue;};
            let Some(first)=instance.proxy_tri else {continue;};
            hit.tri=first+if instance.compact {0}else{hit.tri};
            if best.is_none_or(|old|hit.t<old.t){best=Some(hit);}
        }}
        best
    }
    pub fn triangles(&self) -> usize { self.tri.len() / 3 }
    pub fn automatic_lod(&self)->Option<std::sync::Arc<ClusterLod>> {self.auto_lod.clone()}
    pub fn has_textured_slots(&self) -> bool { self.textured.iter().any(|t|!t.slot.is_empty()) }
    pub fn positions(&self) -> &[f32] { &self.p }
    pub fn normals(&self) -> &[f32] { &self.n }
    pub fn point_indices(&self) -> &[u32] { &self.point }
    pub fn line_primitive_indices(&self) -> &[u32] { &self.line }
    pub fn triangle_indices(&self) -> &[u32] { &self.tri }
    /// Polygon edges as point pairs (no fan diagonals): element selection rebuilds polygons from these.
    pub fn edge_indices(&self) -> &[u32] { &self.edge }
    pub fn new(g: &Geometry) -> Self {
        let started = Instant::now();
        let timing = std::env::var_os("MINIHOU_VIEWPORT_TIMING").is_some();
        // Packed primitives / native instances: drawn at their transforms, CPU-batched into this scene's one buffer.
        let instances = minihou_core::nodes::packed::instance_count(g);
        let shown = viewport_geometry(g);
        let display_ms = started.elapsed().as_secs_f64() * 1000.0;
        let display = shown.as_ref().map_or(g, |(shown, _)| shown.as_ref());
        let source_triangles = if timing && instances > 0 {
            g.packed.iter().map(|s| minihou_core::preview::triangulate_geometry(s).prim.len()).sum::<usize>()
        } else { 0 };
        let mut scene = Self::from_display_geometry(display, timing);
        if instances>0 {
            use minihou_core::nodes::packed;
            let fallback=shown.as_ref().is_some_and(|(_,fallback)|*fallback);
            let ordinary=g.npoints-instances;
            let mut next=ordinary as u32;
            let mut source_map=vec![None;g.packed.len()];
            for (k,source) in g.packed.iter().enumerate() {
                if packed::estimated_display_counts(source).is_none() {continue;}
                let Ok(source_display)=packed::display_geometry(source) else {continue;};
                let source_scene=Self::from_display_geometry(source_display.as_ref(),timing);
                let lod=build_lod(&source_scene);
                source_map[k]=Some(scene.packed_sources.len());
                scene.packed_sources.push(PackedSource {scene:Box::new(source_scene),lod,instances:Vec::new()});
            }
            for i in 0..g.npoints {
                let Some(k)=packed::source_of(g,i) else {continue;};
                let count=if fallback {packed::instance_bounds(g,i).map_or(0,|_|8)}
                    else {packed::estimated_display_counts(&g.packed[k]).map_or(0,|v|v.0 as u32)};
                let range=next..next+count;next+=count;
                let Some(s)=source_map[k] else {continue;};
                let m=packed::xform(g,i);
                let rows=std::array::from_fn(|r|[m[r][0] as f32,m[r][1] as f32,m[r][2] as f32,g.p()[i*3+r]]);
                let proxy_tri=scene.tri.chunks_exact(3).position(|t|range.contains(&t[0]));
                scene.packed_sources[s].instances.push(PackedInstance{rows,range,proxy_tri,compact:fallback});
            }
        } else if instances==0 && scene.triangles()>=50_000 {
            scene.auto_lod=build_lod(&scene);
        }
        if timing {
            eprintln!("viewport Scene::new: instances={instances} source_triangles={source_triangles} viewport_points={} viewport_triangles={} bounds_fallback={} display_geometry_ms={display_ms:.3} total_ms={:.3}",
                scene.p.len()/3, scene.tri.len()/3, shown.as_ref().is_some_and(|(_, fallback)| *fallback), started.elapsed().as_secs_f64()*1000.0);
        }
        scene
    }
    fn from_display_geometry(g: &Geometry, timing: bool) -> Self {
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
        let names=g.detail.get("materials").and_then(|v|v.as_array());
        let bindings=g.detail.get("material_bindings").and_then(|v|v.get("slots")).and_then(|v|v.as_array());
        let has_named_slots = names.into_iter().flatten().chain(bindings.into_iter().flatten())
            .any(|slot| slot.as_str().is_some_and(|name| !name.is_empty()));
        let mut textured=Vec::new();
        if has_named_slots {
          let provenance=preview::triangulate_geometry(g);
          let corner_n=g.vertex.get("N").and_then(|a|a.as_f32());
          textured.reserve(provenance.prim.len());
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
        }
        let bvh_started=Instant::now();
        let pick_bvh=std::sync::Arc::new(crate::picking::Bvh::new(p,&l.tri));
        if timing { eprintln!("viewport BVH build: triangles={} ms={:.3}", l.tri.len()/3, bvh_started.elapsed().as_secs_f64()*1000.0); }
        Self { p:p.to_vec(),n,point,line,tri:l.tri,edge:l.edge,pick_bvh,nlp,nln,normal_points,bbp,center,radius,textured,cd:point_colors(g),packed_sources:Vec::new(),auto_lod:None }
    }
}

#[test]
fn untextured_scene_has_no_surface_tris_and_keeps_render_geometry() {
    let geo=Geometry::from_arrays(vec![0.,0.,0.,1.,0.,0.,0.,1.,0.],
        vec![0,1,2],vec![0,3],Some(vec![true])).unwrap();
    let scene=Scene::new(&geo);
    let layout=preview::layout(geo.p(),&geo.topology.vertex_point,&geo.topology.prim_offsets,
        geo.topology.closed.as_deref(),geo.npoints,usize::MAX,usize::MAX,100_000,8192);
    assert!(scene.textured.is_empty());
    assert!(!scene.has_textured_slots());
    assert_eq!(scene.positions(),geo.p());
    assert_eq!(scene.triangle_indices(),layout.tri);
    assert_eq!(scene.edge_indices(),layout.edge);
}

#[test]
fn lod_bake_source_instances_have_near_far_cuts_and_full_source_picking() {
    use minihou_core::{session::Session,nodes::packed::instance_geometry,math::IDENTITY};
    let graph=minihou_core::Graph::from_value(&serde_json::json!({"nodes":{
        "g":{"type":"grid","params":{"columns":32,"rows":32}},
        "b":{"type":"lod_bake","inputs":{"geo":"g"},"params":{"min_triangles":0}}
    },"display":"b"})).unwrap();
    let mut session=Session::new(graph);
    let source=session.cook(Some("b")).unwrap();
    assert!(session.side_payload::<ClusterLod>("b").is_some());
    let packed=instance_geometry(std::slice::from_ref(&source),&vec![0;60],
        (0..60).flat_map(|i|[i as f32*100.,0.,0.]).collect(),
        &[IDENTITY;60],Default::default()).unwrap();
    let scene=Scene::new(&packed);
    assert_eq!(scene.packed_sources.len(),1);
    assert_eq!(scene.packed_sources[0].instances.len(),60);
    assert_eq!(scene.positions().len()/3,480); // bounds fallback for drawing/pick IDs
    let lod=scene.packed_sources[0].lod.as_ref().unwrap();
    let mut near=Camera::frame([0.;3],1.);near.set_dist(2.);near.set_clip_planes(0.01,10000.);
    let mut far=Camera::frame([0.;3],1.);far.set_dist(2000.);far.set_clip_planes(0.01,10000.);
    let near_ids=lod.select(near.lod_camera([900,900]),1.);
    let far_ids=lod.select(far.lod_camera([900,900]),1.);
    assert!(!near_ids.is_empty() && !far_ids.is_empty());
    assert!(lod.selected_triangles(&far_ids)<=lod.selected_triangles(&near_ids));
    for ids in [&near_ids,&far_ids] {
        let (indices,_)=packed_cut(lod,ids);
        assert!(!indices.is_empty() && indices.iter().all(|&id|(id as usize)<source.npoints));
    }
    let hit=scene.pick_packed([100.,0.,10.],[0.,0.,-1.],|_|true).unwrap();
    let ranges=vec![("near".into(),0..8),("far".into(),8..16)];
    assert_eq!(crate::picked_object(&ranges,scene.triangle_indices(),hit),Some("far"));
}

#[test]
fn heavy_plain_scene_builds_lod_without_bake_node() {
    let graph=minihou_core::Graph::from_value(&serde_json::json!({"nodes":{
        "g":{"type":"grid","params":{"columns":160,"rows":160}}
    },"display":"g"})).unwrap();
    let geo=graph.cook(Some("g")).unwrap();
    let scene=Scene::new(&geo);
    assert!(scene.triangles()>=50_000);
    assert!(scene.automatic_lod().is_some());
}

#[test]
fn lod_bake_after_packed_copy_still_uses_source_cuts() {
    use minihou_core::session::Session;
    let graph=minihou_core::Graph::from_value(&serde_json::json!({"nodes":{
        "source":{"type":"grid","params":{"columns":32,"rows":32}},
        "targets":{"type":"grid","params":{"columns":2,"rows":2,"size_x":100.,"size_y":100.}},
        "copies":{"type":"copy_to_points","inputs":{"source":"source","points":"targets"}},
        "bake":{"type":"lod_bake","inputs":{"geo":"copies"},"params":{"min_triangles":0}}
    },"display":"bake"})).unwrap();
    let mut session=Session::new(graph);
    let packed=session.cook(Some("bake")).unwrap();
    assert_eq!(minihou_core::nodes::packed::instance_count(&packed),4);
    let scene=Scene::new(&packed);
    assert_eq!(scene.packed_sources.len(),1);
    let lod=scene.packed_sources[0].lod.as_ref().unwrap();
    for distance in [2.,2000.] {
        let mut camera=Camera::frame([0.;3],1.);
        camera.set_dist(distance);camera.set_clip_planes(0.01,10000.);
        let ids=lod.select(camera.lod_camera([900,900]),1.);
        let (cut,_)=packed_cut(lod,&ids);
        assert!(!cut.is_empty() && cut.iter().all(|&id|(id as usize)<lod.positions.len()));
    }
}

#[test]
fn textured_packed_bounds_pick_full_source_triangle() {
    use minihou_core::{geometry::Attr,nodes::packed::instance_geometry,math::IDENTITY};
    let mut source=Geometry::from_arrays(vec![0.,0.,0.,1.,0.,0.,0.,1.,0.],
        vec![0,1,2].repeat(1000),(0..=1000).map(|i|i*3).collect(),Some(vec![true;1000])).unwrap();
    source.prim.insert("material_index".into(),std::sync::Arc::new(Attr::i32(1,vec![0;1000])));
    source.detail.insert("materials".into(),serde_json::json!(["checker"]));
    let source=std::sync::Arc::new(source);
    let packed=instance_geometry(&[source],&vec![0;60],
        (0..60).flat_map(|i|[i as f32*100.,0.,0.]).collect(),&[IDENTITY;60],Default::default()).unwrap();
    let scene=Scene::new(&packed);
    assert_eq!(scene.positions().len()/3,480);
    assert_eq!(scene.packed_sources.len(),1);
    assert!(scene.packed_sources[0].lod.is_none());
    assert!(scene.pick_packed([100.2,0.2,10.],[0.,0.,-1.],|_|true).is_some());
    assert!(scene.pick_packed([100.8,0.8,10.],[0.,0.,-1.],|_|true).is_none());
}

#[test]
fn nested_packed_source_shares_one_cut_and_picks_inner_geometry() {
    use minihou_core::{nodes::packed::instance_geometry,math::IDENTITY};
    let triangle=std::sync::Arc::new(Geometry::from_arrays(vec![0.,0.,0.,1.,0.,0.,0.,1.,0.],
        vec![0,1,2],vec![0,3],Some(vec![true])).unwrap());
    let inner=std::sync::Arc::new(instance_geometry(&[triangle],&[0,0],
        vec![0.,0.,0.,2.,0.,0.],&[IDENTITY;2],Default::default()).unwrap());
    let outer=instance_geometry(&[inner],&[0,0],vec![0.,0.,0.,10.,0.,0.],
        &[IDENTITY;2],Default::default()).unwrap();
    let scene=Scene::new(&outer);
    assert_eq!(scene.packed_sources.len(),1);
    assert_eq!(scene.packed_sources[0].instances.len(),2);
    assert_eq!(scene.packed_sources[0].lod.as_ref().unwrap().positions.len(),6);
    let hit=scene.pick_packed([12.2,0.2,10.],[0.,0.,-1.],|_|true).unwrap();
    let ranges=vec![("first".into(),0..6),("second".into(),6..12)];
    assert_eq!(crate::picked_object(&ranges,scene.triangle_indices(),hit),Some("second"));
}

#[test]
fn cube_seams_expand_to_triangle_corners() {
    let graph=minihou_core::Graph::from_json(include_str!("../test_data/textured_cube.minihou")).unwrap();
    let geo=graph.cook(Some("output1")).unwrap(); let scene=Scene::new(&geo);
    assert_eq!(scene.positions().len()/3,8);
    assert_eq!(scene.textured.len(),12);
    assert_eq!(scene.textured.len()*3,36);
    assert_eq!(scene.textured.iter().map(|t|t.prim).collect::<std::collections::BTreeSet<_>>().len(),6);
    assert!(scene.textured.iter().all(|t|t.vertices.iter().all(|v|v.valid==1.)));
    let mut registry=Registry::new(concat!(env!("CARGO_MANIFEST_DIR"),"/src/test_data"));
    let document:serde_json::Value=serde_json::from_str(include_str!("../test_data/textured_cube.minihou")).unwrap();
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

/// Run with `MINIHOU_VIEWPORT_TIMING=1 cargo test -p minihou-app packed_cube_viewport_measure -- --ignored --nocapture`.
#[test]
#[ignore = "manual viewport timing helper"]
fn packed_cube_viewport_measure() {
    use minihou_core::nodes::packed::instance_geometry;
    use std::sync::Arc;
    let cube = Arc::new(Geometry::from_arrays(
        vec![0.,0.,0., 1.,0.,0., 1.,1.,0., 0.,1.,0., 0.,0.,1., 1.,0.,1., 1.,1.,1., 0.,1.,1.],
        vec![0,3,2,1, 4,5,6,7, 0,1,5,4, 2,3,7,6, 0,4,7,3, 1,2,6,5],
        vec![0,4,8,12,16,20,24], Some(vec![true;6])).unwrap());
    let graph = minihou_core::Graph::from_json(include_str!("../test_data/textured_cube.minihou")).unwrap();
    let textured = graph.cook(Some("output1")).unwrap();
    let mut split = (*textured).clone();
    split.prim.insert("material_index".into(), Arc::new(minihou_core::geometry::Attr::i32(1, vec![0,1,0,1,0,1])));
    split.detail.insert("material_bindings".into(), serde_json::json!({"slots":["SampleChecker","alternate"]}));
    let split = Arc::new(split);
    let instance = wgpu::Instance::default();
    let gpu = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()
        .and_then(|adapter| pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok());
    let mut system = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(std::process::id());
    for (label, source) in [("plain_cube", cube), ("textured_cube", textured), ("material_split_cube", split)] {
    for count in [1, 1_000, 10_000] {
        let positions = (0..count).flat_map(|i| [i as f32 * 2.0, 0., 0.]).collect();
        let packed = instance_geometry(&[source.clone()], &vec![0;count], positions,
            &vec![minihou_core::math::IDENTITY;count], Default::default()).unwrap();
        let scene = if label == "material_split_cube" {
            // Preserve a full-expansion baseline even after the viewport budget is enabled.
            let started = Instant::now();
            let expanded = minihou_core::nodes::packed::display_geometry(&packed).unwrap();
            let display_ms = started.elapsed().as_secs_f64() * 1000.0;
            let scene = Scene::from_display_geometry(&expanded, true);
            eprintln!("viewport baseline material_split_cube: copies={count} display_geometry_ms={display_ms:.3} total_scene_ms={:.3}",
                started.elapsed().as_secs_f64() * 1000.0);
            scene
        } else { Scene::new(&packed) };
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        let rss_mb = system.process(pid).map_or(0., |process| process.memory() as f64 / 1_048_576.);
        eprintln!("packed asset={label} count={count} cooked_points={} source_points={} source_triangles={} scene_points={} scene_triangles={} rss_after_scene_mib={rss_mb:.1}",
            packed.npoints, source.npoints, minihou_core::preview::triangulate_geometry(&source).prim.len(), scene.p.len()/3, scene.tri.len()/3);
        if let Some((device, _)) = &gpu {
            let mut uploaded = Gpu::new(device, &scene);
            uploaded.upload(device, &scene);
            system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
            let rss_mb = system.process(pid).map_or(0., |process| process.memory() as f64 / 1_048_576.);
            eprintln!("packed asset={label} count={count} rss_after_upload_mib={rss_mb:.1}");
        }
    }
    }
}

#[test]
fn packed_below_budget_matches_full_expansion() {
    use minihou_core::nodes::packed::{display_geometry, instance_geometry};
    let source = std::sync::Arc::new(Geometry::from_arrays(
        vec![0.,0.,0., 1.,0.,0., 0.,1.,0.], vec![0,1,2], vec![0,3], Some(vec![true])).unwrap());
    let packed = instance_geometry(&[source], &[0,0], vec![0.,0.,0., 2.,0.,0.],
        &[minihou_core::math::IDENTITY;2], Default::default()).unwrap();
    let (display, fallback) = viewport_geometry(&packed).unwrap();
    assert!(!fallback);
    let full = display_geometry(&packed).unwrap();
    assert_eq!(display.p(), full.p());
    assert_eq!(display.topology.vertex_point, full.topology.vertex_point);
    let actual = Scene::new(&packed);
    let expected = Scene::new(&full);
    assert_eq!(actual.p, expected.p);
    assert_eq!(actual.n, expected.n);
    assert_eq!(actual.tri, expected.tri);
    assert_eq!(actual.edge, expected.edge);
    assert_eq!(actual.textured.len(), expected.textured.len());
}

#[test]
fn packed_above_budget_uses_bounds_without_changing_cooked_geometry() {
    use minihou_core::nodes::packed::{estimated_display_counts, instance_geometry, instance_count};
    let source = std::sync::Arc::new(Geometry::from_arrays(
        vec![0.,0.,0., 1.,0.,0., 0.,1.,0.], vec![0,1,2].repeat(1000),
        (0..=1000).map(|i| i*3).collect(), Some(vec![true;1000])).unwrap());
    let packed = instance_geometry(&[source.clone()], &vec![0;100], vec![0.;300],
        &vec![minihou_core::math::IDENTITY;100], Default::default()).unwrap();
    assert_eq!(estimated_display_counts(&packed), Some((300, 100_000)));
    let (display, fallback) = viewport_geometry(&packed).unwrap();
    assert!(fallback);
    assert_eq!((display.npoints, display.nprims()), (800, 600));
    assert_eq!(Scene::new(&packed).triangles(), 1200);
    assert_eq!((packed.npoints, packed.nprims(), instance_count(&packed)), (100, 0, 100));
    assert!(std::sync::Arc::ptr_eq(&packed.packed[0], &source));
    assert_eq!((source.npoints, source.nprims()), (3, 1000));
}
/// Standalone points and non-surface primitive segments survive every proxy cut.
pub(super) fn element_indices(g:&Geometry, triangles:&[u32]) -> (Vec<u32>,Vec<u32>) {
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

