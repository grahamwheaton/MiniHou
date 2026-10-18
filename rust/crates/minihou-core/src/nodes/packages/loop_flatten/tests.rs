use super::*;
use std::sync::Arc;
use crate::geometry::Geometry;
use crate::params::Params;

fn create_perturbed_loop() -> Geometry {
    // 6 vertices around a circle in XY with alternating Z so it is strictly non-planar
    let mut pts = Vec::new();
    let n = 6;
    for i in 0..n {
        let angle = (i as f32) * std::f32::consts::TAU / (n as f32);
        let x = angle.cos();
        let y = angle.sin();
        let z = if i % 2 == 0 { 0.5 } else { -0.5 };
        pts.extend_from_slice(&[x, y, z]);
    }
    let vp: Vec<i32> = (0..n as i32).collect();
    let offs = vec![0, n as i32];
    Geometry::from_arrays(pts, vp, offs, Some(vec![true])).unwrap()
}

fn plane_residual(p: &[f32]) -> f64 {
    let n = p.len() / 3;
    let mut c = [0.0f64; 3];
    for i in 0..n {
        c[0] += p[i * 3] as f64;
        c[1] += p[i * 3 + 1] as f64;
        c[2] += p[i * 3 + 2] as f64;
    }
    c[0] /= n as f64;
    c[1] /= n as f64;
    c[2] /= n as f64;

    let mut cov = [[0.0f64; 3]; 3];
    for i in 0..n {
        let d = [
            p[i * 3] as f64 - c[0],
            p[i * 3 + 1] as f64 - c[1],
            p[i * 3 + 2] as f64 - c[2],
        ];
        for j in 0..3 {
            for k in 0..3 {
                cov[j][k] += d[j] * d[k];
            }
        }
    }
    let normal = jacobi_smallest_eigenvector(cov);

    let mut max_dist = 0.0f64;
    for i in 0..n {
        let dist = ((p[i * 3] as f64 - c[0]) * normal[0]
            + (p[i * 3 + 1] as f64 - c[1]) * normal[1]
            + (p[i * 3 + 2] as f64 - c[2]) * normal[2]).abs();
        if dist > max_dist {
            max_dist = dist;
        }
    }
    max_dist
}

#[test]
fn test_perturbed_3d_boundary_flattens_to_residual_near_zero() {
    let geo = create_perturbed_loop();
    let before_residual = plane_residual(geo.p());
    assert!(before_residual > 0.1, "boundary should be non-planar initially, got {before_residual}");

    let p = Params::resolve(NODE.params, None).unwrap();
    let out = (NODE.cook)(&[vec![Arc::new(geo)]], &p).unwrap();

    let after_residual = plane_residual(out.p());
    assert!(after_residual < 1e-5, "residual after flatten must be near zero, got {after_residual}");
}

#[test]
fn test_both_plane_modes() {
    let geo = create_perturbed_loop();

    // 1. best_fit
    let mut map_best_fit = serde_json::Map::new();
    map_best_fit.insert("plane".into(), serde_json::json!("best_fit"));
    let p_bf = Params::resolve(NODE.params, Some(&map_best_fit)).unwrap();
    let out_bf = (NODE.cook)(&[vec![Arc::new(geo.clone())]], &p_bf).unwrap();
    let res_bf = plane_residual(out_bf.p());
    assert!(res_bf < 1e-5, "best_fit residual must be near zero, got {res_bf}");

    // 2. normal
    let mut map_normal = serde_json::Map::new();
    map_normal.insert("plane".into(), serde_json::json!("normal"));
    let p_norm = Params::resolve(NODE.params, Some(&map_normal)).unwrap();
    let out_norm = (NODE.cook)(&[vec![Arc::new(geo.clone())]], &p_norm).unwrap();
    let res_norm = plane_residual(out_norm.p());
    assert!(res_norm < 1e-5, "normal residual must be near zero, got {res_norm}");
}

#[test]
fn test_empty_and_missing_group() {
    let geo = create_perturbed_loop();

    // Empty group flattens all points
    let mut map_empty = serde_json::Map::new();
    map_empty.insert("group".into(), serde_json::json!(""));
    let p_empty = Params::resolve(NODE.params, Some(&map_empty)).unwrap();
    let out_empty = (NODE.cook)(&[vec![Arc::new(geo.clone())]], &p_empty).unwrap();
    assert!(plane_residual(out_empty.p()) < 1e-5);

    // Missing group returns geometry unchanged without error
    let mut map_missing = serde_json::Map::new();
    map_missing.insert("group".into(), serde_json::json!("nonexistent_group"));
    let p_missing = Params::resolve(NODE.params, Some(&map_missing)).unwrap();
    let out_missing = (NODE.cook)(&[vec![Arc::new(geo.clone())]], &p_missing).unwrap();
    assert_eq!(out_missing.p(), geo.p());
}

#[test]
fn test_influence_zero_is_identity() {
    let geo = create_perturbed_loop();
    let mut map = serde_json::Map::new();
    map.insert("influence".into(), serde_json::json!(0.0));
    let p = Params::resolve(NODE.params, Some(&map)).unwrap();
    let out = (NODE.cook)(&[vec![Arc::new(geo.clone())]], &p).unwrap();
    assert_eq!(out.p(), geo.p());
}
