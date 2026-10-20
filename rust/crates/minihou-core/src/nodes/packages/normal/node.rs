use std::sync::Arc;

use crate::error::Result;
use crate::geometry::{Attr, Geometry};
use crate::nodes::{groups, model, InputSpec, Inputs, NodeSpec, GEO};
use crate::normals;
use crate::params::Params;

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "normal",
    inputs: GEO,
    params: crate::params!(
        "group": "";
        "domain": "point";
        "cusp_angle": 60.0;
        "reverse": false;
        "weighting": "area";
    ),
    cook,
};

fn corner_angle(p: &[f32], q: usize, prev: usize, next: usize) -> f64 {
    let q3 = q * 3;
    let prev3 = prev * 3;
    let next3 = next * 3;
    let v1 = [
        p[prev3] as f64 - p[q3] as f64,
        p[prev3 + 1] as f64 - p[q3 + 1] as f64,
        p[prev3 + 2] as f64 - p[q3 + 2] as f64,
    ];
    let v2 = [
        p[next3] as f64 - p[q3] as f64,
        p[next3 + 1] as f64 - p[q3 + 1] as f64,
        p[next3 + 2] as f64 - p[q3 + 2] as f64,
    ];
    let len1 = (v1[0] * v1[0] + v1[1] * v1[1] + v1[2] * v1[2]).sqrt();
    let len2 = (v2[0] * v2[0] + v2[1] * v2[1] + v2[2] * v2[2]).sqrt();
    if len1 < 1e-12 || len2 < 1e-12 {
        return 0.0;
    }
    let dot = (v1[0] * v2[0] + v1[1] * v2[1] + v1[2] * v2[2]) / (len1 * len2);
    dot.clamp(-1.0, 1.0).acos()
}

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = NODE.one(inputs, 0)?;
    if geo.npoints == 0 {
        return Ok((*geo).clone());
    }

    let group = p.s("group")?;
    let domain = p.e("domain", &["point", "prim", "vertex"])?;
    let cusp_angle = p.f("cusp_angle")?;
    let reverse = p.b("reverse")?;
    let weighting = p.e("weighting", &["area", "angle", "equal"])?;

    let sign = if reverse { -1.0f32 } else { 1.0f32 };
    let p_slice = geo.p();
    let vp = &geo.topology.vertex_point;
    let offs = &geo.topology.prim_offsets;
    let nprims = geo.nprims();
    let nverts = geo.nvertices();
    let npoints = geo.npoints;

    let fnorm = normals::prim_normals(p_slice, &geo.topology);
    let funit_f32 = normals::normalize_to_f32(&fnorm);
    let funit: Vec<[f64; 3]> = (0..nprims)
        .map(|f| [
            funit_f32[f * 3] as f64,
            funit_f32[f * 3 + 1] as f64,
            funit_f32[f * 3 + 2] as f64,
        ])
        .collect();

    let mut out = (*geo).clone();

    match domain.as_str() {
        "prim" => {
            let mask = if group.trim().is_empty() {
                vec![true; nprims]
            } else {
                model::prim_mask(&geo, &group)?
            };

            let mut n_data = if let Some(old) = out.prim.get("N").and_then(|a| a.as_f32()) {
                old.to_vec()
            } else {
                vec![0.0f32; nprims * 3]
            };

            for f in 0..nprims {
                if mask[f] {
                    n_data[f * 3] = funit_f32[f * 3] * sign;
                    n_data[f * 3 + 1] = funit_f32[f * 3 + 1] * sign;
                    n_data[f * 3 + 2] = funit_f32[f * 3 + 2] * sign;
                }
            }
            out.prim.insert("N".into(), Arc::new(Attr::f32(3, n_data)));
            Ok(out)
        }
        "point" => {
            let mask = if group.trim().is_empty() {
                vec![true; npoints]
            } else {
                groups::point_mask(&geo, &group)?
            };

            let (start, prims) = normals::point_prims(vp, offs, npoints);
            let mut n_data = if let Some(old) = out.point.get("N").and_then(|a| a.as_f32()) {
                old.to_vec()
            } else {
                vec![0.0f32; npoints * 3]
            };

            for q in 0..npoints {
                if !mask[q] {
                    continue;
                }
                let mut acc = [0.0f64; 3];
                for &prim_u32 in &prims[start[q] as usize..start[q + 1] as usize] {
                    let f = prim_u32 as usize;
                    match weighting.as_str() {
                        "equal" => {
                            acc[0] += funit[f][0];
                            acc[1] += funit[f][1];
                            acc[2] += funit[f][2];
                        }
                        "angle" => {
                            let s = offs[f] as usize;
                            let e = offs[f + 1] as usize;
                            let count = e - s;
                            if count >= 3 {
                                for i in 0..count {
                                    if vp[s + i] as usize == q {
                                        let prev = vp[s + (i + count - 1) % count] as usize;
                                        let next = vp[s + (i + 1) % count] as usize;
                                        let ang = corner_angle(p_slice, q, prev, next);
                                        acc[0] += funit[f][0] * ang;
                                        acc[1] += funit[f][1] * ang;
                                        acc[2] += funit[f][2] * ang;
                                        break;
                                    }
                                }
                            }
                        }
                        _ => {
                            // "area"
                            acc[0] += fnorm[f][0];
                            acc[1] += fnorm[f][1];
                            acc[2] += fnorm[f][2];
                        }
                    }
                }
                let len = (acc[0] * acc[0] + acc[1] * acc[1] + acc[2] * acc[2]).sqrt();
                if len > 1e-12 {
                    n_data[q * 3] = (acc[0] / len) as f32 * sign;
                    n_data[q * 3 + 1] = (acc[1] / len) as f32 * sign;
                    n_data[q * 3 + 2] = (acc[2] / len) as f32 * sign;
                } else {
                    n_data[q * 3] = 0.0;
                    n_data[q * 3 + 1] = 0.0;
                    n_data[q * 3 + 2] = 0.0;
                }
            }
            out.point.insert("N".into(), Arc::new(Attr::f32(3, n_data)));
            Ok(out)
        }
        "vertex" => {
            let mask = if group.trim().is_empty() {
                vec![true; nprims]
            } else {
                model::prim_mask(&geo, &group)?
            };

            let cos_cusp = (cusp_angle.to_radians()).cos() - 1e-9;
            let (start, prims) = normals::point_prims(vp, offs, npoints);

            let mut n_data = if let Some(old) = out.vertex.get("N").and_then(|a| a.as_f32()) {
                old.to_vec()
            } else {
                vec![0.0f32; nverts * 3]
            };

            for f in 0..nprims {
                if !mask[f] {
                    continue;
                }
                let u_f = funit[f];
                let s = offs[f] as usize;
                let e = offs[f + 1] as usize;
                for v in s..e {
                    let q = vp[v] as usize;
                    let mut acc = [0.0f64; 3];

                    for &nbr_u32 in &prims[start[q] as usize..start[q + 1] as usize] {
                        let nbr = nbr_u32 as usize;
                        let u_nbr = funit[nbr];
                        let dot = u_f[0] * u_nbr[0] + u_f[1] * u_nbr[1] + u_f[2] * u_nbr[2];
                        if dot >= cos_cusp {
                            match weighting.as_str() {
                                "equal" => {
                                    acc[0] += u_nbr[0];
                                    acc[1] += u_nbr[1];
                                    acc[2] += u_nbr[2];
                                }
                                "angle" => {
                                    let ns = offs[nbr] as usize;
                                    let ne = offs[nbr + 1] as usize;
                                    let ncount = ne - ns;
                                    if ncount >= 3 {
                                        for i in 0..ncount {
                                            if vp[ns + i] as usize == q {
                                                let prev = vp[ns + (i + ncount - 1) % ncount] as usize;
                                                let next = vp[ns + (i + 1) % ncount] as usize;
                                                let ang = corner_angle(p_slice, q, prev, next);
                                                acc[0] += u_nbr[0] * ang;
                                                acc[1] += u_nbr[1] * ang;
                                                acc[2] += u_nbr[2] * ang;
                                                break;
                                            }
                                        }
                                    }
                                }
                                _ => {
                                    // "area"
                                    acc[0] += fnorm[nbr][0];
                                    acc[1] += fnorm[nbr][1];
                                    acc[2] += fnorm[nbr][2];
                                }
                            }
                        }
                    }

                    let len = (acc[0] * acc[0] + acc[1] * acc[1] + acc[2] * acc[2]).sqrt();
                    if len > 1e-12 {
                        n_data[v * 3] = (acc[0] / len) as f32 * sign;
                        n_data[v * 3 + 1] = (acc[1] / len) as f32 * sign;
                        n_data[v * 3 + 2] = (acc[2] / len) as f32 * sign;
                    } else {
                        n_data[v * 3] = u_f[0] as f32 * sign;
                        n_data[v * 3 + 1] = u_f[1] as f32 * sign;
                        n_data[v * 3 + 2] = u_f[2] as f32 * sign;
                    }
                }
            }

            out.vertex.insert("N".into(), Arc::new(Attr::f32(3, n_data)));
            out.point.remove("N");
            Ok(out)
        }
        _ => unreachable!(),
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
