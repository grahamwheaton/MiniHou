//! For-each pieces, using native NodeSpec bodies and their parameter validation.
use std::sync::Arc;
use super::{InputSpec,Inputs,NodeSpec,GEO};
use crate::{error::{Error,Result},geometry::{Attr,AttrData,Geometry},params::Params};
pub const FOR_EACH:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None, cook_sidecar: None,name:"for_each",inputs:GEO,params:crate::params!(
    "body":"";"params":"{}";"piece":"connectivity";"method":"connected";"domain":"points";"selection":"";"attribute":"class";"max_pieces":1000),cook:for_each};
pub const FOR_EACH_START:NodeSpec=NodeSpec{ outputs: &["element","index"], cook_multi:None,cook_sidecar:None,
    name:"for_each_start",inputs:GEO,params:crate::params!("method":"connected";"domain":"points";"selection":"";"attribute":"class";"max_pieces":1000),
    cook:|_,_|Err(Error::new("For Each Start must cook through its zone"))};
pub const FOR_EACH_END:NodeSpec=NodeSpec{ outputs: &["generated"],cook_multi:None,cook_sidecar:None,
    name:"for_each_end",inputs:&[
        InputSpec{name:"generated",multi:false,optional:false},
        InputSpec{name:"begin",multi:false,optional:false},
    ],params:&[],cook:|_,_|Err(Error::new("For Each End must cook through its zone"))};
/// Bridge a typed Index into geometry attributes for ordinary body nodes.
pub const FOR_EACH_INDEX:NodeSpec=NodeSpec{ outputs:&["geometry"],cook_multi:None,cook_sidecar:None,
    name:"for_each_index",inputs:&[
        InputSpec{name:"geo",multi:false,optional:false},
        InputSpec{name:"index",multi:false,optional:false},
    ],params:crate::params!("name":"source_index"),
    cook:|_,_|Err(Error::new("Index is only available inside a For Each zone"))};

pub(crate) fn stamp_index(g:&Geometry,index:i64,name:&str)->Result<Geometry> {
    super::group::valid_name(name)?;
    let value=i32::try_from(index).map_err(|_|Error::new("Index exceeds the integer attribute range"))?;
    let mut out=g.clone();
    out.point.insert(name.into(),Arc::new(Attr::i32(1,vec![value;g.npoints])));
    out.detail.insert(name.into(),serde_json::json!(index));
    out.validate()?;
    Ok(out)
}
thread_local!{static DEPTH:std::cell::Cell<usize>=const{std::cell::Cell::new(0)};}
struct Depth;
impl Drop for Depth{fn drop(&mut self){DEPTH.with(|d|d.set(d.get()-1));}}

/// A stable source element id accompanies the geometry fed to a zone body.
pub(crate) struct Partition {
    pub source_index: usize,
    pub value: serde_json::Value,
    pub geometry: Geometry,
}

fn point_subset(g: &Geometry, rows: &[usize]) -> Result<Geometry> {
    let mut out = Geometry::points_only(rows.iter().flat_map(|&i| g.p()[i*3..i*3+3].iter().copied()).collect())?;
    out.point = super::group::gather_table(&g.point, rows);
    out.packed = g.packed.clone();
    out.detail = g.detail.clone();
    Ok(out)
}

fn edge_subset(g: &Geometry, edges: &crate::topo::Edges, rows: &[usize]) -> Result<Geometry> {
    let mut used = vec![false; g.npoints];
    for &i in rows { used[edges.points[2*i] as usize] = true; used[edges.points[2*i+1] as usize] = true; }
    let points: Vec<_> = (0..g.npoints).filter(|&i| used[i]).collect();
    let mut remap = vec![0i32; g.npoints];
    for (i, &old) in points.iter().enumerate() { remap[old] = i as i32; }
    let verts: Vec<i32> = rows.iter().flat_map(|&i| [remap[edges.points[2*i] as usize], remap[edges.points[2*i+1] as usize]]).collect();
    let mut out = Geometry::from_arrays(points.iter().flat_map(|&i| g.p()[3*i..3*i+3].iter().copied()).collect(),
        verts, (0..=rows.len()).map(|i| (2*i) as i32).collect(), Some(vec![false; rows.len()]))?;
    out.point = super::group::gather_table(&g.point, &points);
    out.edge = super::group::gather_table(&g.edge, rows);
    out.packed = g.packed.clone();
    out.detail = g.detail.clone();
    Ok(out)
}

