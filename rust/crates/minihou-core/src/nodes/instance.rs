//! Instancing: points that become instances of Blender objects (minihou/nodes/instance.py).

use std::sync::Arc;

use super::{np_random::Rng, InputSpec, Inputs, NodeSpec};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::Params;

pub const INSTANCE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "instance",
    inputs: &[InputSpec {
        name: "points",
        multi: false,
        optional: false,
    }],
    params: crate::params!(
        "objects": "";
        "pick": "random";
        "pick_attrib": "variant";
        "seed": 0;
        "align": "normal";
        "random_spin": 360.0;
        "scale_min": 1.0;
        "scale_max": 1.0;
        "group": ""
    ),
    cook: instance,
};

pub fn matrix_to_euler_xyz(r: &[[f64; 3]; 3]) -> [f64; 3] {
    let cy = (r[0][0] * r[0][0] + r[1][0] * r[1][0]).sqrt();
    let y = (-r[2][0]).atan2(cy);
    if cy > 1e-9 {
        let x = r[2][1].atan2(r[2][2]);
        let z = r[1][0].atan2(r[0][0]);
        [x, y, z]
    } else {
        let x = (-r[1][2]).atan2(r[1][1]);
        let z = 0.0;
        [x, y, z]
    }
}

#[allow(dead_code)]
pub fn euler_xyz_to_matrix(e: [f64; 3]) -> [[f64; 3]; 3] {
    let (sx, cx) = e[0].sin_cos();
    let (sy, cy) = e[1].sin_cos();
    let (sz, cz) = e[2].sin_cos();
    [
        [cy * cz, sx * sy * cz - cx * sz, cx * sy * cz + sx * sz],
        [cy * sz, sx * sy * sz + cx * cz, cx * sy * sz - sx * cz],
        [-sy, sx * cy, cx * cy],
    ]
}

fn mat_mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

fn rotation_from_z_to(n: [f64; 3]) -> [[f64; 3]; 3] {
    let [nx, ny, c] = n;
    if c < -1.0 + 1e-9 {
        return [[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]];
    }
    let (x, y, s) = (-ny, nx, 1.0 / (1.0 + c));
    [
        [1.0 - s * y * y, s * x * y, y],
        [s * x * y, 1.0 - s * x * x, -x],
        [-y, x, 1.0 - s * (x * x + y * y)],
    ]
}

