//! Houdini-parity `polyfill` SOP: fills holes in polygon geometry.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::mesh_common::{append, NewPoint};
use super::{edge_tools, groups, model, Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::{ParamSpec, Params};
use crate::topo;

mod boundary;
mod cook;
mod patch;

use boundary::{extract_loops, get_boundary_mask};
pub(crate) use cook::polyfill;
use patch::{auto_layout, ear_clip_triangles, params_along};

pub const PARAMS: &[ParamSpec] = crate::params!(
    "boundary_group": "";
    "group": "";
    "fill_group": "";
    "mode": "quadrilaterals";
    "fill_mode": "quadrilaterals";
    "auto_complete_boundaries": true;
    "autocomplete": true;
    "reverse_patches": false;
    "reverse": false;
    "unique_points": false;
    "uniquepoints": false;
    "update_point_normals": true;
    "updatenormals": true;
    "edge_loop": 0.25;
    "edgeloop": 0.25;
    "smooth": 50.0;
    "corner_offset": 0;
    "custom_corners": "";
    "deform_patch": true;
    "deform": true;
    "surface_offset": 0.0;
    "tangent_strength": 0.4;
    "patch_group": "";
    "patchgroup": "";
    "edge_loop_group": "loopgroup";
    "edgeloopgroup": "loopgroup";
    "append_to_groups": false;
    "append_groups": false;
);

pub const POLYFILL: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "polyfill",
    inputs: GEO,
    params: PARAMS,
    cook: polyfill,
};

