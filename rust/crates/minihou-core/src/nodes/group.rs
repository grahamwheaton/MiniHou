//! Group operations and attribute-preserving primitive subsets, mirroring nodes/group.py.
use std::sync::Arc;
use super::{InputSpec, Inputs, NodeSpec, GEO};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, AttrTable, Geometry, Topology}, params::Params};

pub(super) fn gather(a: &Attr, rows: &[usize]) -> Attr {
    let w = a.width;
    let data = match &a.data {
        AttrData::F32(v) => AttrData::F32(rows.iter().flat_map(|&i| v[i*w..(i+1)*w].iter().copied()).collect()),
        AttrData::I32(v) => AttrData::I32(rows.iter().flat_map(|&i| v[i*w..(i+1)*w].iter().copied()).collect()),
    };
    Attr { width: w, data }
}
pub(super) fn gather_table(t: &AttrTable, rows: &[usize]) -> AttrTable {
    t.iter().map(|(n,a)| (n.clone(), Arc::new(gather(a, rows)))).collect()
}
pub(super) fn flags(a: &Attr) -> Vec<bool> {
    match &a.data {
        AttrData::F32(v) => v.chunks(a.width).map(|r| r[0] != 0.0).collect(),
        AttrData::I32(v) => v.chunks(a.width).map(|r| r[0] != 0).collect(),
    }
}
pub(super) fn valid_name(n: &str) -> Result<()> {
    if n.is_empty() || !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || n.as_bytes()[0].is_ascii_digit() {
        return Err(Error::new(format!("invalid attribute/group name {n:?}")));
    }
    Ok(())
}
pub(super) fn centroids(g: &Geometry) -> Vec<f32> {
    centroids64(g).into_iter().map(|x| x as f32).collect()
}
pub(super) fn centroids64(g: &Geometry) -> Vec<f64> {
    let mut p = Vec::with_capacity(g.nprims()*3);
    for r in g.topology.prim_offsets.windows(2) {
        let ids = &g.topology.vertex_point[r[0] as usize..r[1] as usize];
        for c in 0..3 {
            let sum: f64 = ids.iter().map(|&i| g.p()[i as usize*3+c] as f64).sum();
            p.push(sum / ids.len().max(1) as f64);
        }
    }
    p
}
pub(super) fn mask(g: &Geometry, dom: &str, pattern: &str) -> Result<Vec<bool>> {
    if dom == "point" { return super::groups::point_mask(g, pattern); }
    // The existing parser operates on point rows. Present primitive centres and
    // primitive groups as points, converting opposite-domain groups first.
    let mut proxy = Geometry::points_only(centroids(g))?;
    for (name, a) in &g.point {
        if name.starts_with("group_") {
            let f = flags(a);
            let converted = g.topology.prim_offsets.windows(2).map(|r| {
                let ids = &g.topology.vertex_point[r[0] as usize..r[1] as usize];
                (!ids.is_empty() && ids.iter().all(|&i| f[i as usize])) as i32
            }).collect();
            proxy.point.insert(name.clone(), Arc::new(Attr::i32(1, converted)));
        }
    }
    for (name,a) in &g.prim { if name.starts_with("group_") { proxy.point.insert(name.clone(), a.clone()); } }
    super::groups::point_mask(&proxy, pattern)
}
pub(super) fn keep_prims(g: &Geometry, keep: &[bool], compact: bool) -> Result<Geometry> {
    let prims: Vec<_> = (0..g.nprims()).filter(|&i| keep[i]).collect();
    let mut verts = Vec::new();
    let mut offs = vec![0];
    for &i in &prims {
        verts.extend(g.topology.prim_offsets[i] as usize..g.topology.prim_offsets[i+1] as usize);
        offs.push(verts.len() as i32);
    }
    let mut vp: Vec<_> = verts.iter().map(|&i| g.topology.vertex_point[i]).collect();
    let mut used = vec![!compact; g.npoints];
    for &i in &vp { used[i as usize] = true; }
    let rows: Vec<_> = (0..g.npoints).filter(|&i| used[i]).collect();
    if compact {
        let mut remap = vec![0; g.npoints];
        for (i,&old) in rows.iter().enumerate() { remap[old] = i as i32; }
        for i in &mut vp { *i = remap[*i as usize]; }
    }
    let closed = g.topology.has_open().then(|| prims.iter().map(|&i| g.topology.is_closed(i)).collect());
    Ok(Geometry { npoints: rows.len(), topology: Arc::new(Topology { vertex_point: vp, prim_offsets: offs, closed }),
        point: gather_table(&g.point, &rows), vertex: gather_table(&g.vertex, &verts), prim: gather_table(&g.prim, &prims), detail: g.detail.clone() })
}

