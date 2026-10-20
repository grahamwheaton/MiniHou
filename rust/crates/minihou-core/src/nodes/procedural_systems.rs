//! Architecture and procedural systems (minihou/nodes/procedural_systems.py).
use std::sync::Arc;

use super::{InputSpec, Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::Params;

pub const BUILDING_RULES: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "building_rules",
    inputs: GEO,
    params: crate::params!(
        "storeys": 3;
        "storey_height": 3.0;
        "bay_width": 2.5;
        "window_width": 0.55;
        "window_height": 0.55;
        "balcony_probability": 0.25;
        "balcony_depth": 0.7;
        "roof": "flat";
        "roof_height": 1.5;
        "seed": 0;
        "rules": "[]"
    ),
    cook: building_rules,
};

mod building;
use building::building_rules;

pub const REGION_IDENTIFY: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "region_identify",
    inputs: GEO,
    params: crate::params!(
        "mode": "connectivity";
        "attribute": "class";
        "seed": 0
    ),
    cook: region_identify,
};

pub const REGION_TRANSFORM: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "region_transform",
    inputs: GEO,
    params: crate::params!(
        "translate": [0.0, 0.0, 0.0];
        "rotate": [0.0, 0.0, 0.0];
        "scale": [1.0, 1.0, 1.0];
        "random_translate": [0.0, 0.0, 0.0];
        "random_rotate": [0.0, 0.0, 0.0];
        "random_scale": 0.0;
        "extrude": 0.0;
        "random_extrude": 0.0;
        "seed": 0
    ),
    cook: region_transform,
};

fn region_identify(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = REGION_IDENTIFY.one(inputs, 0)?;
    region_identify_impl(&geo, p)
}

fn region_identify_impl(geo: &Geometry, p: &Params) -> Result<Geometry> {
    for &v in geo.p() {
        if !v.is_finite() {
            return Err(Error::new("region positions must be finite"));
        }
    }
    let mode = p.e("mode", &["connectivity", "primitive", "attribute"])?;
    let attribute = p.s("attribute")?;
    let seed = p.i("seed")?;

    let labels: Vec<i32> = match mode.as_str() {
        "connectivity" => {
            let (_, l) = super::attrib_tools::piece_labels(geo);
            l
        }
        "primitive" => (0..geo.nprims() as i32).collect(),
        "attribute" => {
            let a = geo.prim.get(&attribute).ok_or_else(|| {
                Error::new("region attribute must be a scalar integer primitive attribute")
            })?;
            if a.width != 1 {
                return Err(Error::new(
                    "region attribute must be a scalar integer primitive attribute",
                ));
            }
            let vals = match &a.data {
                AttrData::I32(v) => v.clone(),
                _ => {
                    return Err(Error::new(
                        "region attribute must be a scalar integer primitive attribute",
                    ))
                }
            };
            if vals.len() != geo.nprims() {
                return Err(Error::new(
                    "region attribute must be a scalar integer primitive attribute",
                ));
            }
            vals
        }
        _ => unreachable!(),
    };

    let mut keys: Vec<i32> = labels.clone();
    keys.sort_unstable();
    keys.dedup();

    if keys.len() > 10000 {
        return Err(Error::new("at most 10000 regions are supported"));
    }
    if keys.len() * geo.nvertices().max(1) > 50_000_000 {
        return Err(Error::new(
            "region identification work budget exceeded; process fewer faces or regions",
        ));
    }

    let nprims = geo.nprims();
    let mut random = vec![0.0f32; nprims];
    let mut centers = vec![0.0f32; nprims * 3];
    let mut colors = vec![0.0f32; nprims * 3];

    let p_slice = geo.p();
    let vp = &geo.topology.vertex_point;
    let offs = &geo.topology.prim_offsets;

    for key in keys {
        let mut used_pts = Vec::new();
        for p in 0..nprims {
            if labels[p] == key {
                let start = offs[p] as usize;
                let end = offs[p + 1] as usize;
                for &v in &vp[start..end] {
                    used_pts.push(v as usize);
                }
            }
        }
        used_pts.sort_unstable();
        used_pts.dedup();

        let mut center = [0.0f64; 3];
        if !used_pts.is_empty() {
            for &pt in &used_pts {
                center[0] += p_slice[pt * 3] as f64;
                center[1] += p_slice[pt * 3 + 1] as f64;
                center[2] += p_slice[pt * 3 + 2] as f64;
            }
            center[0] /= used_pts.len() as f64;
            center[1] /= used_pts.len() as f64;
            center[2] /= used_pts.len() as f64;
        }

        let mut rng = super::np_random::Rng::new(&[
            (seed as u64) & 0xFFFF_FFFF,
            (key as u64) & 0xFFFF_FFFF,
        ]);
        let rand_val = rng.random() as f32;
        let col_val = [
            (0.15 + 0.70 * rng.random()) as f32,
            (0.15 + 0.70 * rng.random()) as f32,
            (0.15 + 0.70 * rng.random()) as f32,
        ];

        for p in 0..nprims {
            if labels[p] == key {
                random[p] = rand_val;
                centers[p * 3] = center[0] as f32;
                centers[p * 3 + 1] = center[1] as f32;
                centers[p * 3 + 2] = center[2] as f32;
                colors[p * 3] = col_val[0];
                colors[p * 3 + 1] = col_val[1];
                colors[p * 3 + 2] = col_val[2];
            }
        }
    }

    let mut out = geo.clone();
    out.prim.insert("region_id".into(), Arc::new(Attr::i32(1, labels)));
    out.prim
        .insert("region_random".into(), Arc::new(Attr::f32(1, random)));
    out.prim
        .insert("region_center".into(), Arc::new(Attr::f32(3, centers)));
    out.prim.insert("Cd".into(), Arc::new(Attr::f32(3, colors)));
    Ok(out)
}

