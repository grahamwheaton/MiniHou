//! Arc-length curves and Bezier sources, matching nodes/curves.py.
use std::sync::Arc;
use super::{InputSpec, Inputs, NodeSpec, GEO, linspace};
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
pub const HELIX: NodeSpec = NodeSpec { name: "helix", inputs: &[], params: crate::params!(
    "radius": 1.0; "radius_end": -1.0; "height": 2.0; "turns": 3.0; "points": 120), cook: helix };
fn helix(_: &Inputs,p: &Params) -> Result<Geometry> {
    let n=p.i("points")?; if n<2 { return Err(Error::new("points must be at least 2")); } count(n as usize)?;
    let (r,h,turns)=(p.f("radius")?,p.f("height")?,p.f("turns")?);
    let end=p.f("radius_end")?; let end=if end<0.0 {r} else {end};
    let mut out=Vec::new();
    for t in linspace(0.0,1.0,n as usize) {
        let rad=r+(end-r)*t; let a=t*turns*2.0*std::f64::consts::PI;
        out.extend([(rad*a.cos()) as f32,(rad*a.sin()) as f32,(t*h) as f32]);
    }
    curve(out,false)
}
const CONTROLS: &str = r#"[{"p": [-1, 0, 0], "left": [-1.5, 0, 0], "right": [-0.5, 0, 0], "mode": "AUTO"}, {"p": [0, 0, 1], "left": [-0.5, 0, 1], "right": [0.5, 0, 1], "mode": "AUTO"}, {"p": [1, 0, 0], "left": [0.5, 0, 0], "right": [1.5, 0, 0], "mode": "AUTO"}]"#;
pub const BEZIER: NodeSpec = NodeSpec { name: "bezier", inputs: &[InputSpec{name:"geo",multi:false,optional:true}],
    params: crate::params!("controls": CONTROLS; "closed": false; "resolution": 24; "as_mesh": false), cook: bezier };
