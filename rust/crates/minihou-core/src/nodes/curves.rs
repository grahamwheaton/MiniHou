//! Arc-length curves and Bezier sources, matching nodes/curves.py.
use std::sync::Arc;
use super::{InputSpec, Inputs, NodeSpec, GEO};
use super::group::{gather_table, keep_prims};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry, Topology, MAX_ELEMENTS}, params::Params};

pub(crate) fn count(n: usize) -> Result<()> {
    if n > MAX_ELEMENTS.min(20_000_000) { Err(Error::new("curve output exceeds the 20,000,000 generated-element safety limit")) } else { Ok(()) }
}
pub(crate) fn curve(p: Vec<f32>, closed: bool) -> Result<Geometry> {
    let n=p.len()/3; count(n)?;
    let mut g=Geometry::from_arrays(p,(0..n as i32).collect(),vec![0,n as i32],Some(vec![closed]))?;
    g.prim.insert("__viewport_curve".into(),Arc::new(Attr::i32(1,vec![1])));
    Ok(g)
}
pub(crate) fn norm(v: [f64;3]) -> f64 { v.iter().map(|x| x*x).sum::<f64>().sqrt() }
pub(crate) fn sub(a:[f64;3],b:[f64;3])->[f64;3] { std::array::from_fn(|i| a[i]-b[i]) }
/// Exactly the edge order and branch/loop traversal of Python chain_segments.
pub(super) fn chains(n:usize,edges:&[[i32;2]]) -> (Vec<Vec<i32>>,Vec<bool>) {
    let mut adj=vec![Vec::<(i32,usize)>::new();n];
    for (i,&[a,b]) in edges.iter().enumerate() {adj[a as usize].push((b,i));adj[b as usize].push((a,i));}
    let mut used=vec![false;edges.len()]; let mut paths=Vec::new(); let mut closed=Vec::new();
    fn walk(start:i32,first:usize,adj:&[Vec<(i32,usize)>],edges:&[[i32;2]],used:&mut [bool])->Vec<i32> {
        let mut pts=vec![start]; let(mut e,mut p)=(first,start);
        loop { used[e]=true; let [a,b]=edges[e]; let q=if b==p {a} else {b}; pts.push(q);
            if adj[q as usize].len()!=2 {break;}
            if let Some(&(_,j))=adj[q as usize].iter().find(|(_,j)| !used[*j]) {p=q;e=j;} else {break;}
        } pts
    }
    for s in 0..n {if adj[s].len()!=2 {for &(_,e) in &adj[s] {if !used[e] {
        paths.push(walk(s as i32,e,&adj,edges,&mut used));closed.push(false);
    }}}}
    for e in 0..edges.len() {if !used[e] {let mut pts=walk(edges[e][0],e,&adj,edges,&mut used);pts.pop();paths.push(pts);closed.push(true);}}
    (paths,closed)
}
pub const CONVERT_LINE:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None, cook_sidecar: None,name:"convert_line",inputs:GEO,params:crate::params!("join":true),cook:convert_line};
fn convert_line(inputs:&Inputs,p:&Params)->Result<Geometry> {
    let g=CONVERT_LINE.one(inputs,0)?; let e=crate::topo::edges(&g.topology,g.npoints);
    let edges:Vec<_>=e.points.chunks_exact(2).map(|e|[e[0] as i32,e[1] as i32]).collect();
    let (paths,cl)=if p.b("join")? {chains(g.npoints,&edges)} else {(edges.iter().map(|e|e.to_vec()).collect(),vec![false;edges.len()])};
    let mut out=(*g).clone();let mut vp=Vec::new();let mut offs=vec![0];
    for path in paths {vp.extend(path);offs.push(vp.len() as i32);}
    out.topology=Arc::new(Topology{vertex_point:vp,prim_offsets:offs,closed:Some(cl)});out.vertex.clear();out.prim.clear();Ok(out)
}
pub const POLYPATH:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None, cook_sidecar: None,name:"polypath",inputs:GEO,params:&[],cook:polypath};
fn polypath(inputs:&Inputs,_:&Params)->Result<Geometry> {
    let g=POLYPATH.one(inputs,0)?;if !g.topology.has_open(){return Ok((*g).clone());}
    let mut edges=Vec::new();
    for i in 0..g.nprims(){if !g.topology.is_closed(i){let r=&g.topology.vertex_point[g.topology.prim_offsets[i] as usize..g.topology.prim_offsets[i+1] as usize];
        edges.extend(r.windows(2).map(|w|[w[0],w[1]]));}}
    let (paths,cl)=chains(g.npoints,&edges);
    let mut out=keep_prims(&g,&g.topology.closed_mask(),false)?;
    let oldv=out.nvertices();let oldp=out.nprims();let mut topo=(*out.topology).clone();let mut closed=vec![true;oldp];
    for (path,c) in paths.into_iter().zip(cl){topo.vertex_point.extend(path);topo.prim_offsets.push(topo.vertex_point.len() as i32);closed.push(c);}
    for (table,old,new) in [(&mut out.vertex,oldv,topo.nvertices()),(&mut out.prim,oldp,topo.nprims())] {
        for a in table.values_mut(){let mut b=(**a).clone(); match &mut b.data {
            AttrData::F32(v)=>v.resize(new*b.width,0.),AttrData::I32(v)=>v.resize(new*b.width,0)};debug_assert_eq!(a.rows(),old);*a=Arc::new(b);}
    }
    topo.closed=Some(closed);out.topology=Arc::new(topo);Ok(out)
}

