//! Voronoi solvers from fracture.py. Kept separate from the concurrently ported exploded_view.
use super::{InputSpec,Inputs,NodeSpec,model::*,fracture_tools::{extent,pairs,contacts}};
use crate::{error::{Error,Result},geometry::{Attr,AttrData,AttrTable,Geometry},params::Params,topo};
use std::{collections::{BTreeMap,BTreeSet},sync::Arc};
const PORTS:&[InputSpec]=&[InputSpec{name:"geo",multi:false,optional:false},InputSpec{name:"points",multi:false,optional:false}];
pub(super) const INSIDE:&str="vfinsidetmp";
pub const VORONOI_FRACTURE:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name:"voronoi_fracture",inputs:PORTS,
    params:crate::params!("piece_attrib":"piece";"inside_group":"inside";"gap":0.;"create_inside":true;"cluster_pieces":false;"cluster_attrib":"cluster";
        "outgrp_inside":true;"outgrp_inside_name":"inside";"outgrp_outside":false;"outgrp_outside_name":"outside";
        "outgrp_cutpoints":false;"outgrp_cutpoints_name":"cutpoints";"outgrp_cutedges":false;"outgrp_cutedges_name":"cutedges"),cook:fracture,
};
pub const VORONOI_SPLIT:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name:"voronoi_split",inputs:PORTS,
    params:crate::params!("piece_attrib":"piece";"outgrp_cutpoints":false;"outgrp_cutpoints_name":"cutpoints";"outgrp_cutedges":false;"outgrp_cutedges_name":"cutedges"),cook:split,
};
#[derive(Clone)]
struct Plane {origin:[f64;3],normal:[f64;3],other:usize}
struct Cell {id:usize,geo:Geometry,planes:Vec<Plane>}
fn member(g:&Geometry,name:&str)->Vec<bool>{
    g.prim.get(&format!("group_{name}")).map(|a|match &a.data{AttrData::I32(v)=>v.iter().map(|&v|v!=0).collect(),AttrData::F32(v)=>v.iter().map(|&v|v!=0.).collect()}).unwrap_or_else(||vec![false;g.nprims()])
}
fn pad(t:&AttrTable,n:usize)->AttrTable{
    let tail=t.iter().map(|(k,a)|(k.clone(),Arc::new(a.zeros_like(n)))).collect();concat(t,&tail)
}
pub(super) fn cap_plane(g:Geometry,o:[f64;3],n:[f64;3],eps:f64)->Result<Geometry>{
    let e=topo::edges(&g.topology,g.npoints);let (_,nx,_)=corners(&g);let vp=&g.topology.vertex_point;
    let mut succ=vec![None;g.npoints];let mut starts=Vec::new();
    for v in 0..g.nvertices(){let edge=e.of_vertex[v];if edge>=0 && e.count[edge as usize]==1 {
        let a=vp[v] as usize;let b=vp[nx[v]] as usize;
        if succ[b].is_some(){return Ok(g);}
        succ[b]=Some(a);starts.push(b);
    }}
    let mut seen=vec![false;g.npoints];let pos=positions(&g);let mut loops=Vec::new();
    for start in starts{if seen[start]{continue;}let mut lp=Vec::new();let mut p=start;
        loop {if seen[p]{break;}seen[p]=true;lp.push(p);let Some(next)=succ[p] else{break;};p=next;}
        if p==start && lp.len()>=3 && lp.iter().all(|&i|dot(sub(pos[i],o),n).abs()<=eps){loops.push(lp);}
    }
    if loops.is_empty(){return Ok(g);}
    let mut sizes:Vec<_>=(0..g.nprims()).map(|i|range(&g,i).len()).collect();sizes.extend(loops.iter().map(Vec::len));
    let mut vp=g.topology.vertex_point.clone();vp.extend(loops.iter().flatten().map(|&v|v as i32));
    let count:usize=loops.iter().map(Vec::len).sum();
    let mut out=make(g.point.clone(),vp,&sizes,pad(&g.vertex,count),pad(&g.prim,loops.len()),g.detail.clone(),None)?;
    super::material_tables::neutral_new_faces(&mut out,g.nprims(),g.nvertices());
    let mut mask=member(&g,INSIDE);mask.extend(std::iter::repeat_n(true,loops.len()));write_group(&mut out,"prim",INSIDE,&mask)?;Ok(out)
}
fn cells(g:&Geometry,s:&[[f64;3]],gap:f64,cap:bool,eps:f64,prior:&[bool])->Result<Vec<Cell>>{
    let size=extent(g);let mut cells=Vec::new();
    for i in 0..s.len(){let mut piece=g.clone();let mut planes=Vec::new();
        if cap && !piece.prim.contains_key(&format!("group_{INSIDE}")){write_group(&mut piece,"prim",INSIDE,prior)?;}
        let dist:Vec<_>=s.iter().map(|&p|len(sub(p,s[i]))).collect();
        let mut order:Vec<_>=(0..s.len()).collect();order.sort_by(|&a,&b|dist[a].total_cmp(&dist[b]));
        let mut reach=positions(&piece).iter().map(|&p|len(sub(p,s[i]))).fold(0.,f64::max);
        for j in order{
            if i==j || dist[j]<1e-9*size.max(1e-9){continue;}
            if dist[j]/2.-gap/2.>reach{break;}
            let n=mul(sub(s[j],s[i]),1./dist[j]);let o=sub(mul(add(s[i],s[j]),0.5),mul(n,gap/2.));
            let cp=Params::resolve(super::model::CLIP.params,serde_json::json!({"origin":o,"normal":n,"keep":"below","outgrp_above":false}).as_object())?;
            piece=(super::model::CLIP.cook)(&[vec![Arc::new(piece)]],&cp)?;
            if piece.nprims()==0{break;}
            planes.push(Plane{origin:o,normal:n,other:j});
            if cap {piece=cap_plane(piece,o,n,eps)?;}
            reach=positions(&piece).iter().map(|&p|len(sub(p,s[i]))).fold(0.,f64::max);
        }
        if piece.nprims()>0{cells.push(Cell{id:i,geo:piece,planes});}
    }Ok(cells)
}
fn merge(parts:Vec<Geometry>)->Result<Geometry>{
    if parts.is_empty(){return Geometry::points_only(vec![]);}
    super::merge::merge(&[parts.into_iter().map(Arc::new).collect()],&Params::resolve(&[],None)?)
}
fn weld(g:Geometry,tol:f64)->Result<Geometry>{
    fn root(p:&mut[usize],mut i:usize)->usize{while p[i]!=i{p[i]=p[p[i]];i=p[i];}i}
    let mut parent:Vec<_>=(0..g.npoints).collect();
    for (a,b) in pairs(&positions(&g),tol){let a=root(&mut parent,a);let b=root(&mut parent,b);parent[a.max(b)]=a.min(b);}
    let roots:Vec<_>=(0..g.npoints).map(|i|root(&mut parent,i)).collect();
    let used:Vec<_>=roots.iter().copied().collect::<BTreeSet<_>>().into_iter().collect();
    if used.len()==g.npoints{return Ok(g);}
    let mut t=(*g.topology).clone();for v in &mut t.vertex_point{*v=used.binary_search(&roots[*v as usize]).unwrap() as i32;}
    let mut out=g.clone();out.npoints=used.len();out.point=gather(&g.point,&used);out.topology=Arc::new(t);Ok(out)
}
fn clusters(cells:Vec<Cell>,cluster:&[i32],eps:f64)->Result<Vec<Cell>>{
    let mut by=BTreeMap::<i64,Vec<Cell>>::new();
    for cell in cells{let key=cluster[cell.id];by.entry(if key>=0 {key as i64}else{-1-cell.id as i64}).or_default().push(cell);}
    let mut out=Vec::new();
    for (_,mut members) in by{if members.len()==1{out.push(members.remove(0));continue;}
        let ids:BTreeSet<_>=members.iter().map(|c|c.id).collect();
        let mut parts=Vec::new();let mut planes=Vec::new();
        for cell in members{let mut g=cell.geo;g.prim.insert("vfcell".into(),Arc::new(Attr::i32(1,vec![cell.id as i32;g.nprims()])));parts.push(g);planes.extend(cell.planes.into_iter().filter(|p|!ids.contains(&p.other)));}
        let g=merge(parts)?;let inside=member(&g,INSIDE);
        let labels=match &g.prim["vfcell"].data{AttrData::I32(v)=>v,_=>unreachable!()};
        let mut keep=vec![true;g.nprims()];for (a,b,_) in contacts(&g,labels,0.){if inside[a]{keep[a]=false;}if inside[b]{keep[b]=false;}}
        let mut g=weld(super::group::keep_prims(&g,&keep,false)?,(eps*5.).max(1e-9))?;g.prim.remove("vfcell");
        out.push(Cell{id:*ids.first().unwrap(),geo:g,planes});
    }out.sort_by_key(|c|c.id);Ok(out)
}
fn finish(mut c:Cell,p:&Params,eps:f64,inside:Option<&[bool]>,inside_name:Option<&str>)->Result<Geometry>{
    let name=p.s("piece_attrib")?;if !name.is_empty(){c.geo.prim.insert(name,Arc::new(Attr::i32(1,vec![c.id as i32;c.geo.nprims()])));}
    if let Some(mask)=inside{
        if p.b("outgrp_inside")? {if let Some(name)=inside_name.filter(|n|!n.trim().is_empty()){write_group(&mut c.geo,"prim",name.trim(),mask)?;}}
        outgroup(&mut c.geo,p,"outside","prim",&mask.iter().map(|b|!b).collect::<Vec<_>>())?;
        c.geo.prim.remove(&format!("group_{INSIDE}"));
    }
    if p.b("outgrp_cutpoints")? || p.b("outgrp_cutedges")?{
        let pos=positions(&c.geo);let e=topo::edges(&c.geo.topology,c.geo.npoints);let mut points=vec![false;c.geo.npoints];let mut edges=vec![false;e.len()];
        for plane in c.planes{let on:Vec<_>=pos.iter().map(|&p|dot(sub(p,plane.origin),plane.normal).abs()<=eps).collect();
            for (a,&b) in points.iter_mut().zip(&on){*a|=b;}
            for (k,v) in e.points.chunks_exact(2).enumerate(){edges[k]|=on[v[0] as usize]&&on[v[1] as usize];}
        }
        outgroup(&mut c.geo,p,"cutpoints","point",&points)?;outgroup(&mut c.geo,p,"cutedges","edge",&edges)?;
    }Ok(c.geo)
}
fn fracture(i:&Inputs,p:&Params)->Result<Geometry>{
    let g=VORONOI_FRACTURE.one(i,0)?;let points=VORONOI_FRACTURE.one(i,1)?;polygons(&g,"voronoi_fracture")?;
    if g.nprims()==0 || points.npoints==0{return Ok((*g).clone());}
    let cap=p.b("create_inside")?;let gap=p.f("gap")?;if !gap.is_finite() || gap<0.{return Err(Error::new("gap must be nonnegative"));}
    let old_name=p.s("inside_group")?;let inside_name=if old_name!="inside" {old_name}else{p.s("outgrp_inside_name")?};
    let eps=1e-6*extent(&g).max(1e-9);let prior=if cap{member(&g,&inside_name)}else{vec![false;g.nprims()]};
    let mut cs=cells(&g,&positions(&points),gap,cap,eps,&prior)?;
    if p.b("cluster_pieces")?{
        let name=p.s("cluster_attrib")?;let cl=points.point.get(&name).ok_or_else(||Error::new(format!("cluster_pieces needs a point attribute {name:?} on the points (voronoi_fracture_points writes it)")))?;
        if cap && cl.width!=1{return Err(Error::new("cluster attribute must be scalar"));}
        let labels:Vec<_>=match &cl.data{AttrData::I32(v)=>v.chunks_exact(cl.width).map(|v|v[0]).collect(),AttrData::F32(v)=>v.chunks_exact(cl.width).map(|v|v[0] as i32).collect()};
        if cap{cs=clusters(cs,&labels,eps)?;}
    }
    let mut parts=Vec::new();for c in cs{let mask=cap.then(||member(&c.geo,INSIDE));parts.push(finish(c,p,eps,mask.as_deref(),Some(&inside_name))?);}merge(parts)
}
fn split(i:&Inputs,p:&Params)->Result<Geometry>{
    let g=VORONOI_SPLIT.one(i,0)?;let points=VORONOI_SPLIT.one(i,1)?;polygons(&g,"voronoi_split")?;
    if g.nprims()==0 || points.npoints==0{return Ok((*g).clone());}
    let eps=1e-6*extent(&g).max(1e-9);let cs=cells(&g,&positions(&points),0.,false,eps,&[])?;
    merge(cs.into_iter().map(|c|finish(c,p,eps,None,None)).collect::<Result<Vec<_>>>()?)
}