fn get_str_opt(p: &Params, names: &[&str]) -> String {
    for &n in names {
        if let Ok(s) = p.s(n) {
            let s = s.trim();
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    String::new()
}

fn get_str(p: &Params, names: &[&str], default: &str) -> String {
    let s = get_str_opt(p, names);
    if s.is_empty() {
        default.to_string()
    } else {
        s
    }
}

fn get_bool(p: &Params, names: &[&str], default: bool) -> bool {
    for &n in names {
        if let Ok(b) = p.b(n) {
            if b != default {
                return b;
            }
        }
    }
    default
}

fn get_f64(p: &Params, names: &[&str], default: f64) -> f64 {
    for &n in names {
        if let Ok(v) = p.f(n) {
            if (v - default).abs() > 1e-9 {
                return v;
            }
        }
    }
    default
}

fn get_i64(p: &Params, names: &[&str], default: i64) -> i64 {
    for &n in names {
        if let Ok(v) = p.i(n) {
            if v != default {
                return v;
            }
        }
    }
    default
}

fn get_mode(p: &Params) -> String {
    for n in &["mode", "fill_mode"] {
        if let Ok(s) = p.s(n) {
            let s = s.trim().to_lowercase();
            if !s.is_empty() && s != "quadrilaterals" {
                return s;
            }
        }
        if let Ok(i) = p.i(n) {
            return match i {
                0 => "quadrilaterals".to_string(),
                1 => "triangles".to_string(),
                2 => "triangle_fan".to_string(),
                3 => "single_polygon".to_string(),
                4 => "quadrilateral_fan".to_string(),
                5 => "quadrilateral_grid".to_string(),
                _ => "quadrilaterals".to_string(),
            };
        }
    }
    "quadrilaterals".to_string()
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn mul(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn len(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn normalize(a: [f64; 3]) -> [f64; 3] {
    let l = len(a);
    if l < 1e-12 {
        [0., 0., 1.]
    } else {
        mul(a, 1.0 / l)
    }
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    len(sub(a, b))
}

fn angle(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    let (u, v) = (sub(a, b), sub(c, b));
    let (lu, lv) = (len(u), len(v));
    if lu < 1e-12 || lv < 1e-12 {
        return std::f64::consts::FRAC_PI_2;
    }
    (dot(u, v) / (lu * lv)).clamp(-1., 1.).acos()
}

fn newell_normal(pts: &[[f64; 3]]) -> [f64; 3] {
    let mut n = [0.0; 3];
    let count = pts.len();
    for i in 0..count {
        let (a, b) = (pts[i], pts[(i + 1) % count]);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    normalize(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cook(g: Geometry, options: serde_json::Value) -> Result<Geometry> {
        let params = Params::resolve(PARAMS, options.as_object())?;
        polyfill(&[vec![Arc::new(g)]], &params)
    }

    /// Cube with top face removed (face 5): 8 vertices, 5 quad faces, leaving 1 square boundary loop (points 4, 5, 6, 7).
    fn cube_missing_face() -> Geometry {
        let mut g = Geometry::from_arrays(
            vec![
                -1., -1., -1., 1., -1., -1., 1., 1., -1., -1., 1., -1.,
                -1., -1., 1., 1., -1., 1., 1., 1., 1., -1., 1., 1.,
            ],
            vec![
                0, 3, 2, 1,
                0, 1, 5, 4,
                1, 2, 6, 5,
                2, 3, 7, 6,
                3, 0, 4, 7,
            ],
            vec![0, 4, 8, 12, 16, 20],
            None,
        )
        .unwrap();
        g.vertex.insert(
            "uv".into(),
            Arc::new(Attr::f32(
                2,
                (0..5).flat_map(|_| [0., 0., 1., 0., 1., 1., 0., 1.]).collect(),
            )),
        );
        g.prim.insert("material_index".into(), Arc::new(Attr::i32(1, vec![1; 5])));
        g
    }

    /// Cylinder/ring of `n` quads around Z, open at z=0 (points 0..n) and z=1 (points n..2n).
    fn cylinder_ring(n: usize) -> Geometry {
        let mut p = Vec::new();
        for z in [0.0, 1.0] {
            for i in 0..n {
                let a = i as f64 / n as f64 * std::f64::consts::TAU;
                p.extend([a.cos() as f32, a.sin() as f32, z as f32]);
            }
        }
        let mut vp = Vec::new();
        for i in 0..n {
            let j = (i + 1) % n;
            vp.extend([i, j, n + j, n + i].map(|x| x as i32));
        }
        Geometry::from_arrays(p, vp, (0..=n as i32).map(|i| i * 4).collect(), None).unwrap()
    }

    /// Open quad strip of 2 quads: vertices 0, 1, 2 at y=0, vertices 3, 4, 5 at y=1.
    fn open_quad_strip() -> Geometry {
        Geometry::from_arrays(
            vec![
                0., 0., 0.,  1., 0., 0.,  2., 0., 0.,
                0., 1., 0.,  1., 1., 0.,  2., 1., 0.,
            ],
            vec![0, 1, 4, 3,  1, 2, 5, 4],
            vec![0, 4, 8],
            None,
        )
        .unwrap()
    }

    #[test]
    fn test_closed_boundary_fills() {
        let g = cube_missing_face();
        assert_eq!(g.nprims(), 5);
        let out = cook(g, json!({})).unwrap();
        assert_eq!(out.nprims(), 6);
        assert!(out.p().iter().all(|x| x.is_finite()));
    }

    #[test]
    fn test_existing_graphs_still_cook() {
        let g = cube_missing_face();
        let out = cook(g, json!({})).unwrap();
        assert_eq!(out.nprims(), 6);
        let cap = out.nprims() - 1;
        assert_eq!(out.prim["material_index"].as_i32().unwrap()[cap], 0);
        let start = out.topology.prim_offsets[cap] as usize * 2;
        assert!(out.vertex["uv"].as_f32().unwrap()[start..].iter().all(|v| v.is_nan()));
    }

    #[test]
    fn test_fill_group_param_backward_compat() {
        let g = cube_missing_face();
        let out = cook(g, json!({"fill_group": "my_cap"})).unwrap();
        assert!(out.prim.contains_key("group_my_cap"));
        let vals = out.prim["group_my_cap"].as_i32().unwrap();
        assert_eq!(vals.len(), 6);
        assert_eq!(vals[..5], [0, 0, 0, 0, 0]);
        assert_eq!(vals[5], 1);
    }

    #[test]
    fn test_boundary_group() {
        let g = cylinder_ring(6);
        assert_eq!(g.nprims(), 6);
        let out_both = cook(g.clone(), json!({"mode": "single_polygon", "edge_loop": 0.0})).unwrap();
        assert_eq!(out_both.nprims(), 8);

        let out_single = cook(g, json!({"boundary_group": "p0-5", "mode": "single_polygon", "edge_loop": 0.0})).unwrap();
        assert_eq!(out_single.nprims(), 7);
    }

    #[test]
    fn test_fill_modes_on_hex_hole() {
        let g = cylinder_ring(6);
        let out_single = cook(g.clone(), json!({"mode": "single_polygon"})).unwrap();
        assert_eq!(out_single.nprims(), 8);

        let out_tris = cook(g.clone(), json!({"mode": "triangles"})).unwrap();
        assert_eq!(out_tris.nprims(), 6 + 4 * 2);

        let out_trifan = cook(g.clone(), json!({"mode": "triangle_fan"})).unwrap();
        assert_eq!(out_trifan.nprims(), 6 + 6 * 2);

        let out_quadfan = cook(g.clone(), json!({"mode": "quadrilateral_fan"})).unwrap();
        assert_eq!(out_quadfan.nprims(), 6 + 3 * 2);

        let out_quads = cook(g, json!({"mode": "quadrilaterals"})).unwrap();
        assert_eq!(out_quads.nprims(), 6 + 3 * 2);
    }

    #[test]
    fn test_fill_mode_quadrilateral_grid() {
        let g = cylinder_ring(8);
        let out = cook(g, json!({"mode": "quadrilateral_grid", "boundary_group": "p0-7"})).unwrap();
        assert!(out.nprims() > 8);
        assert!(out.p().iter().all(|x| x.is_finite()));
    }

    #[test]
    fn test_auto_complete_boundaries() {
        let g = open_quad_strip();
        let out_on = cook(g.clone(), json!({"boundary_group": "p0-1, p1-2, p2-5", "auto_complete_boundaries": true})).unwrap();
        assert!(out_on.nprims() > g.nprims());

        let out_off = cook(g.clone(), json!({"boundary_group": "p0-1, p1-2, p2-5", "auto_complete_boundaries": false})).unwrap();
        assert_eq!(out_off.nprims(), g.nprims());
    }

    #[test]
    fn test_reverse_patches() {
        let g = cube_missing_face();
        let out_norm = cook(g.clone(), json!({"reverse_patches": false, "mode": "single_polygon"})).unwrap();
        let out_rev = cook(g, json!({"reverse_patches": true, "mode": "single_polygon"})).unwrap();

        let cap_norm: Vec<i32> = out_norm.topology.vertex_point[out_norm.topology.prim_offsets[5] as usize..].to_vec();
        let cap_rev: Vec<i32> = out_rev.topology.vertex_point[out_rev.topology.prim_offsets[5] as usize..].to_vec();
        let mut expected_rev = cap_norm;
        expected_rev.reverse();
        assert_eq!(cap_rev, expected_rev);
    }

    #[test]
    fn test_unique_points() {
        let g = cube_missing_face();
        let out_shared = cook(g.clone(), json!({"unique_points": false})).unwrap();
        let out_unique = cook(g.clone(), json!({"unique_points": true})).unwrap();

        assert_eq!(out_shared.npoints, g.npoints);
        assert!(out_unique.npoints > g.npoints);
        let cap_unique: Vec<i32> = out_unique.topology.vertex_point[out_unique.topology.prim_offsets[5] as usize..].to_vec();
        for &v in &cap_unique {
            assert!(v >= g.npoints as i32);
        }
    }

    #[test]
    fn test_update_point_normals() {
        let mut g = cube_missing_face();
        g.point.insert("N".into(), Arc::new(Attr::f32(3, vec![0.0; g.npoints * 3])));

        let out_updated = cook(g.clone(), json!({"update_point_normals": true})).unwrap();
        let n_updated = out_updated.point["N"].as_f32().unwrap();
        assert!(n_updated.iter().any(|&x| x.abs() > 0.01));

        let out_kept = cook(g, json!({"update_point_normals": false})).unwrap();
        let n_kept = out_kept.point["N"].as_f32().unwrap();
        assert!(n_kept.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn test_edge_loop_on_open_boundary() {
        let g = open_quad_strip();
        let out_01 = cook(g.clone(), json!({"boundary_group": "p0-1, p1-2, p2-5", "edge_loop": 0.1, "auto_complete_boundaries": true})).unwrap();
        let out_05 = cook(g.clone(), json!({"boundary_group": "p0-1, p1-2, p2-5", "edge_loop": 0.5, "auto_complete_boundaries": true})).unwrap();

        assert!(out_01.prim.contains_key("group_loopgroup"));
        assert!(out_05.prim.contains_key("group_loopgroup"));
        assert_ne!(out_01.p(), out_05.p());
    }

    #[test]
    fn test_smooth() {
        let g = cylinder_ring(6);
        let out_unsmoothed = cook(g.clone(), json!({"mode": "triangle_fan", "smooth": 0.0, "deform_patch": false})).unwrap();
        let out_smoothed = cook(g, json!({"mode": "triangle_fan", "smooth": 50.0, "deform_patch": false})).unwrap();

        assert_eq!(out_unsmoothed.npoints, out_smoothed.npoints);
        assert!(out_unsmoothed.p().iter().all(|x| x.is_finite()));
        assert!(out_smoothed.p().iter().all(|x| x.is_finite()));
    }

    #[test]
    fn test_corner_offset() {
        let g = cylinder_ring(6);
        let out_0 = cook(g.clone(), json!({"mode": "quadrilateral_fan", "corner_offset": 0, "boundary_group": "p0-5"})).unwrap();
        let out_1 = cook(g, json!({"mode": "quadrilateral_fan", "corner_offset": 1, "boundary_group": "p0-5"})).unwrap();

        let cap_0: Vec<i32> = out_0.topology.vertex_point[out_0.topology.prim_offsets[6] as usize..out_0.topology.prim_offsets[7] as usize].to_vec();
        let cap_1: Vec<i32> = out_1.topology.vertex_point[out_1.topology.prim_offsets[6] as usize..out_1.topology.prim_offsets[7] as usize].to_vec();
        assert_ne!(cap_0, cap_1);
    }

    #[test]
    fn test_custom_corners() {
        let mut g = cylinder_ring(6);
        g.point.insert("group_mycorner".into(), Arc::new(Attr::i32(1, vec![0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0])));
        let out_custom = cook(g, json!({"mode": "quadrilateral_fan", "custom_corners": "mycorner", "boundary_group": "p0-5"})).unwrap();
        let cap_custom: Vec<i32> = out_custom.topology.vertex_point[out_custom.topology.prim_offsets[6] as usize..out_custom.topology.prim_offsets[7] as usize].to_vec();
        assert_eq!(cap_custom[0], 2);
    }

    #[test]
    fn test_deform_patch_and_surface_offset() {
        let g = cube_missing_face();
        let np = g.npoints;
        let out_flat = cook(g.clone(), json!({"mode": "triangle_fan", "surface_offset": 0.0, "tangent_strength": 0.0, "smooth": 0.0})).unwrap();
        let out_deformed = cook(g.clone(), json!({"mode": "triangle_fan", "surface_offset": 1.0, "tangent_strength": 0.0, "deform_patch": true, "smooth": 0.0})).unwrap();
        let out_disabled = cook(g, json!({"mode": "triangle_fan", "surface_offset": 1.0, "tangent_strength": 0.0, "deform_patch": false, "smooth": 0.0})).unwrap();

        let flat_center = &out_flat.p()[np * 3..];
        let def_center = &out_deformed.p()[np * 3..];
        let dis_center = &out_disabled.p()[np * 3..];

        assert_ne!(flat_center, def_center);
        assert_eq!(flat_center, dis_center);
    }

    #[test]
    fn test_output_groups_and_append() {
        let mut g = cube_missing_face();
        g.prim.insert("group_patch".into(), Arc::new(Attr::i32(1, vec![1, 0, 0, 0, 0])));

        let out_replace = cook(g.clone(), json!({"append_to_groups": false})).unwrap();
        let vals_replace = out_replace.prim["group_patch"].as_i32().unwrap();
        assert_eq!(vals_replace[0], 0);
        assert_eq!(vals_replace[5], 1);

        let out_append = cook(g, json!({"append_to_groups": true})).unwrap();
        let vals_append = out_append.prim["group_patch"].as_i32().unwrap();
        assert_eq!(vals_append[0], 1);
        assert_eq!(vals_append[5], 1);
    }

    #[test]
    fn test_no_nan_across_operations() {
        let g = cube_missing_face();
        for mode in &["quadrilaterals", "triangles", "triangle_fan", "quadrilateral_fan", "single_polygon"] {
            let out = cook(g.clone(), json!({
                "mode": mode,
                "smooth": 50.0,
                "surface_offset": 0.5,
                "tangent_strength": 0.4,
                "update_point_normals": true,
            })).unwrap();
            assert!(out.p().iter().all(|x| x.is_finite()));
            if let Some(n) = out.point.get("N") {
                assert!(n.as_f32().unwrap().iter().all(|x| x.is_finite()));
            }
        }
    }
}
