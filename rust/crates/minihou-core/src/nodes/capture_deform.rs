//! Spatial falloff, trilinear lattice and local affine point deformers from minihou/nodes/capture_deform.py.
use std::sync::Arc;
use super::{GEO, InputSpec, Inputs, NodeSpec, linspace};
use crate::{
    error::{Error, Result},
    geometry::{Attr, AttrData, Geometry},
    params::Params,
};

pub const LATTICE_GRID: NodeSpec = NodeSpec {
    name: "lattice_grid",
    inputs: &[],
    params: crate::params!(
        "center": [0.0, 0.0, 0.0];
        "size": [2.0, 2.0, 2.0];
        "columns": 2;
        "rows": 2;
        "layers": 2;
        "offsets": "[]"
    ),
    cook: lattice_grid,
};

fn lattice_grid(_: &Inputs, p: &Params) -> Result<Geometry> {
    let center = p.v3("center")?;
    let size = p.v3("size")?;
    if size.iter().any(|&s| s <= 0.0) {
        return Err(Error::new("lattice_grid size must be positive in all three dimensions"));
    }
    let cols = p.i("columns")?;
    let rows = p.i("rows")?;
    let layers = p.i("layers")?;
    if !(2..=64).contains(&cols) || !(2..=64).contains(&rows) || !(2..=64).contains(&layers) {
        return Err(Error::new("columns, rows, layers must be 2..64"));
    }

    let x_axis = linspace(center[0] - size[0] / 2.0, center[0] + size[0] / 2.0, cols as usize);
    let y_axis = linspace(center[1] - size[1] / 2.0, center[1] + size[1] / 2.0, rows as usize);
    let z_axis = linspace(center[2] - size[2] / 2.0, center[2] + size[2] / 2.0, layers as usize);

    let n = (cols * rows * layers) as usize;
    let mut pts = Vec::with_capacity(n * 3);
    for &z in &z_axis {
        for &y in &y_axis {
            for &x in &x_axis {
                pts.push(x as f32);
                pts.push(y as f32);
                pts.push(z as f32);
            }
        }
    }

    let offsets_str = p.s("offsets")?;
    if !offsets_str.trim().is_empty() && offsets_str.trim() != "[]" {
        let val: serde_json::Value = serde_json::from_str(&offsets_str)
            .map_err(|e| Error::new(format!("invalid lattice offsets JSON: {e}")))?;
        if let Some(arr) = val.as_array() {
            if !arr.is_empty() {
                if arr.len() != pts.len() {
                    return Err(Error::new("lattice offsets must be a finite flat XYZ array"));
                }
                for (i, v) in arr.iter().enumerate() {
                    let delta = v.as_f64().ok_or_else(|| Error::new("lattice offsets must be a finite flat XYZ array"))?;
                    if !delta.is_finite() {
                        return Err(Error::new("lattice offsets must be a finite flat XYZ array"));
                    }
                    pts[i] += delta as f32;
                }
            }
        }
    }

    let mut out = Geometry::points_only(pts)?;
    out.detail.insert(
        "lattice_dimensions".into(),
        serde_json::json!([cols, rows, layers]),
    );
    Ok(out)
}

const DEFORM_INPUTS: &[InputSpec] = &[
    InputSpec { name: "geo", multi: false, optional: false },
    InputSpec { name: "rest", multi: false, optional: false },
    InputSpec { name: "deformed", multi: false, optional: false },
];

pub const LATTICE: NodeSpec = NodeSpec {
    name: "lattice",
    inputs: DEFORM_INPUTS,
    params: crate::params!("group": ""; "outside": "clamp"),
    cook: lattice,
};

pub const POINT_DEFORM: NodeSpec = NodeSpec {
    name: "point_deform",
    inputs: DEFORM_INPUTS,
    params: crate::params!("radius": 0.0; "max_points": 16; "group": ""),
    cook: point_deform,
};

fn validate_controls(rest: &Geometry, deformed: &Geometry) -> Result<()> {
    if rest.npoints == 0 || deformed.npoints == 0 {
        return Err(Error::new("connect nonempty rest and deformed control inputs"));
    }
    if rest.npoints != deformed.npoints
        || super::sim_common::topology_digest(&rest.topology) != super::sim_common::topology_digest(&deformed.topology)
    {
        return Err(Error::new("rest and deformed controls must have identical topology and point correspondence"));
    }
    if rest.p().iter().any(|x| !x.is_finite()) || deformed.p().iter().any(|x| !x.is_finite()) {
        return Err(Error::new("control positions must be finite"));
    }
    Ok(())
}

