use super::*;

/// Longest overlay normal list kept: more points than this are thinned evenly.
const NORMAL_LINES: usize = 60_000;
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

