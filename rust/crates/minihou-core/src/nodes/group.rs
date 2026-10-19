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
    Ok(Geometry { packed: g.packed.clone(), npoints: rows.len(), topology: Arc::new(Topology { vertex_point: vp, prim_offsets: offs, closed }),
        point: gather_table(&g.point, &rows), vertex: gather_table(&g.vertex, &verts), edge: Default::default(),
        prim: gather_table(&g.prim, &prims), detail: g.detail.clone() })
}

pub const GROUP_CREATE:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None,name:"group_create",inputs:&[InputSpec{name:"geo",multi:false,optional:false},InputSpec{name:"bounds",multi:false,optional:true}],params:crate::params!(
    "name":"group1";"domain":"points";"method":"pattern";"pattern":"*";"box_center":[0.,0.,0.];"box_size":[1.,1.,1.];"direction":[0.,0.,1.];"angle":45.;"planes":"";"plane_offset":0.;"plane_tolerance":0.0001;"percent":50.;"seed":0;"combine":"replace";
    "base_enable":true,unsupported;"bounds_enable":false,unsupported;"bounds_type":"box",unsupported;"bounds_partial":false,unsupported;
    "normals_enable":false,unsupported;"backface_from":"",unsupported;"normals_opposite":false,unsupported;"edges_enable":false,unsupported;
    "min_edge_angle_on":false,unsupported;"min_edge_angle":20.,unsupported;"max_edge_angle_on":false,unsupported;"max_edge_angle":20.,unsupported;
    "min_edge_length_on":false,unsupported;"min_edge_length":0.,unsupported;"max_edge_length_on":false,unsupported;"max_edge_length":1.,unsupported;
    "unshared_edges":false,unsupported;"boundary_groups":false,unsupported;"random_enable":false,unsupported;"seed_attrib":"",unsupported),cook:create};