fn lattice(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = LATTICE.one(inputs, 0)?;
    let rest = LATTICE.one(inputs, 1)?;
    let deformed = LATTICE.one(inputs, 2)?;
    validate_controls(&rest, &deformed)?;

    if geo.p().iter().any(|x| !x.is_finite()) {
        return Err(Error::new("deformation requires finite positions"));
    }

    let outside = p.e("outside", &["clamp", "unchanged"])?;
    let sel = super::group::mask(&geo, "point", &p.s("group")?)?;

    let rp = rest.p();
    let dp = deformed.p();
    let m = rest.npoints;

    // Extract unique axes
    let mut axes = [Vec::new(), Vec::new(), Vec::new()];
    for k in 0..3 {
        let mut vals: Vec<f64> = (0..m).map(|i| rp[i * 3 + k] as f64).collect();
        vals.sort_by(|a, b| a.total_cmp(b));
        let mut u: Vec<f64> = Vec::new();
        for v in vals {
            if u.last().map_or(true, |&last| (v - last).abs() > 1e-9) {
                u.push(v);
            }
        }
        axes[k] = u;
    }

    let dims = [axes[0].len(), axes[1].len(), axes[2].len()];
    if dims.iter().any(|&d| d < 2) || dims[0] * dims[1] * dims[2] != m {
        return Err(Error::new("rest lattice must form a complete rectangular 3D grid aligned with X, Y and Z"));
    }

    let mut delta = vec![[0.0f64; 3]; dims[0] * dims[1] * dims[2]];
    let mut seen = vec![false; delta.len()];

    for i in 0..m {
        let p_rest = [rp[i * 3] as f64, rp[i * 3 + 1] as f64, rp[i * 3 + 2] as f64];
        let p_def = [dp[i * 3] as f64, dp[i * 3 + 1] as f64, dp[i * 3 + 2] as f64];
        let ix = axes[0].iter().position(|&v| (v - p_rest[0]).abs() <= 1e-9)
            .ok_or_else(|| Error::new("rest lattice has duplicate or missing control points"))?;
        let iy = axes[1].iter().position(|&v| (v - p_rest[1]).abs() <= 1e-9)
            .ok_or_else(|| Error::new("rest lattice has duplicate or missing control points"))?;
        let iz = axes[2].iter().position(|&v| (v - p_rest[2]).abs() <= 1e-9)
            .ok_or_else(|| Error::new("rest lattice has duplicate or missing control points"))?;

        let flat_idx = (ix * dims[1] + iy) * dims[2] + iz;
        if seen[flat_idx] {
            return Err(Error::new("rest lattice has duplicate or missing control points"));
        }
        seen[flat_idx] = true;
        delta[flat_idx] = [p_def[0] - p_rest[0], p_def[1] - p_rest[1], p_def[2] - p_rest[2]];
    }

    let map_pt = |pt: [f64; 3]| -> [f64; 3] {
        let mut inside = true;
        let mut indices = [0usize; 3];
        let mut weights = [0.0f64; 3];

        for k in 0..3 {
            let axis = &axes[k];
            let lo = axis[0];
            let hi = *axis.last().unwrap();
            if pt[k] < lo || pt[k] > hi {
                inside = false;
            }
            let q = pt[k].clamp(lo, hi);
            let idx = axis[..axis.len() - 1].partition_point(|&v| v <= q).saturating_sub(1).min(axis.len() - 2);
            indices[k] = idx;
            let denom = axis[idx + 1] - axis[idx];
            weights[k] = if denom > 0.0 { (q - axis[idx]) / denom } else { 0.0 };
        }

        if outside == "unchanged" && !inside {
            return pt;
        }

        let mut d = [0.0f64; 3];
        for cx in 0..=1 {
            let wx = if cx == 1 { weights[0] } else { 1.0 - weights[0] };
            let ix = indices[0] + cx;
            for cy in 0..=1 {
                let wy = if cy == 1 { weights[1] } else { 1.0 - weights[1] };
                let iy = indices[1] + cy;
                for cz in 0..=1 {
                    let wz = if cz == 1 { weights[2] } else { 1.0 - weights[2] };
                    let iz = indices[2] + cz;
                    let w = wx * wy * wz;
                    let del = delta[(ix * dims[1] + iy) * dims[2] + iz];
                    d[0] += w * del[0];
                    d[1] += w * del[1];
                    d[2] += w * del[2];
                }
            }
        }
        [pt[0] + d[0], pt[1] + d[1], pt[2] + d[2]]
    };

    apply_deformation(&geo, &sel, map_pt)
}

