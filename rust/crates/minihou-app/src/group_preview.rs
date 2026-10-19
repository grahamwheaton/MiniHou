//! Cached overlay geometry from the cooked membership of the active Group Create and selection nodes.
use super::*;
#[derive(Default)]
pub struct State {key:Option<(u64,String)>}
impl State {
    pub fn update(&mut self,gpu:&mut Gpu,device:&wgpu::Device,g:Option<&minihou_core::Geometry>,input:Option<&minihou_core::Geometry>,doc:&Value,selected:&str,revision:u64) {
        let key=(revision,selected.to_owned());if self.key.as_ref()==Some(&key){return;}
        self.key=Some(key);
        let def=&doc["nodes"][selected];
        let group_name = node_group_name(def);
        let buffers = group_name.and_then(|name| g.map(|g| geometry(g, name)))
            .filter(|(faces,lines)| !faces.is_empty() || !lines.is_empty())
            .or_else(|| input.and_then(|g| input_group_geometry(g, def)));
        let (faces,lines)=buffers.unwrap_or_default();gpu.set_group_overlay(device,&faces,&lines);
    }
}
/// Input patterns belong to the selected node's input topology, even when another node is displayed.
fn input_group_geometry(g:&minihou_core::Geometry, def:&Value)->Option<(Vec<f32>,Vec<f32>)> {
    let typ=def["type"].as_str()?;
    let params=def["params"].as_object()?;
    for (name,value) in params {
        let meta=minihou_app::param_meta(typ,name);
        if !ui::params::is_input_group_field(name,meta) {continue;}
        let Some(text)=value.as_str().filter(|s| !s.trim().is_empty()) else {continue;};
        let named=geometry(g,text);
        if !named.0.is_empty() || !named.1.is_empty() {return Some(named);}
        let Some(mode)=ui::params::pick_domain_for(typ,name,meta,&def["params"]) else {continue;};
        let scene=Scene::new(g);
        let topo=element_select::Topology::new(scene.positions(),scene.triangle_indices(),scene.edge_indices());
        let selection=topo.parse_group(text,mode);
        let highlight=picking::highlight(scene.positions(),scene.triangle_indices(),&selection,scene.radius*0.012);
        if !highlight.0.is_empty() || !highlight.1.is_empty() {return Some(highlight);}
    }
    None
}
pub fn node_group_name(def: &Value) -> Option<&str> {
    match def["type"].as_str()? {
        "group_create" => Some(def["params"]["name"].as_str().unwrap_or("group1")),
        "boundary_select" => Some(def["params"]["output_group_name"].as_str().or_else(|| def["params"]["group_name"].as_str()).unwrap_or("boundary")),
        "select_sharp_edges" => Some(def["params"]["output_group_name"].as_str().or_else(|| def["params"]["group_name"].as_str()).unwrap_or("sharp_edges")),
        "edge_group" | "edge_group_promote" => Some(def["params"]["name"].as_str().unwrap_or("edges")),
        "select_loop" => Some(def["params"]["group_name"].as_str().unwrap_or("loop")),
        "select_ring" => Some(def["params"]["group_name"].as_str().unwrap_or("ring")),
        "select_path" => Some(def["params"]["group_name"].as_str().unwrap_or("path")),
        _ => def["params"]["output_group_name"].as_str().or_else(|| def["params"]["group_name"].as_str()),
    }
}
pub fn geometry(g:&minihou_core::Geometry,name:&str)->(Vec<f32>,Vec<f32>) {
    let key=format!("group_{name}");let mut faces=Vec::new();let mut lines=Vec::new();
    let member=|a:&minihou_core::geometry::Attr| match &a.data {
        minihou_core::geometry::AttrData::F32(v)=>v.chunks(a.width).map(|r|r[0]!=0.).collect::<Vec<_>>(),
        minihou_core::geometry::AttrData::I32(v)=>v.chunks(a.width).map(|r|r[0]!=0).collect::<Vec<_>>(),
    };
    if let Some(a)=g.prim.get(&key) {
        let mask=member(a);let layout=minihou_core::coplanar::preview_layout(g);
        for (tri,&prim) in layout.tri.chunks_exact(3).zip(&layout.tri_prim) {if mask.get(prim as usize)==Some(&true) {for &id in tri {faces.extend_from_slice(&g.p()[id as usize*3..id as usize*3+3]);}}}
        let mut edges=std::collections::BTreeSet::new();
        for (prim,selected) in mask.into_iter().take(g.topology.nprims()).enumerate() {if selected {
            let r=&g.topology.prim_offsets[prim..prim+2];let ids=&g.topology.vertex_point[r[0] as usize..r[1] as usize];
            for (i,&a) in ids.iter().enumerate() {if i+1<ids.len()||g.topology.is_closed(prim) {let b=ids[(i+1)%ids.len()];edges.insert((a.min(b),a.max(b)));}}
        }}
        for (a,b) in edges {for id in [a,b] {lines.extend_from_slice(&g.p()[id as usize*3..id as usize*3+3]);}}
    }else if let Some(a)=g.edge.get(&key).or_else(|| g.edge.get(&format!("edge_group_{name}"))) {
        let e=minihou_core::topo::edges(&g.topology,g.npoints);
        for (i,on) in member(a).into_iter().enumerate() {if on&&i*2+1<e.points.len() {for k in 0..2 {let id=e.points[i*2+k] as usize;lines.extend_from_slice(&g.p()[id*3..id*3+3]);}}}
    }else if let Some(a)=g.vertex.get(&format!("edge_group_{name}")) {
        let e=minihou_core::topo::edges(&g.topology,g.npoints);
        let on = member(a);
        let mut edge_on = vec![false; e.len()];
        for (v, &is_on) in on.iter().enumerate() {
            if is_on {
                let ed = e.of_vertex[v];
                if ed >= 0 { edge_on[ed as usize] = true; }
            }
        }
        for (i, is_on) in edge_on.into_iter().enumerate() {
            if is_on && i*2+1 < e.points.len() {
                for k in 0..2 {
                    let id = e.points[i*2+k] as usize;
                    lines.extend_from_slice(&g.p()[id*3..id*3+3]);
                }
            }
        }
    }else if let Some(a)=g.point.get(&key) {
        let selection=picking::Selection {mode:ui::SelectionMode::Points,points:member(a).into_iter().enumerate().filter(|(_,v)|*v).map(|(i,_)|i as u32).collect(),..Default::default()};
        (_,lines)=picking::highlight(g.p(),&[],&selection,0.01);
    }
    (faces,lines)
}

