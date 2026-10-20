use crate::nodes::{Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, Geometry};
use crate::params::{Params};
use crate::nodes::attrib_tools::{set, piece_labels};
pub(super) const NODE:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None, cook_sidecar: None,name:"measure",inputs:GEO,params:crate::params!("measure":"area";"name":""),cook:measure};
fn measure(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=NODE.one(inputs,0)?;let mode=p.e("measure",&["area","perimeter","curvature","piece_area","piece_volume"])?;let name=p.s("name")?;let name=if name.is_empty(){mode.clone()}else{name};
    let face=crate::normals::prim_normals(g.p(),&g.topology);let norm=|v:&[f64;3]|v.iter().map(|x|x*x).sum::<f64>().sqrt();
    if mode=="area"{return Ok(set(&g,"prim",name,Attr::f32(1,face.iter().map(|v|(norm(v)/2.) as f32).collect())));}
    if mode=="perimeter"{let mut values=Vec::new();for i in 0..g.nprims(){let ids=&g.topology.vertex_point[g.topology.prim_offsets[i] as usize..g.topology.prim_offsets[i+1] as usize];let mut length=0.;
        let n=if g.topology.is_closed(i){ids.len()}else{ids.len().saturating_sub(1)};for j in 0..n{let(a,b)=(ids[j] as usize,ids[(j+1)%ids.len()] as usize);length+=(0..3).map(|k|{let d=g.p()[a*3+k] as f64-g.p()[b*3+k] as f64;d*d}).sum::<f64>().sqrt();}values.push(length as f32);
    }return Ok(set(&g,"prim",name,Attr::f32(1,values)));}
    if mode=="curvature"{
        let e=crate::topo::edges(&g.topology,g.npoints);let mut sums=vec![[0.;3];g.npoints];let mut degree=vec![0usize;g.npoints];let mut l2=vec![0.;g.npoints];let mut acc=vec![[0.;3];g.npoints];let mut poly_count=vec![0usize;e.len()];
        // Python sums first all contributions to lower endpoints, then higher.
        for side in 0..2{for pair in e.points.chunks_exact(2){let(a,b)=(pair[side] as usize,pair[1-side] as usize);degree[a]+=1;let mut dist=0.;for k in 0..3{sums[a][k]+=g.p()[b*3+k] as f64;let d=g.p()[a*3+k] as f64-g.p()[b*3+k] as f64;dist+=d*d;}l2[a]+=dist;}}
        for i in 0..g.nprims(){for v in g.topology.prim_offsets[i] as usize..g.topology.prim_offsets[i+1] as usize{let pt=g.topology.vertex_point[v] as usize;for k in 0..3{acc[pt][k]+=face[i][k];}if g.topology.is_closed(i) && e.of_vertex[v]>=0{poly_count[e.of_vertex[v] as usize]+=1;}}}
        let normals=crate::normals::normalize_to_f32(&acc);let mut boundary=vec![false;g.npoints];for(i,&n)in poly_count.iter().enumerate(){if n==1{boundary[e.points[i*2] as usize]=true;boundary[e.points[i*2+1] as usize]=true;}}
        let values=(0..g.npoints).map(|i|{if boundary[i]{0.}else{let deg=degree[i].max(1) as f64;let h=(0..3).map(|k|(g.p()[i*3+k] as f64-sums[i][k]/deg)*normals[i*3+k] as f64).sum::<f64>();(2.*h/(l2[i]/deg).max(1e-24)) as f32}}).collect();
        return Ok(set(&g,"point",name,Attr::f32(1,values)));
    }
    let(_,labels)=piece_labels(&g);if labels.iter().any(|&i|i<0){return Err(Error::new("piece measures require nonempty primitives"));}
    let mut totals=vec![0.;labels.iter().copied().max().map_or(0,|i|i as usize+1)];let centres=crate::nodes::group::centroids64(&g);
    for(i,&lab)in labels.iter().enumerate(){totals[lab as usize]+=if mode=="piece_area"{norm(&face[i])/2.}else{(0..3).map(|k|face[i][k]*centres[i*3+k]).sum::<f64>()/6.};}
    Ok(set(&g,"prim",name,Attr::f32(1,labels.into_iter().map(|i|totals[i as usize] as f32).collect())))
}


#[cfg(test)]
#[path = "tests.rs"]
mod tests;