// ---------------------------------------------------------------------------
// POINT DEFORM (local affine fitting)
// ---------------------------------------------------------------------------

/// 3x3 symmetric eigendecomposition via Jacobi algorithm.
fn sym_eigh_3x3(m: &[[f64; 3]; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut a = *m;
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    for _ in 0..50 {
        let mut max_off = 0.0;
        let mut p = 0;
        let mut q = 1;
        for i in 0..3 {
            for j in i + 1..3 {
                let off = a[i][j].abs();
                if off > max_off {
                    max_off = off;
                    p = i;
                    q = j;
                }
            }
        }
        if max_off < 1e-15 {
            break;
        }

        let app = a[p][p];
        let aqq = a[q][q];
        let apq = a[p][q];

        let theta = (aqq - app) / (2.0 * apq);
        let t = if theta >= 0.0 {
            1.0 / (theta + (theta * theta + 1.0).sqrt())
        } else {
            -1.0 / (-theta + (theta * theta + 1.0).sqrt())
        };
        let c = 1.0 / (t * t + 1.0).sqrt();
        let s = t * c;

        let tau = s / (1.0 + c);

        a[p][p] = app - t * apq;
        a[q][q] = aqq + t * apq;
        a[p][q] = 0.0;
        a[q][p] = 0.0;

        for r in 0..3 {
            if r != p && r != q {
                let arp = a[r][p];
                let arq = a[r][q];
                a[r][p] = arp - s * (arq + tau * arp);
                a[p][r] = a[r][p];
                a[r][q] = arq + s * (arp - tau * arq);
                a[q][r] = a[r][q];
            }
        }

        for r in 0..3 {
            let vrp = v[r][p];
            let vrq = v[r][q];
            v[r][p] = vrp - s * (vrq + tau * vrp);
            v[r][q] = vrq + s * (vrp - tau * vrq);
        }
    }

    let w = [a[0][0], a[1][1], a[2][2]];
    (w, v)
}

fn sym_pinv_3x3(m: &[[f64; 3]; 3], rcond: f64) -> [[f64; 3]; 3] {
    let (w, v) = sym_eigh_3x3(m);
    let max_w = w[0].abs().max(w[1].abs()).max(w[2].abs()).max(1e-12);
    let mut inv_w = [0.0; 3];
    for i in 0..3 {
        if w[i] > rcond * max_w {
            inv_w[i] = 1.0 / w[i];
        }
    }

    let mut out = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            for k in 0..3 {
                out[i][j] += v[i][k] * inv_w[k] * v[j][k];
            }
        }
    }
    out
}

