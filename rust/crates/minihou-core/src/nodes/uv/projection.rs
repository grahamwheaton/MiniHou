use std::sync::Arc;

use crate::{
    error::Result,
    geometry::{Attr, Geometry},
    math,
    nodes::{group, Inputs, NodeSpec, GEO},
    params::Params,
};

pub const UV_PROJECT: NodeSpec = NodeSpec {
    outputs: &["geometry"],
    cook_multi: None,
    cook_sidecar: None,
    name: "uv_project",
    inputs: GEO,
    params: crate::params!(
        "method": "box";
        "center": [0., 0., 0.];
        "rotate": [0., 0., 0.];
        "scale": [1., 1., 1.];
        "rotate_uv": 0.;
        "group": ""
    ),
    cook: project,
};

fn project(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = UV_PROJECT.one(inputs, 0)?;
    let method = p.e("method", &["box", "planar", "cylindrical", "spherical"])?;
    let center = p.v3("center")?;
    let r = math::transpose(&math::euler_xyz(p.v3("rotate")?));
    let scale = p.v3("scale")?.map(|v| if v.abs() < 1e-12 { 1. } else { v });
    let local: Vec<[f64; 3]> = g
        .topology
        .vertex_point
        .iter()
        .map(|&i| {
            let q = std::array::from_fn(|k| g.p()[i as usize * 3 + k] as f64 - center[k]);
            let q = math::apply(&r, q);
            std::array::from_fn(|k| q[k] / scale[k])
        })
        .collect();
    let max = local.iter().flatten().map(|v| v.abs()).fold(0f64, f64::max).max(1.);
    let normals = crate::normals::normalize_to_f32(&crate::normals::prim_normals(g.p(), &g.topology));
    let mut uv = vec![[0f64; 2]; g.nvertices()];
    let mut on_axis = vec![false; g.nvertices()];
    for i in 0..g.nprims() {
        let normal = math::apply(&r, std::array::from_fn(|k| normals[i * 3 + k] as f64));
        let mut axis = 0;
        for k in 1..3 {
            if normal[k].abs() > normal[axis].abs() {
                axis = k;
            }
        }
        let sign = if normal[axis] < 0. { -1. } else { 1. };
        let start = g.topology.prim_offsets[i] as usize;
        let end = g.topology.prim_offsets[i + 1] as usize;
        for v in start..end {
            let q = local[v];
            uv[v] = match method.as_str() {
                "planar" => [q[0] + 0.5, q[1] + 0.5],
                "box" => match axis {
                    0 => [q[1] * sign + 0.5, q[2] + 0.5],
                    1 => [-q[0] * sign + 0.5, q[2] + 0.5],
                    _ => [q[0] * sign + 0.5, q[1] + 0.5],
                },
                "cylindrical" => {
                    on_axis[v] = q[0].hypot(q[1]) < 1e-9 * max;
                    [
                        (q[1].atan2(q[0]) / (2. * std::f64::consts::PI)).rem_euclid(1.),
                        q[2] + 0.5,
                    ]
                }
                _ => {
                    let radius = q.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-12);
                    on_axis[v] = q[0].hypot(q[1]) < 1e-6 * radius;
                    [
                        (q[1].atan2(q[0]) / (2. * std::f64::consts::PI)).rem_euclid(1.),
                        1. - (q[2] / radius).clamp(-1., 1.).acos() / std::f64::consts::PI,
                    ]
                }
            };
        }
        if method == "cylindrical" || method == "spherical" {
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for v in start..end {
                if !on_axis[v] {
                    lo = lo.min(uv[v][0]);
                    hi = hi.max(uv[v][0]);
                }
            }
            if hi - lo > 0.5 {
                for v in start..end {
                    if uv[v][0] < 0.5 {
                        uv[v][0] += 1.;
                    }
                }
            }
            let mut sum = 0.;
            let mut n = 0;
            for v in start..end {
                if !on_axis[v] {
                    sum += uv[v][0];
                    n += 1;
                }
            }
            for v in start..end {
                if on_axis[v] {
                    uv[v][0] = sum / n.max(1) as f64;
                }
            }
        }
    }
    let (sn, cs) = p.f("rotate_uv")?.to_radians().sin_cos();
    let sel = group::mask(&g, "prim", &p.s("group")?)?;
    let old_vertex = g.vertex.get("uv").filter(|a| a.width == 2).and_then(|a| a.as_f32());
    let old_point = g.point.get("uv").filter(|a| a.width == 2).and_then(|a| a.as_f32());
    let restricted = p.s("group")?.split(|c: char| c == ',' || c.is_whitespace()).any(|s| !s.is_empty());
    let mut values = Vec::with_capacity(g.nvertices() * 2);
    for (i, &selected) in sel.iter().enumerate() {
        for v in g.topology.prim_offsets[i] as usize..g.topology.prim_offsets[i + 1] as usize {
            if restricted && !selected {
                let old = if let Some(a) = old_vertex {
                    [a[v * 2], a[v * 2 + 1]]
                } else if let Some(a) = old_point {
                    let pt = g.topology.vertex_point[v] as usize;
                    [a[pt * 2], a[pt * 2 + 1]]
                } else {
                    [0.; 2]
                };
                values.extend(old);
            } else {
                let [x, y] = uv[v].map(|v| v - 0.5);
                values.extend([(x * cs - y * sn + 0.5) as f32, (x * sn + y * cs + 0.5) as f32]);
            }
        }
    }
    let mut out = (*g).clone();
    out.point.remove("uv");
    out.prim.remove("uv");
    out.vertex.insert("uv".into(), Arc::new(Attr::f32(2, values)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planar_and_group_preserve_point_uv() {
        let mut g = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 0., 1., 0.],
            vec![0, 1, 2, 2, 1, 0],
            vec![0, 3, 6],
            None,
        )
        .unwrap();
        g.point.insert("uv".into(), Arc::new(Attr::f32(2, vec![2.; 6])));
        let p = Params::resolve(
            UV_PROJECT.params,
            serde_json::json!({"method": "planar", "group": "0"}).as_object(),
        )
        .unwrap();
        let out = project(&[vec![Arc::new(g)]], &p).unwrap();
        assert_eq!(
            out.vertex["uv"].as_f32().unwrap(),
            [0.5, 0.5, 1.5, 0.5, 0.5, 1.5, 2., 2., 2., 2., 2., 2.]
        );
        assert!(!out.point.contains_key("uv"));
        out.validate().unwrap();
    }

    #[test]
    fn seam_crossing_uses_continuous_u() {
        let g = Geometry::from_arrays(
            vec![1., -0.1, 0., 1., 0.1, 0., 1., 0.2, 1.],
            vec![0, 1, 2],
            vec![0, 3],
            None,
        )
        .unwrap();
        let p = Params::resolve(
            UV_PROJECT.params,
            serde_json::json!({"method": "cylindrical"}).as_object(),
        )
        .unwrap();
        let out = project(&[vec![Arc::new(g)]], &p).unwrap();
        let u = out.vertex["uv"].as_f32().unwrap();
        assert!(u[2] > 1.);
        assert!((u[2] - u[0]).abs() < 0.1);
    }
}
