//! Live inputs are owned by a Session, never by process-global node state.
use std::{collections::HashMap, sync::Arc};
use serde_json::Value;
use crate::{Geometry, Attr, Error, Result, params::Params};
use super::{Inputs, NodeSpec};

#[derive(Clone, Debug)]
pub enum ExternalInput {
    Object { geo: Arc<Geometry>, matrix: Option<[f64;16]>, info: Value },
    Image { pixels: Arc<Vec<f32>>, width: usize, height: usize, meta: Value },
    Missing(String),
}
pub type ExternalInputs = HashMap<String, ExternalInput>;
pub const BLENDER_OBJECT: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, name:"blender_object", inputs:&[], params:crate::params!("object":""; "space":"world"; "join_edges":true; "reset_transform":false), cook:waiting_object };
/// Dynamic Blender interface schema stored on modifier_input graph nodes.
/// Geometry is the node's regular output; scalar values resolve through parameter links.
pub const MODIFIER_INPUT_KINDS: &[&str] = &[
    "float", "int", "bool", "vector", "color", "rotation", "matrix", "string", "menu",
    "object", "collection", "material", "image", "texture", "geometry", "bundle", "closure",
];
pub const MODIFIER_INPUT: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, name:"modifier_input", inputs:&[], params:&[], cook:modifier_input };
pub const OBJECT_INPUT: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, name:"object_input", inputs:&[], params:crate::params!("binding":""), cook:object_input };
fn object_input(_: &Inputs, _: &Params) -> Result<Geometry> { Err(Error::waiting("Choose an object in this modifier's Object input picker in Blender")) }
/// What a Blender Object node tells the user while nothing is picked.
pub const PICK_HINT: &str = "Pick an object in the MiniHou group in Blender, or connect Blender";
/// External store key of a `blender_object` node's group picker feed (set only in Blender-bound cooks).
pub fn picker_external(id: &str) -> String { format!("picker:{id}") }

/// Modifier-local feeds use a distinct namespace from named world-space Blender objects.
pub(crate) fn binding_key(id: &str, p: &Params) -> Result<String> {
    let binding=p.s("binding")?;
    Ok(format!("object_input:{}",if binding.is_empty(){id}else{&binding}))
}
pub(crate) fn cook_binding(id: &str, p: &Params, store: &ExternalInputs) -> Result<Geometry> {
    match store.get(&binding_key(id,p)?) {
        Some(ExternalInput::Object{geo,..})=>Ok((**geo).clone()),
        Some(ExternalInput::Missing(message))=>Err(Error::waiting(message)),
        _=>object_input(&[],p),
    }
}
fn waiting_object(_: &Inputs, p: &Params) -> Result<Geometry> { object(p, &ExternalInputs::new()) }
fn modifier_input(_: &Inputs, _: &Params) -> Result<Geometry> { Err(Error::new("modifier_input requires a MiniHou modifier instance and its preceding stack geometry")) }

pub fn cook_external(typ: &str, p: &Params, store: &ExternalInputs) -> Option<Result<Geometry>> {
    match typ {
        "blender_object" => Some(object(p,store)),
        "blender_image" => Some(super::image_nodes::blender_image(p,store)),
        "modifier_input" => Some(match store.get("modifier_input") { Some(ExternalInput::Object{geo,..}) => Ok((**geo).clone()), _ => modifier_input(&[],p) }),
        _ => None,
    }
}
/// Missing-object reasons the add-on sends that mean "nothing to read yet", not a broken object.
fn absent(reason: &str) -> bool { reason.starts_with("no object named") || reason == "not found" }

