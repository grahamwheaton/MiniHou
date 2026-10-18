use super::*;
use std::sync::Arc;
use crate::geometry::Geometry;
use crate::params::Params;

fn run_node(geo: Geometry, params: serde_json::Value) -> Geometry {
    let map = match params {
        serde_json::Value::Object(m) => Some(m),
        _ => None,
    };
    let p = Params::resolve(NODE.params, map.as_ref()).unwrap();
    (NODE.cook)(&[vec![Arc::new(geo)]], &p).unwrap()
}

fn make_perturbed_polygon(n: usize, radii: &[f32], z_noise: &[f32]) -> Geometry {
    let mut pts = Vec::with_capacity(n * 3);
    let mut vp = Vec::with_capacity(n);
    for i in 0..n {
        let theta = (i as f32 / n as f32) * std::f32::consts::TAU;
        let r = radii[i % radii.len()];
        let z = z_noise[i % z_noise.len()];
        pts.push(r * theta.cos());
        pts.push(r * theta.sin());
        pts.push(z);
        vp.push(i as i32);
    }
    let offs = vec![0, n as i32];
    Geometry::from_arrays(pts, vp, offs, Some(vec![true])).unwrap()
}

#[test]
fn test_perturbed_boundary_to_circle_radius() {
    let n = 8;
    // Perturbed radii: alternating 0.8 and 1.2, average is 1.0; perturbed z
    let radii = [0.8, 1.2];
    let z_noise = [0.1, -0.1, 0.05, -0.05];
    let geo = make_perturbed_polygon(n, &radii, &z_noise);

    let cooked = run_node(
        geo,
        serde_json::json!({
            "influence": 1.0,
            "fit_inside": false,
            "group": ""
        }),
    );

    let p_slice = cooked.p();
    let mut center = [0.0f32; 3];
    for i in 0..n {
        center[0] += p_slice[i * 3];
        center[1] += p_slice[i * 3 + 1];
        center[2] += p_slice[i * 3 + 2];
    }
    center[0] /= n as f32;
    center[1] /= n as f32;
    center[2] /= n as f32;

    assert!(center[0].abs() < 1e-4, "center x should be ~0");
    assert!(center[1].abs() < 1e-4, "center y should be ~0");
    assert!(center[2].abs() < 1e-4, "center z should be ~0");

    let expected_radius = 1.0f32;
    for i in 0..n {
        let x = p_slice[i * 3] - center[0];
        let y = p_slice[i * 3 + 1] - center[1];
        let z_val = p_slice[i * 3 + 2] - center[2];
        let r = (x * x + y * y + z_val * z_val).sqrt();
        assert!(z_val.abs() < 1e-4, "point {i} z not in plane: {z_val}");
        assert!((r - expected_radius).abs() < 1e-3, "point {i} radius {r} != {expected_radius}");
    }

    // Check angle spacing
    for i in 0..n {
        let next = (i + 1) % n;
        let x1 = p_slice[i * 3] - center[0];
        let y1 = p_slice[i * 3 + 1] - center[1];
        let x2 = p_slice[next * 3] - center[0];
        let y2 = p_slice[next * 3 + 1] - center[1];
        let dot = (x1 * x2 + y1 * y2) / (expected_radius * expected_radius);
        let angle = dot.clamp(-1.0, 1.0).acos();
        let expected_angle = std::f32::consts::TAU / n as f32;
        assert!(
            (angle - expected_angle).abs() < 1e-3,
            "angle between {i} and {next} is {angle}, expected {expected_angle}"
        );
    }
}

#[test]
fn test_fit_inside() {
    let n = 8;
    let radii = [0.8, 1.2];
    let z_noise = [0.0];
    let geo = make_perturbed_polygon(n, &radii, &z_noise);

    let cooked = run_node(
        geo,
        serde_json::json!({
            "influence": 1.0,
            "fit_inside": true,
            "group": ""
        }),
    );

    let p_slice = cooked.p();
    let expected_radius = 0.8f32; // Minimum radius
    for i in 0..n {
        let x = p_slice[i * 3];
        let y = p_slice[i * 3 + 1];
        let z = p_slice[i * 3 + 2];
        let r = (x * x + y * y + z * z).sqrt();
        assert!((r - expected_radius).abs() < 1e-3, "point {i} radius {r} != {expected_radius}");
    }
}

#[test]
fn test_influence_zero_is_identity() {
    let n = 6;
    let radii = [0.5, 1.5];
    let z_noise = [0.2, -0.2];
    let geo = make_perturbed_polygon(n, &radii, &z_noise);

    let cooked = run_node(
        geo.clone(),
        serde_json::json!({
            "influence": 0.0,
            "fit_inside": false,
            "group": ""
        }),
    );

    assert_eq!(cooked.p(), geo.p());
}
