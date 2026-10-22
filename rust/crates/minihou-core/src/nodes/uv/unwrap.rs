use std::sync::Arc;

use crate::{
    error::{Error, Result},
    geometry::{Attr, AttrData, Geometry},
    nodes::{model, modeling_tools, Inputs, NodeSpec, GEO},
    params::Params,
};

mod islands;
mod guess;
mod lscm;

use islands::{face_islands, make_discs};
use guess::get_pins;
use lscm::lscm;
use super::packing::pack;

pub const UV_FLATTEN: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "uv_flatten",
    inputs: GEO,
    params: crate::params!(
        "seam_angle": 70.0;
        "split_attrib": "";
        "padding": 0.02;
        "island_attrib": "island"
    ),
    cook: flatten_uv,
};

fn flatten_uv(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = UV_FLATTEN.one(inputs, 0)?;
    if geo.topology.has_open() {
        let n = (0..geo.nprims()).filter(|&i| !geo.topology.is_closed(i)).count();
        return Err(Error::new(format!(
            "uv_flatten works on polygons, but the input has {n} open curve(s); remove them first, e.g. blast with group '@closed == 0'"
        )));
    }
    if geo.nprims() == 0 {
        return Ok((*geo).clone());
    }

    let seam_angle = p.f("seam_angle")?;
    let split_attrib = p.s("split_attrib")?;
    let padding = p.f("padding")?;
    let island_attrib = p.s("island_attrib")?;

    let nprims = geo.nprims();
    let nv = geo.nvertices();
    let topo = &geo.topology;
    let vp = &topo.vertex_point;
    let (pv, nx, _) = model::corners(&geo);
    let tw = modeling_tools::twins(&geo);

    let pn = crate::normals::prim_normals(geo.p(), topo);
    let un: Vec<[f64; 3]> = pn.iter().map(|n| {
        let mut len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len < 1e-20 { len = 1.0; }
        [n[0] / len, n[1] / len, n[2] / len]
    }).collect();

    let cos_thresh = seam_angle.to_radians().cos() - 1e-9;
    let mut seam = vec![false; nv];
    for v in 0..nv {
        match tw[v] {
            None => seam[v] = true,
            Some(tw_v) => {
                let p1 = pv[v];
                let p2 = pv[tw_v];
                let dot = un[p1][0] * un[p2][0] + un[p1][1] * un[p2][1] + un[p1][2] * un[p2][2];
                if dot < cos_thresh {
                    seam[v] = true;
                }
            }
        }
    }

    if !split_attrib.is_empty() {
        let attr = geo.prim.get(&split_attrib).ok_or_else(|| {
            Error::new(format!("no primitive attribute '{split_attrib}' to split islands by"))
        })?;
        let w = attr.width;
        match &attr.data {
            AttrData::F32(data) => {
                for v in 0..nv {
                    if let Some(tw_v) = tw[v] {
                        let p1 = pv[v];
                        let p2 = pv[tw_v];
                        if data[p1 * w..(p1 + 1) * w] != data[p2 * w..(p2 + 1) * w] {
                            seam[v] = true;
                        }
                    }
                }
            }
            AttrData::I32(data) => {
                for v in 0..nv {
                    if let Some(tw_v) = tw[v] {
                        let p1 = pv[v];
                        let p2 = pv[tw_v];
                        if data[p1 * w..(p1 + 1) * w] != data[p2 * w..(p2 + 1) * w] {
                            seam[v] = true;
                        }
                    }
                }
            }
        }
    }

    let init_label = face_islands(nprims, nv, &pv, &tw, &seam);
    let label = make_discs(&geo, &pv, &nx, &tw, &seam, &pn, init_label, 6);
    let k = if label.is_empty() { 0 } else { *label.iter().max().unwrap() + 1 };

    let npoints = geo.npoints.max(1) as u64;
    let mut keys = Vec::with_capacity(nv);
    for v in 0..nv {
        let isl = label[pv[v]] as u64;
        let pt = vp[v] as u64;
        keys.push(isl * npoints + pt);
    }
    let mut ukeys = keys.clone();
    ukeys.sort_unstable();
    ukeys.dedup();
    let nu = ukeys.len();

    let mut uid = Vec::with_capacity(nv);
    for v in 0..nv {
        let idx = ukeys.binary_search(&keys[v]).unwrap();
        uid.push(idx);
    }

    let island_of: Vec<usize> = ukeys.iter().map(|&k| (k / npoints) as usize).collect();
    let p_coords = geo.p();
    let pu: Vec<[f64; 3]> = ukeys.iter().map(|&k| {
        let pt = (k % npoints) as usize;
        [p_coords[pt * 3] as f64, p_coords[pt * 3 + 1] as f64, p_coords[pt * 3 + 2] as f64]
    }).collect();

    let mut tris_v = Vec::new();
    let mut tri_prim = Vec::new();
    for i in 0..nprims {
        let s = topo.prim_offsets[i] as usize;
        let e = topo.prim_offsets[i + 1] as usize;
        let count = (e - s).saturating_sub(2);
        for j in 0..count {
            tris_v.push([s, s + j + 1, s + j + 2]);
            tri_prim.push(i);
        }
    }
    let tris_u: Vec<[usize; 3]> = tris_v.iter().map(|tri| [
        uid[tri[0]],
        uid[tri[1]],
        uid[tri[2]],
    ]).collect();

    let (pins, pin_uv) = get_pins(&pu, &island_of, k);
    let mut uv = lscm(&pu, &tris_u, &pins, &pin_uv, &island_of, k);

    let mut a3_sum = vec![0f64; k];
    let mut a2_sum = vec![0f64; k];
    for (t, tri) in tris_u.iter().enumerate() {
        let u0 = tri[0];
        let u1 = tri[1];
        let u2 = tri[2];
        let p0 = pu[u0];
        let p1 = pu[u1];
        let p2 = pu[u2];
        let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let cross = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let a3 = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt() / 2.0;

        let uv0 = uv[u0];
        let uv1 = uv[u1];
        let uv2 = uv[u2];
        let du1 = [uv1[0] - uv0[0], uv1[1] - uv0[1]];
        let du2 = [uv2[0] - uv0[0], uv2[1] - uv0[1]];
        let a2 = (du1[0] * du2[1] - du1[1] * du2[0]) / 2.0;

        let isl = label[tri_prim[t]];
        a3_sum[isl] += a3;
        a2_sum[isl] += a2;
    }

    let mut flip = vec![false; k];
    let mut scale = vec![1.0f64; k];
    for i in 0..k {
        flip[i] = a2_sum[i] < 0.0;
        let abs_a2 = a2_sum[i].abs();
        if abs_a2 > 1e-30 {
            scale[i] = (a3_sum[i] / abs_a2.max(1e-30)).sqrt();
        }
    }

    for u in 0..nu {
        let isl = island_of[u];
        if flip[isl] {
            uv[u][0] = -uv[u][0];
        }
    }

    let uv = pack(uv, &island_of, &scale, k, padding);

    let mut out = (*geo).clone();
    out.point.remove("uv");
    out.prim.remove("uv");

    let mut flat_uv = Vec::with_capacity(nv * 2);
    for v in 0..nv {
        let u_idx = uid[v];
        flat_uv.push(uv[u_idx][0] as f32);
        flat_uv.push(uv[u_idx][1] as f32);
    }
    out.vertex.insert("uv".into(), Arc::new(Attr::f32(2, flat_uv)));

    if !island_attrib.is_empty() {
        let label_i32: Vec<i32> = label.iter().map(|&l| l as i32).collect();
        out.prim.insert(island_attrib, Arc::new(Attr::i32(1, label_i32)));
    }

    Ok(out)
}