pub const GROUP_CREATE:NodeSpec=NodeSpec{name:"group_create",inputs:&[InputSpec{name:"geo",multi:false,optional:false},InputSpec{name:"bounds",multi:false,optional:true}],params:crate::params!(
    "name":"group1";"domain":"points";"method":"pattern";"pattern":"*";"box_center":[0.,0.,0.];"box_size":[1.,1.,1.];"direction":[0.,0.,1.];"angle":45.;"percent":50.;"seed":0;"combine":"replace";
    "base_enable":true,unsupported;"bounds_enable":false,unsupported;"bounds_type":"box",unsupported;"bounds_partial":false,unsupported;
    "normals_enable":false,unsupported;"backface_from":"",unsupported;"normals_opposite":false,unsupported;"edges_enable":false,unsupported;
    "min_edge_angle_on":false,unsupported;"min_edge_angle":20.,unsupported;"max_edge_angle_on":false,unsupported;"max_edge_angle":20.,unsupported;
    "min_edge_length_on":false,unsupported;"min_edge_length":0.,unsupported;"max_edge_length_on":false,unsupported;"max_edge_length":1.,unsupported;
    "unshared_edges":false,unsupported;"boundary_groups":false,unsupported;"random_enable":false,unsupported;"seed_attrib":"",unsupported),cook:create};
