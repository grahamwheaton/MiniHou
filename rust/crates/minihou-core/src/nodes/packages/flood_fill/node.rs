//! Deterministic topology flood fill on points, edges, or faces.
use std::{cmp::{Ordering, Reverse}, collections::{BinaryHeap, VecDeque}, sync::Arc};

use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry}, nodes::{InputSpec, Inputs, NodeOutputs, NodeSpec}, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry", "frontier"], cook_multi: Some(cook_multi),
    name: "flood_fill",
    inputs: &[InputSpec { name: "geometry", multi: false, optional: false }],
    params: crate::params!(
        "domain": "points"; "seed_group": ""; "avoid_group": ""; "allowed_group": "";
        "barrier_group": ""; "target_group": ""; "max_steps": -1; "max_distance": -1.0;
        "max_elements": -1; "stop_at_boundary": false; "max_angle": 180.0;
        "attr_name": ""; "attr_mode": "neighbor_diff"; "attr_tolerance": 0.0;
        "direction": [0.0, 0.0, 0.0]; "max_deviation": 180.0;
        "mode": "flood"; "output_group": "flood"; "combine": "replace";
        "frontier_group": ""; "store_step_distance": false; "store_surface_distance": false
    ),
    cook,
};

#[derive(Clone, Copy, Debug, PartialEq)]
struct QueueItem { distance: f64, index: usize, seed: usize, previous: usize, steps: i32 }
impl Eq for QueueItem {}
impl Ord for QueueItem {
    fn cmp(&self, other: &Self) -> Ordering {
        self.distance.total_cmp(&other.distance).then(self.index.cmp(&other.index))
            .then(self.seed.cmp(&other.seed)).then(self.steps.cmp(&other.steps))
    }
}
impl PartialOrd for QueueItem { fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) } }

fn positions(g: &Geometry, domain: &str, edges: &[(i32, i32)]) -> Vec<[f64; 3]> {
    let p = g.p();
    let point = |i: usize| [p[3*i] as f64, p[3*i+1] as f64, p[3*i+2] as f64];
    match domain {
        "points" => (0..g.npoints).map(point).collect(),
        "edges" => edges.iter().map(|&(a,b)| { let a=point(a as usize); let b=point(b as usize); [(a[0]+b[0])/2.0,(a[1]+b[1])/2.0,(a[2]+b[2])/2.0] }).collect(),
        _ => g.topology.prim_offsets.windows(2).map(|w| {
            let ids = &g.topology.vertex_point[w[0] as usize..w[1] as usize];
            let mut c = [0.0;3];
            for &id in ids { let q=point(id as usize); for k in 0..3 { c[k]+=q[k]; } }
            for x in &mut c { *x/=ids.len().max(1) as f64; } c
        }).collect(),
    }
}

fn topology(g: &Geometry, domain: &str, edges: &[(i32,i32)]) -> (Vec<Vec<usize>>, Vec<bool>) {
    let n = match domain { "points" => g.npoints, "edges" => edges.len(), _ => g.nprims() };
    let mut adj = vec![Vec::new(); n];
    let mut boundary = vec![false; n];
    let mut edge_faces = vec![Vec::new(); edges.len()];
    let mut point_edges = vec![Vec::new(); g.npoints];
    let edge_index: std::collections::BTreeMap<_,_> = edges.iter().copied().enumerate().map(|(i,e)|(e,i)).collect();
    for (i,&(a,b)) in edges.iter().enumerate() {
        point_edges[a as usize].push(i);
        if a != b { point_edges[b as usize].push(i); }
        if domain == "points" && a != b { adj[a as usize].push(b as usize); adj[b as usize].push(a as usize); }
    }
    for face in 0..g.nprims() {
        let s=g.topology.prim_offsets[face] as usize; let e=g.topology.prim_offsets[face+1] as usize;
        for v in s..e {
            let next=if v+1<e { v+1 } else if g.topology.is_closed(face) { s } else { continue };
            let a=g.topology.vertex_point[v]; let b=g.topology.vertex_point[next];
            edge_faces[*edge_index.get(&(a.min(b),a.max(b))).unwrap()].push(face);
        }
    }
    for (i, faces) in edge_faces.iter().enumerate() {
        if faces.len() == 1 && g.topology.is_closed(faces[0]) {
            if domain == "edges" { boundary[i]=true; }
            let (a,b)=edges[i];
            if domain == "points" { boundary[a as usize]=true; boundary[b as usize]=true; }
            if domain == "faces" { boundary[faces[0]]=true; }
        }
    }
    for incident in &point_edges {
        if incident.len() == 1 {
            if domain == "points" {
                let (a,b)=edges[incident[0]];
                boundary[if point_edges[a as usize].len()==1 {a as usize} else {b as usize}]=true;
            } else if domain == "edges" { boundary[incident[0]]=true; }
        }
    }
    if domain == "edges" {
        for incident in point_edges { for &a in &incident { for &b in &incident { if a!=b { adj[a].push(b); } } } }
    } else if domain == "faces" {
        for faces in edge_faces { for &a in &faces { for &b in &faces { if a!=b { adj[a].push(b); } } } }
    }
    for row in &mut adj { row.sort_unstable(); row.dedup(); }
    (adj,boundary)
}