fn point_deform(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = POINT_DEFORM.one(inputs, 0)?;
    let rest = POINT_DEFORM.one(inputs, 1)?;
    let deformed = POINT_DEFORM.one(inputs, 2)?;
    validate_controls(&rest, &deformed)?;

    if geo.p().iter().any(|x| !x.is_finite()) {
        return Err(Error::new("deformation requires finite positions"));
    }

    if (geo.npoints as u64) * (rest.npoints as u64) > 50_000_000 {
        return Err(Error::new("initial point_deform limit: 50 million query/control pairs; reduce the control cloud"));
    }

    let radius = p.f("radius")?;
    let max_points = p.i("max_points")? as usize;
    let sel = super::group::mask(&geo, "point", &p.s("group")?)?;

    let rp = rest.p();
    let dp = deformed.p();
    let m = rest.npoints;

    let mut a_pts = Vec::with_capacity(m);
    let mut delta = Vec::with_capacity(m);
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];

    for i in 0..m {
        let a = [rp[i * 3] as f64, rp[i * 3 + 1] as f64, rp[i * 3 + 2] as f64];
        let b = [dp[i * 3] as f64, dp[i * 3 + 1] as f64, dp[i * 3 + 2] as f64];
        for k in 0..3 {
            lo[k] = lo[k].min(a[k]);
            hi[k] = hi[k].max(a[k]);
        }
        a_pts.push(a);
        delta.push([b[0] - a[0], b[1] - a[1], b[2] - a[2]]);
    }

    let size = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(hi[2] - lo[2]).max(1e-6);
    let floor = (size * 1e-4).powi(2);

    let map_pt = |pt: [f64; 3]| -> [f64; 3] {
        let mut d2_pairs: Vec<(f64, usize)> = a_pts.iter().enumerate().map(|(idx, &a)| {
            let d2 = (a[0] - pt[0]).powi(2) + (a[1] - pt[1]).powi(2) + (a[2] - pt[2]).powi(2);
            (d2, idx)
        }).collect();

        d2_pairs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut kept: Vec<(f64, usize)> = d2_pairs.into_iter().take(max_points).collect();
        if radius > 0.0 {
            let r2 = radius * radius;
            kept.retain(|&(d2, _)| d2 <= r2);
        }
        if kept.is_empty() {
            return pt;
        }

        let mut weights = Vec::with_capacity(kept.len());
        let mut w_sum = 0.0;
        for &(d2, _) in &kept {
            let w = 1.0 / (d2 + floor);
            weights.push(w);
            w_sum += w;
        }
        for w in &mut weights {
            *w /= w_sum;
        }

        let mut center = [0.0; 3];
        let mut mean = [0.0; 3];
        for (i, &(_, idx)) in kept.iter().enumerate() {
            let w = weights[i];
            for k in 0..3 {
                center[k] += w * a_pts[idx][k];
                mean[k] += w * delta[idx][k];
            }
        }

        // Form M = X_w^T X_w and R = X_w^T Y_w
        let mut m_mat = [[0.0; 3]; 3];
        let mut r_mat = [[0.0; 3]; 3];
        for (i, &(_, idx)) in kept.iter().enumerate() {
            let w = weights[i];
            let x = [a_pts[idx][0] - center[0], a_pts[idx][1] - center[1], a_pts[idx][2] - center[2]];
            let y = [delta[idx][0] - mean[0], delta[idx][1] - mean[1], delta[idx][2] - mean[2]];
            for r in 0..3 {
                for c in 0..3 {
                    m_mat[r][c] += w * x[r] * x[c];
                    r_mat[r][c] += w * x[r] * y[c];
                }
            }
        }

        let m_pinv = sym_pinv_3x3(&m_mat, 1e-8);
        // grad = pinv(M) @ R
        let mut grad = [[0.0; 3]; 3];
        for r in 0..3 {
            for c in 0..3 {
                for k in 0..3 {
                    grad[r][c] += m_pinv[r][k] * r_mat[k][c];
                }
            }
        }

        let p_sub_c = [pt[0] - center[0], pt[1] - center[1], pt[2] - center[2]];
        let mut disp = mean;
        for c in 0..3 {
            for r in 0..3 {
                disp[c] += p_sub_c[r] * grad[r][c];
            }
        }

        [pt[0] + disp[0], pt[1] + disp[1], pt[2] + disp[2]]
    };

    apply_deformation(&geo, &sel, map_pt)
}