fn create(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=GROUP_CREATE.one(inputs,0)?;let name=p.s("name")?;valid_name(&name)?;let dom=p.e("domain",&["points","primitives","edges"])?;
    if dom=="edges"{return Err(Error::new("group_create edge groups require the edge-group subsystem, not supported by the Rust backend yet"));}
    let dom=if dom=="points"{"point"}else{"prim"};let n=if dom=="point"{g.npoints}else{g.nprims()};let method=p.e("method",&["pattern","bounding_box","normal","random","boundary","filters"])?;
    let mut m=match method.as_str(){
        "pattern"=>mask(&g,dom,&p.s("pattern")?)?,
        "bounding_box"=>{
            let mut c=p.v3("box_center")?;let mut size=p.v3("box_size")?;
            if let Some(bounds)=inputs[1].first(){if bounds.npoints>0{for k in 0..3{let lo=bounds.p().chunks_exact(3).map(|p|p[k] as f64).fold(f64::INFINITY,f64::min);let hi=bounds.p().chunks_exact(3).map(|p|p[k] as f64).fold(f64::NEG_INFINITY,f64::max);c[k]=(lo+hi)/2.;size[k]=hi-lo;}}}
            let pos=if dom=="point"{g.p().iter().map(|&v|v as f64).collect()}else{centroids64(&g)};
            pos.chunks_exact(3).map(|q|(0..3).all(|k|q[k]>=c[k]-size[k].abs()/2. && q[k]<=c[k]+size[k].abs()/2.)).collect()
        },
        "normal"=>{
            let d=p.v3("direction")?;let ln=d.iter().map(|v|v*v).sum::<f64>().sqrt().max(1e-12);let d=d.map(|v|v/ln);
            let normals=if dom=="point"{g.point.get("N").filter(|a|a.width==3).and_then(|a|a.as_f32()).map(|v|v.to_vec()).unwrap_or_else(||crate::normals::point_normals(g.p(),&g.topology,g.npoints))}
                else{crate::normals::normalize_to_f32(&crate::normals::prim_normals(g.p(),&g.topology))};
            let limit=p.f("angle")?.to_radians().cos()-1e-9;
            normals.chunks_exact(3).map(|q|(0..3).map(|k|q[k] as f64*d[k]).sum::<f64>()>=limit).collect()
        },
        "boundary"=>{
            let e=crate::topo::edges(&g.topology,g.npoints);let mut bp=vec![false;g.npoints];let mut prim=vec![false;g.nprims()];
            let mut polygon_count=vec![0usize;e.len()];
            for i in 0..g.nprims(){if g.topology.is_closed(i){for v in g.topology.prim_offsets[i] as usize..g.topology.prim_offsets[i+1] as usize{let edge=e.of_vertex[v];if edge>=0{polygon_count[edge as usize]+=1;}}}}
            for (edge,&count)in polygon_count.iter().enumerate(){if count==1{bp[e.points[edge*2] as usize]=true;bp[e.points[edge*2+1] as usize]=true;}}
            for i in 0..g.nprims(){for v in g.topology.prim_offsets[i] as usize..g.topology.prim_offsets[i+1] as usize{let edge=e.of_vertex[v];if edge>=0 && e.count[edge as usize]==1{prim[i]=true;}}}
            if dom=="point"{bp}else{prim}
        },
        "random"=>return Err(Error::new("group_create random requires NumPy-compatible PCG64/SeedSequence, not supported by the Rust backend yet")),
        _=>return Err(Error::new("group_create combinable filters are not supported by the Rust backend yet")),
    };
    debug_assert_eq!(m.len(),n);let key=format!("group_{name}");let combine=p.e("combine",&["replace","union","intersect","subtract"])?;
    if let Some(a)=g.table(dom).get(&key){for(dst,old)in m.iter_mut().zip(flags(a)){*dst=match combine.as_str(){"union"=>*dst || old,"intersect"=>*dst && old,"subtract"=>old && !*dst,_=>*dst};}}
    else if combine=="intersect" || combine=="subtract"{m.fill(false);}
    let mut out=(*g).clone();let a=Arc::new(Attr::i32(1,m.into_iter().map(i32::from).collect()));if dom=="point"{out.prim.remove(&key);out.point.insert(key,a);}else{out.point.remove(&key);out.prim.insert(key,a);}Ok(out)
}

pub const GROUP_PROMOTE: NodeSpec = NodeSpec { name: "group_promote", inputs: GEO,
    params: crate::params!("name": "group1"; "to": "points"), cook: promote };
fn promote(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = GROUP_PROMOTE.one(inputs,0)?;
    let pattern = p.s("name")?;
    let to = p.e("to", &["points","primitives"])?;
    let mut out = (*g).clone();
    let source = if to == "points" { &g.prim } else { &g.point };
    let names: Vec<_> = source.keys().filter_map(|n| n.strip_prefix("group_"))
        .filter(|n| super::attribs::matches(&pattern,n)).map(str::to_owned).collect();
    if names.is_empty() { return Err(Error::new(format!("no source group matching {pattern:?}"))); }
    for name in names {
    let key = format!("group_{name}");
    if to == "points" {
        let a = g.prim.get(&key).ok_or_else(|| Error::new(format!("no primitive group {name:?}")))?;
        let f = flags(a); let mut m = vec![0; g.npoints];
        for (i,&b) in f.iter().enumerate() { if b {
            for &v in &g.topology.vertex_point[g.topology.prim_offsets[i] as usize..g.topology.prim_offsets[i+1] as usize] { m[v as usize] = 1; }
        } }
        out.prim.remove(&key); out.point.insert(key, Arc::new(Attr::i32(1,m)));
    } else {
        let a = g.point.get(&key).ok_or_else(|| Error::new(format!("no point group {name:?}")))?;
        let f = flags(a);
        let m = g.topology.prim_offsets.windows(2).map(|r| {
            let ids = &g.topology.vertex_point[r[0] as usize..r[1] as usize];
            (!ids.is_empty() && ids.iter().all(|&i| f[i as usize])) as i32
        }).collect();
        out.point.remove(&key); out.prim.insert(key, Arc::new(Attr::i32(1,m)));
    }
    }
    Ok(out)
}