/// The external data a `blender_object` node reads: its group picker feed when the cook is Blender-bound
/// (`picker:<id>` present), else the object named by its `object` param. Waiting conditions are
/// [`Error::waiting`]; a picked object that cannot be read stays a real error.
pub fn resolve_object<'a>(id: &str, p: &Params, store: &'a ExternalInputs) -> Result<&'a ExternalInput> {
    if let Some(feed)=store.get(&picker_external(id)) {
        return match feed {
            ExternalInput::Object{..}=>Ok(feed),
            ExternalInput::Missing(m) if m.is_empty() => Err(Error::waiting(format!("no object picked. {PICK_HINT}"))),
            ExternalInput::Missing(m)=>Err(Error::new(m.clone())),
            _=>Err(Error::new("invalid picker input")),
        };
    }
    let name=p.s("object")?;
    if name.is_empty() { return Err(Error::waiting(format!("no object set. {PICK_HINT} (or type an object name)"))); }
    if ["MiniHou_Output","MiniHou_Proxy"].contains(&name.as_str()) { return Err(Error::new(format!("{name:?} is MiniHou's own output; reading it would feed the result back into itself"))); }
    let data=store.get(&format!("object:{name}")).ok_or_else(|| Error::waiting(format!("waiting for Blender object {name:?}. {PICK_HINT}")))?;
    match data {
        ExternalInput::Object{..}=>Ok(data),
        ExternalInput::Missing(e) if absent(e) => Err(Error::waiting(format!("Blender object {name:?} not found. {PICK_HINT}"))),
        ExternalInput::Missing(e)=>Err(Error::new(format!("Blender object {name:?}: {e}"))),
        _=>Err(Error::new(format!("Blender object {name:?}: invalid input"))),
    }
}
/// Waiting text for an external-data node, without cooking it (so cache hits still report the state).
/// Nodes using the waiting state: blender_object, blender_image, object_input.
pub fn waiting_reason(id: &str, typ: &str, p: &Params, store: &ExternalInputs) -> Option<String> {
    let err=match typ {
        "blender_object"=>resolve_object(id,p,store).err(),
        "blender_image"=>super::image_nodes::resolve_image(p,store).err(),
        "object_input"=>match binding_key(id,p) {
            Ok(k)=>match store.get(&k) {
                Some(ExternalInput::Object{..})=>None,
                Some(ExternalInput::Missing(m))=>Some(Error::waiting(m.clone())),
                _=>Some(Error::waiting("Choose an object in this modifier's Object input picker in Blender")),
            },
            Err(e)=>Some(e),
        },
        _=>None,
    }?;
    err.waiting_text().map(str::to_owned)
}
pub fn cook_object(id: &str, p: &Params, store: &ExternalInputs) -> Result<Geometry> { build_object(p, resolve_object(id,p,store)?) }
fn object(p: &Params, store: &ExternalInputs) -> Result<Geometry> { cook_object("", p, store) }
fn build_object(p: &Params, data: &ExternalInput) -> Result<Geometry> {
    let ExternalInput::Object { geo, matrix, info }=data else { return Err(Error::new("invalid input")); };
    let mut out=(**geo).clone();
    if p.b("join_edges")? && geo.topology.has_open() && !info["components"].as_array().is_some_and(|a| a.iter().any(|v| v=="curves")) {
        out=join_edges(geo)?;
    }
    if p.e("space", &["world","object"])?=="object" { return Ok(out); }
    let Some(&m0)=matrix.as_ref() else { return Ok(out); };
    // Reset transform: keep rotation and scale, drop the location so the object sits at the origin.
    let mut m=m0;
    if p.b("reset_transform")? { m[3]=0.; m[7]=0.; m[11]=0.; }
    if m.iter().enumerate().all(|(i,x)| (*x-if i/4==i%4 {1.} else {0.}).abs() <= 1e-8+1e-5*if i/4==i%4 {1.} else {0.}) { return Ok(out); }
    let a=[[m[0],m[1],m[2]],[m[4],m[5],m[6]],[m[8],m[9],m[10]]];
    let det=a[0][0]*(a[1][1]*a[2][2]-a[1][2]*a[2][1])-a[0][1]*(a[1][0]*a[2][2]-a[1][2]*a[2][0])+a[0][2]*(a[1][0]*a[2][1]-a[1][1]*a[2][0]);
    if det==0. { return Err(Error::new("Singular matrix")); }
    let mut normal=[[0.;3];3];
    for i in 0..3 { for j in 0..3 { normal[i][j]=(a[(i+1)%3][(j+1)%3]*a[(i+2)%3][(j+2)%3]-a[(i+1)%3][(j+2)%3]*a[(i+2)%3][(j+1)%3])/det; } }
    let positions=out.p().chunks_exact(3).flat_map(|v| (0..3).map(move |i| (v[0] as f64*a[i][0]+v[1] as f64*a[i][1]+v[2] as f64*a[i][2]+m[i*4+3]) as f32)).collect();
    out.set_p(positions);
    for table in [&mut out.point,&mut out.vertex,&mut out.prim] {
        if let Some(attr)=table.get("N").filter(|a| a.width==3) {
            let values=numbers(attr);
            let mut ns=Vec::with_capacity(values.len());
            for v in values.chunks_exact(3) {
                let n=normal.map(|row| row[0]*v[0]+row[1]*v[1]+row[2]*v[2]);
                let len=n.iter().map(|x|x*x).sum::<f64>().sqrt().max(1e-12);
                ns.extend(n.map(|x|(x/len) as f32));
            }
            table.insert("N".into(),Arc::new(Attr::f32(3,ns)));
        }
    }
    Ok(out)
}
pub(crate) fn numbers(a: &Attr) -> Vec<f64> { match &a.data { crate::AttrData::F32(v)=>v.iter().map(|&x|x as f64).collect(), crate::AttrData::I32(v)=>v.iter().map(|&x|x as f64).collect() } }