pub(crate) fn lengths(g:&Geometry,i:usize)->(Vec<usize>,Vec<f64>,f64) {
    let ids:Vec<_>=g.topology.vertex_point[g.topology.prim_offsets[i] as usize..g.topology.prim_offsets[i+1] as usize].iter().map(|&i|i as usize).collect();
    let mut cum=vec![0.];let mut total=0.;
    for j in 0..ids.len(){let next=if j+1<ids.len(){j+1}else{0};
        let d=if j+1==ids.len() && !g.topology.is_closed(i){0.}else{(0..3).map(|k|{let d=g.p()[ids[next]*3+k] as f64-g.p()[ids[j]*3+k] as f64;d*d}).sum::<f64>().sqrt()};
        total+=d;cum.push(total);
    } (ids,cum,total)
}
pub(crate) fn sample(g:&Geometry,prims:&[usize],lists:&[Vec<f64>])->Result<Geometry> {
    let n:usize=lists.iter().map(Vec::len).sum();count(n)?;
    let mut pa=Vec::new();let mut pb=Vec::new();let mut ts=Vec::new();let mut positions=Vec::new();let mut u=Vec::new();let mut offs=vec![0];
    for (&i,list) in prims.iter().zip(lists){let(ids,cum,total)=lengths(g,i);
        for &s in list {let j=cum[..ids.len()].partition_point(|&v| v<=s).saturating_sub(1).min(ids.len()-1);
            let a=ids[j];let b=ids[(j+1)%ids.len()];let seg=cum[j+1]-cum[j];let t=if seg>0. {((s-cum[j])/seg).clamp(0.,1.)}else{0.};
            pa.push(a);pb.push(b);ts.push(t);u.push(if total>0. {(s/total) as f32}else{0.});
            for k in 0..3 {positions.push((g.p()[a*3+k] as f64*(1.-t)+g.p()[b*3+k] as f64*t) as f32);}
        } offs.push(pa.len() as i32);
    }
    let mut out=Geometry::from_arrays(positions,(0..n as i32).collect(),offs,Some(prims.iter().map(|&i|g.topology.is_closed(i)).collect()))?;
    for (name,a) in &g.point {if name=="P" {continue;} let mut b=super::group::gather(a,&pa);
        if let AttrData::F32(v)=&mut b.data {if let AttrData::F32(src)=&a.data {
            for row in 0..n {let t=ts[row] as f32;for k in 0..a.width {v[row*a.width+k]=src[pa[row]*a.width+k]*(1.-t)+src[pb[row]*a.width+k]*t;}}
        }} out.point.insert(name.clone(),Arc::new(b));
    }
    out.point.insert("curveu".into(),Arc::new(Attr::f32(1,u)));out.prim=gather_table(&g.prim,prims);out.detail=g.detail.clone();Ok(out)
}
pub const CARVE:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None, cook_sidecar: None,name:"carve",inputs:GEO,params:crate::params!("start":0.0;"end":1.0),cook:carve};
fn carve(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=CARVE.one(inputs,0)?;let a=p.f("start")?;let b=p.f("end")?;let(lo,hi)=(a.min(b),a.max(b));let mut prims=Vec::new();let mut lists=Vec::new();
    for i in 0..g.nprims(){if g.topology.is_closed(i) || g.topology.prim_offsets[i+1]-g.topology.prim_offsets[i]<2{continue;}
        let (_,cum,total)=lengths(&g,i);let(a,b)=(lo*total,hi*total);let mut s=vec![a];
        if b>a {s.extend(cum.into_iter().filter(|&v|v>a+1e-12 && v<b-1e-12));s.push(b);s.dedup();}else{s.push(a);}
        prims.push(i);lists.push(s);
    }
    if prims.is_empty(){return Ok((*g).clone());}let out=sample(&g,&prims,&lists)?;
    if !g.topology.closed_mask().iter().any(|&c|c){return Ok(out);}
    let rest=keep_prims(&g,&g.topology.closed_mask(),false)?;
    super::merge::merge(&[vec![Arc::new(rest),Arc::new(out)]],&Params::resolve(&[],None)?)
}



