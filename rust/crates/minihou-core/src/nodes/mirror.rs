//! Mirror SOP (Houdini parity). The defaults reproduce minihou/nodes/model.py `mirror` exactly; the Rust-only parameters
//! add Houdini's Group/Operation, Distance (here `offset`, since `distance` is the historical seam tolerance), Reverse,
//! Only Consolidate Unshared Edges, and a Blender-style `cut` (bisect: drop the far side before mirroring).
use super::{
    Inputs,
    model::{
        CLIP, MIRROR, add, compact, concat, dot, gather, len, make, mul, outgroup, positions, prim_mask, range, sub,
    },
};
use crate::{
    error::Result,
    geometry::{Attr, AttrData, AttrTable, Geometry},
    params::Params,
};
use std::{
    collections::HashMap,
    sync::Arc,
};

/// Width-3 float attributes that are directions, so they reflect with the plane. Positions (P) are handled separately;
/// colours, UVs and other data stay unchanged.
pub const VECTOR_ATTRS: &[&str] = &["N", "v", "up", "accel", "force", "tangentu", "tangentv"];

fn reflect(t: &AttrTable, n: [f64; 3]) -> AttrTable {
    let mut out = t.clone();
    for name in VECTOR_ATTRS {
        let Some(a) = t.get(*name) else { continue };
        if a.width != 3 {
            continue;
        }
        if let AttrData::F32(v) = &a.data {
            let b = v
                .chunks_exact(3)
                .flat_map(|q| {
                    let q = [q[0] as f64, q[1] as f64, q[2] as f64];
                    sub(q, mul(n, 2. * dot(q, n))).map(|x| x as f32)
                })
                .collect();
            out.insert((*name).into(), Arc::new(Attr::f32(3, b)));
        }
    }
    out
}

/// The effective plane: unit normal (flipped by `reverse`) and origin moved `offset` along the direction.
pub fn plane(origin: [f64; 3], dir: [f64; 3], offset: f64, reverse: bool) -> ([f64; 3], [f64; 3]) {
    let l = len(dir);
    let d = if l < 1e-12 { [1., 0., 0.] } else { mul(dir, 1. / l) };
    let o = add(origin, mul(d, offset));
    (o, if reverse { mul(d, -1.) } else { d })
}

pub(super) fn mirror(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g0 = MIRROR.one(inputs, 0)?;
    let (o, n) = plane(p.v3("origin")?, p.v3("normal")?, p.f("offset")?, p.b("reverse")?);
    let cut = p.b("cut")?;
    let g: Geometry = if cut && g0.nprims() > 0 {
        let mut m = serde_json::Map::new();
        m.insert("origin".into(), serde_json::json!(o));
        m.insert("normal".into(), serde_json::json!(n));
        m.insert("keep".into(), "above".into());
        m.insert("outgrp_above".into(), false.into());
        let cp = Params::resolve(CLIP.params, Some(&m))?;
        (CLIP.cook)(&[vec![g0.clone()]], &cp)?
    } else {
        (*g0).clone()
    };
    let group = p.s("group")?;
    let selected_only = p.e("operation", &["mirror_all", "mirror_selected"])? == "mirror_selected";
    let sel: Vec<bool> = if selected_only && !group.trim().is_empty() {
        prim_mask(&g, &group)?
    } else {
        vec![true; g.nprims()]
    };
    let all = sel.iter().all(|&s| s);
    let pos = positions(&g);
    let d: Vec<_> = pos.iter().map(|&q| dot(sub(q, o), n)).collect();
    let mut mp = reflect(&g.point, n);
    mp.insert(
        "P".into(),
        Arc::new(Attr::f32(
            3,
            pos.iter().zip(&d).flat_map(|(&q, &d)| sub(q, mul(n, 2. * d)).map(|x| x as f32)).collect(),
        )),
    );
    let mprims: Vec<usize> = (0..g.nprims()).filter(|&i| sel[i]).collect();
    let rev: Vec<_> = mprims.iter().flat_map(|&i| range(&g, i).rev()).collect();
    let mv = gather(&reflect(&g.vertex, n), &rev);
    let mpr = gather(&reflect(&g.prim, n), &mprims);
    let keep = p.b("keep_original")?;
    let consolidate = p.b("consolidate_seam")?;
    let tol = p.f("distance")?;
    let unshared_only = p.b("unshared_only")?;
    let boundary = if unshared_only { unshared_points(&g, &sel, |i| d[i].abs() <= tol) } else { vec![true; g.npoints] };
    let seam: Vec<_> =
        d.iter().enumerate().map(|(i, d)| keep && consolidate && d.abs() <= tol && boundary[i]).collect();
    let osizes: Vec<_> = (0..g.nprims()).map(|i| range(&g, i).len()).collect();
    let msizes: Vec<_> = mprims.iter().map(|&i| osizes[i]).collect();
    let mut sizes = Vec::new();
    let mut vp = Vec::new();
    if keep {
        vp.extend(&g.topology.vertex_point);
        sizes.extend(&osizes);
    }
    sizes.extend(&msizes);
    vp.extend(rev.iter().map(|&v| {
        let q = g.topology.vertex_point[v] as usize;
        if !keep || seam[q] { q as i32 } else { (q + g.npoints) as i32 }
    }));
    let closed = g.topology.closed.as_ref().map(|c| {
        let mc = mprims.iter().map(|&i| c[i]);
        if keep { c.iter().copied().chain(mc).collect() } else { mc.collect() }
    });
    let mut out = make(
        if keep { concat(&g.point, &mp) } else { mp },
        vp,
        &sizes,
        if keep { concat(&g.vertex, &mv) } else { mv },
        if keep { concat(&g.prim, &mpr) } else { mpr },
        g.detail.clone(),
        closed,
    )?;
    let first = if keep { g.nprims() } else { 0 };
    outgroup(&mut out, p, "mirrored", "prim", &(0..sizes.len()).map(|i| i >= first).collect::<Vec<_>>())?;
    let np = out.npoints;
    outgroup(
        &mut out,
        p,
        "mirroredpoints",
        "point",
        &(0..np).map(|i| !keep || i >= g.npoints || seam[i]).collect::<Vec<_>>(),
    )?;
    outgroup(
        &mut out,
        p,
        "seam",
        "point",
        &(0..np).map(|i| keep && i < g.npoints && seam[i]).collect::<Vec<_>>(),
    )?;
    if keep || !all { compact(out) } else { Ok(out) }
}

