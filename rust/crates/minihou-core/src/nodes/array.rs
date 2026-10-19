//! `array` (Blender Geometry Nodes / Array modifier parity): `count` copies of the input, each shifted by
//! `relative_offset * bounds_size + constant_offset` from the previous one. With `merge_distance > 0` points of a copy that lie
//! within the distance of a point of the previous copy are welded (the copy seams), so a strip of quads becomes one mesh.
//! Attributes, groups and material tables are kept (copies are merged with the `merge` node). Limit: Blender's object offset
//! is not supported.
use std::collections::HashMap;
use std::sync::Arc;

use super::{merge::MERGE, model, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Geometry, Topology};
use crate::params::Params;

pub const ARRAY: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "array",
    inputs: GEO,
    params: crate::params!("count": 2; "relative_offset": [1., 0., 0.]; "constant_offset": [0., 0., 0.]; "merge_distance": 0.0),
    cook: array,
};

const MAX_COUNT: i64 = 10_000;

fn array(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = ARRAY.one(inputs, 0)?;
    let count = p.i("count")?;
    if !(1..=MAX_COUNT).contains(&count) {
        return Err(Error::new(format!("count must be 1..{MAX_COUNT}")));
    }
    let rel = p.v3("relative_offset")?;
    let cst = p.v3("constant_offset")?;
    let dist = p.f("merge_distance")?;
    if !rel.iter().chain(&cst).all(|x| x.is_finite()) || !dist.is_finite() {
        return Err(Error::new("array offsets and merge distance must be finite"));
    }
    if count == 1 || g.npoints == 0 {
        return Ok((*g).clone());
    }
    let pts = g.p();
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for q in pts.chunks_exact(3) {
        for a in 0..3 {
            let v = q[a] as f64;
            if v.is_finite() {
                lo[a] = lo[a].min(v);
                hi[a] = hi[a].max(v);
            }
        }
    }
    let size: Vec<f64> = (0..3).map(|a| if hi[a] >= lo[a] { hi[a] - lo[a] } else { 0. }).collect();
    let step: [f64; 3] = std::array::from_fn(|a| rel[a] * size[a] + cst[a]);
    let copies: Vec<Arc<Geometry>> = (0..count)
        .map(|k| {
            let mut c = (*g).clone();
            let o = step.map(|s| s * k as f64);
            c.set_p(pts.chunks_exact(3).flat_map(|q| [0, 1, 2].map(|a| (q[a] as f64 + o[a]) as f32)).collect());
            Arc::new(c)
        })
        .collect();
    let out = (MERGE.cook)(&[copies], p)?;
    if dist > 0. { weld_seams(out, g.npoints, dist) } else { Ok(out) }
}

/// Weld each point of copy k (points `k*n .. (k+1)*n`) onto the nearest point of copy k-1 within `dist`; chains collapse to the
/// first copy's point. Unused welded points are dropped; their attributes keep the surviving point's values.
fn weld_seams(g: Geometry, n: usize, dist: f64) -> Result<Geometry> {
    let p = g.p();
    let pos = |i: usize| [p[i * 3] as f64, p[i * 3 + 1] as f64, p[i * 3 + 2] as f64];
    let cell = |q: [f64; 3]| q.map(|x| (x / dist).floor() as i64);
    let copies = g.npoints / n;
    let mut target: Vec<usize> = (0..g.npoints).collect();
    for k in 1..copies {
        let mut grid: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
        for i in (k - 1) * n..k * n {
            grid.entry(cell(pos(i))).or_default().push(i);
        }
        for i in k * n..(k + 1) * n {
            let q = pos(i);
            let c = cell(q);
            let mut best: Option<(f64, usize)> = None;
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let Some(v) = grid.get(&[c[0] + dx, c[1] + dy, c[2] + dz]) else { continue };
                        for &j in v {
                            let r = pos(j);
                            let d2 = (0..3).map(|a| (q[a] - r[a]).powi(2)).sum::<f64>();
                            if d2 <= dist * dist && best.is_none_or(|b| d2 < b.0) {
                                best = Some((d2, j));
                            }
                        }
                    }
                }
            }
            if let Some((_, j)) = best {
                target[i] = target[j];
            }
        }
    }
    let mut keep = Vec::new();
    let mut remap = vec![0i32; g.npoints];
    for i in 0..g.npoints {
        if target[i] == i {
            remap[i] = keep.len() as i32;
            keep.push(i);
        }
    }
    for i in 0..g.npoints {
        remap[i] = remap[target[i]];
    }
    let out = Geometry { packed: Default::default(), edge: Default::default(),
        npoints: keep.len(),
        topology: Arc::new(Topology {
            vertex_point: g.topology.vertex_point.iter().map(|&v| remap[v as usize]).collect(),
            prim_offsets: g.topology.prim_offsets.clone(),
            closed: g.topology.closed.clone(),
        }),
        point: model::gather(&g.point, &keep),
        vertex: g.vertex,
        prim: g.prim,
        detail: g.detail,
    };
    out.validate()?;
    Ok(out)
}

