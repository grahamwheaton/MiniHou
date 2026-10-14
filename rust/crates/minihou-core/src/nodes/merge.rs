//! `merge` (`minihou/nodes/merge.py`): concatenate any number of geometries; attributes missing on an input are zero-filled.

use std::sync::Arc;

use super::{InputSpec, Inputs, NodeSpec};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, AttrTable, Geometry, Topology, MAX_ELEMENTS};
use crate::params::Params;

pub const MERGE: NodeSpec = NodeSpec {
    name: "merge",
    inputs: &[InputSpec { name: "geos", multi: true, optional: false }],
    params: &[],
    cook: merge,
};

/// Detail entries the Python merge treats specially (material lists, instance objects); not ported yet.
const SPECIAL_DETAIL: &[&str] = &["materials", "usd_materials", "instance_objects"];

fn merge(inputs: &Inputs, _p: &Params) -> Result<Geometry> {
    let geos = &inputs[0];
    if geos.is_empty() {
        return Err(Error::new("input 'geos' is not connected"));
    }
    if geos.len() == 1 {
        return Ok((*geos[0]).clone());
    }
    if let Some(k) = geos.iter().flat_map(|g| g.detail.keys()).find(|k| SPECIAL_DETAIL.contains(&k.as_str())) {
        return Err(Error::new(format!("merging geometry with detail {k:?} (materials / instances) is not supported by the Rust backend yet")));
    }
    let npoints: usize = geos.iter().map(|g| g.npoints).sum();
    let nverts: usize = geos.iter().map(|g| g.nvertices()).sum();
    if npoints > MAX_ELEMENTS || nverts > MAX_ELEMENTS {
        return Err(Error::new(format!("the merged geometry would have more than {MAX_ELEMENTS} points or vertices")));
    }
    let mut vp = Vec::with_capacity(nverts);
    let mut offs = Vec::with_capacity(geos.iter().map(|g| g.nprims()).sum::<usize>() + 1);
    let (mut pbase, mut vbase) = (0i32, 0i32);
    for g in geos {
        let t = &g.topology;
        vp.extend(t.vertex_point.iter().map(|&v| v + pbase));
        offs.extend(t.prim_offsets[..t.nprims()].iter().map(|&o| o + vbase));
        pbase += g.npoints as i32;
        vbase += g.nvertices() as i32;
    }
    offs.push(vbase);
    let closed = geos.iter().any(|g| g.topology.has_open()).then(|| geos.iter().flat_map(|g| g.topology.closed_mask()).collect());
    let mut out = Geometry { npoints, topology: Arc::new(Topology { vertex_point: vp, prim_offsets: offs, closed }), ..Default::default() };
    out.point = concat_tables(geos.iter().map(|g| (&g.point, g.npoints)).collect(), "point")?;
    out.vertex = concat_tables(geos.iter().map(|g| (&g.vertex, g.nvertices())).collect(), "vertex")?;
    out.prim = concat_tables(geos.iter().map(|g| (&g.prim, g.nprims())).collect(), "prim")?;
    for g in geos {
        for (k, v) in &g.detail {
            out.detail.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    Ok(out)
}

fn concat_tables(parts: Vec<(&AttrTable, usize)>, domain: &str) -> Result<AttrTable> {
    let mut names: Vec<&String> = Vec::new();
    for (t, _) in &parts {
        for n in t.keys() {
            if !names.contains(&n) {
                names.push(n);
            }
        }
    }
    let mut out = AttrTable::new();
    for name in names {
        let have: Vec<&Arc<Attr>> = parts.iter().filter_map(|(t, _)| t.get(name)).collect();
        let r = have[0];
        if have.iter().any(|a| a.width != r.width) {
            return Err(Error::new(format!("attribute {name:?} has mismatched shapes across inputs")));
        }
        if have.iter().any(|a| a.dtype() != r.dtype()) {
            return Err(Error::new(format!(
                "{domain} attribute {name:?} is float on one input and integer on another: not supported by the Rust backend yet"
            )));
        }
        let pieces: Vec<Attr> = parts.iter().map(|(t, n)| t.get(name).map(|a| (**a).clone()).unwrap_or_else(|| r.zeros_like(*n))).collect();
        let data = match &r.data {
            AttrData::F32(_) => AttrData::F32(pieces.into_iter().flat_map(|a| match a.data { AttrData::F32(v) => v, _ => unreachable!() }).collect()),
            AttrData::I32(_) => AttrData::I32(pieces.into_iter().flat_map(|a| match a.data { AttrData::I32(v) => v, _ => unreachable!() }).collect()),
        };
        out.insert(name.clone(), Arc::new(Attr { width: r.width, data }));
    }
    Ok(out)
}