pub const CURVE_SIMPLIFY:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None, cook_sidecar: None,name:"curve_simplify",inputs:GEO,params:crate::params!(
    "method":"distance";"tolerance":0.01;"angle":10.;"smooth":0;"keep":""),cook:simplify};
fn dot(a:[f64;3],b:[f64;3])->f64{(0..3).map(|i|a[i]*b[i]).sum()}
fn rdp(q:&[[f64;3]],lo:usize,hi:usize,tol:f64,keep:&mut[bool]){
    let mut stack=vec![(lo,hi)];while let Some((a,b))=stack.pop(){if b-a<2{continue;}let seg=sub(q[b],q[a]);let ln=dot(seg,seg);let(mut far,mut dist)=(a+1,-1.);
        for j in a+1..b{let v=sub(q[j],q[a]);let d=if ln<1e-24{norm(v)}else{let u=(dot(v,seg)/ln).clamp(0.,1.);norm(std::array::from_fn(|k|v[k]-u*seg[k]))};if d>dist{dist=d;far=j;}}
        if dist>tol{keep[far]=true;stack.push((a,far));stack.push((far,b));}
    }
}
fn simplify(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=CURVE_SIMPLIFY.one(inputs,0)?;let t=&g.topology;
    if !t.prim_offsets.windows(2).any(|r|r[1]-r[0]>=3){return Err(Error::new("curve_simplify needs curves with at least three points"));}
    let method=p.e("method",&["distance","angle"])?;let tolerance=p.f("tolerance")?;let angle=p.f("angle")?.to_radians();let smooth=p.i("smooth")?;
    if tolerance<0. || smooth<0{return Err(Error::new("tolerance and smooth must be nonnegative"));}
    let pin=super::group::mask(&g,"point",&p.s("keep")?)?;let has_pin=p.s("keep")?.split(|c:char|c==',' || c.is_whitespace()).any(|s|!s.is_empty());
    let mut rows=Vec::new();let mut offs=vec![0];
    for i in 0..g.nprims(){let start=t.prim_offsets[i] as usize;let end=t.prim_offsets[i+1] as usize;let n=end-start;let cl=t.is_closed(i);
        if n<3 || (cl && n<=3){rows.extend(start..end);offs.push(rows.len() as i32);continue;}
        let q:Vec<[f64;3]>=(start..end).map(|v|{let pt=t.vertex_point[v] as usize;std::array::from_fn(|k|g.p()[pt*3+k] as f64)}).collect();let mut keep=vec![false;n];keep[0]=true;
        if method=="angle"{
            if !cl{keep[n-1]=true;}let mut dirs=Vec::new();for j in 0..if cl{n}else{n-1}{let d=sub(q[(j+1)%n],q[j]);let l=norm(d);dirs.push(d.map(|v|v/if l>1e-30{l}else{1.}));}
            let mut total=0.;for j in 1..if cl{n}else{n-1}{total+=dot(dirs[j-1],dirs[j]).clamp(-1.,1.).acos();if total>=angle{keep[j]=true;total=0.;}}
        }else if cl{
            let mut far=0;for j in 1..n{if norm(sub(q[j],q[0]))>norm(sub(q[far],q[0])){far=j;}}keep[far]=true;rdp(&q,0,far,tolerance,&mut keep);
            let mut ring=q.clone();ring.push(q[0]);let mut tmp=vec![false;n+1];rdp(&ring,far,n,tolerance,&mut tmp);for j in 0..n{keep[j]|=tmp[j];}
        }else{keep[n-1]=true;rdp(&q,0,n-1,tolerance,&mut keep);}
        if has_pin{for j in 0..n{keep[j]|=pin[t.vertex_point[start+j] as usize];}}
        if cl && keep.iter().filter(|&&v|v).count()<3{for j in [0,n/3,2*n/3]{keep[j]=true;}}
        rows.extend((0..n).filter(|&j|keep[j]).map(|j|start+j));offs.push(rows.len() as i32);
    }
    let mut used:Vec<_>=rows.iter().map(|&v|t.vertex_point[v] as usize).collect();used.sort_unstable();used.dedup();let mut remap=vec![0;g.npoints];for(i,&v)in used.iter().enumerate(){remap[v]=i as i32;}
    let vp:Vec<_>=rows.iter().map(|&v|remap[t.vertex_point[v] as usize]).collect();let mut positions:Vec<[f64;3]>=used.iter().map(|&pt|std::array::from_fn(|k|g.p()[pt*3+k] as f64)).collect();
    for _ in 0..smooth{let mut next=positions.clone();for i in 0..g.nprims(){let start=offs[i] as usize;let end=offs[i+1] as usize;let n=end-start;if n<3{continue;}
        for j in 0..n{if !t.is_closed(i) && (j==0 || j==n-1){continue;}let pt=vp[start+j] as usize;if has_pin && pin[used[pt]]{continue;}let a=vp[start+(j+n-1)%n] as usize;let b=vp[start+(j+1)%n] as usize;
            next[pt]=std::array::from_fn(|k|0.5*positions[pt][k]+0.25*(positions[a][k]+positions[b][k]));}
    }positions=next;}
    let mut out=Geometry::from_arrays(positions.into_iter().flatten().map(|v|v as f32).collect(),vp,offs,if t.has_open(){Some(t.closed_mask())}else{None})?;
    let positions=out.p().to_vec();out.point=gather_table(&g.point,&used);out.set_p(positions);
    out.vertex=gather_table(&g.vertex,&rows);out.prim=g.prim.clone();out.detail=g.detail.clone();Ok(out)
}

