use std::sync::Arc;
use crate::{nodes::{Inputs, NodeSpec, GEO}, error::Result, geometry::{AttrData, AttrTable, Geometry}, params::Params};
use crate::nodes::model::{prim_mask, range, rebuild, corners};
pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "reverse",
    inputs: GEO,
    params: crate::params!("group":"";"reverse_normals":true),
    cook: reverse,
};
fn negate(t: &mut AttrTable, mask: &[bool]) {
    if let Some(a) = t.get("N") {
        let mut a = (**a).clone();
        match &mut a.data {
            AttrData::F32(v) => {
                for (i, row) in v.chunks_mut(a.width).enumerate() {
                    if mask[i] {
                        for x in row {
                            *x = -*x;
                        }
                    }
                }
            }
            AttrData::I32(v) => {
                for (i, row) in v.chunks_mut(a.width).enumerate() {
                    if mask[i] {
                        for x in row {
                            *x = -*x;
                        }
                    }
                }
            }
        }
        t.insert("N".into(), Arc::new(a));
    }
}
fn reverse(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let sel = prim_mask(&g, &p.s("group")?)?;
    let mut faces = Vec::new();
    for i in 0..g.nprims() {
        let mut f: Vec<_> = range(&g, i).collect();
        if sel[i] {
            f.reverse();
        }
        faces.push(f);
    }
    let mut out = rebuild(
        &g,
        &faces,
        &(0..g.nprims()).collect::<Vec<_>>(),
        g.topology.closed.clone(),
    )?;
    if p.b("reverse_normals")? {
        let (pv, _, _) = corners(&out);
        negate(
            &mut out.vertex,
            &pv.iter().map(|&i| sel[i]).collect::<Vec<_>>(),
        );
        negate(&mut out.prim, &sel);
        let mut ps = vec![false; g.npoints];
        for i in 0..g.nprims() {
            if sel[i] {
                for v in range(&g, i) {
                    ps[g.topology.vertex_point[v] as usize] = true;
                }
            }
        }
        negate(&mut out.point, &ps);
    }
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