fn region_transform(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = REGION_TRANSFORM.one(inputs, 0)?;
    for &v in geo.p() {
        if !v.is_finite() {
            return Err(Error::new("region positions must be finite"));
        }
    }
    if geo.topology.has_open() {
        return Err(Error::new("region_transform requires polygon faces"));
    }
    for p_idx in 0..geo.nprims() {
        let size = geo.topology.prim_offsets[p_idx + 1] - geo.topology.prim_offsets[p_idx];
        if size < 3 {
            return Err(Error::new("region_transform requires polygon faces"));
        }
    }

    let seed = p.i("seed")?;
    let geo = if !geo.prim.contains_key("region_id") {
        let id_params = Params::resolve(
            REGION_IDENTIFY.params,
            serde_json::json!({
                "mode": "connectivity",
                "attribute": "class",
                "seed": seed
            })
            .as_object(),
        )?;
        region_identify_impl(&geo, &id_params)?
    } else {
        (*geo).clone()
    };

    let labels_attr = geo
        .prim
        .get("region_id")
        .ok_or_else(|| Error::new("region_id must be a scalar integer primitive attribute"))?;
    if labels_attr.width != 1 {
        return Err(Error::new(
            "region_id must be a scalar integer primitive attribute",
        ));
    }
    let labels = match &labels_attr.data {
        AttrData::I32(v) => v,
        _ => {
            return Err(Error::new(
                "region_id must be a scalar integer primitive attribute",
            ))
        }
    };
    if labels.len() != geo.nprims() {
        return Err(Error::new(
            "region_id must be a scalar integer primitive attribute",
        ));
    }

    let mut keys: Vec<i32> = labels.clone();
    keys.sort_unstable();
    keys.dedup();

    if keys.len() > 10000 || geo.nvertices() > 1_000_000 {
        return Err(Error::new(
            "region processing limit: 10000 regions and 1000000 input vertices",
        ));
    }
    if keys.len() * geo.nvertices().max(1) > 50_000_000 {
        return Err(Error::new(
            "region processing work budget exceeded; process fewer faces or regions",
        ));
    }

    let translate = p.v3("translate")?;
    let rotate = p.v3("rotate")?;
    let scale = p.v3("scale")?;
    let random_translate = p.v3("random_translate")?;
    let random_rotate = p.v3("random_rotate")?;
    let random_scale = p.f("random_scale")?;
    let extrude = p.f("extrude")?;
    let random_extrude = p.f("random_extrude")?;

    let mut parts = Vec::new();
    for &key in &keys {
        let mask: Vec<bool> = labels.iter().map(|&l| l == key).collect();
        let mut part = super::group::keep_prims(&geo, &mask, true)?;

        let mut center = [0.0f64; 3];
        if part.npoints > 0 {
            for c in part.p().chunks_exact(3) {
                center[0] += c[0] as f64;
                center[1] += c[1] as f64;
                center[2] += c[2] as f64;
            }
            center[0] /= part.npoints as f64;
            center[1] /= part.npoints as f64;
            center[2] /= part.npoints as f64;
        }

        let mut rng = super::np_random::Rng::new(&[
            (seed as u64) & 0xFFFF_FFFF,
            (key as u64) & 0xFFFF_FFFF,
        ]);
        let t = [
            translate[0] + (-1.0 + 2.0 * rng.random()) * random_translate[0],
            translate[1] + (-1.0 + 2.0 * rng.random()) * random_translate[1],
            translate[2] + (-1.0 + 2.0 * rng.random()) * random_translate[2],
        ];
        let r = [
            rotate[0] + (-1.0 + 2.0 * rng.random()) * random_rotate[0],
            rotate[1] + (-1.0 + 2.0 * rng.random()) * random_rotate[1],
            rotate[2] + (-1.0 + 2.0 * rng.random()) * random_rotate[2],
        ];
        let s_var = 1.0 + (-random_scale + 2.0 * random_scale * rng.random());
        let s = [scale[0] * s_var, scale[1] * s_var, scale[2] * s_var];
        let d = extrude + (-random_extrude + 2.0 * random_extrude * rng.random());

        if d != 0.0 {
            part = super::model::extrude(&part, "", d, 0.0, "connected", true, None)?;
            part.point.remove("N");
            part.vertex.remove("N");
            part.prim.remove("N");
        }

        let r_rad = [r[0].to_radians(), r[1].to_radians(), r[2].to_radians()];
        let rot_mat = super::pcg::euler_xyz_to_matrix(r_rad);

        let mut new_p = Vec::with_capacity(part.npoints * 3);
        for c in part.p().chunks_exact(3) {
            let p_rel = [
                (c[0] as f64 - center[0]) * s[0],
                (c[1] as f64 - center[1]) * s[1],
                (c[2] as f64 - center[2]) * s[2],
            ];
            let px = rot_mat[0][0] * p_rel[0]
                + rot_mat[0][1] * p_rel[1]
                + rot_mat[0][2] * p_rel[2]
                + center[0]
                + t[0];
            let py = rot_mat[1][0] * p_rel[0]
                + rot_mat[1][1] * p_rel[1]
                + rot_mat[1][2] * p_rel[2]
                + center[1]
                + t[1];
            let pz = rot_mat[2][0] * p_rel[0]
                + rot_mat[2][1] * p_rel[1]
                + rot_mat[2][2] * p_rel[2]
                + center[2]
                + t[2];
            if !px.is_finite() || !py.is_finite() || !pz.is_finite() {
                return Err(Error::new("transformed region positions must be finite"));
            }
            if px.abs() > f32::MAX as f64
                || py.abs() > f32::MAX as f64
                || pz.abs() > f32::MAX as f64
            {
                return Err(Error::new(
                    "transformed region positions exceed float32 range",
                ));
            }
            new_p.extend([px as f32, py as f32, pz as f32]);
        }
        part.set_p(new_p);

        let transform_normals = |attr: &Attr| -> Attr {
            if let AttrData::F32(vals) = &attr.data {
                let mut out = Vec::with_capacity(vals.len());
                for c in vals.chunks_exact(attr.width) {
                    let n_unscaled = [c[0] as f64 / s[0], c[1] as f64 / s[1], c[2] as f64 / s[2]];
                    let nx = rot_mat[0][0] * n_unscaled[0]
                        + rot_mat[0][1] * n_unscaled[1]
                        + rot_mat[0][2] * n_unscaled[2];
                    let ny = rot_mat[1][0] * n_unscaled[0]
                        + rot_mat[1][1] * n_unscaled[1]
                        + rot_mat[1][2] * n_unscaled[2];
                    let nz = rot_mat[2][0] * n_unscaled[0]
                        + rot_mat[2][1] * n_unscaled[1]
                        + rot_mat[2][2] * n_unscaled[2];
                    let len = (nx * nx + ny * ny + nz * nz).sqrt();
                    let inv = if len < 1e-20 { 1.0 } else { 1.0 / len };
                    out.extend([(nx * inv) as f32, (ny * inv) as f32, (nz * inv) as f32]);
                }
                Attr {
                    width: attr.width,
                    data: AttrData::F32(out),
                }
            } else {
                attr.clone()
            }
        };

        if let Some(n_attr) = part.point.get("N").cloned() {
            part.point.insert("N".into(), Arc::new(transform_normals(&n_attr)));
        }
        if let Some(n_attr) = part.vertex.get("N").cloned() {
            part.vertex
                .insert("N".into(), Arc::new(transform_normals(&n_attr)));
        }
        if let Some(n_attr) = part.prim.get("N").cloned() {
            part.prim.insert("N".into(), Arc::new(transform_normals(&n_attr)));
        }

        part.prim
            .insert("region_id".into(), Arc::new(Attr::i32(1, vec![key; part.nprims()])));
        let mut region_centers = Vec::with_capacity(part.nprims() * 3);
        for _ in 0..part.nprims() {
            region_centers.extend([
                (center[0] + t[0]) as f32,
                (center[1] + t[1]) as f32,
                (center[2] + t[2]) as f32,
            ]);
        }
        part.prim
            .insert("region_center".into(), Arc::new(Attr::f32(3, region_centers)));
        parts.push(part);
    }

    if parts.is_empty() {
        Ok(geo)
    } else {
        super::merge::merge(
            &[parts.into_iter().map(Arc::new).collect()],
            &Params::resolve(&[], None)?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad() -> Geometry {
        let p = vec![0.0, 0.0, 0.0, 8.0, 0.0, 0.0, 8.0, 6.0, 0.0, 0.0, 6.0, 0.0];
        let vp = vec![0, 1, 2, 3];
        let offs = vec![0, 4];
        Geometry::from_arrays(p, vp, offs, None).unwrap()
    }

    #[test]
    fn test_building_rules_counts_and_groups() {
        let q = quad();
        let p = Params::resolve(
            BUILDING_RULES.params,
            serde_json::json!({
                "storeys": 3,
                "bay_width": 2.0,
                "balcony_probability": 0.0
            })
            .as_object(),
        )
        .unwrap();
        let out = building_rules(&[vec![Arc::new(q)]], &p).unwrap();
        let crate::geometry::AttrData::I32(comp) = &out.prim["component"].data else {
            panic!("component must be an integer attribute");
        };
        assert_eq!(comp.iter().filter(|&&c| c == 0).count(), 42);
        assert_eq!(comp.iter().filter(|&&c| c == 1).count(), 42);
        assert_eq!(comp.iter().filter(|&&c| c == 2).count(), 0);
        assert_eq!(comp.iter().filter(|&&c| c == 3).count(), 1);
        assert_eq!(comp.iter().filter(|&&c| c == 4).count(), 1);

        let max_z = out.p().chunks_exact(3).map(|c| c[2]).fold(f32::NEG_INFINITY, f32::max);
        assert!((max_z - 9.0).abs() < 1e-5);
    }

    #[test]
    fn test_region_identify_and_transform() {
        let q = quad();
        let p_id = Params::resolve(
            REGION_IDENTIFY.params,
            serde_json::json!({"mode": "primitive"}).as_object(),
        )
        .unwrap();
        let marked = region_identify(&[vec![Arc::new(q.clone())]], &p_id).unwrap();
        assert_eq!(marked.nprims(), 1);
        let AttrData::I32(region_id) = &marked.prim["region_id"].data else {
            panic!("region_id must be I32");
        };
        assert_eq!(region_id, &[0]);
        assert!(marked.prim.contains_key("region_random"));
        assert!(marked.prim.contains_key("region_center"));
        assert!(marked.prim.contains_key("Cd"));

        let p_xform = Params::resolve(
            REGION_TRANSFORM.params,
            serde_json::json!({"translate": [1.0, 2.0, 3.0]}).as_object(),
        )
        .unwrap();
        let xformed = region_transform(&[vec![Arc::new(marked)]], &p_xform).unwrap();
        assert_eq!(xformed.npoints, 4);
        assert!((xformed.p()[0] - 1.0).abs() < 1e-5);
        assert!((xformed.p()[1] - 2.0).abs() < 1e-5);
        assert!((xformed.p()[2] - 3.0).abs() < 1e-5);
    }

    #[test]
    fn test_region_transform_extrude() {
        let q = quad();
        let p_xform = Params::resolve(
            REGION_TRANSFORM.params,
            serde_json::json!({"extrude": 0.5}).as_object(),
        )
        .unwrap();
        let xformed = region_transform(&[vec![Arc::new(q)]], &p_xform).unwrap();
        // Quad extruded with back=true produces 6 faces (1 front, 1 back, 4 sides)
        assert_eq!(xformed.nprims(), 6);
    }
}