// PolyPath semantics: polygons retained first; new curves have zeroed vertex/primitive attributes.
fn join_edges(g: &Geometry) -> Result<Geometry> {
    let t=&g.topology;
    let mut edges=Vec::new(); let mut adjacency=vec![Vec::new();g.npoints];
    let mut vp=Vec::new(); let mut offsets=vec![0]; let mut closed=Vec::new(); let mut corners=Vec::new(); let mut prims=Vec::new();
    for i in 0..g.nprims() {
        let range=t.prim_offsets[i] as usize..t.prim_offsets[i+1] as usize;
        if t.is_closed(i) { corners.extend(range.clone()); prims.push(i); vp.extend_from_slice(&t.vertex_point[range]); offsets.push(vp.len() as i32); closed.push(true); }
        else { for pair in t.vertex_point[range].windows(2) { let id=edges.len(); edges.push((pair[0],pair[1])); adjacency[pair[0] as usize].push(id); adjacency[pair[1] as usize].push(id); } }
    }
    let mut used=vec![false;edges.len()];
    fn walk(start:usize,edge:usize,edges:&[(i32,i32)],adjacency:&[Vec<usize>],used:&mut [bool])->Vec<i32>{
        let mut chain=vec![start as i32];let mut cur=start;let mut e=edge;
        loop{used[e]=true;let (a,b)=edges[e];let next=if b as usize==cur{a as usize}else{b as usize};chain.push(next as i32);
            if adjacency[next].len()!=2{break;}
            let Some(&ne)=adjacency[next].iter().find(|&&j|!used[j])else{break;};cur=next;e=ne;
        }chain
    }
    for start in 0..g.npoints {if adjacency[start].len()==2 {continue;}for &edge in &adjacency[start]{
        if used[edge]{continue;}let chain=walk(start,edge,&edges,&adjacency,&mut used);
        vp.extend(chain);offsets.push(vp.len() as i32);closed.push(false);
    }}
    for edge in 0..edges.len(){if used[edge]{continue;}let mut chain=walk(edges[edge].0 as usize,edge,&edges,&adjacency,&mut used);chain.pop();
        vp.extend(chain);offsets.push(vp.len() as i32);closed.push(true);
    }
    let mut out=Geometry::from_arrays(g.p().to_vec(),vp,offsets,Some(closed))?;
    out.point=g.point.clone();out.detail=g.detail.clone();
    for (source, dest, rows, count) in [(&g.vertex,&mut out.vertex,&corners,out.topology.nvertices()),(&g.prim,&mut out.prim,&prims,out.topology.nprims())] {
        for (name,a) in source { let mut attr=a.zeros_like(count); match (&a.data,&mut attr.data) { (crate::AttrData::F32(v),crate::AttrData::F32(o))=>for (i,&r) in rows.iter().enumerate(){o[i*a.width..(i+1)*a.width].copy_from_slice(&v[r*a.width..(r+1)*a.width]);}, (crate::AttrData::I32(v),crate::AttrData::I32(o))=>for (i,&r) in rows.iter().enumerate(){o[i*a.width..(i+1)*a.width].copy_from_slice(&v[r*a.width..(r+1)*a.width]);}, _=>unreachable!() }; dest.insert(name.clone(),Arc::new(attr)); }
    }
    Ok(out)
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn object_input_binding_is_local_and_invalidates_session_cache() {
        use serde_json::json;
        let doc=json!({"display":"src","nodes":{"src":{"type":"object_input"}}});
        let mut session=crate::session::Session::new(crate::Graph::from_value(&doc).unwrap());
        assert_eq!(session.cook(None).unwrap().npoints,0);
        assert!(session.last_stats().waiting[0].1.contains("Object input picker"));
        for x in [1.,2.] {
            session.set_external("object_input:src".into(),ExternalInput::Object{geo:Arc::new(Geometry::points_only(vec![x,0.,0.]).unwrap()),matrix:Some([2.;16]),info:Value::Null});
            assert_eq!(session.cook(None).unwrap().p(),&[x,0.,0.]);
            assert!(session.last_stats().cooked.contains(&"src".into()));
        }
        assert!(session.last_stats().waiting.is_empty());
        session.set_param("src","binding",json!("tree")).unwrap();assert_eq!(session.cook(None).unwrap().npoints,0);
        session.set_external("object_input:tree".into(),ExternalInput::Object{geo:Arc::new(Geometry::points_only(vec![9.,0.,0.]).unwrap()),matrix:None,info:Value::Null});
        assert_eq!(session.cook(None).unwrap().p()[0],9.);
    }
    #[test] fn world_normals_and_local_space() {
        let mut g=Geometry::points_only(vec![1.,2.,3.]).unwrap(); g.point.insert("N".into(),Arc::new(Attr::f32(3,vec![1.,1.,0.])));
        let mut store=ExternalInputs::new();store.insert("object:Cube".into(),ExternalInput::Object{geo:Arc::new(g),matrix:Some([2.,0.,0.,4.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.]),info:Value::Null});
        let p=Params::resolve(BLENDER_OBJECT.params,serde_json::json!({"object":"Cube"}).as_object()).unwrap();
        let out=object(&p,&store).unwrap();assert_eq!(out.p(),&[6.,2.,3.]);assert!((out.point["N"].as_f32().unwrap()[0]-0.4472136).abs()<1e-7);
    }
    #[test] fn reset_transform_drops_location_but_keeps_rotation_and_scale() {
        let mut store=ExternalInputs::new();
        store.insert("object:Cube".into(),ExternalInput::Object{geo:Arc::new(Geometry::points_only(vec![1.,2.,3.]).unwrap()),matrix:Some([2.,0.,0.,4.,0.,1.,0.,5.,0.,0.,1.,6.,0.,0.,0.,1.]),info:Value::Null});
        let kept=Params::resolve(BLENDER_OBJECT.params,serde_json::json!({"object":"Cube"}).as_object()).unwrap();
        assert_eq!(object(&kept,&store).unwrap().p(),&[6.,7.,9.]);
        let reset=Params::resolve(BLENDER_OBJECT.params,serde_json::json!({"object":"Cube","reset_transform":true}).as_object()).unwrap();
        assert_eq!(object(&reset,&store).unwrap().p(),&[2.,2.,3.]);
    }
    #[test] fn joins_segments_and_keeps_point_attributes() {
        let g=Geometry::from_arrays(vec![0.;9],vec![0,1,1,2],vec![0,2,4],Some(vec![false,false])).unwrap();
        let o=join_edges(&g).unwrap();assert_eq!(o.topology.vertex_point,vec![0,1,2]);assert_eq!(o.nprims(),1);
    }
    #[test] fn session_updates_only_input_dependents_and_preserves_store_on_replace() {
        use crate::{Graph,session::Session};use serde_json::json;
        let doc=json!({"display":"out","nodes":{
            "src":{"type":"blender_object","params":{"object":"Cube"}},
            "static":{"type":"point"},
            "out":{"type":"merge","inputs":{"geos":["src","static"]}}
        }});
        let mut session=Session::new(Graph::from_value(&doc).unwrap());
        assert_eq!(session.cook(None).unwrap().npoints,1);
        assert_eq!(session.last_stats().waiting.len(),1);
        for x in [1.,2.] {
            session.set_external("object:Cube".into(),ExternalInput::Object{geo:Arc::new(Geometry::points_only(vec![x,0.,0.]).unwrap()),matrix:None,info:Value::Null});
            assert_eq!(session.cook(None).unwrap().p()[0],x);
            if x==2. {assert!(session.last_stats().cached.contains(&"static".into()));assert!(session.last_stats().cooked.contains(&"src".into()));}
        }
        session.replace_graph(Graph::from_value(&doc).unwrap());assert_eq!(session.cook(None).unwrap().p()[0],2.);
        session.set_external("object:Cube".into(),ExternalInput::Missing("no object named 'Cube' in the Blender file".into()));
        assert_eq!(session.cook(None).unwrap().npoints,1);
        assert!(session.last_stats().waiting[0].1.contains("not found"));
        session.set_external("object:Cube".into(),ExternalInput::Missing("unsupported object type CAMERA".into()));
        assert!(session.cook(None).unwrap_err().to_string().contains("CAMERA"));
    }
    #[test] fn picker_feed_wins_over_the_name_and_clearing_waits() {
        use crate::{Graph,session::Session};use serde_json::json;
        let doc=json!({"display":"src","nodes":{"src":{"type":"blender_object","params":{"object":"Cube"}}}});
        let mut session=Session::new(Graph::from_value(&doc).unwrap());
        session.set_external("object:Cube".into(),ExternalInput::Object{geo:Arc::new(Geometry::points_only(vec![1.,0.,0.]).unwrap()),matrix:None,info:Value::Null});
        assert_eq!(session.cook(None).unwrap().p()[0],1.);
        session.set_external(picker_external("src"),ExternalInput::Object{geo:Arc::new(Geometry::points_only(vec![5.,0.,0.]).unwrap()),matrix:None,info:Value::Null});
        assert_eq!(session.cook(None).unwrap().p()[0],5.);
        session.set_external(picker_external("src"),ExternalInput::Missing(String::new()));
        assert_eq!(session.cook(None).unwrap().npoints,0);
        assert_eq!(session.last_stats().waiting,vec![("src".into(),format!("no object picked. {PICK_HINT}"))]);
        // Still reported when served from cache.
        assert_eq!(session.cook(None).unwrap().npoints,0);
        assert_eq!(session.last_stats().waiting.len(),1);
        session.set_external(picker_external("src"),ExternalInput::Missing("Blender object \"Cam\" is a CAMERA".into()));
        assert!(session.cook(None).unwrap_err().to_string().contains("CAMERA"));
    }
    #[test] fn waiting_marker_round_trips() {
        assert_eq!(Error::waiting("x").waiting_text(),Some("x"));
        assert_eq!(Error::new("x").waiting_text(),None);
        assert_eq!(Error::waiting("x").in_node("a","b").waiting_text(),None);
    }
}