#[cfg(test)]
mod tests{
    use super::*;

    use super::super::transform::tests::make_test_grid;

    fn table(v: &serde_json::Value) -> std::collections::BTreeMap<String, Arc<Attr>> {
        v.as_object()
            .unwrap()
            .iter()
            .map(|(name, attr)| {
                let width = attr["width"].as_i64().unwrap() as usize;
                let rows = attr["data"].as_array().unwrap();
                let attr = if attr["dtype"].as_str().unwrap() == "float32" {
                    Attr::f32(
                        width,
                        rows.iter().map(|x| x.as_f64().unwrap() as f32).collect(),
                    )
                } else {
                    Attr::i32(
                        width,
                        rows.iter().map(|x| x.as_i64().unwrap() as i32).collect(),
                    )
                };
                (name.clone(), Arc::new(attr))
            })
            .collect()
    }

    fn geometry_from_json(v: &serde_json::Value) -> Geometry {
        let point = table(&v["point"]);
        let closed = v["closed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_bool().unwrap())
            .collect();
        let topo = crate::geometry::Topology {
            vertex_point: v["vp"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_i64().unwrap() as i32)
                .collect(),
            prim_offsets: v["offsets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_i64().unwrap() as i32)
                .collect(),
            closed: Some(closed),
        ..Default::default()};
        let g = Geometry { packed: Default::default(), edge: Default::default(),
            npoints: point["P"].rows(),
            topology: Arc::new(topo),
            point,
            vertex: table(&v["vertex"]),
            prim: table(&v["prim"]),
            detail: v["detail"].as_object().unwrap().clone(),
        };
        g.validate().unwrap();
        g
    }

    #[test]
    fn test_uv_flatten_matches_python_parity() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../test_data/uv_flatten_parity.json")).unwrap();
        for c in cases.as_array().unwrap() {
            let name = c["name"].as_str().unwrap();
            let spec = &UV_FLATTEN;
            let defaults = c["defaults"].as_object().unwrap();
            assert_eq!(spec.params.len(), defaults.len(), "{name}: default count");
            for (pname, value) in defaults {
                let p = spec.params.iter().find(|p| p.name == pname).unwrap_or_else(|| panic!("missing Python parameter {pname}"));
                assert!(crate::params::same_value(value, &(p.default)()), "{name}: default {}", p.name);
            }
            let inputs: Vec<_> = c["inputs"].as_array().unwrap().iter().map(|v| {
                if v.is_null() { vec![] } else { vec![Arc::new(geometry_from_json(v))] }
            }).collect();
            let result = Params::resolve(spec.params, c["params"].as_object()).and_then(|p| (spec.cook)(&inputs, &p));
            if c.get("error").is_some() {
                assert!(result.is_err(), "{name}: expected error");
                continue;
            }
            let out = result.unwrap_or_else(|e| panic!("{name}: {e}"));
            out.validate().unwrap();
            let expected = geometry_from_json(&c["expected"]);
            assert_eq!(out.npoints, expected.npoints, "{name}: point count");
            assert_eq!(out.topology.vertex_point, expected.topology.vertex_point, "{name}: corner points");
            assert_eq!(out.topology.prim_offsets, expected.topology.prim_offsets, "{name}: offsets");
            assert_eq!(out.topology.closed_mask(), expected.topology.closed_mask(), "{name}: closed flags");

            let out_uv = out.vertex["uv"].as_f32().unwrap();
            let exp_uv = expected.vertex["uv"].as_f32().unwrap();
            assert_eq!(out_uv.len(), exp_uv.len(), "{name}: uv length");
            for i in 0..out_uv.len() {
                assert!(
                    (out_uv[i] - exp_uv[i]).abs() <= 2e-3,
                    "{name}: uv[{i}] Rust {} vs Python {}", out_uv[i], exp_uv[i]
                );
            }

            if expected.prim.contains_key("island") {
                let out_isl = out.prim["island"].as_i32().unwrap();
                let exp_isl = expected.prim["island"].as_i32().unwrap();
                assert_eq!(out_isl, exp_isl, "{name}: island prim attrib");
            }
        }
    }

    #[test]
    fn test_uv_flatten_grid_developable() {
        let grid = make_test_grid();
        let p = Params::resolve(UV_FLATTEN.params, serde_json::json!({}).as_object()).unwrap();
        let out = flatten_uv(&[vec![Arc::new(grid)]], &p).unwrap();
        out.validate().unwrap();
        let uv = out.vertex["uv"].as_f32().unwrap();
        assert!(uv.iter().all(|&v| v >= 0.0 && v <= 1.0));
        assert_eq!(out.prim["island"].as_i32().unwrap(), &[0, 0, 0, 0]);
    }

    #[test]
    fn test_uv_flatten_errors() {
        let grid = make_test_grid();
        // Unknown split attribute
        let p_err_split = Params::resolve(
            UV_FLATTEN.params,
            serde_json::json!({"split_attrib": "nonexistent"}).as_object(),
        )
        .unwrap();
        assert!(flatten_uv(&[vec![Arc::new(grid)]], &p_err_split).is_err());

        // Open curve geometry
        let curve = Geometry::from_arrays(
            vec![0., 0., 0., 1., 1., 1.],
            vec![0, 1],
            vec![0, 2],
            Some(vec![false]),
        )
        .unwrap();
        let p_default = Params::resolve(UV_FLATTEN.params, serde_json::json!({}).as_object()).unwrap();
        let err = flatten_uv(&[vec![Arc::new(curve)]], &p_default).unwrap_err();
        assert!(err.to_string().contains("open curve"));
    }

    #[test]
    fn test_uv_flatten_edge_cases() {
        // 1. Empty geometry (0 points, 0 prims)
        let empty = Geometry::points_only(vec![]).unwrap();
        let p = Params::resolve(UV_FLATTEN.params, serde_json::json!({}).as_object()).unwrap();
        let out_empty = flatten_uv(&[vec![Arc::new(empty)]], &p).unwrap();
        assert_eq!(out_empty.npoints, 0);
        assert_eq!(out_empty.nprims(), 0);

        // 2. Single triangle
        let tri = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 0., 1., 0.],
            vec![0, 1, 2],
            vec![0, 3],
            None,
        ).unwrap();
        let out_tri = flatten_uv(&[vec![Arc::new(tri)]], &p).unwrap();
        assert_eq!(out_tri.nprims(), 1);
        let uv_tri = out_tri.vertex["uv"].as_f32().unwrap();
        assert_eq!(uv_tri.len(), 6); // 3 vertices * 2 coords
        for &coord in uv_tri {
            assert!(coord >= 0.0 && coord <= 1.0, "UV in unit square: {coord}");
        }