fn mask(g: &Geometry, domain: &str, edges: &[(i32,i32)], text: &str, empty: bool) -> Result<Vec<bool>> {
    let n=match domain {"points"=>g.npoints,"edges"=>edges.len(),_=>g.nprims()};
    if text.trim().is_empty() { return Ok(vec![empty;n]); }
    if domain == "points" { return crate::nodes::groups::point_mask(g,text); }
    if domain == "faces" { return crate::nodes::group::mask(g,"prim",text); }
    let mut xyz=Vec::with_capacity(3*n);
    for (a,b) in edges { for k in 0..3 { xyz.push((g.p()[*a as usize*3+k]+g.p()[*b as usize*3+k])*0.5); } }
    let mut proxy=Geometry::points_only(xyz)?;
    for (name,attr) in &g.edge { if name.starts_with("group_") { proxy.point.insert(name.clone(),attr.clone()); } }
    crate::nodes::groups::point_mask(&proxy,text)
}

fn attribute(g: &Geometry, domain: &str, name: &str) -> Result<Vec<Vec<f64>>> {
    let table=match domain {"points"=>&g.point,"edges"=>&g.edge,_=>&g.prim};
    let a=table.get(name).ok_or_else(|| Error::new(format!("missing {domain} attribute {name:?}")))?;
    Ok(match &a.data {
        AttrData::F32(v)=>v.chunks(a.width).map(|r|r.iter().map(|&x|x as f64).collect()).collect(),
        AttrData::I32(v)=>v.chunks(a.width).map(|r|r.iter().map(|&x|x as f64).collect()).collect(),
    })
}

fn distance(a: &[f64], b: &[f64]) -> f64 { a.iter().zip(b).map(|(x,y)|(x-y)*(x-y)).sum::<f64>().sqrt() }
fn angle(a: &[f64;3], b: &[f64;3]) -> f64 {
    let la=distance(a,&[0.0;3]); let lb=distance(b,&[0.0;3]);
    if la<1e-12 || lb<1e-12 { return 0.0; }
    ((0..3).map(|k|a[k]*b[k]).sum::<f64>()/(la*lb)).clamp(-1.0,1.0).acos().to_degrees()
}

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    Ok(cook_multi(inputs, p)?.0.remove(0))
}