pub const BLAST: NodeSpec = NodeSpec { name: "blast", inputs: GEO, params: crate::params!(
    "group":"";"domain":"primitives";"delete_non_selected":false;"keep_unused_points":false), cook: blast };
fn blast(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=BLAST.one(inputs,0)?;let dom=p.e("domain",&["primitives","points","edges"])?;
    if dom=="edges" {return Err(Error::new("blast edge selections require the edge-group parser, not supported by the Rust backend yet"));}
    let dom=if dom=="points"{"point"}else{"prim"};let pattern=p.s("group")?;let n=if dom=="point"{g.npoints}else{g.nprims()};
    let mut m=if pattern.split(|c:char|c==',' || c.is_whitespace()).all(|s|s.is_empty()){vec![false;n]}else{mask(&g,dom,&pattern)?};
    if !p.b("delete_non_selected")?{for v in &mut m{*v=!*v;}}
    if m.iter().all(|&v|v){return Ok((*g).clone());}
    if dom=="point"{super::modify::keep_points(&g,&m)}else{keep_prims(&g,&m,!p.b("keep_unused_points")?)}
}
pub const GROUP_EXPAND:NodeSpec=NodeSpec{name:"group_expand",inputs:GEO,params:crate::params!(
    "group":"group1";"name":"";"domain":"primitives";"mode":"expand";"steps":1;"connectivity":"points"),cook:expand};
