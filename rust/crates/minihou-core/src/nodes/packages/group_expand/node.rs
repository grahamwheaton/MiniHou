use std::sync::Arc;
use crate::{nodes::{Inputs, NodeSpec, GEO}, error::{Error, Result}, geometry::{Attr, Geometry}, params::Params};
use crate::nodes::group::{mask, valid_name};

pub(super) const NODE:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None, cook_sidecar: None,name:"group_expand",inputs:GEO,params:crate::params!(
    "group":"group1";"name":"";"domain":"primitives";"mode":"expand";"steps":1;"connectivity":"points"),cook:expand};
fn expand(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=NODE.one(inputs,0)?;let dom=p.e("domain",&["primitives","points"])?;let dom=if dom=="points"{"point"}else{"prim"};
    let pattern=p.s("group")?;let name=p.s("name")?;let name=if name.is_empty(){pattern.clone()}else{name};valid_name(&name)?;
    let mut m=mask(&g,dom,&pattern)?;let contract=p.e("mode",&["expand","contract"])?=="contract";let edges=p.e("connectivity",&["points","edges"])?=="edges";
    let steps=p.i("steps")?;if !(1..=1000).contains(&steps){return Err(Error::new("steps must be 1..1000"));}
    let e=crate::topo::edges(&g.topology,g.npoints);let mut pv=Vec::new();for i in 0..g.nprims(){pv.extend(std::iter::repeat_n(i,(g.topology.prim_offsets[i+1]-g.topology.prim_offsets[i]) as usize));}
    for _ in 0..steps{
        let old:Vec<_>=m.iter().map(|&v|if contract{!v}else{v}).collect();let mut grown=old.clone();
        if dom=="point"{for pair in e.points.chunks_exact(2){let(a,b)=(pair[0] as usize,pair[1] as usize);grown[a]|=old[b];grown[b]|=old[a];}}
        else if edges{let mut hit=vec![false;e.len()];for(v,&i)in e.of_vertex.iter().enumerate(){if i>=0 && old[pv[v]]{hit[i as usize]=true;}}for(v,&i)in e.of_vertex.iter().enumerate(){if i>=0 && hit[i as usize]{grown[pv[v]]=true;}}}
        else{let mut hit=vec![false;g.npoints];for(v,&pt)in g.topology.vertex_point.iter().enumerate(){if old[pv[v]]{hit[pt as usize]=true;}}for(v,&pt)in g.topology.vertex_point.iter().enumerate(){if hit[pt as usize]{grown[pv[v]]=true;}}}
        m=grown.into_iter().map(|v|if contract{!v}else{v}).collect();
    }
    let mut out=(*g).clone();let key=format!("group_{name}");let a=Arc::new(Attr::i32(1,m.into_iter().map(i32::from).collect()));
    if dom=="point"{out.prim.remove(&key);out.point.insert(key,a);}else{out.point.remove(&key);out.prim.insert(key,a);}Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
