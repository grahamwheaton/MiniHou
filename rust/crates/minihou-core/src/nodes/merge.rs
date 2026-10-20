//! `merge` (`minihou/nodes/merge.py`): concatenate any number of geometries; attributes missing on an input are zero-filled.

use std::sync::Arc;

use super::{InputSpec, Inputs, NodeSpec};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, AttrTable, Geometry, Topology, MAX_ELEMENTS};
use crate::params::Params;

pub const MERGE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "merge",
    inputs: &[InputSpec { name: "geos", multi: true, optional: false }],
    params: &[],
    cook: merge,
};

/// Instance-object index reconciliation is separate from material slots.
const SPECIAL_DETAIL: &[&str] = &["instance_objects"];

pub(super) fn merge(inputs: &Inputs, _p: &Params) -> Result<Geometry> {
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
    let mut reconciled = super::material_tables::reconcile(&geos.iter().map(|g| (**g).clone()).collect::<Vec<_>>())?;
    super::model::promote_corner_uv_sets(&mut reconciled);
    let geos: Vec<_> = reconciled.iter().collect();
    let npoints: usize = geos.iter().map(|g| g.npoints).sum();
    let nverts: usize = geos.iter().map(|g| g.nvertices()).sum();
    if npoints > MAX_ELEMENTS || nverts > MAX_ELEMENTS {
        return Err(Error::new(format!("the merged geometry would have more than {MAX_ELEMENTS} points or vertices")));
    }
    let mut vp = Vec::with_capacity(nverts);
    let mut offs = Vec::with_capacity(geos.iter().map(|g| g.nprims()).sum::<usize>() + 1);
    let (mut pbase, mut vbase) = (0i32, 0i32);
    for g in &geos {
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
    super::packed::merge_packed(&geos, &mut out);
    for g in &geos {
        for (k, v) in &g.detail {
            out.detail.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    super::material_tables::merge_usd_sources(&reconciled, &mut out)?;
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
        let pieces: Vec<Attr> = parts.iter().map(|(t, n)| t.get(name).map(|a| (**a).clone()).unwrap_or_else(|| {
            if domain == "vertex" && matches!(r.width, 2 | 3) && !matches!(name.as_str(), "N" | "Cd" | "P") && r.as_f32().is_some() {
                Attr::f32(r.width, vec![f32::NAN; n * r.width])
            } else { r.zeros_like(*n) }
        })).collect();
        let data = match &r.data {
            AttrData::F32(_) => AttrData::F32(pieces.into_iter().flat_map(|a| match a.data { AttrData::F32(v) => v, _ => unreachable!() }).collect()),
            AttrData::I32(_) => AttrData::I32(pieces.into_iter().flat_map(|a| match a.data { AttrData::I32(v) => v, _ => unreachable!() }).collect()),
        };
        out.insert(name.clone(), Arc::new(Attr { width: r.width, data }));
    }
    Ok(out)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_identity_remaps_faces_and_keeps_uv_seam() {
        let mut a = Geometry::from_arrays(vec![0.,0.,0., 1.,0.,0., 0.,1.,0.], vec![0,1,2], vec![0,3], None).unwrap();
        a.vertex.insert("uv".into(), Arc::new(Attr::f32(2, vec![0.,0., 1.,0., 0.,1.])));
        a.vertex.insert("Lightmap".into(), Arc::new(Attr::f32(2, vec![0.,0., 1.,0., 0.,1.])));
        a.prim.insert("material_index".into(), Arc::new(Attr::i32(1, vec![1])));
        a.detail.insert("materials".into(), serde_json::json!(["", "Wood"]));
        a.detail.insert("material_bindings".into(), serde_json::json!({"version":1,"slots":[null,"doc:A"]}));
        let mut b = a.clone();
        b.prim.insert("material_index".into(), Arc::new(Attr::i32(1, vec![0])));
        b.detail.insert("materials".into(), serde_json::json!(["Wood", "Stone"]));
        b.detail.insert("material_bindings".into(), serde_json::json!({"version":1,"slots":["doc:B","doc:C"]}));
        b.vertex.insert("uv".into(), Arc::new(Attr::f32(2, vec![0.5,0.5, 1.,0., 0.,1.])));
        let out = merge(&[vec![Arc::new(a), Arc::new(b)]], &Params::resolve(&[], None).unwrap()).unwrap();
        assert_eq!(out.prim["material_index"].as_i32().unwrap(), &[1, 2]);
        assert_eq!(out.detail["material_bindings"]["slots"], serde_json::json!([null,"doc:A","doc:B","doc:C"]));
        assert_eq!(out.vertex["uv"].as_f32().unwrap()[6], 0.5);
        assert_eq!(out.vertex["Lightmap"].rows(), 6);
        out.validate().unwrap();
    }

    #[test]
    fn same_legacy_name_with_different_usd_sources_keeps_both() {
        let mut a = Geometry::from_arrays(vec![0.,0.,0., 1.,0.,0., 0.,1.,0.], vec![0,1,2], vec![0,3], None).unwrap();
        a.detail.insert("materials".into(), serde_json::json!(["Mat"]));
        a.detail.insert("usd_materials".into(), serde_json::json!({"Mat":"/A"}));
        let mut b = a.clone();
        b.detail.insert("usd_materials".into(), serde_json::json!({"Mat":"/B"}));
        let out = merge(&[vec![Arc::new(a), Arc::new(b)]], &Params::resolve(&[],None).unwrap()).unwrap();
        assert_eq!(out.prim["material_index"].as_i32().unwrap(), &[0,1]);
        assert_eq!(out.detail["usd_materials"]["Mat"], "/A");
        assert_eq!(out.detail["usd_materials"]["Mat (2)"], "/B");
    }

    #[test]
    fn merge_promotes_point_uv_when_other_input_uses_corners() {
        let mut a = Geometry::from_arrays(vec![0.,0.,0., 1.,0.,0., 0.,1.,0.], vec![0,1,2], vec![0,3], None).unwrap();
        a.vertex.insert("uv".into(), Arc::new(Attr::f32(2,vec![0.,0.,1.,0.,0.,1.])));
        let mut b = a.clone();
        b.vertex.remove("uv");
        b.point.insert("uv".into(), Arc::new(Attr::f32(2,vec![0.2,0.3,1.,0.,0.,1.])));
        let out = merge(&[vec![Arc::new(a),Arc::new(b)]],&Params::resolve(&[],None).unwrap()).unwrap();
        assert_eq!(&out.vertex["uv"].as_f32().unwrap()[6..8], &[0.2,0.3]);
    }

}