fn point_roots(g: &Geometry) -> Vec<usize> {
    let mut parent: Vec<_> = (0..g.npoints).collect();
    fn root(parent: &mut [usize], i: usize) -> usize {
        if parent[i] != i { let previous = parent[i]; parent[i] = root(parent, previous); }
        parent[i]
    }
    for bounds in g.topology.prim_offsets.windows(2) {
        let vertices = &g.topology.vertex_point[bounds[0] as usize..bounds[1] as usize];
        for pair in vertices.windows(2) {
            let (a,b) = (root(&mut parent, pair[0] as usize), root(&mut parent, pair[1] as usize));
            parent[a.max(b)] = a.min(b);
        }
    }
    (0..g.npoints).map(|i| root(&mut parent, i)).collect()
}

/// Partition without evaluating a body. New zones and the old single-node body share these rules.
pub(crate) fn partitions(g: &Geometry, method: &str, domain: &str, attribute: &str,
    selection: &str, limit: usize) -> Result<Vec<Partition>> {
    if !["connected", "attribute", "domain", "legacy_primitive"].contains(&method) {
        return Err(Error::new(format!("unknown For Each method {method:?}")));
    }
    if !["points", "edges", "faces", "instances", "primitives"].contains(&domain) {
        return Err(Error::new(format!("unknown For Each domain {domain:?}")));
    }
    let edges = (domain == "edges").then(|| crate::topo::edges(&g.topology, g.npoints));
    let n = match domain { "points" | "instances" => g.npoints, "edges" => edges.as_ref().unwrap().len(), _ => g.nprims() };
    let selected = if selection.trim().is_empty() { vec![true; n] } else {
        match domain {
            "edges" => super::group::edge_mask(g, edges.as_ref().unwrap(), selection)?,
            "points" | "instances" => super::group::mask(g, "point", selection)?,
            _ => super::group::mask(g, "prim", selection)?,
        }
    };
    let candidates: Vec<usize> = (0..n).filter(|&i| selected[i] && match domain {
        "instances" => super::packed::source_of(g, i).is_some(),
        "faces" => g.topology.is_closed(i) &&
            g.topology.prim_offsets[i+1] - g.topology.prim_offsets[i] >= 3,
        _ => true,
    }).collect();
    let attr = if method == "attribute" {
        let table = match domain { "points" | "instances" => &g.point, "edges" => &g.edge, _ => &g.prim };
        Some(table.get(attribute).ok_or_else(|| Error::new(format!("no {domain} attribute {attribute:?}")))?)
    } else { None };
    let roots = (method == "connected" && domain != "instances" && domain != "primitives").then(|| point_roots(g));
    let legacy_labels = (method == "connected" && domain == "primitives")
        .then(|| super::attrib_tools::piece_labels(g).1);
    let mut keyed: Vec<(f64, usize)> = Vec::with_capacity(candidates.len());
    for i in candidates {
        let key = if let Some(a) = attr {
            match &a.data { AttrData::F32(v) => v[i*a.width] as f64,
                AttrData::I32(v) => v[i*a.width] as f64 }
        } else if method == "connected" {
            match domain {
                "primitives" => legacy_labels.as_ref().unwrap()[i] as f64,
                "edges" => roots.as_ref().unwrap()[edges.as_ref().unwrap().points[2*i] as usize] as f64,
                "faces" => {
                    let first = g.topology.prim_offsets[i] as usize;
                    roots.as_ref().unwrap()[g.topology.vertex_point[first] as usize] as f64
                },
                _ => roots.as_ref().unwrap()[i] as f64,
            }
        } else { i as f64 };
        if !key.is_finite() { return Err(Error::new("non-finite For Each attribute labels are unsupported")); }
        keyed.push((key, i));
    }
    keyed.sort_by(|a,b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut groups: Vec<(f64, Vec<usize>)> = Vec::new();
    for (key, i) in keyed {
        if groups.last().is_none_or(|(last,_)| *last != key) { groups.push((key, Vec::new())); }
        groups.last_mut().unwrap().1.push(i);
    }
    if groups.len() > limit { return Err(Error::new(format!("{} pieces is more than max_pieces ({limit})", groups.len()))); }
    groups.into_iter().map(|(key, rows)| {
        let geometry = match domain {
            "points" | "instances" => point_subset(g, &rows)?,
            "edges" => edge_subset(g, edges.as_ref().unwrap(), &rows)?,
            _ => {
                let mut keep = vec![false; g.nprims()]; for &i in &rows { keep[i] = true; }
                super::group::keep_prims(g, &keep, true)?
            }
        };
        let value = if attr.is_some_and(|a| matches!(a.data, AttrData::F32(_))) {
            serde_json::json!(key)
        } else { serde_json::json!(key as i64) };
        Ok(Partition { source_index: *rows.iter().min().unwrap(), value, geometry })
    }).collect()
}
fn for_each(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=FOR_EACH.one(inputs,0)?;let body=p.s("body")?;
    if body.is_empty(){return Err(Error::new("set 'body' to a node type"));}
    let nt=super::get(&body).ok_or_else(||Error::new(format!("no native node type {body:?}; graph-system bodies are not supported yet")))?;
    if nt.inputs.is_empty(){return Err(Error::new(format!("{body:?} has no geometry input to feed pieces into")));}
    DEPTH.with(|d|{if d.get()>=32{Err(Error::new("for_each body recursion exceeds 32 levels"))}else{d.set(d.get()+1);Ok(())}})?;
    let _depth=Depth;
    let text=p.s("params")?;let overrides:serde_json::Value=serde_json::from_str(if text.is_empty(){"{}"}else{&text}).map_err(|e|Error::new(format!("params must be JSON: {e}")))?;
    let values=Params::resolve(nt.params,Some(overrides.as_object().ok_or_else(||Error::new("params must be a JSON object"))?))?;
    let piece=p.e("piece",&["connectivity","attribute","primitive"])?;let limit=p.i("max_pieces")?;
    if !(1..=10000).contains(&limit){return Err(Error::new("max_pieces must be 1..10000"));}
    let chosen=p.e("method", &["connected", "attribute", "domain", "legacy_primitive"])?;
    // Params::resolve_node handles saved graphs. Direct NodeSpec cooks also accept the
    // historical Piece field, used by the original Python/Rust parity tests.
    let legacy=chosen=="connected" && piece!="connectivity";
    let method=if legacy {if piece=="primitive" {"legacy_primitive"} else {"attribute"}} else {&chosen};
    let domain=if legacy {"primitives".to_owned()} else {p.e("domain", &["points","edges","faces","instances","primitives"])?};
    let parts=partitions(&g,method,&domain,&p.s("attribute")?,&p.s("selection")?,limit as usize)?;
    let count=parts.len();
    let mut outs=Vec::new();
    for (i,partition) in parts.into_iter().enumerate(){
        let mut part=partition.geometry;
        part.detail.insert("iteration".into(),serde_json::json!(i));part.detail.insert("numiterations".into(),serde_json::json!(count));part.detail.insert("value".into(),partition.value);
        part.detail.insert("source_index".into(),serde_json::json!(partition.source_index));
        let mut args=vec![Vec::new();nt.inputs.len()];args[0].push(Arc::new(part));
        for port in &nt.inputs[1..]{if !port.optional{return Err(Error::new(format!("body {body:?} requires additional input {:?}",port.name)));}}
        outs.push(Arc::new((nt.cook)(&args,&values)?));
    }
    if outs.is_empty(){return if domain=="primitives" && p.s("selection")?.is_empty() {Ok((*g).clone())} else {Geometry::points_only(vec![])};}
    let mut out=super::merge::merge(&[outs],&Params::resolve(&[],None)?)?;
    for key in ["iteration","numiterations","value","source_index"]{out.detail.remove(key);}Ok(out)
}
#[cfg(test)]
mod tests{
    use super::*;
    use crate::geometry::Attr;
    use serde_json::json;
    #[test]
    fn method_defaults_and_legacy_graph_round_trip() {
        let defaults = Params::resolve(FOR_EACH.params, None).unwrap();
        assert_eq!(defaults.s("method").unwrap(), "connected");
        assert_eq!(defaults.s("domain").unwrap(), "points");
        for piece in ["connectivity", "attribute", "primitive"] {
            let saved = json!({"display":"each","nodes":{
                "input":{"type":"point"},
                "each":{"type":"for_each","inputs":{"geo":"input"},
                    "params":{"body":"null","piece":piece}}
            }});
            let graph = crate::graph::Graph::from_value(&saved).unwrap();
            assert_eq!(graph.to_value(), saved);
            if piece != "attribute" { let _ = graph.cook(None).unwrap(); }
        }
    }
    #[test]
    fn primitive_pieces_duplicate_shared_points_and_clear_iteration(){
        let g=Geometry::from_arrays(vec![0.,0.,0.,1.,0.,0.,2.,0.,0.],vec![0,1,1,2],vec![0,2,4],Some(vec![false,false])).unwrap();
        let p=Params::resolve(FOR_EACH.params,serde_json::json!({"body":"transform","piece":"primitive","params":"{\"translate\":[0,0,1]}"}).as_object()).unwrap();
        let out=for_each(&[vec![Arc::new(g)]],&p).unwrap();assert_eq!(out.npoints,4);assert_eq!(out.nprims(),2);assert_eq!(out.p()[2],1.);assert!(!out.detail.contains_key("iteration"));out.validate().unwrap();
    }
    #[test]
    fn bad_body_and_piece_limit_fail(){
        let g=Arc::new(Geometry::from_arrays(vec![0.;6],vec![0,1],vec![0,1,2],None).unwrap());
        for v in [serde_json::json!({"body":"grid"}),serde_json::json!({"body":"null","piece":"primitive","max_pieces":1}),serde_json::json!({"body":"null","params":"[]"})]{
            assert!(for_each(&[vec![g.clone()]],&Params::resolve(FOR_EACH.params,v.as_object()).unwrap()).is_err());
        }
    }
    fn mixed() -> Geometry {
        let mut g=Geometry::from_arrays(vec![0.,0.,0., 1.,0.,0., 0.,1.,0., 0.,2.,0., 5.,0.,0.],
            vec![0,1,2,2,3],vec![0,3,5],Some(vec![true,false])).unwrap();
        g.point.insert("class".into(),Arc::new(Attr::i32(1,vec![0,0,1,1,2])));
        g.prim.insert("class".into(),Arc::new(Attr::i32(1,vec![5,7])));
        g.edge.insert("class".into(),Arc::new(Attr::i32(1,vec![1,1,2,3])));
        g.point.insert(super::super::packed::SOURCE.into(),Arc::new(Attr::i32(1,vec![0,0,0,0,1])));
        g.packed=Arc::new(vec![Arc::new(Geometry::points_only(vec![0.,0.,0.]).unwrap())]);
        g.validate().unwrap();g
    }
    #[test]
    fn all_domains_selection_and_legacy_curve() {
        let g=mixed();
        for (domain, expected) in [("points",5),("edges",4),("faces",1),("instances",1)] {
            let parts=partitions(&g,"domain",domain,"class","",100).unwrap();
            assert_eq!(parts.len(),expected,"{domain}");
            for p in &parts {p.geometry.validate().unwrap();}
            assert!(parts.windows(2).all(|p|p[0].source_index<p[1].source_index));
        }
        let instance=partitions(&g,"domain","instances","class","4",100).unwrap();
        assert_eq!(instance.len(),1);
        assert_eq!(super::super::packed::instance_count(&instance[0].geometry),1);
        assert_eq!(instance[0].source_index,4);
        assert!(partitions(&g,"domain","instances","class","0",100).unwrap().is_empty());
        let legacy=partitions(&g,"legacy_primitive","primitives","class","",100).unwrap();
        assert_eq!(legacy.len(),2);
        assert!(!legacy[1].geometry.topology.is_closed(0));
        assert_eq!(partitions(&g,"domain","faces","class","",100).unwrap().len(),1);
    }
    #[test]
    fn attribute_points_edges_faces_instances_and_empty() {
        let g=mixed();
        for (domain, expected) in [("points",3),("edges",3),("faces",1),("instances",1)] {
            let parts=partitions(&g,"attribute",domain,"class","",100).unwrap();
            assert_eq!(parts.len(),expected,"{domain}");
            for p in parts {p.geometry.validate().unwrap();}
        }
        let points=partitions(&g,"attribute","points","class","",100).unwrap();
        assert_eq!(points.iter().map(|p|p.source_index).collect::<Vec<_>>(),vec![0,2,4]);
        assert_eq!(super::super::packed::instance_count(&points[2].geometry),1);
        let connected=partitions(&g,"connected","points","class","",100).unwrap();
        assert_eq!(connected.len(),2);
        assert_eq!(connected[0].geometry.npoints,4);
        let empty=Geometry::points_only(vec![]).unwrap();
        for domain in ["points","edges","faces","instances"] {
            assert!(partitions(&empty,"domain",domain,"class","",100).unwrap().is_empty());
        }
    }
    #[test]
    fn body_cooks_point_and_instance_attributes_without_unpacking() {
        let g=Arc::new(mixed());
        for (method,domain,expected_points,expected_prims) in [
            ("attribute","points",5,0), ("attribute","instances",1,0),
            ("domain","faces",3,1), ("legacy_primitive","primitives",5,2),
        ] {
            let params=Params::resolve(FOR_EACH.params,json!({"body":"null","method":method,"domain":domain,"attribute":"class"}).as_object()).unwrap();
            let out=for_each(&[vec![g.clone()]],&params).unwrap();
            assert_eq!((out.npoints,out.nprims()),(expected_points,expected_prims),"{method}/{domain}");
            assert_eq!(super::super::packed::instance_count(&out),if domain=="points"||domain=="instances" {1}else{0});
            out.validate().unwrap();
        }
    }
}