/// Membership of the named output group as a selection (primitive, edge, then point groups), for the target button click.
pub fn selection(g:&minihou_core::Geometry,name:&str)->Option<picking::Selection> {
    let key=format!("group_{name}");
    let on=|a:&minihou_core::geometry::Attr|->Vec<u32> {match &a.data {
        minihou_core::geometry::AttrData::F32(v)=>v.chunks(a.width).enumerate().filter(|(_,r)|r[0]!=0.).map(|(i,_)|i as u32).collect(),
        minihou_core::geometry::AttrData::I32(v)=>v.chunks(a.width).enumerate().filter(|(_,r)|r[0]!=0).map(|(i,_)|i as u32).collect(),
    }};
    if let Some(a)=g.prim.get(&key) {
        let members:std::collections::BTreeSet<u32>=on(a).into_iter().collect();let layout=minihou_core::coplanar::preview_layout(g);
        let tris=layout.tri_prim.iter().enumerate().filter(|(_,p)|members.contains(p)).map(|(t,_)|t as u32).collect();
        return Some(picking::Selection {mode:ui::SelectionMode::Faces,tris,..Default::default()});
    }
    if let Some(a)=g.edge.get(&key).or_else(|| g.edge.get(&format!("edge_group_{name}"))) {
        let e=minihou_core::topo::edges(&g.topology,g.npoints);
        let edges=on(a).into_iter().filter(|&i|(i as usize)*2+1<e.points.len()).map(|i|(e.points[i as usize*2] as u32,e.points[i as usize*2+1] as u32)).collect();
        return Some(picking::Selection {mode:ui::SelectionMode::Edges,edges,..Default::default()});
    }
    if let Some(a)=g.vertex.get(&format!("edge_group_{name}")) {
        let e=minihou_core::topo::edges(&g.topology,g.npoints);
        let mut edge_set = std::collections::BTreeSet::new();
        for v in on(a) {
            let ed = e.of_vertex[v as usize];
            if ed >= 0 { edge_set.insert(ed as usize); }
        }
        let edges = edge_set.into_iter().filter(|&i| i*2+1 < e.points.len()).map(|i|(e.points[i*2] as u32, e.points[i*2+1] as u32)).collect();
        return Some(picking::Selection {mode:ui::SelectionMode::Edges,edges,..Default::default()});
    }
    g.point.get(&key).map(|a|picking::Selection {mode:ui::SelectionMode::Points,points:on(a),..Default::default()})
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn point_groups_preview_only_their_members() {
        let doc=json!({"nodes":{"box":{"type":"box"},"group":{"type":"group_create","inputs":{"geo":"box"},"params":{"domain":"points","name":"tip","pattern":"0"}}},"display":"group"});
        let g=Graph::from_value(&doc).unwrap().cook(Some("group")).unwrap();
        let (faces,lines)=geometry(&g,"tip");assert!(faces.is_empty());assert_eq!(lines.len(),18);
        assert_eq!(geometry(&g,"missing"),(vec![],vec![]));
    }
    #[test] fn input_group_previews_on_its_source_geometry() {
        let doc=json!({"nodes":{"box":{"type":"box"},"blast":{"type":"blast","inputs":{"geo":"box"},"params":{"domain":"edges","group":"0-1"}}}});
        let g=Graph::from_value(&doc).unwrap().cook(Some("box")).unwrap();
        let (faces,lines)=input_group_geometry(&g,&doc["nodes"]["blast"]).unwrap();
        assert!(faces.is_empty());
        assert_eq!(lines.len(),6);
    }
    #[test] fn edge_groups_preview_their_edges() {
        let doc=json!({"nodes":{"box":{"type":"box"},"group":{"type":"group_create","inputs":{"geo":"box"},"params":{"domain":"edges","name":"e","pattern":"0"}}},"display":"group"});
        let g=Graph::from_value(&doc).unwrap().cook(Some("group")).unwrap();
        let (faces,lines)=geometry(&g,"e");assert!(faces.is_empty());assert_eq!(lines.len(),6);
    }
    #[test] fn edge_group_node_previews_its_named_group() {
        let doc=json!({"nodes":{"box":{"type":"box"},"eg":{"type":"edge_group","inputs":{"geo":"box"},"params":{"name":"e","pattern":"0"}}},"display":"eg"});
        assert_eq!(node_group_name(&doc["nodes"]["eg"]),Some("e"));
        let g=Graph::from_value(&doc).unwrap().cook(Some("eg")).unwrap();
        assert_eq!(geometry(&g,"e").1.len(),6);
    }
    #[test] fn select_sharp_edges_previews_sharp_edges() {
        let doc=json!({"nodes":{"box":{"type":"box"},"sharp":{"type":"select_sharp_edges","inputs":{"geo":"box"},"params":{"angle":30.0,"output_group_name":"sharp_edges"}}},"display":"sharp"});
        let g=Graph::from_value(&doc).unwrap().cook(Some("sharp")).unwrap();
        let (faces,lines)=geometry(&g,"sharp_edges");
        assert!(faces.is_empty());
        // 12 edges * 2 points * 3 coordinates = 72 floats
        assert_eq!(lines.len(), 72);
        assert_eq!(node_group_name(&doc["nodes"]["sharp"]), Some("sharp_edges"));
        let sel = selection(&g, "sharp_edges").unwrap();
        assert_eq!(sel.mode, ui::SelectionMode::Edges);
        assert_eq!(sel.edges.len(), 12);
    }
    #[test] fn boundary_select_previews_boundary_edges() {
        let doc=json!({"nodes":{"grid":{"type":"grid","params":{"rows":2,"columns":2}},"boundary":{"type":"boundary_select","inputs":{"geo":"grid"},"params":{"output_group_name":"boundary"}}},"display":"boundary"});
        let g=Graph::from_value(&doc).unwrap().cook(Some("boundary")).unwrap();
        let (faces,lines)=geometry(&g,"boundary");
        assert!(faces.is_empty());
        // A single quad (2x2 grid) has 4 boundary edges = 4 * 2 * 3 = 24 floats
        assert_eq!(lines.len(), 24);
        assert_eq!(node_group_name(&doc["nodes"]["boundary"]), Some("boundary"));
        let sel = selection(&g, "boundary").unwrap();
        assert_eq!(sel.mode, ui::SelectionMode::Edges);
        assert_eq!(sel.edges.len(), 4);
    }
}
