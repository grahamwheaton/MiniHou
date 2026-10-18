//! Bend, twist, taper and rest-position capture from nodes/deform.py.
use std::sync::Arc;
use super::{InputSpec,Inputs,NodeSpec};
use crate::{error::{Error,Result},geometry::{Attr,Geometry},params::Params};
pub const CLEAN: NodeSpec = NodeSpec {
    name: "clean",
    inputs: super::GEO,
    params: crate::params!(
        "fuse_distance": 0.0;
        "remove_degenerate": true;
        "remove_unused_points": true;
    ),
    cook: clean,
};

fn drop_degenerate(g: &Geometry) -> Result<Geometry> {
    if g.nprims() == 0 {
        return Ok(g.clone());
    }
    let topo = &g.topology;
    let nprims = g.nprims();
    let vp = &topo.vertex_point;
    let offs = &topo.prim_offsets;

    let mut kept_vp = Vec::new();
    let mut kept_sizes = Vec::new();
    let mut kept_vert_indices = Vec::new();
    let mut kept_prims = Vec::new();
    let mut kept_closed = Vec::new();

    for prim in 0..nprims {
        let start = offs[prim] as usize;
        let end = offs[prim + 1] as usize;
        let count = end.saturating_sub(start);
        if count == 0 {
            continue;
        }
        let is_closed = topo.is_closed(prim);

        let mut corners = Vec::new();
        let mut vert_ix = Vec::new();
        for j in 0..count {
            let pt = vp[start + j];
            if j + 1 < count && pt == vp[start + j + 1] {
                continue;
            }
            corners.push(pt);
            vert_ix.push(start + j);
        }

        if is_closed && corners.len() > 1 && corners.first() == corners.last() {
            corners.pop();
            vert_ix.pop();
        }

        let min_corners = if is_closed { 3 } else { 2 };
        if corners.len() >= min_corners {
            kept_sizes.push(corners.len());
            kept_vp.extend(corners);
            kept_vert_indices.extend(vert_ix);
            kept_prims.push(prim);
            kept_closed.push(is_closed);
        }
    }

    let closed_opt = if topo.has_open() {
        Some(kept_closed)
    } else {
        None
    };

    super::model::make(
        g.point.clone(),
        kept_vp,
        &kept_sizes,
        super::model::gather(&g.vertex, &kept_vert_indices),
        super::model::gather(&g.prim, &kept_prims),
        g.detail.clone(),
        closed_opt,
    )
}

fn clean(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = CLEAN.one(inputs, 0)?;
    let fuse_distance = p.f("fuse_distance")?;
    let remove_degenerate = p.b("remove_degenerate")?;
    let remove_unused_points = p.b("remove_unused_points")?;

    let mut out = if fuse_distance > 0.0 {
        super::primitives::fuse_points(&geo, fuse_distance, None)?
    } else {
        (*geo).clone()
    };

    if remove_degenerate {
        out = drop_degenerate(&out)?;
    }
    if remove_unused_points {
        out = super::model::compact(out)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn clean_removes_degenerates_and_unused() {
        let p = vec![
            0.0, 0.0, 0.0,
            1.0, 0.0, 0.0,
            1.0, 1.0, 0.0,
            0.0, 1.0, 0.0,
            5.0, 5.0, 5.0,
        ];
        let g = Arc::new(Geometry::from_arrays(p, vec![0, 1, 1, 2, 3, 0, 1, 2, 2, 2], vec![0, 6, 8, 9, 10], None).unwrap());
        let params = Params::resolve(CLEAN.params, serde_json::json!({"fuse_distance": 0.0, "remove_degenerate": true, "remove_unused_points": true}).as_object()).unwrap();
        let out = clean(&[vec![g]], &params).unwrap();
        assert_eq!(out.npoints, 4);
        assert_eq!(out.nprims(), 1);
        assert_eq!(out.topology.vertex_point, vec![0, 1, 2, 3]);
    }
}