use super::Inputs;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Attr;
    use serde_json::json;

    /// A unit quad in XY with a point colour, a prim material index and a prim group.
    fn quad() -> Geometry {
        let mut g = Geometry::from_arrays(vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.], vec![0, 1, 2, 3], vec![0, 4], None)
            .unwrap();
        g.point.insert("Cd".into(), Arc::new(Attr::f32(3, [1., 0.5, 0.].repeat(4))));
        g.prim.insert("material_index".into(), Arc::new(Attr::i32(1, vec![2])));
        g.prim.insert("group_top".into(), Arc::new(Attr::i32(1, vec![1])));
        g
    }
    fn cook(g: Geometry, v: serde_json::Value) -> Geometry {
        let p = Params::resolve(ARRAY.params, v.as_object()).unwrap();
        let out = (ARRAY.cook)(&[vec![Arc::new(g)]], &p).unwrap();
        out.validate().unwrap();
        assert!(out.p().iter().all(|x| x.is_finite()));
        out
    }
    fn xs(g: &Geometry) -> Vec<f32> {
        g.p().chunks_exact(3).map(|c| c[0]).collect()
    }

    #[test]
    fn array_count_one_is_identity() {
        let g = quad();
        let out = cook(g.clone(), json!({"count": 1, "constant_offset": [5., 0., 0.]}));
        assert_eq!(out.p(), g.p());
        assert_eq!(out.nprims(), 1);
    }

    #[test]
    fn array_three_copies_along_x() {
        let out = cook(quad(), json!({"count": 3, "relative_offset": [0., 0., 0.], "constant_offset": [2., 0., 0.]}));
        assert_eq!((out.npoints, out.nprims()), (12, 3));
        assert_eq!(xs(&out)[8..], [4., 5., 5., 4.]);
    }

    #[test]
    fn array_relative_offset_uses_bounds() {
        let mut g = quad();
        g.set_p(g.p().iter().enumerate().map(|(i, &x)| if i % 3 == 0 { x * 3. } else { x }).collect());
        let out = cook(g, json!({"count": 2, "relative_offset": [1., 0., 0.], "constant_offset": [0.5, 0., 0.]}));
        assert_eq!(xs(&out)[4..], [3.5, 6.5, 6.5, 3.5]);
    }

    #[test]
    fn array_merges_seams_within_distance() {
        let out = cook(quad(), json!({"count": 3, "merge_distance": 0.001}));
        assert_eq!((out.npoints, out.nprims()), (8, 3));
        let unmerged = cook(quad(), json!({"count": 3}));
        assert_eq!(unmerged.npoints, 12);
        // A gap larger than the distance is not welded.
        let gap = cook(quad(), json!({"count": 2, "constant_offset": [0.1, 0., 0.], "merge_distance": 0.01}));
        assert_eq!(gap.npoints, 8);
    }

    #[test]
    fn array_preserves_attributes_groups_and_materials() {
        let out = cook(quad(), json!({"count": 3, "merge_distance": 0.001}));
        assert_eq!(out.point["Cd"].width, 3);
        assert_eq!(out.point["Cd"].data, Attr::f32(3, [1., 0.5, 0.].repeat(8)).data);
        assert_eq!(out.prim["material_index"].data, Attr::i32(1, vec![2, 2, 2]).data);
        assert_eq!(out.prim["group_top"].data, Attr::i32(1, vec![1, 1, 1]).data);
    }

    #[test]
    fn array_empty_input_and_bad_params() {
        let e = Geometry::from_arrays(vec![], vec![], vec![0], None).unwrap();
        assert_eq!(cook(e, json!({"count": 4})).npoints, 0);
        let p = Params::resolve(ARRAY.params, json!({"count": 0}).as_object()).unwrap();
        assert!((ARRAY.cook)(&[vec![Arc::new(quad())]], &p).is_err());
    }

    #[test]
    fn array_is_deterministic() {
        let v = json!({"count": 5, "relative_offset": [0.5, 0.2, 0.], "merge_distance": 0.3});
        let (a, b) = (cook(quad(), v.clone()), cook(quad(), v));
        assert_eq!(a.p(), b.p());
        assert_eq!(a.topology.vertex_point, b.topology.vertex_point);
    }
}