fn create(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=GROUP_CREATE.one(inputs,0)?;let name=p.s("name")?;valid_name(&name)?;let dom=p.e("domain",&["points","primitives","edges"])?;
    if dom=="edges"{return create_edges(&g,inputs,p,&name);}
    let dom=if dom=="points"{"point"}else{"prim"};let n=if dom=="point"{g.npoints}else{g.nprims()};let method=p.e("method",&["pattern","bounding_box","normal","coplanar","random","boundary","filters"])?;
    let mut m=match method.as_str(){
        "pattern"=>mask(&g,dom,&p.s("pattern")?)?,
        "bounding_box"=>{
            let mut c=p.v3("box_center")?;let mut size=p.v3("box_size")?;
            if let Some(bounds)=inputs[1].first(){if bounds.npoints>0{for k in 0..3{let lo=bounds.p().chunks_exact(3).map(|p|p[k] as f64).fold(f64::INFINITY,f64::min);let hi=bounds.p().chunks_exact(3).map(|p|p[k] as f64).fold(f64::NEG_INFINITY,f64::max);c[k]=(lo+hi)/2.;size[k]=hi-lo;}}}
            let pos=if dom=="point"{g.p().iter().map(|&v|v as f64).collect()}else{centroids64(&g)};
            pos.chunks_exact(3).map(|q|(0..3).all(|k|q[k]>=c[k]-size[k].abs()/2. && q[k]<=c[k]+size[k].abs()/2.)).collect()
        },
        "coplanar"=>{
            if dom!="prim" {return Err(Error::new("coplanar groups require the primitives domain"));}
            let text=p.s("planes")?;
            let planes=if text.trim().is_empty() {
                let direction=p.v3("direction")?;let length=direction.iter().map(|v|v*v).sum::<f64>().sqrt();
                if !length.is_finite()||length<1e-15 {return Err(Error::new("coplanar direction must be nonzero"));}
                vec![crate::coplanar::Plane {normal:direction.map(|v|v/length),offset:p.f("plane_offset")?}]
            }else{crate::coplanar::parse(&text)?};
            crate::coplanar::mask(&g,&planes,p.f("angle")?,p.f("plane_tolerance")?)?
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

/// Edge selection text: edge ids, `a-b` point pairs (either order), `*`, `^` exclusion, and edge group names.
fn edge_mask(g:&Geometry,e:&crate::topo::Edges,pattern:&str)->Result<Vec<bool>>{
    let n=e.len();let tokens:Vec<_>=pattern.split(|c:char|c==','||c.is_whitespace()).filter(|s|!s.is_empty()).collect();
    if tokens.is_empty(){return Ok(vec![true;n]);}
    let mut sel=vec![tokens.iter().all(|t|t.starts_with('^'));n];
    let mut excluded=vec![false;n];
    for t in tokens{
        let ex=t.starts_with('^');let t=t.strip_prefix('^').unwrap_or(t);if t.is_empty(){continue;}
        let mut hit=vec![false;n];
        if t=="*"{hit.fill(true);}
        else if let Some((a,b))=t.split_once('-').filter(|(a,b)|a.parse::<i64>().is_ok()&&b.parse::<i64>().is_ok()){
            let (a,b)=(a.parse::<i64>().unwrap(),b.parse::<i64>().unwrap());let (a,b)=(a.min(b),a.max(b));
            for i in 0..n{if e.points[i*2]==a&&e.points[i*2+1]==b{hit[i]=true;}}
        }else if let Ok(i)=t.parse::<usize>(){if i<n{hit[i]=true;}}
        else{for(k,a)in g.edge.iter(){if let Some(nm)=k.strip_prefix("group_"){if super::attribs::matches(t,nm){for(h,f)in hit.iter_mut().zip(flags(a)){*h|=f;}}}}}
        for((s,x),h)in sel.iter_mut().zip(&mut excluded).zip(hit){if ex{*x|=h;}else{*s|=h;}}
    }
    for(s,x)in sel.iter_mut().zip(excluded){*s &= !x;}
    Ok(sel)
}
fn create_edges(g:&Geometry,inputs:&Inputs,p:&Params,name:&str)->Result<Geometry>{
    let e=crate::topo::edges(&g.topology,g.npoints);let n=e.len();
    let method=p.e("method",&["pattern","bounding_box","normal","coplanar","random","boundary","filters"])?;
    let mut m=match method.as_str(){
        "pattern"=>edge_mask(g,&e,&p.s("pattern")?)?,
        "bounding_box"=>{
            let mut c=p.v3("box_center")?;let mut size=p.v3("box_size")?;
            if let Some(bounds)=inputs[1].first(){if bounds.npoints>0{for k in 0..3{let lo=bounds.p().chunks_exact(3).map(|p|p[k] as f64).fold(f64::INFINITY,f64::min);let hi=bounds.p().chunks_exact(3).map(|p|p[k] as f64).fold(f64::NEG_INFINITY,f64::max);c[k]=(lo+hi)/2.;size[k]=hi-lo;}}}
            let pt=g.p();
            (0..n).map(|i|{let (a,b)=(e.points[i*2] as usize,e.points[i*2+1] as usize);(0..3).all(|k|{let q=(pt[a*3+k] as f64+pt[b*3+k] as f64)/2.;q>=c[k]-size[k].abs()/2.&&q<=c[k]+size[k].abs()/2.})}).collect()
        },
        "boundary"=>{
            let mut cnt=vec![0usize;n];
            for i in 0..g.nprims(){if g.topology.is_closed(i){for v in g.topology.prim_offsets[i] as usize..g.topology.prim_offsets[i+1] as usize{let ed=e.of_vertex[v];if ed>=0{cnt[ed as usize]+=1;}}}}
            cnt.into_iter().map(|c|c==1).collect()
        },
        _=>return Err(Error::new(format!("group_create method {method:?} is not supported for edge groups"))),
    };
    let key=format!("group_{name}");let combine=p.e("combine",&["replace","union","intersect","subtract"])?;
    if let Some(a)=g.edge.get(&key){for(dst,old)in m.iter_mut().zip(flags(a)){*dst=match combine.as_str(){"union"=>*dst||old,"intersect"=>*dst&&old,"subtract"=>old&&!*dst,_=>*dst};}}
    else if combine=="intersect"||combine=="subtract"{m.fill(false);}
    let mut out=g.clone();out.edge.insert(key,Arc::new(Attr::i32(1,m.into_iter().map(i32::from).collect())));Ok(out)
}

pub const BLAST: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, name: "blast", inputs: GEO, params: crate::params!(
    "group":"";"domain":"primitives";"delete_non_selected":false;"keep_unused_points":false), cook: blast };
fn blast(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=BLAST.one(inputs,0)?;let dom=p.e("domain",&["primitives","points","edges"])?;
    if dom=="edges" {
        // Edges are implied by topology, so blasting them cannot delete anything: the selected (or, with delete_non_selected, the
        // unselected) edges are only cleared from every edge group. Topology and points are left unchanged.
        let pattern=p.s("group")?;let e=crate::topo::edges(&g.topology,g.npoints);
        let mut m=if pattern.split(|c:char|c==',' || c.is_whitespace()).all(|s|s.is_empty()){vec![false;e.len()]}else{edge_mask(&g,&e,&pattern)?};
        if p.b("delete_non_selected")?{for v in &mut m{*v=!*v;}}
        let mut out=(*g).clone();
        for (k,a) in g.edge.iter(){if k.starts_with("group_"){let f:Vec<i32>=flags(a).iter().zip(&m).map(|(&o,&d)|(o && !d) as i32).collect();out.edge.insert(k.clone(),Arc::new(Attr::i32(1,f)));}}
        return Ok(out);
    }
    let dom=if dom=="points"{"point"}else{"prim"};let pattern=p.s("group")?;let n=if dom=="point"{g.npoints}else{g.nprims()};
    let mut m=if pattern.split(|c:char|c==',' || c.is_whitespace()).all(|s|s.is_empty()){vec![false;n]}else{mask(&g,dom,&pattern)?};
    if !p.b("delete_non_selected")?{for v in &mut m{*v=!*v;}}
    if m.iter().all(|&v|v){return Ok((*g).clone());}
    if dom=="point"{super::modify::keep_points(&g,&m)}else{keep_prims(&g,&m,!p.b("keep_unused_points")?)}
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
    fn create_uses_bounds_input_and_combines_on_one_domain(){
        let mut g=Geometry::points_only(vec![0.,0.,0.,2.,0.,0.]).unwrap();g.point.insert("group_region".into(),Arc::new(Attr::i32(1,vec![0,1])));
        let bounds=Geometry::points_only(vec![-0.5,-0.5,-0.5,0.5,0.5,0.5]).unwrap();
        let p=Params::resolve(GROUP_CREATE.params,serde_json::json!({"name":"region","method":"bounding_box","combine":"union"}).as_object()).unwrap();
        let out=create(&[vec![Arc::new(g)],vec![Arc::new(bounds)]],&p).unwrap();assert_eq!(out.point["group_region"].data,AttrData::I32(vec![1,1]));out.validate().unwrap();
        assert!(Params::resolve(GROUP_CREATE.params,serde_json::json!({"bounds_enable":true}).as_object()).is_err());
    }
    fn cube_open(drop_top:bool)->Geometry{
        let pts=vec![0.,0.,0., 1.,0.,0., 1.,1.,0., 0.,1.,0., 0.,0.,1., 1.,0.,1., 1.,1.,1., 0.,1.,1.];
        let mut g=Geometry::points_only(pts).unwrap();
        let mut faces=vec![[0,3,2,1],[0,1,5,4],[1,2,6,5],[2,3,7,6],[3,0,4,7]];if !drop_top{faces.push([4,5,6,7]);}
        let vp:Vec<i32>=faces.iter().flatten().copied().collect();let offs:Vec<i32>=(0..=faces.len() as i32).map(|i|i*4).collect();
        g.topology=Arc::new(Topology{vertex_point:vp,prim_offsets:offs,closed:None});g
    }
    fn edge_group(g:Geometry,j:serde_json::Value)->Geometry{
        let mut j=j;j["domain"]="edges".into();let p=Params::resolve(GROUP_CREATE.params,j.as_object()).unwrap();
        let out=create(&[vec![Arc::new(g)],vec![]],&p).unwrap();out.validate().unwrap();out
    }
    fn count(g:&Geometry)->usize{flags(&g.edge["group_e"]).iter().filter(|&&b|b).count()}
    #[test]
    fn edge_group_pattern_pair_box_and_boundary(){
        let g=cube_open(false);assert_eq!(g.nedges(),12);
        let out=edge_group(g.clone(),serde_json::json!({"name":"e","pattern":"0-1"}));
        let f=flags(&out.edge["group_e"]);let e=crate::topo::edges(&out.topology,out.npoints);
        assert_eq!(count(&out),1);let i=f.iter().position(|&b|b).unwrap();assert_eq!((e.points[i*2],e.points[i*2+1]),(0,1));
        assert_eq!(count(&edge_group(g.clone(),serde_json::json!({"name":"e","pattern":"3"}))),1);
        let b=edge_group(g.clone(),serde_json::json!({"name":"e","method":"bounding_box","box_center":[0.5,0.5,0.0],"box_size":[2.,2.,0.1]}));
        assert_eq!(count(&b),4);
        assert_eq!(count(&edge_group(g,serde_json::json!({"name":"e","method":"boundary"}))),0);
        assert_eq!(count(&edge_group(cube_open(true),serde_json::json!({"name":"e","method":"boundary"}))),4);
        let pt=create(&[vec![Arc::new(cube_open(false))],vec![]],&Params::resolve(GROUP_CREATE.params,serde_json::json!({"name":"p","pattern":"0-2"}).as_object()).unwrap()).unwrap();
        assert_eq!(flags(&pt.point["group_p"]).iter().filter(|&&x|x).count(),3);assert!(pt.edge.is_empty());
    }
    #[test]
    fn blast_edges_clears_group_members_keeps_topology(){
        let g=edge_group(cube_open(false),serde_json::json!({"name":"e","pattern":"*"}));
        let p=Params::resolve(BLAST.params,serde_json::json!({"domain":"edges","group":"0-1"}).as_object()).unwrap();
        let out=blast(&[vec![Arc::new(g)]],&p).unwrap();out.validate().unwrap();
        assert_eq!(out.nedges(),12);assert_eq!(count(&out),11);
    }
}