mod sweep;
use sweep::{cross3, dot3, normalize3, rmf_frames, sweep};

const SWEEP_INPUTS: &[InputSpec] = &[
    InputSpec { name: "geo", multi: false, optional: false },
    InputSpec { name: "section", multi: false, optional: true },
];

pub const SWEEP: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "sweep",
    inputs: SWEEP_INPUTS,
    params: crate::params!(
        "radius": 0.05;
        "section_scale": 1.0;
        "divisions": 8;
        "caps": true;
        "twist": 0.0;
        "outgrp_sides": false;
        "outgrp_sides_name": "sweepsides";
        "outgrp_startcap": false;
        "outgrp_startcap_name": "startcap";
        "outgrp_endcap": false;
        "outgrp_endcap_name": "endcap"
    ),
    cook: sweep,
};

const PATH_DEFORM_INPUTS: &[InputSpec] = &[
    InputSpec { name: "geo", multi: false, optional: false },
    InputSpec { name: "curve", multi: false, optional: false },
];

pub const PATH_DEFORM: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "path_deform",
    inputs: PATH_DEFORM_INPUTS,
    params: crate::params!(
        "axis": "x";
        "frame": "minimal_rotation";
        "up": [0.0, 0.0, 1.0];
        "offset": 0.0;
        "stretch": false;
        "twist": 0.0;
        "group": "";
        "strength": 1.0
    ),
    cook: path_deform,
};