fn norm(v: [f64;3]) -> f64 { v.iter().map(|x| x*x).sum::<f64>().sqrt() }
fn sub(a:[f64;3],b:[f64;3])->[f64;3] { std::array::from_fn(|i| a[i]-b[i]) }
fn bezier(inputs:&Inputs,p:&Params)->Result<Geometry> {
    let value:serde_json::Value=serde_json::from_str(&p.s("controls")?).map_err(|e| Error::new(e.to_string()))?;
    let cs=value.as_array().ok_or_else(|| Error::new("Bezier curves need 2 to 10000 control points"))?;
    if !(2..=10000).contains(&cs.len()) { return Err(Error::new("Bezier curves need 2 to 10000 control points")); }
    let mut xyz=Vec::<[[f64;3];3]>::new(); let mut modes=Vec::new();
    for c in cs {
        let mode=c.get("mode").and_then(|v| v.as_str()).unwrap_or("FREE");
        if !["AUTO","VECTOR","ALIGNED","MIRRORED","FREE"].contains(&mode) { return Err(Error::new("Unknown Bezier handle mode")); }
        let mut row=[[0.;3];3];
        for (j,name) in ["p","left","right"].iter().enumerate() {
            let a=c.get(*name).and_then(|v| v.as_array()).filter(|v| v.len()==3).ok_or_else(|| Error::new("Bezier handles must be finite XYZ vectors"))?;
            for i in 0..3 { row[j][i]=a[i].as_f64().filter(|x| x.is_finite()).ok_or_else(|| Error::new("Bezier handles must be finite XYZ vectors"))?; }
        }
        xyz.push(row); modes.push(mode);
    }
    let closed=p.b("closed")?; let n=xyz.len();
    for i in 0..n {
        let pt=xyz[i][0];
        let prev=if closed || i>0 {xyz[(i+n-1)%n][0]} else {pt};
        let next=if closed || i+1<n {xyz[(i+1)%n][0]} else {pt};
        if modes[i]=="VECTOR" {
            xyz[i][1]=std::array::from_fn(|k| pt[k]+(prev[k]-pt[k])/3.);
            xyz[i][2]=std::array::from_fn(|k| pt[k]+(next[k]-pt[k])/3.);
        } else if modes[i]=="AUTO" {
            let d=sub(next,prev); let ln=norm(d).max(1e-12);
            xyz[i][1]=std::array::from_fn(|k| pt[k]-d[k]/ln*norm(sub(pt,prev))/3.);
            xyz[i][2]=std::array::from_fn(|k| pt[k]+d[k]/ln*norm(sub(next,pt))/3.);
        } else if modes[i]=="ALIGNED" || modes[i]=="MIRRORED" {
            let d=sub(xyz[i][2],pt); let ln=norm(d);
            if ln>1e-12 { let size=if modes[i]=="MIRRORED" {ln} else {norm(sub(xyz[i][1],pt))};
                xyz[i][1]=std::array::from_fn(|k| pt[k]-d[k]*size/ln); }
        }
    }
    let res=p.i("resolution")?; if !(1..=1024).contains(&res) {return Err(Error::new("resolution must be 1..1024"));}
    let seg=if closed {n} else {n-1}; count(seg*res as usize+usize::from(!closed))?;
    let mut out=Vec::new();
    for i in 0..seg { for j in 0..res {
        let t=j as f64/res as f64; let u=1.-t; let a=xyz[i]; let b=xyz[(i+1)%n];
        for k in 0..3 { out.push((u.powi(3)*a[0][k]+3.*u*u*t*a[2][k]+3.*u*t*t*b[1][k]+t.powi(3)*b[0][k]) as f32); }
    } }
    if !closed { out.extend(xyz[n-1][0].map(|x| x as f32)); }
    let mut out=curve(out,closed)?;
    if p.b("as_mesh")? { out.prim.remove("__viewport_curve"); }
    if let Some(g)=inputs[0].first() { super::merge::merge(&[vec![g.clone(),Arc::new(out)]],&Params::resolve(&[],None)?) } else {Ok(out)}
}

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
pub const CONVERT_LINE:NodeSpec=NodeSpec{name:"convert_line",inputs:GEO,params:crate::params!("join":true),cook:convert_line};
fn convert_line(inputs:&Inputs,p:&Params)->Result<Geometry> {
    let g=CONVERT_LINE.one(inputs,0)?; let e=crate::topo::edges(&g.topology,g.npoints);
    let edges:Vec<_>=e.points.chunks_exact(2).map(|e|[e[0] as i32,e[1] as i32]).collect();
    let (paths,cl)=if p.b("join")? {chains(g.npoints,&edges)} else {(edges.iter().map(|e|e.to_vec()).collect(),vec![false;edges.len()])};
    let mut out=(*g).clone();let mut vp=Vec::new();let mut offs=vec![0];
    for path in paths {vp.extend(path);offs.push(vp.len() as i32);}
    out.topology=Arc::new(Topology{vertex_point:vp,prim_offsets:offs,closed:Some(cl)});out.vertex.clear();out.prim.clear();Ok(out)
}
pub const POLYPATH:NodeSpec=NodeSpec{name:"polypath",inputs:GEO,params:&[],cook:polypath};
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
pub const CARVE:NodeSpec=NodeSpec{name:"carve",inputs:GEO,params:crate::params!("start":0.0;"end":1.0),cook:carve};
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

pub const FILLET:NodeSpec=NodeSpec{name:"fillet",inputs:GEO,params:crate::params!(
    "radius":0.2;"divisions":6;"min_angle":1.;"group":""),cook:fillet};
