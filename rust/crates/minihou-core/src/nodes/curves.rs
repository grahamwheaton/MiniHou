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

fn path_deform(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = PATH_DEFORM.one(inputs, 0)?;
    let curve = PATH_DEFORM.one(inputs, 1)?;

    if geo.npoints == 0 {
        return Ok((*geo).clone());
    }
    if curve.nprims() == 0 {
        return Err(Error::new("the curve input is empty"));
    }

    let ct = &curve.topology;
    let start = ct.prim_offsets[0] as usize;
    let end = ct.prim_offsets[1] as usize;
    if end - start < 2 {
        return Err(Error::new("the curve needs at least two points"));
    }

    let cp_slice = curve.p();
    let mut c_pts: Vec<[f64; 3]> = (start..end).map(|v| {
        let pt = ct.vertex_point[v] as usize;
        [cp_slice[pt * 3] as f64, cp_slice[pt * 3 + 1] as f64, cp_slice[pt * 3 + 2] as f64]
    }).collect();

    let is_cl = ct.is_closed(0);
    if is_cl {
        c_pts.push(c_pts[0]);
    }

    let (mut t_vec, mut n_vec) = rmf_frames(&c_pts, &[c_pts.len()], &[false]);

    let mut s_cum = vec![0.0];
    let mut total_len = 0.0;
    for i in 0..c_pts.len() - 1 {
        let d = norm(sub(c_pts[i + 1], c_pts[i]));
        total_len += d;
        s_cum.push(total_len);
    }
    if total_len <= 0.0 {
        return Err(Error::new("the curve has no length"));
    }

    let up = p.v3("up")?;
    let frame = p.e("frame", &["minimal_rotation", "up_vector"])?;
    if frame == "up_vector" {
        for i in 0..t_vec.len() {
            let t = t_vec[i];
            let t_dot_u = dot3(t, up);
            let upp = [up[0] - t_dot_u * t[0], up[1] - t_dot_u * t[1], up[2] - t_dot_u * t[2]];
            if norm(upp) > 1e-6 {
                n_vec[i] = normalize3(upp);
            }
        }
    }

    let t0 = t_vec[0];
    let t0_dot_u = dot3(up, t0);
    let mut upp = [up[0] - t0_dot_u * t0[0], up[1] - t0_dot_u * t0[1], up[2] - t0_dot_u * t0[2]];
    let upp_len = norm(upp);
    if frame == "minimal_rotation" && upp_len > 1e-9 {
        upp = [upp[0] / upp_len, upp[1] / upp_len, upp[2] / upp_len];
        let n0_cross_upp = cross3(n_vec[0], upp);
        let ang = dot3(n0_cross_upp, t0).atan2(dot3(n_vec[0], upp));
        let (sn, cs) = ang.sin_cos();
        for i in 0..n_vec.len() {
            let bv = cross3(t_vec[i], n_vec[i]);
            n_vec[i] = [
                n_vec[i][0] * cs + bv[0] * sn,
                n_vec[i][1] * cs + bv[1] * sn,
                n_vec[i][2] * cs + bv[2] * sn,
            ];
        }
    }

    let axis = p.e("axis", &["x", "y", "z"])?;
    let k = match axis.as_str() { "x" => 0, "y" => 1, "z" => 2, _ => 0 };

    let stretch = p.b("stretch")?;
    let offset = p.f("offset")?;
    let twist = p.f("twist")?;
    let strength = p.f("strength")?;
    if !strength.is_finite() { return Err(Error::new("path_deform strength must be finite")); }
    let selected = super::group::mask(&geo, "point", &p.s("group")?)?;

    let p_slice = geo.p();
    let mut min_k = f64::INFINITY;
    let mut max_k = f64::NEG_INFINITY;
    for i in 0..geo.npoints {
        let val = p_slice[i * 3 + k] as f64;
        min_k = min_k.min(val);
        max_k = max_k.max(val);
    }
    let ptp_k = (max_k - min_k).max(1e-12);

    let mut new_p = Vec::with_capacity(geo.npoints * 3);
    let mut new_normals = geo.point.get("N").filter(|a| a.width == 3).map(|_| Vec::with_capacity(geo.npoints * 3));
    let n_slice = geo.point.get("N").and_then(|a| a.as_f32());

    for pt_idx in 0..geo.npoints {
        let orig = [
            p_slice[pt_idx * 3] as f64,
            p_slice[pt_idx * 3 + 1] as f64,
            p_slice[pt_idx * 3 + 2] as f64,
        ];
        if !selected[pt_idx] || strength == 0.0 {
            new_p.extend(orig.map(|v| v as f32));
            if let (Some(n_src), Some(n_dst)) = (n_slice, &mut new_normals) {
                n_dst.extend_from_slice(&n_src[pt_idx * 3..pt_idx * 3 + 3]);
            }
            continue;
        }
        let mut along = orig[k] - min_k;
        if stretch {
            along *= total_len / ptp_k;
        }
        along += offset;

        let i = s_cum[..s_cum.len() - 1]
            .partition_point(|&val| val <= along)
            .saturating_sub(1)
            .min(s_cum.len() - 2);
        let seg_len = (s_cum[i + 1] - s_cum[i]).max(1e-30);
        let f = ((along - s_cum[i]) / seg_len).clamp(0.0, 1.0);

        let mut base = [
            c_pts[i][0] * (1.0 - f) + c_pts[i + 1][0] * f,
            c_pts[i][1] * (1.0 - f) + c_pts[i + 1][1] * f,
            c_pts[i][2] * (1.0 - f) + c_pts[i + 1][2] * f,
        ];
        let tp = normalize3([
            t_vec[i][0] * (1.0 - f) + t_vec[i + 1][0] * f,
            t_vec[i][1] * (1.0 - f) + t_vec[i + 1][1] * f,
            t_vec[i][2] * (1.0 - f) + t_vec[i + 1][2] * f,
        ]);
        let mut np = [
            n_vec[i][0] * (1.0 - f) + n_vec[i + 1][0] * f,
            n_vec[i][1] * (1.0 - f) + n_vec[i + 1][1] * f,
            n_vec[i][2] * (1.0 - f) + n_vec[i + 1][2] * f,
        ];
        let dot_np_tp = dot3(np, tp);
        np = normalize3([
            np[0] - dot_np_tp * tp[0],
            np[1] - dot_np_tp * tp[1],
            np[2] - dot_np_tp * tp[2],
        ]);

        let extra = if along < 0.0 {
            along
        } else if along > total_len {
            along - total_len
        } else {
            0.0
        };
        base = [
            base[0] + tp[0] * extra,
            base[1] + tp[1] * extra,
            base[2] + tp[2] * extra,
        ];

        if twist != 0.0 {
            let a = twist.to_radians() * along.clamp(0.0, total_len) / total_len;
            let (sn, cs) = a.sin_cos();
            let bp = cross3(tp, np);
            np = [
                np[0] * cs + bp[0] * sn,
                np[1] * cs + bp[1] * sn,
                np[2] * cs + bp[2] * sn,
            ];
        }
        let bp = cross3(tp, np);

        let (col0, col1, col2) = match axis.as_str() {
            "x" => (tp, [-bp[0], -bp[1], -bp[2]], np),
            "y" => (bp, tp, np),
            "z" => ([-bp[0], -bp[1], -bp[2]], np, tp),
            _ => unreachable!(),
        };

        let mut local = orig;
        local[k] = 0.0;

        let res_pt = [
            base[0] + col0[0] * local[0] + col1[0] * local[1] + col2[0] * local[2],
            base[1] + col0[1] * local[0] + col1[1] * local[1] + col2[1] * local[2],
            base[2] + col0[2] * local[0] + col1[2] * local[1] + col2[2] * local[2],
        ];
        for k in 0..3 { new_p.push((orig[k] + strength * (res_pt[k] - orig[k])) as f32); }

        if let (Some(n_src), Some(n_dst)) = (n_slice, &mut new_normals) {
            let norm_orig = [
                n_src[pt_idx * 3] as f64,
                n_src[pt_idx * 3 + 1] as f64,
                n_src[pt_idx * 3 + 2] as f64,
            ];
            let res_norm = [
                col0[0] * norm_orig[0] + col1[0] * norm_orig[1] + col2[0] * norm_orig[2],
                col0[1] * norm_orig[0] + col1[1] * norm_orig[1] + col2[1] * norm_orig[2],
                col0[2] * norm_orig[0] + col1[2] * norm_orig[1] + col2[2] * norm_orig[2],
            ];
            for k in 0..3 { n_dst.push((norm_orig[k] + strength * (res_norm[k] - norm_orig[k])) as f32); }
        }
    }

    let mut out = (*geo).clone();
    out.set_p(new_p);
    if let Some(n_dst) = new_normals {
        out.point.insert("N".into(), Arc::new(Attr::f32(3, n_dst)));
    }
    Ok(out)
}

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