mod path_deform;
use path_deform::path_deform;

#[cfg(test)]
mod tests {
    use super::*;
    fn cook(spec:&NodeSpec,g:Geometry,v:serde_json::Value)->Geometry{(spec.cook)(&[vec![Arc::new(g)]],&Params::resolve(spec.params,v.as_object()).unwrap()).unwrap()}
    #[test]
    fn carve_keeps_original_knots(){
        let mut g=curve(vec![0.,0.,0.,0.,0.,1.,0.,0.,2.,0.,0.,3.,0.,0.,4.],false).unwrap();
        g.point.insert("id".into(),Arc::new(Attr::i32(1,vec![10,11,12,13,14])));
        let c=cook(&CARVE,g,serde_json::json!({"start":0.25,"end":0.6}));assert_eq!(c.p(),[0.,0.,1.,0.,0.,2.,0.,0.,2.4]);c.validate().unwrap();
    }
    #[test]
    fn branch_chains(){
        let (paths,cl)=chains(4,&[[0,1],[1,2],[1,3]]);assert_eq!(paths,vec![vec![0,1],vec![1,2],vec![1,3]]);assert_eq!(cl,[false;3]);
    }
    #[test]
    fn simplify_preserves_attributes(){
        let mut g=curve(vec![0.,0.,0.,1.,0.,0.,2.,0.,0.],false).unwrap();g.point.insert("id".into(),Arc::new(Attr::i32(1,vec![5,6,7])));
        let out=cook(&CURVE_SIMPLIFY,g,serde_json::json!({}));assert_eq!(out.npoints,2);assert_eq!(out.point["id"].data,AttrData::I32(vec![5,7]));out.validate().unwrap();
    }
    #[test]
    fn sweep_line_circle() {
        let line = curve(vec![0., 0., 0., 0., 0., 1.], false).unwrap();
        let p = Params::resolve(SWEEP.params, serde_json::json!({"divisions": 4, "caps": true}).as_object()).unwrap();
        let out = sweep(&[vec![Arc::new(line)], vec![]], &p).unwrap();
        assert_eq!(out.nprims(), 4 + 2); // 4 side quads + 2 caps
    }
    #[test]
    fn path_deform_straight_line() {
        let pts = Geometry::points_only(vec![0.5, 0.0, 0.0, 1.5, 0.0, 0.0]).unwrap();
        let crv = curve(vec![0., 0., 0., 10., 0., 0.], false).unwrap();
        let p = Params::resolve(PATH_DEFORM.params, serde_json::json!({"axis": "x"}).as_object()).unwrap();
        let out = path_deform(&[vec![Arc::new(pts)], vec![Arc::new(crv)]], &p).unwrap();
        assert_eq!(out.npoints, 2);
    }
    #[test]
    fn path_deform_group_and_strength() {
        let mut pts = Geometry::points_only(vec![0.5, 0., 0., 1.5, 0., 0.]).unwrap();
        pts.point.insert("group_pick".into(), Arc::new(Attr::i32(1, vec![1, 0])));
        let crv = curve(vec![0., 0., 0., 10., 0., 0.], false).unwrap();
        let p = Params::resolve(PATH_DEFORM.params, serde_json::json!({"group":"pick","strength":0.5}).as_object()).unwrap();
        let out = path_deform(&[vec![Arc::new(pts)], vec![Arc::new(crv)]], &p).unwrap();
        assert!((out.p()[0] - 0.25).abs() < 1e-6);
        assert!((out.p()[3] - 1.5).abs() < 1e-6);
    }
}