fn expand(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=GROUP_EXPAND.one(inputs,0)?;let dom=p.e("domain",&["primitives","points"])?;let dom=if dom=="points"{"point"}else{"prim"};
    let pattern=p.s("group")?;let name=p.s("name")?;let name=if name.is_empty(){pattern.clone()}else{name};valid_name(&name)?;
    let mut m=mask(&g,dom,&pattern)?;let contract=p.e("mode",&["expand","contract"])?=="contract";let edges=p.e("connectivity",&["points","edges"])?=="edges";
    let steps=p.i("steps")?;if !(1..=1000).contains(&steps){return Err(Error::new("steps must be 1..1000"));}
    let e=crate::topo::edges(&g.topology,g.npoints);let mut pv=Vec::new();for i in 0..g.nprims(){pv.extend(std::iter::repeat_n(i,(g.topology.prim_offsets[i+1]-g.topology.prim_offsets[i]) as usize));}
    for _ in 0..steps{
        let old:Vec<_>=m.iter().map(|&v|if contract{!v}else{v}).collect();let mut grown=old.clone();
        if dom=="point"{for pair in e.points.chunks_exact(2){let(a,b)=(pair[0] as usize,pair[1] as usize);grown[a]|=old[b];grown[b]|=old[a];}}
        else if edges{let mut hit=vec![false;e.len()];for(v,&i)in e.of_vertex.iter().enumerate(){if i>=0 && old[pv[v]]{hit[i as usize]=true;}}for(v,&i)in e.of_vertex.iter().enumerate(){if i>=0 && hit[i as usize]{grown[pv[v]]=true;}}}
        else{let mut hit=vec![false;g.npoints];for(v,&pt)in g.topology.vertex_point.iter().enumerate(){if old[pv[v]]{hit[pt as usize]=true;}}for(v,&pt)in g.topology.vertex_point.iter().enumerate(){if hit[pt as usize]{grown[pv[v]]=true;}}}
        m=grown.into_iter().map(|v|if contract{!v}else{v}).collect();
    }
    let mut out=(*g).clone();let key=format!("group_{name}");let a=Arc::new(Attr::i32(1,m.into_iter().map(i32::from).collect()));
    if dom=="point"{out.prim.remove(&key);out.point.insert(key,a);}else{out.point.remove(&key);out.prim.insert(key,a);}Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subset_preserves_domains_and_open_flags() {
        let mut g = Geometry::from_arrays(vec![0.;12],vec![2,0,1,2],vec![0,2,4],Some(vec![false,true])).unwrap();
        g.vertex.insert("uv".into(),Arc::new(Attr::f32(1,vec![1.,2.,3.,4.])));
        let out = keep_prims(&g,&[true,false],true).unwrap();
        assert_eq!(out.npoints,2); assert_eq!(out.topology.vertex_point,[1,0]);
        assert_eq!(out.vertex["uv"].as_f32().unwrap(),[1.,2.]);
        assert!(!out.topology.is_closed(0)); out.validate().unwrap();
    }
    #[test]
    fn blast_empty_is_no_selection_and_midpoint_splits_curve(){
        let g=Arc::new(Geometry::from_arrays(vec![0.;15],vec![0,1,2,3,4],vec![0,5],Some(vec![false])).unwrap());
        let p=Params::resolve(BLAST.params,None).unwrap();assert_eq!(blast(&[vec![g.clone()]],&p).unwrap().npoints,5);
        let p=Params::resolve(BLAST.params,serde_json::json!({"domain":"points","group":"2"}).as_object()).unwrap();let out=blast(&[vec![g]],&p).unwrap();assert_eq!(out.topology.prim_offsets,[0,2,4]);assert_eq!(out.npoints,4);out.validate().unwrap();
    }
    #[test]
    fn group_expand_one_ring_and_promote_wildcards(){
        let mut g=Geometry::from_arrays(vec![0.;12],vec![0,1,2,3],vec![0,4],Some(vec![false])).unwrap();g.point.insert("group_tip".into(),Arc::new(Attr::i32(1,vec![1,0,0,0])));
        let p=Params::resolve(GROUP_EXPAND.params,serde_json::json!({"domain":"points","group":"tip"}).as_object()).unwrap();let out=expand(&[vec![Arc::new(g)]],&p).unwrap();assert_eq!(out.point["group_tip"].data,AttrData::I32(vec![1,1,0,0]));
        let p=Params::resolve(GROUP_PROMOTE.params,serde_json::json!({"name":"t*","to":"primitives"}).as_object()).unwrap();let out=promote(&[vec![Arc::new(out)]],&p).unwrap();assert_eq!(out.prim["group_tip"].data,AttrData::I32(vec![0]));assert!(!out.point.contains_key("group_tip"));
    }
    #[test]
    fn create_uses_bounds_input_and_combines_on_one_domain(){
        let mut g=Geometry::points_only(vec![0.,0.,0.,2.,0.,0.]).unwrap();g.point.insert("group_region".into(),Arc::new(Attr::i32(1,vec![0,1])));
        let bounds=Geometry::points_only(vec![-0.5,-0.5,-0.5,0.5,0.5,0.5]).unwrap();
        let p=Params::resolve(GROUP_CREATE.params,serde_json::json!({"name":"region","method":"bounding_box","combine":"union"}).as_object()).unwrap();
        let out=create(&[vec![Arc::new(g)],vec![Arc::new(bounds)]],&p).unwrap();assert_eq!(out.point["group_region"].data,AttrData::I32(vec![1,1]));out.validate().unwrap();
        assert!(Params::resolve(GROUP_CREATE.params,serde_json::json!({"bounds_enable":true}).as_object()).is_err());
    }
}
