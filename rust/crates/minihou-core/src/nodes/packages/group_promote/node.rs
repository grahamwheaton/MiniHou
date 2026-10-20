use std::sync::Arc;
use crate::{nodes::{Inputs, NodeSpec, GEO}, error::{Error, Result}, geometry::{Attr, Geometry}, params::Params};
use crate::nodes::group::flags;

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None, name: "group_promote", inputs: GEO,
    params: crate::params!("name": "group1"; "to": "points"), cook: promote };
fn promote(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs,0)?;
    let pattern = p.s("name")?;
    let to = p.e("to", &["points","primitives"])?;
    let mut out = (*g).clone();
    let source = if to == "points" { &g.prim } else { &g.point };
    let names: Vec<_> = source.keys().filter_map(|n| n.strip_prefix("group_"))
        .filter(|n| crate::nodes::attribs::matches(&pattern,n)).map(str::to_owned).collect();
    if names.is_empty() { return Err(Error::new(format!("no source group matching {pattern:?}"))); }
    for name in names {
    let key = format!("group_{name}");
    if to == "points" {
        let a = g.prim.get(&key).ok_or_else(|| Error::new(format!("no primitive group {name:?}")))?;
        let f = flags(a); let mut m = vec![0; g.npoints];
        for (i,&b) in f.iter().enumerate() { if b {
            for &v in &g.topology.vertex_point[g.topology.prim_offsets[i] as usize..g.topology.prim_offsets[i+1] as usize] { m[v as usize] = 1; }
        } }
        out.prim.remove(&key); out.point.insert(key, Arc::new(Attr::i32(1,m)));
    } else {
        let a = g.point.get(&key).ok_or_else(|| Error::new(format!("no point group {name:?}")))?;
        let f = flags(a);
        let m = g.topology.prim_offsets.windows(2).map(|r| {
            let ids = &g.topology.vertex_point[r[0] as usize..r[1] as usize];
            (!ids.is_empty() && ids.iter().all(|&i| f[i as usize])) as i32
        }).collect();
        out.point.remove(&key); out.prim.insert(key, Arc::new(Attr::i32(1,m)));
    }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