        // 3. Degenerate zero-area triangle: collinear vertices
        let degen = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 2., 0., 0.],
            vec![0, 1, 2],
            vec![0, 3],
            None,
        ).unwrap();
        // Degenerate triangle must either succeed safely with clamped UVs or return an error, never panic or hang
        let _ = flatten_uv(&[vec![Arc::new(degen)]], &p);

        // 4. Determinism
        let grid = make_test_grid();
        let out1 = flatten_uv(&[vec![Arc::new(grid.clone())]], &p).unwrap();
        let out2 = flatten_uv(&[vec![Arc::new(grid.clone())]], &p).unwrap();
        assert_eq!(out1.vertex["uv"].data, out2.vertex["uv"].data);
        assert_eq!(out1.prim["island"].data, out2.prim["island"].data);

        // 5. Attribute preservation
        let mut grid_attr = grid;
        grid_attr.point.insert("Cd".into(), Arc::new(Attr::f32(3, vec![1.0; grid_attr.npoints * 3])));
        grid_attr.prim.insert("material_index".into(), Arc::new(Attr::i32(1, vec![5; grid_attr.nprims()])));
        let out_attr = flatten_uv(&[vec![Arc::new(grid_attr.clone())]], &p).unwrap();
        assert_eq!(out_attr.point["Cd"].data, grid_attr.point["Cd"].data);
        assert_eq!(out_attr.prim["material_index"].data, grid_attr.prim["material_index"].data);
        assert_eq!(out_attr.topology.vertex_point, grid_attr.topology.vertex_point);
    }
}

