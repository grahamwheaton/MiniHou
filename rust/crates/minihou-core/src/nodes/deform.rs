//! Bend, twist, taper and rest-position capture from nodes/deform.py.
use std::sync::Arc;
use super::{InputSpec,Inputs,NodeSpec};
use crate::{error::{Error,Result},geometry::{Attr,Geometry},params::Params};
fn dot(a:[f64;3],b:[f64;3])->f64{(0..3).map(|i|a[i]*b[i]).sum()}
fn cross(a:[f64;3],b:[f64;3])->[f64;3]{[a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]]}
pub const POINT_RELAX: NodeSpec = NodeSpec {
    name: "point_relax",
    inputs: &[
        InputSpec {
            name: "points",
            multi: false,
            optional: false,
        },
        InputSpec {
            name: "surface",
            multi: false,
            optional: true,
        },
    ],
    params: crate::params!(
        "iterations": 10;
        "radius": 0.0;
        "update_normals": true;
    ),
    cook: point_relax,
};

pub(super) fn closest_point_triangle(p: [f64; 3], tri: &[[f64; 3]; 3]) -> [f64; 3] {
    let [a, b, c] = *tri;
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let ap = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = [p[0] - b[0], p[1] - b[1], p[2] - b[2]];
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return [a[0] + v * ab[0], a[1] + v * ab[1], a[2] + v * ab[2]];
    }
    let cp = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return [a[0] + w * ac[0], a[1] + w * ac[1], a[2] + w * ac[2]];
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        let bc = [c[0] - b[0], c[1] - b[1], c[2] - b[2]];
        return [b[0] + w * bc[0], b[1] + w * bc[1], b[2] + w * bc[2]];
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    [
        a[0] + ab[0] * v + ac[0] * w,
        a[1] + ab[1] * v + ac[1] * w,
        a[2] + ab[2] * v + ac[2] * w,
    ]
}

fn point_relax(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let points = POINT_RELAX.one(inputs, 0)?;
    let surface = inputs.get(1).and_then(|t| t.first());
    let iterations = p.i("iterations")?.max(0) as usize;
    let radius = p.f("radius")?;
    let update_normals = p.b("update_normals")?;

    let n = points.npoints;
    if n < 2 || iterations == 0 {
        return Ok((*points).clone());
    }

    let mut P: Vec<[f64; 3]> = points
        .p()
        .chunks_exact(3)
        .map(|c| [c[0] as f64, c[1] as f64, c[2] as f64])
        .collect();

    let (surface_tris, r) = if let Some(surf) = surface {
        let p_slice = surf.p();
        let topo = &surf.topology;
        let vp = &topo.vertex_point;
        let offs = &topo.prim_offsets;
        let mut tris = Vec::new();
        let mut area_sum = 0.0;
        for prim in 0..surf.nprims() {
            let start = offs[prim] as usize;
            let end = offs[prim + 1] as usize;
            let k = end.saturating_sub(start);
            if k < 3 {
                continue;
            }
            let a = [
                p_slice[vp[start] as usize * 3] as f64,
                p_slice[vp[start] as usize * 3 + 1] as f64,
                p_slice[vp[start] as usize * 3 + 2] as f64,
            ];
            for j in 0..(k - 2) {
                let b = [
                    p_slice[vp[start + j + 1] as usize * 3] as f64,
                    p_slice[vp[start + j + 1] as usize * 3 + 1] as f64,
                    p_slice[vp[start + j + 1] as usize * 3 + 2] as f64,
                ];
                let c = [
                    p_slice[vp[start + j + 2] as usize * 3] as f64,
                    p_slice[vp[start + j + 2] as usize * 3 + 1] as f64,
                    p_slice[vp[start + j + 2] as usize * 3 + 2] as f64,
                ];
                let cr = cross(
                    [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
                    [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
                );
                area_sum += 0.5 * dot(cr, cr).sqrt();
                tris.push([a, b, c]);
            }
        }
        if tris.is_empty() {
            return Ok((*points).clone());
        }
        let calc_r = if radius > 0.0 {
            radius
        } else {
            (area_sum / n as f64).sqrt() * 1.07
        };
        (Some(tris), calc_r)
    } else {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for pt in &P {
            for k in 0..3 {
                lo[k] = lo[k].min(pt[k]);
                hi[k] = hi[k].max(pt[k]);
            }
        }
        let mut ext = [
            (hi[0] - lo[0]).max(1e-9),
            (hi[1] - lo[1]).max(1e-9),
            (hi[2] - lo[2]).max(1e-9),
        ];
        ext.sort_by(|a, b| a.total_cmp(b));
        let calc_r = if radius > 0.0 {
            radius
        } else {
            ((ext[1] * ext[2]) / n as f64).sqrt() * 1.07
        };
        (None, calc_r)
    };

    let mut last_tri_indices = vec![0usize; n];

    for _ in 0..iterations {
        let r_sq = r * r;
        let mut disp = vec![[0.0f64; 3]; n];
        for i in 0..n {
            for j in (i + 1)..n {
                let diff = [P[i][0] - P[j][0], P[i][1] - P[j][1], P[i][2] - P[j][2]];
                let d_sq = dot(diff, diff);
                if d_sq < r_sq && d_sq > 0.0 {
                    let d = d_sq.sqrt();
                    let factor = (r - d) / d.max(1e-12) * 0.25;
                    for k in 0..3 {
                        let push = factor * diff[k];
                        disp[i][k] += push;
                        disp[j][k] -= push;
                    }
                }
            }
        }
        for i in 0..n {
            for k in 0..3 {
                P[i][k] += disp[i][k];
            }
        }

        if let Some(ref tris) = surface_tris {
            for i in 0..n {
                let pt = P[i];
                let mut best_dist_sq = f64::INFINITY;
                let mut best_pt = pt;
                let mut best_tri = 0;
                for (ti, tri) in tris.iter().enumerate() {
                    let cp = closest_point_triangle(pt, tri);
                    let diff = [pt[0] - cp[0], pt[1] - cp[1], pt[2] - cp[2]];
                    let dsq = dot(diff, diff);
                    if dsq < best_dist_sq {
                        best_dist_sq = dsq;
                        best_pt = cp;
                        best_tri = ti;
                    }
                }
                P[i] = best_pt;
                last_tri_indices[i] = best_tri;
            }
        }
    }

    let mut out = (*points).clone();
    let flat_pos: Vec<f32> = P.iter().flat_map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect();
    out.set_p(flat_pos);

    if let Some(ref tris) = surface_tris {
        if update_normals {
            let mut normals = Vec::with_capacity(n * 3);
            for &ti in &last_tri_indices {
                let tri = &tris[ti];
                let e1 = [tri[1][0] - tri[0][0], tri[1][1] - tri[0][1], tri[1][2] - tri[0][2]];
                let e2 = [tri[2][0] - tri[0][0], tri[2][1] - tri[0][1], tri[2][2] - tri[0][2]];
                let cr = cross(e1, e2);
                let ln = dot(cr, cr).sqrt().max(1e-20);
                normals.push((cr[0] / ln) as f32);
                normals.push((cr[1] / ln) as f32);
                normals.push((cr[2] / ln) as f32);
            }
            out.point.insert("N".into(), Arc::new(Attr::f32(3, normals)));
        }
    }

    Ok(out)
}

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