fn fillet(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=FILLET.one(inputs,0)?;if g.nprims()==0{return Ok((*g).clone());}
    let radius=p.f("radius")?;let divisions=p.i("divisions")?;if !(1..=256).contains(&divisions){return Err(Error::new("divisions must be 1..256"));}
    let min_angle=p.f("min_angle")?.max(1e-3).to_radians();let sel=super::group::mask(&g,"point",&p.s("group")?)?;
    let mut positions=Vec::new();let mut weights=Vec::<(usize,usize,usize,f64,f64,f64)>::new();let mut offs=vec![0];let mut rounded=false;
    for i in 0..g.nprims(){let start=g.topology.prim_offsets[i] as usize;let end=g.topology.prim_offsets[i+1] as usize;let n=end-start;let cl=g.topology.is_closed(i);
        let ids:Vec<_>=g.topology.vertex_point[start..end].iter().map(|&v|v as usize).collect();
        let q:Vec<[f64;3]>=ids.iter().map(|&pt|std::array::from_fn(|k|g.p()[pt*3+k] as f64)).collect();
        for j in 0..n{let prev=if j==0{if cl{n-1}else{0}}else{j-1};let next=if j+1==n{if cl{0}else{j}}else{j+1};
            let a=sub(q[prev],q[j]);let b=sub(q[next],q[j]);let(la,lb)=(norm(a),norm(b));let interior=n>=3 && (cl || (j>0 && j+1<n));let ok=interior && la>1e-12 && lb>1e-12;
            let a=a.map(|v|v/if ok{la}else{1.});let b=b.map(|v|v/if ok{lb}else{1.});let theta=dot(a,b).clamp(-1.,1.).acos();let turn=std::f64::consts::PI-theta;
            let corner=ok && sel[ids[j]] && radius>0. && turn>=min_angle && theta>1e-3;
            if !corner{positions.extend(q[j].map(|v|v as f32));weights.push((ids[j],ids[prev],ids[next],0.,0.,0.));continue;}
            rounded=true;let half=theta/2.;let end_prev=!cl && (prev==0 || prev+1==n);let end_next=!cl && (next==0 || next+1==n);
            let room=(if end_prev{la}else{la/2.}).min(if end_next{lb}else{lb/2.});let tang=(radius/half.tan()).min(room);let r=tang*half.tan();
            let bis=std::array::from_fn(|k|a[k]+b[k]);let bn=norm(bis);let bis=bis.map(|v|v/if bn>1e-12{bn}else{1.});
            let centre=std::array::from_fn::<_,3,_>(|k|q[j][k]+bis[k]*(r/half.sin().max(1e-12)));
            let u1=std::array::from_fn::<_,3,_>(|k|(q[j][k]+a[k]*tang-centre[k])/r.max(1e-12));let u2=std::array::from_fn::<_,3,_>(|k|(q[j][k]+b[k]*tang-centre[k])/r.max(1e-12));let om=turn.max(1e-6);
            count(weights.len()+divisions as usize+1)?;
            for k in 0..=divisions{let s=k as f64/divisions as f64;let w1=((1.-s)*om).sin()/om.sin();let w2=(s*om).sin()/om.sin();
                positions.extend((0..3).map(|c|(centre[c]+r*(u1[c]*w1+u2[c]*w2)) as f32));weights.push((ids[j],ids[prev],ids[next],tang/la,tang/lb,s));}
        }offs.push(weights.len() as i32);
    }
    if !rounded{return Ok((*g).clone());}count(weights.len())?;
    let mut out=Geometry::from_arrays(positions,(0..weights.len() as i32).collect(),offs,Some(g.topology.closed_mask()))?;
    for(name,attr)in &g.point{if name=="P"{continue;}let mut a=attr.zeros_like(weights.len());match(&attr.data,&mut a.data){
        (AttrData::F32(src),AttrData::F32(dst))=>for(i,&(ic,ip,inx,fa,fb,s))in weights.iter().enumerate(){let(fa,fb,s)=(fa as f32,fb as f32,s as f32);for k in 0..a.width{let v1=src[ic*a.width+k]*(1.-fa)+src[ip*a.width+k]*fa;let v2=src[ic*a.width+k]*(1.-fb)+src[inx*a.width+k]*fb;dst[i*a.width+k]=v1*(1.-s)+v2*s;}},
        (AttrData::I32(src),AttrData::I32(dst))=>for(i,&(ic,_,_,_,_,_))in weights.iter().enumerate(){dst[i*a.width..(i+1)*a.width].copy_from_slice(&src[ic*a.width..(ic+1)*a.width]);},_=>unreachable!(),
    }out.point.insert(name.clone(),Arc::new(a));}
    out.prim=g.prim.clone();out.detail=g.detail.clone();Ok(out)
}