fn cook_multi(inputs: &Inputs, p: &Params) -> Result<NodeOutputs> {
    let g=inputs.first().and_then(|v|v.first()).ok_or_else(||Error::new("geometry is not connected"))?;
    let domain=p.e("domain", &["points","edges","faces"])?;
    let mode=p.e("mode", &["flood","flood_to_target","regions"])?;
    let combine=p.e("combine", &["replace","union","intersect","subtract"])?;
    let attr_mode=p.e("attr_mode", &["seed_diff","neighbor_diff","threshold"])?;
    let output=p.s("output_group")?; crate::nodes::group::valid_name(&output)?;
    let frontier=p.s("frontier_group")?;
    if !frontier.is_empty() { crate::nodes::group::valid_name(&frontier)?; if frontier==output { return Err(Error::new("frontier_group must differ from output_group")); } }
    let max_steps=p.i("max_steps")?; let max_elements=p.i("max_elements")?;
    let max_distance=p.f("max_distance")?; let max_angle=p.f("max_angle")?;
    let deviation=p.f("max_deviation")?; let tolerance=p.f("attr_tolerance")?;
    if !max_distance.is_finite() || !max_angle.is_finite() || !(0.0..=180.0).contains(&max_angle) || !deviation.is_finite() || !(0.0..=180.0).contains(&deviation) || !tolerance.is_finite() || tolerance<0.0 { return Err(Error::new("invalid flood fill limit")); }
    let direction=p.v3("direction")?;
    if direction.iter().any(|v|!v.is_finite()) { return Err(Error::new("direction must be finite")); }
    let edge_pairs=g.topology.edge_pairs(); let n=match domain.as_str(){"points"=>g.npoints,"edges"=>edge_pairs.len(),_=>g.nprims()};
    let (adj,boundary)=topology(g,&domain,&edge_pairs);
    let pos=positions(g,&domain,&edge_pairs);
    let seed_text=p.s("seed_group")?;
    let seeds=mask(g,&domain,&edge_pairs,&seed_text,false)?;
    let avoid=mask(g,&domain,&edge_pairs,&p.s("avoid_group")?,false)?;
    let allowed=mask(g,&domain,&edge_pairs,&p.s("allowed_group")?,true)?;
    let barrier=mask(g,&domain,&edge_pairs,&p.s("barrier_group")?,false)?;
    let target=mask(g,&domain,&edge_pairs,&p.s("target_group")?,false)?;
    let attr_name=p.s("attr_name")?;
    let attrs=if attr_name.is_empty(){None}else{Some(attribute(g,&domain,&attr_name)?)};
    let normals=if domain=="faces" { Some(crate::normals::prim_normals(g.p(),&g.topology)) } else { None };
    let mut selected=vec![false;n]; let mut steps=vec![-1;n]; let mut surface=vec![-1.0f32;n];
    let mut region=vec![-1;n]; let mut owner=vec![usize::MAX;n];
    let mut heap=BinaryHeap::new(); let mut queue=VecDeque::new();
    let mut count=0usize;
    let eligible=|i:usize| allowed[i] && !avoid[i] && !barrier[i];
    let mut next_region=0i32;
    let use_distance=max_distance>=0.0 || (max_steps<0 && p.b("store_surface_distance")?);
    for i in 0..n {
        if !eligible(i) || (mode!="regions" && !seeds[i]) || (mode=="regions" && !seed_text.is_empty() && !seeds[i]) { continue; }
        if mode=="regions" && selected[i] { continue; }
        let rid=if mode=="regions" { let r=next_region; next_region+=1; r } else { 0 };
        let item=QueueItem{distance:0.0,index:i,seed:i,previous:i,steps:0};
        if use_distance { heap.push(Reverse(item)); } else { queue.push_back(item); }
        if mode=="regions" {
            // Drain each island before starting another seed, preserving stable region IDs.
            traverse(&adj,&boundary,&pos,normals.as_deref(),attrs.as_deref(),&eligible,&target,
                &direction,max_angle,deviation,tolerance,&attr_mode,max_steps,max_distance,max_elements,
                p.b("stop_at_boundary")?,use_distance,&mode,rid,&mut heap,&mut queue,
                &mut selected,&mut steps,&mut surface,&mut region,&mut owner,&mut count)?;
        }
    }
    if mode!="regions" {
        traverse(&adj,&boundary,&pos,normals.as_deref(),attrs.as_deref(),&eligible,&target,
            &direction,max_angle,deviation,tolerance,&attr_mode,max_steps,max_distance,max_elements,
            p.b("stop_at_boundary")?,use_distance,&mode,0,&mut heap,&mut queue,
            &mut selected,&mut steps,&mut surface,&mut region,&mut owner,&mut count)?;
    }
    let table=match domain.as_str(){"points"=>&g.point,"edges"=>&g.edge,_=>&g.prim};
    let old=table.get(&format!("group_{output}")).map(|a|crate::nodes::group::flags(a)).unwrap_or_else(||vec![false;n]);
    let combined:Vec<i32>=selected.iter().zip(old).map(|(&new,old)|match combine.as_str(){"union"=>new||old,"intersect"=>new&&old,"subtract"=>old&&!new,_=>new} as i32).collect();
    let mut out=(**g).clone();
    let dst=match domain.as_str(){"points"=>&mut out.point,"edges"=>&mut out.edge,_=>&mut out.prim};
    dst.insert(format!("group_{output}"),Arc::new(Attr::i32(1,combined)));
    if !frontier.is_empty() { let flags=(0..n).map(|i| (selected[i] && (adj[i].iter().any(|&j|!selected[j]) || boundary[i])) as i32).collect(); dst.insert(format!("group_{frontier}"),Arc::new(Attr::i32(1,flags))); }
    if p.b("store_step_distance")? { dst.insert("step_distance".into(),Arc::new(Attr::i32(1,steps))); }
    if p.b("store_surface_distance")? { dst.insert("surface_distance".into(),Arc::new(Attr::f32(1,surface))); }
    if mode=="regions" { dst.insert("region_id".into(),Arc::new(Attr::i32(1,region))); }
    let mut frontier_edges = Vec::new();
    match domain.as_str() {
        "points" => {
            for &(a, b) in &edge_pairs {
                if selected[a as usize] != selected[b as usize] { frontier_edges.push((a,b)); }
            }
        }
        "edges" => {
            for (i, &pair) in edge_pairs.iter().enumerate() {
                if selected[i] && (adj[i].iter().any(|&j| !selected[j]) || boundary[i]) { frontier_edges.push(pair); }
            }
        }
        _ => {
            let mut owners: std::collections::BTreeMap<(i32,i32), Vec<usize>> = std::collections::BTreeMap::new();
            for face in 0..g.nprims() {
                let s = g.topology.prim_offsets[face] as usize;
                let e = g.topology.prim_offsets[face+1] as usize;
                for v in s..e {
                    let next = if v+1<e {v+1} else if g.topology.is_closed(face) {s} else {continue};
                    let a=g.topology.vertex_point[v]; let b=g.topology.vertex_point[next];
                    owners.entry((a.min(b),a.max(b))).or_default().push(face);
                }
            }
            for (edge, faces) in owners {
                if faces.iter().any(|&i|selected[i]) && (faces.len()==1 || faces.iter().any(|&i|!selected[i])) {
                    frontier_edges.push(edge);
                }
            }
        }
    }
    let mut points = Vec::with_capacity(frontier_edges.len()*6);
    let mut vertices = Vec::with_capacity(frontier_edges.len()*2);
    let mut offsets = vec![0];
    for (a,b) in frontier_edges {
        let base = (points.len()/3) as i32;
        points.extend_from_slice(&g.p()[a as usize*3..a as usize*3+3]);
        points.extend_from_slice(&g.p()[b as usize*3..b as usize*3+3]);
        vertices.extend([base,base+1]);
        offsets.push(vertices.len() as i32);
    }
    let closed = vec![false; offsets.len()-1];
    let frontier_geo = Geometry::from_arrays(points,vertices,offsets,Some(closed))?;
    Ok(vec![out,frontier_geo].into())
}