fn instance(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let points = INSTANCE.one(inputs, 0)?;
    let objects_str = p.s("objects")?;
    let names: Vec<String> = objects_str
        .replace(',', " ")
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();
    if names.is_empty() {
        return Err(Error::new("list the Blender objects to instance, e.g. 'Tree Rock'"));
    }
    let n = points.npoints;
    let seed = p.i("seed")?;
    let mut rng = Rng::new(&[(seed as u64) & 0xFFFF_FFFF]);
    let pick = p.e("pick", &["random", "cycle", "attribute"])?;
    let pick_attrib = p.s("pick_attrib")?;

    let idx: Vec<usize> = match pick.as_str() {
        "random" => (0..n).map(|_| rng.integers(names.len())).collect(),
        "cycle" => (0..n).map(|i| i % names.len()).collect(),
        "attribute" => {
            let a = points.point.get(&pick_attrib).ok_or_else(|| {
                Error::new(format!("no point attribute {:?} to pick objects by", pick_attrib))
            })?;
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let v = match &a.data {
                    AttrData::I32(vals) => vals.get(i * a.width).copied().unwrap_or(0) as i64,
                    AttrData::F32(vals) => vals.get(i * a.width).copied().unwrap_or(0.0) as i64,
                };
                out.push(v.rem_euclid(names.len() as i64) as usize);
            }
            out
        }
        _ => unreachable!(),
    };

    let random_spin = p.f("random_spin")?;
    let align = p.e("align", &["normal", "up"])?;
    let scale_min = p.f("scale_min")?;
    let scale_max = p.f("scale_max")?;
    let group = p.s("group")?;

    let mut rot_flat = Vec::with_capacity(n * 3);
    let n_attr = points.point.get("N");

    for i in 0..n {
        let spin = (rng.random() * random_spin).to_radians();
        let (s_sin, s_cos) = spin.sin_cos();
        let r_spin = [
            [s_cos, -s_sin, 0.0],
            [s_sin, s_cos, 0.0],
            [0.0, 0.0, 1.0],
        ];

        let r = if align == "normal" {
            if let Some(attr) = n_attr {
                let n_vec = match &attr.data {
                    AttrData::F32(v) => [
                        v[i * attr.width] as f64,
                        v[i * attr.width + 1] as f64,
                        v[i * attr.width + 2] as f64,
                    ],
                    AttrData::I32(v) => [
                        v[i * attr.width] as f64,
                        v[i * attr.width + 1] as f64,
                        v[i * attr.width + 2] as f64,
                    ],
                };
                let len = (n_vec[0] * n_vec[0] + n_vec[1] * n_vec[1] + n_vec[2] * n_vec[2])
                    .sqrt()
                    .max(1e-30);
                let n_norm = [n_vec[0] / len, n_vec[1] / len, n_vec[2] / len];
                let r_norm = rotation_from_z_to(n_norm);
                mat_mul(&r_norm, &r_spin)
            } else {
                r_spin
            }
        } else {
            r_spin
        };
        let euler = matrix_to_euler_xyz(&r);
        rot_flat.extend([euler[0] as f32, euler[1] as f32, euler[2] as f32]);
    }

    let pscale_attr = points.point.get("pscale");
    let mut scale_flat = Vec::with_capacity(n * 3);
    for i in 0..n {
        let mut s = scale_min + rng.random() * (scale_max - scale_min);
        if let Some(attr) = pscale_attr {
            let pscale_val = match &attr.data {
                AttrData::F32(v) => v.get(i * attr.width).copied().unwrap_or(1.0) as f64,
                AttrData::I32(v) => v.get(i * attr.width).copied().unwrap_or(1) as f64,
            };
            s *= pscale_val;
        }
        let s_f = s as f32;
        scale_flat.extend([s_f, s_f, s_f]);
    }

    let sel = super::groups::point_mask(&points, &group)?;
    let mut inst = Vec::with_capacity(n);
    for i in 0..n {
        if sel[i] {
            inst.push((idx[i] + 1) as i32);
        } else {
            inst.push(0);
        }
    }

    let mut out = (*points).clone();
    out.point.insert("instance".into(), Arc::new(Attr::i32(1, inst)));
    out.point.insert("rot".into(), Arc::new(Attr::f32(3, rot_flat)));
    out.point.insert("scale".into(), Arc::new(Attr::f32(3, scale_flat)));
    out.detail.insert(
        "instance_objects".into(),
        serde_json::Value::Array(names.into_iter().map(serde_json::Value::String).collect()),
    );
    out.detail.remove("instance_bounds");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instance_node_basic() {
        let p = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let mut g = Geometry::points_only(p).unwrap();
        g.point.insert(
            "N".into(),
            Arc::new(Attr::f32(3, vec![0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0])),
        );
        let params = Params::resolve(
            INSTANCE.params,
            serde_json::json!({
                "objects": "Tree Rock",
                "random_spin": 0.0,
                "scale_min": 2.0,
                "scale_max": 3.0,
                "pick": "cycle"
            })
            .as_object(),
        )
        .unwrap();
        let out = instance(&[vec![Arc::new(g)]], &params).unwrap();
        assert_eq!(out.npoints, 3);
        let AttrData::I32(inst_attr) = &out.point["instance"].data else {
            panic!("instance must be I32");
        };
        assert_eq!(inst_attr, &[1, 2, 1]);
        let scale_attr = out.point["scale"].as_f32().unwrap();
        for &s in scale_attr {
            assert!(s >= 2.0 && s <= 3.0);
        }
        let objs = out.detail["instance_objects"].as_array().unwrap();
        assert_eq!(objs.len(), 2);
        assert_eq!(objs[0].as_str().unwrap(), "Tree");
        assert_eq!(objs[1].as_str().unwrap(), "Rock");
    }

    #[test]
    fn test_euler_round_trip() {
        let angles = [0.1, -0.2, 0.5];
        let mat = euler_xyz_to_matrix(angles);
        let got = matrix_to_euler_xyz(&mat);
        for i in 0..3 {
            assert!((got[i] - angles[i]).abs() < 1e-8);
        }
    }
}