fn apply_deformation<F>(geo: &Geometry, sel: &[bool], mapping: F) -> Result<Geometry>
where
    F: Fn([f64; 3]) -> [f64; 3],
{
    let p_slice = geo.p();
    let n = geo.npoints;
    let mut new_p = Vec::with_capacity(n * 3);

    for i in 0..n {
        let orig = [p_slice[i * 3] as f64, p_slice[i * 3 + 1] as f64, p_slice[i * 3 + 2] as f64];
        let mapped = if sel[i] { mapping(orig) } else { orig };
        if mapped.iter().any(|x| !x.is_finite()) {
            return Err(Error::new("deformation produced nonfinite positions"));
        }
        new_p.push(mapped[0] as f32);
        new_p.push(mapped[1] as f32);
        new_p.push(mapped[2] as f32);
    }

    let mut out = (*geo).clone();
    out.set_p(new_p);

    // Normal transform via finite-difference Jacobian
    let has_n = geo.point.contains_key("N") || geo.vertex.contains_key("N") || geo.prim.contains_key("N");
    if has_n {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for i in 0..n {
            for k in 0..3 {
                lo[k] = lo[k].min(p_slice[i * 3 + k] as f64);
                hi[k] = hi[k].max(p_slice[i * 3 + k] as f64);
            }
        }
        let ptp = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(hi[2] - lo[2]).max(1.0);
        let h = ptp * 1e-5;

        let mut jac = vec![[[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]; n];
        for i in 0..n {
            if !sel[i] {
                continue;
            }
            let orig = [p_slice[i * 3] as f64, p_slice[i * 3 + 1] as f64, p_slice[i * 3 + 2] as f64];
            for k in 0..3 {
                let mut p_plus = orig;
                let mut p_minus = orig;
                p_plus[k] += h;
                p_minus[k] -= h;
                let m_plus = mapping(p_plus);
                let m_minus = mapping(p_minus);
                for r in 0..3 {
                    jac[i][r][k] = (m_plus[r] - m_minus[r]) / (2.0 * h);
                }
            }
        }

        // Point normals
        if let Some(attr) = geo.point.get("N") {
            if let Some(src) = attr.as_f32() {
                let mut dst = src.to_vec();
                for i in 0..n {
                    if sel[i] {
                        let inv_j = invert_3x3(&jac[i]);
                        let norm = [src[i * 3] as f64, src[i * 3 + 1] as f64, src[i * 3 + 2] as f64];
                        let mut new_n = [0.0; 3];
                        for c in 0..3 {
                            for r in 0..3 {
                                new_n[c] += norm[r] * inv_j[r][c];
                            }
                        }
                        let l = (new_n[0] * new_n[0] + new_n[1] * new_n[1] + new_n[2] * new_n[2]).sqrt().max(1e-30);
                        dst[i * 3] = (new_n[0] / l) as f32;
                        dst[i * 3 + 1] = (new_n[1] / l) as f32;
                        dst[i * 3 + 2] = (new_n[2] / l) as f32;
                    }
                }
                out.point.insert("N".into(), Arc::new(Attr::f32(3, dst)));
            }
        }

        // Vertex normals
        if let Some(attr) = geo.vertex.get("N") {
            if let Some(src) = attr.as_f32() {
                let mut dst = src.to_vec();
                for v in 0..geo.nvertices() {
                    let pt = geo.topology.vertex_point[v] as usize;
                    if sel[pt] {
                        let inv_j = invert_3x3(&jac[pt]);
                        let norm = [src[v * 3] as f64, src[v * 3 + 1] as f64, src[v * 3 + 2] as f64];
                        let mut new_n = [0.0; 3];
                        for c in 0..3 {
                            for r in 0..3 {
                                new_n[c] += norm[r] * inv_j[r][c];
                            }
                        }
                        let l = (new_n[0] * new_n[0] + new_n[1] * new_n[1] + new_n[2] * new_n[2]).sqrt().max(1e-30);
                        dst[v * 3] = (new_n[0] / l) as f32;
                        dst[v * 3 + 1] = (new_n[1] / l) as f32;
                        dst[v * 3 + 2] = (new_n[2] / l) as f32;
                    }
                }
                out.vertex.insert("N".into(), Arc::new(Attr::f32(3, dst)));
            }
        }
    }

    Ok(out)
}

fn invert_3x3(m: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);

    if det.abs() > 1e-12 {
        let inv_det = 1.0 / det;
        [
            [
                (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * inv_det,
                (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * inv_det,
                (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * inv_det,
            ],
            [
                (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * inv_det,
                (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * inv_det,
                (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * inv_det,
            ],
            [
                (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * inv_det,
                (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * inv_det,
                (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * inv_det,
            ],
        ]
    } else {
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    }
}

pub const SOFT_TRANSFORM: NodeSpec = NodeSpec {
    name: "soft_transform",
    inputs: GEO,
    params: crate::params!(
        "center": [0.0, 0.0, 0.0];
        "radius": 1.0;
        "translate": [0.0, 0.0, 0.0];
        "rotate": [0.0, 0.0, 0.0];
        "scale": [1.0, 1.0, 1.0];
        "falloff": "smooth";
        "group": "";
    ),
    cook: soft_transform,
};

fn soft_transform(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = SOFT_TRANSFORM.one(inputs, 0)?;
    let center = p.v3("center")?;
    let radius = p.f("radius")?;
    if radius <= 0.0 {
        return Err(Error::new("radius must be positive"));
    }
    let translate = p.v3("translate")?;
    let rotate = p.v3("rotate")?;
    let scale = p.v3("scale")?;
    let falloff = p.e("falloff", &["smooth", "linear"])?;
    let is_smooth = falloff == "smooth";
    let group = p.s("group")?;
    let sel = super::group::mask(&geo, "point", &group)?;

    let r_mat = crate::math::euler_xyz(rotate);

    let map_pt = |pt: [f64; 3]| -> [f64; 3] {
        let dx = pt[0] - center[0];
        let dy = pt[1] - center[1];
        let dz = pt[2] - center[2];
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        let mut w = (1.0 - dist / radius).clamp(0.0, 1.0);
        if is_smooth {
            w = w * w * (3.0 - 2.0 * w);
        }

        let local = [dx * scale[0], dy * scale[1], dz * scale[2]];
        let rot = crate::math::apply(&r_mat, local);
        let target = [
            rot[0] + center[0] + translate[0],
            rot[1] + center[1] + translate[1],
            rot[2] + center[2] + translate[2],
        ];

        [
            pt[0] + w * (target[0] - pt[0]),
            pt[1] + w * (target[1] - pt[1]),
            pt[2] + w * (target[2] - pt[2]),
        ]
    };

    apply_deformation(&geo, &sel, map_pt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lattice_grid_points_and_dims() {
        let p = Params::resolve(LATTICE_GRID.params, None).unwrap();
        let g = lattice_grid(&[], &p).unwrap();
        assert_eq!(g.npoints, 8);
        assert_eq!(g.detail["lattice_dimensions"], serde_json::json!([2, 2, 2]));
    }

    #[test]
    fn controls_require_matching_topology_but_accept_equivalent_closed_masks() {
        let rest = Geometry::from_arrays(vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            vec![0, 1, 2], vec![0, 3], None).unwrap();
        let mut deformed = rest.clone();
        Arc::make_mut(&mut deformed.topology).closed = Some(vec![true]);
        validate_controls(&rest, &deformed).unwrap();
        Arc::make_mut(&mut deformed.topology).vertex_point.swap(0, 1);
        assert!(validate_controls(&rest, &deformed).unwrap_err().0.contains("identical topology"));
        Arc::make_mut(&mut deformed.topology).vertex_point.swap(0, 1);
        Arc::make_mut(&mut deformed.topology).closed = Some(vec![false]);
        assert!(validate_controls(&rest, &deformed).is_err());
    }

    #[test]
    fn point_deform_single_control() {
        let g = Geometry::points_only(vec![0.0, 0.0, 0.0]).unwrap();
        let one = Geometry::points_only(vec![0.0, 0.0, 0.0]).unwrap();
        let moved = Geometry::points_only(vec![1.0, 2.0, 3.0]).unwrap();
        let p = Params::resolve(POINT_DEFORM.params, None).unwrap();
        let out = point_deform(&[vec![Arc::new(g)], vec![Arc::new(one)], vec![Arc::new(moved)]], &p).unwrap();
        assert_eq!(out.p(), &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_soft_transform_falloff() {
        let pts = vec![0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0];
        let mut g = Geometry::points_only(pts).unwrap();
        g.point.insert("group_move".to_string(), Arc::new(Attr::i32(1, vec![1, 1, 1, 0])));
        let p = Params::resolve(SOFT_TRANSFORM.params, serde_json::json!({
            "translate": [0.0, 2.0, 0.0],
            "group": "move"
        }).as_object()).unwrap();
        let out = soft_transform(&[vec![Arc::new(g)]], &p).unwrap();
        let p_out = out.p();
        assert!((p_out[1] - 2.0).abs() < 1e-5);
        assert!((p_out[4] - 1.0).abs() < 1e-5);
        assert!(p_out[7].abs() < 1e-5);
        assert!(p_out[10].abs() < 1e-5);
    }
}