#[allow(clippy::too_many_arguments)]
fn traverse(adj:&[Vec<usize>],boundary:&[bool],pos:&[[f64;3]],normals:Option<&[[f64;3]]>,attrs:Option<&[Vec<f64>]>,
    eligible:&impl Fn(usize)->bool,target:&[bool],direction:&[f64;3],max_angle:f64,deviation:f64,tolerance:f64,attr_mode:&str,
    max_steps:i64,max_distance:f64,max_elements:i64,stop_at_boundary:bool,use_distance:bool,mode:&str,rid:i32,
    heap:&mut BinaryHeap<Reverse<QueueItem>>,queue:&mut VecDeque<QueueItem>,selected:&mut [bool],steps:&mut [i32],
    surface:&mut [f32],region:&mut [i32],owner:&mut [usize],count:&mut usize) -> Result<()> {
    while let Some(item)=if use_distance {heap.pop().map(|Reverse(x)|x)} else {queue.pop_front()} {
        let i=item.index;
        if selected[i] || !eligible(i) || (max_steps>=0 && item.steps as i64>max_steps) || (max_distance>=0.0 && item.distance>max_distance+1e-9) {continue;}
        if max_elements>=0 && *count as i64>=max_elements {break;}
        selected[i]=true; steps[i]=item.steps; surface[i]=item.distance as f32; region[i]=rid; owner[i]=item.seed; *count+=1;
        if mode=="flood_to_target" && target[i] {break;}
        if stop_at_boundary && boundary[i] {continue;}
        for &j in &adj[i] {
            if selected[j] || !eligible(j) || (stop_at_boundary && boundary[j]) {continue;}
            if let Some(ns)=normals { if angle(&ns[i],&ns[j])>max_angle+1e-9 {continue;} }
            let movement=[pos[j][0]-pos[i][0],pos[j][1]-pos[i][1],pos[j][2]-pos[i][2]];
            if normals.is_none() && item.previous!=i {
                let incoming=[pos[i][0]-pos[item.previous][0],pos[i][1]-pos[item.previous][1],pos[i][2]-pos[item.previous][2]];
                if angle(&incoming,&movement)>max_angle+1e-9 {continue;}
            }
            if direction.iter().any(|&v|v!=0.0) && angle(direction,&movement)>deviation+1e-9 {continue;}
            if let Some(a)=attrs {
                let reference=if attr_mode=="seed_diff" {&a[item.seed]} else {&a[i]};
                let diff=if attr_mode=="threshold" {distance(&a[j],&vec![0.0;a[j].len()])} else {distance(reference,&a[j])};
                if diff>tolerance+1e-9 {continue;}
            }
            let length=distance(&pos[i],&pos[j]);
            let next=QueueItem{distance:item.distance+length,index:j,seed:item.seed,previous:i,steps:item.steps.saturating_add(1)};
            if max_steps>=0 && next.steps as i64>max_steps {continue;}
            if use_distance {heap.push(Reverse(next));} else {queue.push_back(next);}
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
