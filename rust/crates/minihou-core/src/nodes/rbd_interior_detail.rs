//! Render tessellation of material fracture caps, including the boundary of untouched faces.
use std::{collections::{BTreeMap, BTreeSet}, sync::Arc};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, AttrTable, Geometry}, params::Params, topo};
use super::model;

type Key = (u8, usize, usize, usize);
type Blend = ([usize; 3], [f64; 3]);

fn interpolate(table: &AttrTable, rows: &[Blend]) -> AttrTable {
    table.iter().map(|(name, attr)| {
        let w = attr.width;
        let value = match &attr.data {
            AttrData::F32(v) => Attr::f32(w, rows.iter().flat_map(|(ids, wt)| (0..w).map(move |c| {
                (0..3).filter(|&k| wt[k]!=0.).map(|k| v[ids[k]*w+c] as f64 * wt[k]).sum::<f64>() as f32
            })).collect()),
            AttrData::I32(v) => Attr::i32(w, rows.iter().flat_map(|(ids, wt)| {
                let mut best=0; for k in 1..3 { if wt[k]>wt[best] {best=k;} }
                v[ids[best]*w..(ids[best]+1)*w].to_vec()
            }).collect()),
        };
        (name.clone(), Arc::new(value))
    }).collect()
}

pub(super) fn apply(g: Geometry, p: &Params) -> Result<Geometry> {
    let n=p.i("detail_divisions")?;
    let amp=p.f("detail_amplitude")?;
    let freq=p.f("detail_frequency")?;
    if !(1..=16).contains(&n) || !amp.is_finite() || amp<0. || !freq.is_finite() || freq<0. {
        return Err(Error::new("invalid interior detail controls"));
    }
    let n=n as usize;
    let name=p.s("outgrp_inside_name")?;
    let group=format!("group_{}", if name.is_empty(){"inside"}else{&name});
    let inside=g.prim.get(&group).ok_or_else(||Error::new("interior_detail needs the inside faces group switched on"))?;
    let inside=super::group::flags(inside);
    let mut tris=Vec::<([usize;3],usize)>::new();
    for f in 0..g.nprims() { if !inside[f] {continue;} let r=model::range(&g,f);
        for v in r.start+1..r.end.saturating_sub(1) { tris.push(([r.start,v,v+1],f)); }
    }
    if tris.is_empty() {return Ok(g);}
    let vp=&g.topology.vertex_point;
    let mut lat=Vec::new(); let mut locate=BTreeMap::new();
    for i in 0..=n {for j in 0..=n-i {locate.insert((i,j),lat.len());lat.push((i,j));}}
    let mut keys=BTreeSet::<Key>::new();
    let mut lattice=Vec::<Vec<(Option<Key>,usize,[f64;3])>>::new();
    let mut side_counts=BTreeMap::<(usize,usize),usize>::new();
    for (t,(corners,_)) in tris.iter().enumerate() {
        let cp=corners.map(|v|vp[v] as usize);
        for (a,b) in [(0,1),(1,2),(2,0)] {let edge=(cp[a].min(cp[b]),cp[a].max(cp[b]));*side_counts.entry(edge).or_default()+=1;}
        let mut row=Vec::new();
        for (m,&(i,j)) in lat.iter().enumerate() {
            let k=[n-i-j,i,j]; let wt=[k[0] as f64/n as f64,k[1] as f64/n as f64,k[2] as f64/n as f64];
            let nonzero:Vec<_>=(0..3).filter(|&c|k[c]!=0).collect();
            let (key,point)=match nonzero.len() {
                1 => (None,cp[nonzero[0]]),
                2 => {let a=nonzero[0];let b=nonzero[1];let lo=cp[a].min(cp[b]);let hi=cp[a].max(cp[b]);
                    let step=if cp[a]>cp[b]{k[a]}else{k[b]}; let key=(1,lo,hi,step);keys.insert(key);(Some(key),0)},
                _ => {let key=(2,t,m,0);keys.insert(key);(Some(key),0)},
            }; row.push((key,point,wt));
        } lattice.push(row);
    }
    if g.npoints+keys.len()>i32::MAX as usize {return Err(Error::new("interior detail exceeds point limit"));}
    let key_ids:BTreeMap<_,_>=keys.iter().enumerate().map(|(i,&key)|(key,g.npoints+i)).collect();
    let mut point_rows=Vec::with_capacity(g.npoints+keys.len());
    for i in 0..g.npoints {point_rows.push(([i,i,i],[1.,0.,0.]));}
    for &key in &keys {
        let (tri,entry)=if key.0==2 {(key.1,key.2)} else {
            lattice.iter().enumerate().find_map(|(t,row)|row.iter().position(|x|x.0==Some(key)).map(|m|(t,m))).unwrap()
        };
        let cp=tris[tri].0.map(|v|vp[v] as usize);
        point_rows.push((cp,lattice[tri][entry].2));
    }
    let point=interpolate(&g.point,&point_rows);
    let border:BTreeSet<_>=side_counts.iter().filter_map(|(&edge,&count)|(count==1).then_some(edge)).collect();
    let mut edge_new=BTreeMap::<(usize,usize),Vec<(usize,usize)>>::new();
    for &key in &keys {if key.0==1 && border.contains(&(key.1,key.2)) {
        edge_new.entry((key.1,key.2)).or_default().push((key.3,key_ids[&key]));
    }}
    let mut fixed=vec![false;g.npoints+keys.len()];
    for f in 0..g.nprims() {if !inside[f] {for v in model::range(&g,f){fixed[vp[v] as usize]=true;}}}
    for &(a,b) in &border {fixed[a]=true;fixed[b]=true;}
    for steps in edge_new.values() {for &(_,id) in steps {fixed[id]=true;}}
    let mut new_vp=Vec::new(); let mut sizes=Vec::new();let mut vertex_rows=Vec::<Blend>::new();let mut prim_rows=Vec::new();
    for f in 0..g.nprims(){if inside[f]{continue;}let r=model::range(&g,f);let mut size=0;
        for v in r.clone(){let next=if v+1<r.end{v+1}else{r.start};let a=vp[v] as usize;let b=vp[next] as usize;
            new_vp.push(a as i32);vertex_rows.push(([v,v,v],[1.,0.,0.]));size+=1;
            if let Some(steps)=edge_new.get(&(a.min(b),a.max(b))) {
                let sequence:Box<dyn Iterator<Item=&(usize,usize)>>=if a<b {Box::new(steps.iter())}else{Box::new(steps.iter().rev())};
                for &(step,id) in sequence {let fr=if a<b {step as f64/n as f64}else{1.-step as f64/n as f64};
                    new_vp.push(id as i32);vertex_rows.push(([v,next,next],[1.-fr,fr,0.]));size+=1;}
            }
        } sizes.push(size);prim_rows.push(f);
    }
    let kept_prims=prim_rows.len();let kept_corners=vertex_rows.len();
    for (t,(corners,f)) in tris.iter().enumerate(){
        for i in 0..n {for j in 0..n-i {
            let a=locate[&(i,j)];let b=locate[&(i+1,j)];let c=locate[&(i,j+1)];
            for tri in [[a,b,c]].into_iter().chain((i+j<n-1).then(||[b,locate[&(i+1,j+1)],c])) {
                for m in tri {let (key,old,wt)=lattice[t][m];new_vp.push(key.map_or(old,|k|key_ids[&k]) as i32);vertex_rows.push((*corners,wt));}
                sizes.push(3);prim_rows.push(*f);
            }
        }}
    }
    let mut vertex=interpolate(&g.vertex,&vertex_rows);
    // The render cap has a neutral binding. Give its generated corners a finite
    // placeholder UV so render consumers never see the invalid marker.
    for (name, attr) in &mut vertex {
        if matches!(name.as_str(),"N"|"Cd"|"P") || !matches!(attr.width,2|3) {continue;}
        if let Some(data)=attr.as_f32() {let mut values=data.to_vec();
            for x in &mut values[kept_corners*attr.width..] {if !x.is_finite(){*x=0.;}}
            *attr=Arc::new(Attr::f32(attr.width,values));
        }
    }
    let prim=model::gather(&g.prim,&prim_rows);
    let mut out=model::make(point,new_vp,&sizes,vertex,prim,g.detail.clone(),None)?;
    // Caps already carry a neutral slot and an invalid UV marker. Tessellated children
    // inherit that slot; retained outside corners keep their source UVs.
    let mut pos=out.p().to_vec();let seed=p.i("seed")?;
    for (i,xyz) in pos.chunks_exact_mut(3).enumerate(){if fixed[i]{continue;}
        let q=[xyz[0]*freq as f32,xyz[1]*freq as f32,xyz[2]*freq as f32];
        for c in 0..3 {xyz[c]+=(crate::noise::fbm3_one(q,seed+1013*c as i64,3,2.,0.5) as f64*amp) as f32;}
    }
    out.set_p(pos);
    let edges=topo::edges(&out.topology,out.npoints);let mut has_in=vec![false;edges.len()];let mut has_out=has_in.clone();
    for (f,_) in sizes.iter().enumerate(){for v in model::range(&out,f){let id=edges.of_vertex[v];if id>=0 {if f<kept_prims {has_out[id as usize]=true;}else{has_in[id as usize]=true;}}}}
    let seam:Vec<_>=has_in.iter().zip(&has_out).map(|(&a,&b)|a&&b).collect();
    let mut seam_points=vec![false;out.npoints];for (e,&yes) in seam.iter().enumerate(){if yes{seam_points[edges.points[2*e] as usize]=true;seam_points[edges.points[2*e+1] as usize]=true;}}
    model::outgroup(&mut out,p,"cutpoints","point",&seam_points)?;
    model::outgroup(&mut out,p,"cutedges","edge",&seam)?;
    out.validate()?;Ok(out)
}