pub const CURVE_SIMPLIFY:NodeSpec=NodeSpec{name:"curve_simplify",inputs:GEO,params:crate::params!(
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

fn normalize3(v: [f64; 3]) -> [f64; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 1e-30 { [v[0] / l, v[1] / l, v[2] / l] } else { [0.0, 0.0, 0.0] }
}

fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn rmf_frames(p: &[[f64; 3]], sizes: &[usize], closed: &[bool]) -> (Vec<[f64; 3]>, Vec<[f64; 3]>) {
    let n = p.len();
    let mut t_vec = vec![[0.0; 3]; n];
    let mut n_vec = vec![[0.0; 3]; n];

    let mut first = 0;
    for (&sz, &cl) in sizes.iter().zip(closed) {
        if sz == 0 {
            continue;
        }
        for k in 0..sz {
            let cur = first + k;
            let prev = if k == 0 { if cl { first + sz - 1 } else { first } } else { cur - 1 };
            let nxt = if k == sz - 1 { if cl { first } else { first + sz - 1 } } else { cur + 1 };
            let d = [p[nxt][0] - p[prev][0], p[nxt][1] - p[prev][1], p[nxt][2] - p[prev][2]];
            t_vec[cur] = normalize3(d);
        }

        let t0 = t_vec[first];
        let helper = if t0[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
        n_vec[first] = normalize3(cross3(cross3(t0, helper), t0));

        for k in 1..sz {
            let cur = first + k;
            let p0 = p[cur - 1];
            let p1 = p[cur];
            let v1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
            let c1 = dot3(v1, v1).max(1e-30);
            let n_prev = n_vec[cur - 1];
            let t_prev = t_vec[cur - 1];
            let dot_v1_n = dot3(v1, n_prev);
            let dot_v1_t = dot3(v1, t_prev);
            let r_l = [
                n_prev[0] - (2.0 / c1 * dot_v1_n) * v1[0],
                n_prev[1] - (2.0 / c1 * dot_v1_n) * v1[1],
                n_prev[2] - (2.0 / c1 * dot_v1_n) * v1[2],
            ];
            let t_l = [
                t_prev[0] - (2.0 / c1 * dot_v1_t) * v1[0],
                t_prev[1] - (2.0 / c1 * dot_v1_t) * v1[1],
                t_prev[2] - (2.0 / c1 * dot_v1_t) * v1[2],
            ];
            let v2 = [t_vec[cur][0] - t_l[0], t_vec[cur][1] - t_l[1], t_vec[cur][2] - t_l[2]];
            let c2 = dot3(v2, v2).max(1e-30);
            let dot_v2_rl = dot3(v2, r_l);
            n_vec[cur] = [
                r_l[0] - (2.0 / c2 * dot_v2_rl) * v2[0],
                r_l[1] - (2.0 / c2 * dot_v2_rl) * v2[1],
                r_l[2] - (2.0 / c2 * dot_v2_rl) * v2[2],
            ];
        }

        for k in 0..sz {
            let cur = first + k;
            let d = dot3(n_vec[cur], t_vec[cur]);
            n_vec[cur] = normalize3([
                n_vec[cur][0] - d * t_vec[cur][0],
                n_vec[cur][1] - d * t_vec[cur][1],
                n_vec[cur][2] - d * t_vec[cur][2],
            ]);
        }
        first += sz;
    }
    (t_vec, n_vec)
}

const SWEEP_INPUTS: &[InputSpec] = &[
    InputSpec { name: "geo", multi: false, optional: false },
    InputSpec { name: "section", multi: false, optional: true },
];

pub const SWEEP: NodeSpec = NodeSpec {
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

fn sweep(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = SWEEP.one(inputs, 0)?;
    let section = inputs.get(1).and_then(|v| v.first());

    let radius = p.f("radius")?;
    let section_scale = p.f("section_scale")?;
    let divisions = p.i("divisions")?;
    if divisions < 3 {
        return Err(Error::new("divisions must be at least 3"));
    }
    let caps = p.b("caps")?;
    let twist = p.f("twist")?;

    let t = &geo.topology;
    let mut usable = Vec::new();
    for prim_idx in 0..geo.nprims() {
        if t.prim_offsets[prim_idx + 1] - t.prim_offsets[prim_idx] >= 2 {
            usable.push(prim_idx);
        }
    }
    if usable.is_empty() {
        return Err(Error::new("sweep needs curves; make them with line, curve, helix, convert_line or resample"));
    }

    let p_slice = geo.p();
    let mut curve_pts: Vec<[f64; 3]> = Vec::new();
    let mut sizes = Vec::new();
    let mut closed = Vec::new();
    let mut vp_indices = Vec::new();
    let mut u_coords = Vec::new();

    for &prim_idx in &usable {
        let start = t.prim_offsets[prim_idx] as usize;
        let end = t.prim_offsets[prim_idx + 1] as usize;
        let is_cl = t.is_closed(prim_idx);
        let count = end - start;
        sizes.push(count);
        closed.push(is_cl);

        let mut pts = Vec::with_capacity(count);
        let mut vps = Vec::with_capacity(count);
        for v in start..end {
            let pt = t.vertex_point[v] as usize;
            vps.push(pt);
            pts.push([p_slice[pt * 3] as f64, p_slice[pt * 3 + 1] as f64, p_slice[pt * 3 + 2] as f64]);
        }

        let mut s_cum = vec![0.0];
        let mut total = 0.0;
        for i in 0..count - 1 {
            let d = (0..3).map(|k| (pts[i + 1][k] - pts[i][k]).powi(2)).sum::<f64>().sqrt();
            total += d;
            s_cum.push(total);
        }
        if is_cl {
            let d = (0..3).map(|k| (pts[0][k] - pts[count - 1][k]).powi(2)).sum::<f64>().sqrt();
            total += d;
        }
        let total = if total > 0.0 { total } else { 1.0 };
        for &s in &s_cum {
            u_coords.push((s / total) as f32);
        }

        curve_pts.extend(pts);
        vp_indices.extend(vps);
    }

    let (t_vec, n_vec) = rmf_frames(&curve_pts, &sizes, &closed);

    // Cross section
    let (sx, sy, sec_closed, vcoord): (Vec<f64>, Vec<f64>, bool, Vec<f64>) = if let Some(sec) = section {
        let st = &sec.topology;
        let prim_sec = (0..sec.nprims()).find(|&pr| st.prim_offsets[pr + 1] - st.prim_offsets[pr] >= 2)
            .ok_or_else(|| Error::new("sweep: the cross-section input has no polygon or curve"))?;
        let start = st.prim_offsets[prim_sec] as usize;
        let end = st.prim_offsets[prim_sec + 1] as usize;
        let sp_slice = sec.p();
        let pts: Vec<[f64; 3]> = (start..end).map(|v| {
            let pt = st.vertex_point[v] as usize;
            [sp_slice[pt * 3] as f64, sp_slice[pt * 3 + 1] as f64, sp_slice[pt * 3 + 2] as f64]
        }).collect();
        let is_cl = st.is_closed(prim_sec);
        let n_sec = pts.len();
        if n_sec < 2 {
            return Err(Error::new("sweep: the cross-section needs at least two points"));
        }
        let c = [
            pts.iter().map(|p| p[0]).sum::<f64>() / n_sec as f64,
            pts.iter().map(|p| p[1]).sum::<f64>() / n_sec as f64,
            pts.iter().map(|p| p[2]).sum::<f64>() / n_sec as f64,
        ];
        let sx: Vec<f64> = pts.iter().map(|p| p[0] - c[0]).collect();
        let sy: Vec<f64> = pts.iter().map(|p| p[1] - c[1]).collect();

        let mut s_cum = vec![0.0];
        let mut total = 0.0;
        for i in 0..n_sec - 1 {
            let d = ((sx[i + 1] - sx[i]).powi(2) + (sy[i + 1] - sy[i]).powi(2)).sqrt();
            total += d;
            s_cum.push(total);
        }
        if is_cl {
            let d = ((sx[0] - sx[n_sec - 1]).powi(2) + (sy[0] - sy[n_sec - 1]).powi(2)).sqrt();
            total += d;
        }
        let total = if total > 0.0 { total } else { 1.0 };
        let mut vc: Vec<f64> = s_cum.iter().map(|&s| s / total).collect();
        if is_cl {
            vc.push(1.0);
        }
        (sx, sy, is_cl && n_sec >= 3, vc)
    } else {
        let d = divisions as usize;
        let mut sx = Vec::with_capacity(d);
        let mut sy = Vec::with_capacity(d);
        let mut vc = Vec::with_capacity(d + 1);
        for i in 0..d {
            let a = i as f64 * 2.0 * std::f64::consts::PI / d as f64;
            sx.push(a.cos());
            sy.push(a.sin());
            vc.push(i as f64 / d as f64);
        }
        vc.push(1.0);
        (sx, sy, true, vc)
    };

    let d_count = sx.len();
    let num_ring_pts = curve_pts.len();
    let mut ring_pts = Vec::with_capacity(num_ring_pts * d_count * 3);

    let width_attr = geo.point.get("width").or_else(|| geo.point.get("pscale"));
    let width_data = width_attr.and_then(|a| a.as_f32());

    for (i, p) in curve_pts.iter().enumerate() {
        let scale = if section.is_none() { radius } else { section_scale };
        let scale = if let Some(wd) = width_data {
            scale * wd[vp_indices[i] * width_attr.unwrap().width] as f64
        } else {
            scale
        };

        let u = u_coords[i] as f64;
        let tw = twist.to_radians() * u;
        let (sn, cs) = tw.sin_cos();

        let n = n_vec[i];
        let bn = cross3(t_vec[i], n);

        for j in 0..d_count {
            let cx = cs * sx[j] - sn * sy[j];
            let cy = sn * sx[j] + cs * sy[j];
            let pt = [
                p[0] + scale * (cx * n[0] + cy * bn[0]),
                p[1] + scale * (cx * n[1] + cy * bn[1]),
                p[2] + scale * (cx * n[2] + cy * bn[2]),
            ];
            ring_pts.push(pt[0] as f32);
            ring_pts.push(pt[1] as f32);
            ring_pts.push(pt[2] as f32);
        }
    }

    let nseg = if sec_closed { d_count } else { d_count - 1 };
    let mut quads = Vec::new();
    let mut uv_quads = Vec::new();
    let mut prim_src = Vec::new();

    let mut curve_start = 0usize;
    for (prim_num, (&sz, &cl)) in sizes.iter().zip(&closed).enumerate() {
        for local_i in 0..sz {
            let has_next = local_i < sz - 1 || cl;
            if !has_next {
                continue;
            }
            let i0 = curve_start + local_i;
            let i1 = if local_i == sz - 1 { curve_start } else { i0 + 1 };

            let u0 = u_coords[i0];
            let u1 = if i1 == curve_start { 1.0f32 } else { u_coords[i1] };

            for js in 0..nseg {
                let j_next = (js + 1) % d_count;
                let a = (i0 * d_count + js) as i32;
                let b = (i0 * d_count + j_next) as i32;
                let c = (i1 * d_count + j_next) as i32;
                let d = (i1 * d_count + js) as i32;

                quads.push([a, b, c, d]);
                let v0 = vcoord[js] as f32;
                let v1 = vcoord[js + 1] as f32;
                uv_quads.push([[u0, v0], [u0, v1], [u1, v1], [u1, v0]]);
                prim_src.push(usable[prim_num]);
            }
        }
        curve_start += sz;
    }

    let mut flat_vp = Vec::new();
    let mut flat_uv = Vec::new();
    let mut sizes_out = Vec::new();

    for (q, uvs) in quads.iter().zip(&uv_quads) {
        flat_vp.extend_from_slice(q);
        for uv in uvs {
            flat_uv.extend_from_slice(uv);
        }
        sizes_out.push(4i32);
    }
    let nq = quads.len();
    let mut ncaps = 0usize;

    if caps && sec_closed {
        let mut c_start = 0usize;
        for (prim_num, (&sz, &cl)) in sizes.iter().zip(&closed).enumerate() {
            if !cl {
                let s_ring = c_start;
                let e_ring = c_start + sz - 1;

                // Start cap (reversed)
                for j in (0..d_count).rev() {
                    flat_vp.push((s_ring * d_count + j) as i32);
                    flat_uv.push(0.5 + 0.5 * (sx[j] / 1.0) as f32);
                    flat_uv.push(0.5 + 0.5 * (sy[j] / 1.0) as f32);
                }
                sizes_out.push(d_count as i32);
                prim_src.push(usable[prim_num]);

                // End cap
                for j in 0..d_count {
                    flat_vp.push((e_ring * d_count + j) as i32);
                    flat_uv.push(0.5 + 0.5 * (sx[j] / 1.0) as f32);
                    flat_uv.push(0.5 + 0.5 * (sy[j] / 1.0) as f32);
                }
                sizes_out.push(d_count as i32);
                prim_src.push(usable[prim_num]);

                ncaps += 1;
            }
            c_start += sz;
        }
    }

    let mut offsets = vec![0i32];
    for &sz in &sizes_out {
        offsets.push(offsets.last().unwrap() + sz);
    }

    let mut out = Geometry::from_arrays(ring_pts, flat_vp, offsets, None)?;

    // Point attributes replicated d_count times
    for (name, attr) in &geo.point {
        if name == "P" || name == "N" {
            continue;
        }
        let w = attr.width;
        let mut rep = attr.zeros_like(num_ring_pts * d_count);
        match (&attr.data, &mut rep.data) {
            (AttrData::F32(src), AttrData::F32(dst)) => {
                for i in 0..num_ring_pts {
                    let orig_pt = vp_indices[i];
                    for j in 0..d_count {
                        let dst_idx = i * d_count + j;
                        dst[dst_idx * w..(dst_idx + 1) * w].copy_from_slice(&src[orig_pt * w..(orig_pt + 1) * w]);
                    }
                }
            }
            (AttrData::I32(src), AttrData::I32(dst)) => {
                for i in 0..num_ring_pts {
                    let orig_pt = vp_indices[i];
                    for j in 0..d_count {
                        let dst_idx = i * d_count + j;
                        dst[dst_idx * w..(dst_idx + 1) * w].copy_from_slice(&src[orig_pt * w..(orig_pt + 1) * w]);
                    }
                }
            }
            _ => unreachable!(),
        }
        out.point.insert(name.clone(), Arc::new(rep));
    }

    let mut curveu = Vec::with_capacity(num_ring_pts * d_count);
    for &u in &u_coords {
        for _ in 0..d_count {
            curveu.push(u);
        }
    }
    out.point.insert("curveu".into(), Arc::new(Attr::f32(1, curveu)));
    out.vertex.insert("uv".into(), Arc::new(Attr::f32(2, flat_uv)));

    // Prim attributes gathered
    for (name, attr) in &geo.prim {
        let w = attr.width;
        let mut gathered = attr.zeros_like(prim_src.len());
        match (&attr.data, &mut gathered.data) {
            (AttrData::F32(src), AttrData::F32(dst)) => {
                for (out_i, &src_i) in prim_src.iter().enumerate() {
                    dst[out_i * w..(out_i + 1) * w].copy_from_slice(&src[src_i * w..(src_i + 1) * w]);
                }
            }
            (AttrData::I32(src), AttrData::I32(dst)) => {
                for (out_i, &src_i) in prim_src.iter().enumerate() {
                    dst[out_i * w..(out_i + 1) * w].copy_from_slice(&src[src_i * w..(src_i + 1) * w]);
                }
            }
            _ => unreachable!(),
        }
        out.prim.insert(name.clone(), Arc::new(gathered));
    }

    if p.b("outgrp_sides")? {
        let grp_name = p.s("outgrp_sides_name")?;
        if !grp_name.is_empty() {
            let mut vals = vec![0i32; out.nprims()];
            for v in &mut vals[..nq] { *v = 1; }
            out.prim.insert(grp_name, Arc::new(Attr::i32(1, vals)));
        }
    }

    if p.b("outgrp_startcap")? {
        let grp_name = p.s("outgrp_startcap_name")?;
        if !grp_name.is_empty() {
            let mut vals = vec![0i32; out.nprims()];
            for i in 0..ncaps {
                vals[nq + i * 2] = 1;
            }
            out.prim.insert(grp_name, Arc::new(Attr::i32(1, vals)));
        }
    }

    if p.b("outgrp_endcap")? {
        let grp_name = p.s("outgrp_endcap_name")?;
        if !grp_name.is_empty() {
            let mut vals = vec![0i32; out.nprims()];
            for i in 0..ncaps {
                vals[nq + i * 2 + 1] = 1;
            }
            out.prim.insert(grp_name, Arc::new(Attr::i32(1, vals)));
        }
    }

    Ok(out)
}

const PATH_DEFORM_INPUTS: &[InputSpec] = &[
    InputSpec { name: "geo", multi: false, optional: false },
    InputSpec { name: "curve", multi: false, optional: false },
];

pub const PATH_DEFORM: NodeSpec = NodeSpec {
    name: "path_deform",
    inputs: PATH_DEFORM_INPUTS,
    params: crate::params!(
        "axis": "x";
        "frame": "minimal_rotation";
        "up": [0.0, 0.0, 1.0];
        "offset": 0.0;
        "stretch": false;
        "twist": 0.0
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
        new_p.push(res_pt[0] as f32);
        new_p.push(res_pt[1] as f32);
        new_p.push(res_pt[2] as f32);

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
            n_dst.push(res_norm[0] as f32);
            n_dst.push(res_norm[1] as f32);
            n_dst.push(res_norm[2] as f32);
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
    fn simplify_preserves_attributes_and_fillet_keeps_ends(){
        let mut g=curve(vec![0.,0.,0.,1.,0.,0.,2.,0.,0.],false).unwrap();g.point.insert("id".into(),Arc::new(Attr::i32(1,vec![5,6,7])));
        let out=cook(&CURVE_SIMPLIFY,g,serde_json::json!({}));assert_eq!(out.npoints,2);assert_eq!(out.point["id"].data,AttrData::I32(vec![5,7]));out.validate().unwrap();
        let g=curve(vec![0.,0.,0.,1.,0.,0.,1.,1.,0.],false).unwrap();let out=cook(&FILLET,g,serde_json::json!({"radius":0.2,"divisions":2}));
        assert_eq!(out.npoints,5);assert_eq!(&out.p()[..3],&[0.,0.,0.]);assert_eq!(&out.p()[12..],&[1.,1.,0.]);assert!((out.p()[3]-0.8).abs()<1e-6);assert!((out.p()[10]-0.2).abs()<1e-6);out.validate().unwrap();
    }
    #[test]
    fn bezier_vector_handles_and_optional_input(){
        let p=Params::resolve(BEZIER.params,serde_json::json!({"controls":"[{\"p\":[0,0,0],\"left\":[0,0,0],\"right\":[0,0,0],\"mode\":\"VECTOR\"},{\"p\":[3,0,0],\"left\":[0,0,0],\"right\":[0,0,0],\"mode\":\"VECTOR\"}]","resolution":3}).as_object()).unwrap();
        let g=bezier(&[vec![]],&p).unwrap();assert_eq!(g.p(),[0.,0.,0.,1.,0.,0.,2.,0.,0.,3.,0.,0.]);
        let g=bezier(&[vec![Arc::new(Geometry::points_only(vec![9.,9.,9.]).unwrap())]],&p).unwrap();assert_eq!(g.npoints,5);
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
}