/// Points of in-plane edges used by exactly one (selected) primitive: Houdini's "Only Consolidate Unshared Edges".
fn unshared_points(g: &Geometry, sel: &[bool], on: impl Fn(usize) -> bool) -> Vec<bool> {
    let mut count: HashMap<(i32, i32), u32> = HashMap::new();
    let vp = &g.topology.vertex_point;
    let closed = |i: usize| g.topology.closed.as_ref().map_or(true, |c| c[i]);
    let mut edges = Vec::new();
    for i in 0..g.nprims() {
        if !sel[i] {
            continue;
        }
        let r = range(g, i);
        let k = r.len();
        if k < 2 {
            continue;
        }
        let m = if closed(i) { k } else { k - 1 };
        for j in 0..m {
            let a = vp[r.start + j];
            let b = vp[r.start + (j + 1) % k];
            let e = (a.min(b), a.max(b));
            *count.entry(e).or_default() += 1;
            edges.push(e);
        }
    }
    let mut out = vec![false; g.npoints];
    for e in edges {
        if count[&e] == 1 && on(e.0 as usize) && on(e.1 as usize) {
            out[e.0 as usize] = true;
            out[e.1 as usize] = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Two quads side by side in the XY plane spanning x in [0, 2]; their shared edge is at x = 1, the left edge at x = 0.
    fn strip() -> Geometry {
        let mut g = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0., 2., 0., 0., 2., 1., 0.],
            vec![0, 1, 2, 3, 1, 4, 5, 2],
            vec![0, 4, 8],
            None,
        )
        .unwrap();
        g.point.insert("N".into(), Arc::new(Attr::f32(3, [1., 0., 1.].repeat(6))));
        g.point.insert("Cd".into(), Arc::new(Attr::f32(3, [1., 0.5, 0.].repeat(6))));
        g.vertex.insert("uv".into(), Arc::new(Attr::f32(3, (0..8).flat_map(|i| [i as f32, 0., 0.]).collect())));
        g.prim.insert("material_index".into(), Arc::new(Attr::i32(1, vec![3, 7])));
        g.prim.insert("group_left".into(), Arc::new(Attr::i32(1, vec![1, 0])));
        g
    }
    fn cook(g: Geometry, v: serde_json::Value) -> Geometry {
        let p = Params::resolve(MIRROR.params, v.as_object()).unwrap();
        let out = (MIRROR.cook)(&[vec![Arc::new(g)]], &p).unwrap();
        out.validate().unwrap();
        out
    }
    fn xs(g: &Geometry) -> Vec<f32> {
        g.p().chunks_exact(3).map(|c| c[0]).collect()
    }
    fn face_normal_z(g: &Geometry, prim: usize) -> f64 {
        let pos = positions(g);
        let r = range(g, prim);
        let v: Vec<_> = r.map(|v| pos[g.topology.vertex_point[v] as usize]).collect();
        let (a, b) = (sub(v[1], v[0]), sub(v[2], v[0]));
        a[0] * b[1] - a[1] * b[0]
    }

    #[test]
    fn mirror_defaults_weld_the_seam_and_flip_winding() {
        let out = cook(strip(), json!({}));
        assert_eq!(out.nprims(), 4);
        // Two points on x = 0 welded: 6 originals + 4 copies.
        assert_eq!(out.npoints, 10);
        assert!(face_normal_z(&out, 0) > 0. && face_normal_z(&out, 2) > 0., "winding flipped so faces keep +Z");
    }

    #[test]
    fn mirror_attributes_reflect_vectors_only() {
        let out = cook(strip(), json!({"keep_original": false}));
        let AttrData::F32(n) = &out.point["N"].data else { panic!() };
        assert_eq!(&n[..3], &[-1., 0., 1.]);
        let AttrData::F32(cd) = &out.point["Cd"].data else { panic!() };
        assert_eq!(&cd[..3], &[1., 0.5, 0.]);
        let AttrData::F32(uv) = &out.vertex["uv"].data else { panic!() };
        // Reversed corners keep their own UVs: prim 0's corners 3,2,1,0.
        assert_eq!(uv[0], 3.);
        assert_eq!(out.prim["material_index"].data, AttrData::I32(vec![3, 7]));
        assert_eq!(out.prim["group_left"].data, AttrData::I32(vec![1, 0]));
    }

    #[test]
    fn mirror_reverse_and_cut_keep_the_chosen_side() {
        // Plane at x = 1: +side keeps the right quad and mirrors it to the left; reverse keeps the left quad.
        let fwd = cook(strip(), json!({"origin": [1., 0., 0.], "cut": true}));
        assert_eq!(fwd.nprims(), 2);
        assert!(xs(&fwd).iter().all(|&x| (0. ..=2.).contains(&x)));
        let rev = cook(strip(), json!({"origin": [1., 0., 0.], "cut": true, "reverse": true}));
        assert_eq!(rev.nprims(), 2);
        assert!(xs(&rev).iter().all(|&x| (0. ..=2.).contains(&x)));
        // Different source quads survive: the kept original is prim 0 in both outputs.
        let mid = |g: &Geometry| range(g, 0).map(|v| g.p()[g.topology.vertex_point[v] as usize * 3]).sum::<f32>();
        assert!(mid(&fwd) > 4. - 1e-4 && mid(&rev) < 4. + 1e-4 && mid(&fwd) != mid(&rev));
        assert!(face_normal_z(&fwd, 1) > 0. && face_normal_z(&rev, 1) > 0.);
        assert_eq!(fwd.npoints, 6, "seam at x = 1 welded");
    }

    #[test]
    fn mirror_offset_moves_the_plane_along_direction() {
        let out = cook(strip(), json!({"offset": -1.0, "keep_original": false}));
        // Plane at x = -1: x -> -2 - x.
        let x = xs(&out);
        assert!(x.iter().all(|&v| (-4. ..=-2.).contains(&v)), "{x:?}");
    }

    #[test]
    fn mirror_unshared_only_skips_interior_points() {
        // Plane at x = 1 without a cut: points 1 and 2 lie on the shared (interior) edge.
        let all = cook(strip(), json!({"origin": [1., 0., 0.]}));
        let only = cook(strip(), json!({"origin": [1., 0., 0.], "unshared_only": true}));
        assert_eq!(all.npoints, 10);
        assert_eq!(only.npoints, 12);
        // A boundary point on the plane is still welded.
        let edge = cook(strip(), json!({"unshared_only": true}));
        assert_eq!(edge.npoints, 10);
    }

    #[test]
    fn mirror_selected_and_output_group() {
        let out = cook(
            strip(),
            json!({"operation": "mirror_selected", "group": "left", "outgrp_mirrored": true}),
        );
        assert_eq!(out.nprims(), 3);
        assert_eq!(out.prim["group_mirrored"].data, AttrData::I32(vec![0, 0, 1]));
        assert_eq!(out.prim["material_index"].data, AttrData::I32(vec![3, 7, 3]));
        // Group is ignored by Mirror All.
        assert_eq!(cook(strip(), json!({"group": "left"})).nprims(), 4);
    }

    #[test]
    fn mirror_plane_helper() {
        let (o, n) = plane([1., 2., 3.], [0., 0., 2.], 0.5, true);
        assert_eq!(o, [1., 2., 3.5]);
        assert_eq!(n, [0., 0., -1.]);
    }
}
