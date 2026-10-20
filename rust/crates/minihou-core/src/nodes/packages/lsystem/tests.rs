use super::*;

fn run_lsystem(params: serde_json::Value) -> Result<Geometry> {
    let p = Params::resolve(NODE.params, params.as_object()).unwrap();
    lsystem(&[vec![], vec![], vec![], vec![]], &p)
}

#[test]
fn python_branch_golden() {
    let p = Params::resolve(
        NODE.params,
        serde_json::json!({
            "premise": "F[+F]F",
            "rules": "",
            "generations": 0.,
            "step_size": 1.,
            "thickness": 0.2,
            "angle": 90.
        })
        .as_object(),
    )
    .unwrap();
    let out = lsystem(&[vec![], vec![], vec![], vec![]], &p).unwrap();
    assert_eq!(out.p(), &[0., 0., 0., 0., 0., 1., -1., 0., 1., 0., 0., 2.]);
    assert_eq!(out.topology.vertex_point, [0, 1, 1, 2, 1, 3]);
    assert_eq!(out.topology.prim_offsets, [0, 2, 4, 6]);
    assert_eq!(out.point["width"].as_f32().unwrap(), &[0.2, 0.2, 0.14, 0.2]);
    assert_eq!(out.detail["lsystem_symbols"], 6);
}

#[test]
fn unsupported_parametric_rule_is_explicit() {
    let p = Params::resolve(NODE.params, serde_json::json!({"rules": "A(i)=F(i+1)"}).as_object()).unwrap();
    assert!(lsystem(&[vec![], vec![], vec![], vec![]], &p).is_err());
}

#[test]
fn python_default_counts() {
    let p = Params::resolve(NODE.params, None).unwrap();
    let out = lsystem(&[vec![], vec![], vec![], vec![]], &p).unwrap();
    assert_eq!(
        (
            out.npoints,
            out.nprims(),
            out.nvertices(),
            out.detail["lsystem_symbols"].as_u64().unwrap()
        ),
        (4097, 1536, 5632, 11116)
    );
}

#[test]
fn test_lsystem_empty_and_single_element() {
    let p_empty = Params::resolve(NODE.params, serde_json::json!({"premise": "", "rules": ""}).as_object()).unwrap();
    let out_empty = lsystem(&[vec![], vec![], vec![], vec![]], &p_empty).unwrap();
    assert_eq!(out_empty.npoints, 0);
    assert_eq!(out_empty.nprims(), 0);

    let p_single = Params::resolve(
        NODE.params,
        serde_json::json!({"premise": "F", "rules": "", "generations": 0.}).as_object(),
    )
    .unwrap();
    let out_single = lsystem(&[vec![], vec![], vec![], vec![]], &p_single).unwrap();
    assert_eq!(out_single.npoints, 2);
    assert_eq!(out_single.nprims(), 1);
    assert_eq!(out_single.topology.vertex_point, &[0, 1]);
}

#[test]
fn test_lsystem_errors_and_unsupported() {
    let p_neg_gen = Params::resolve(NODE.params, serde_json::json!({"generations": -1.}).as_object()).unwrap();
    assert!(lsystem(&[vec![], vec![], vec![], vec![]], &p_neg_gen).is_err());
    let p_huge_gen = Params::resolve(NODE.params, serde_json::json!({"generations": 13.}).as_object()).unwrap();
    assert!(lsystem(&[vec![], vec![], vec![], vec![]], &p_huge_gen).is_err());

    let p_tube = Params::resolve(NODE.params, serde_json::json!({"type": "tube"}).as_object()).unwrap();
    assert!(lsystem(&[vec![], vec![], vec![], vec![]], &p_tube).is_err());

    let dummy = Arc::new(Geometry::points_only(vec![0., 0., 0.]).unwrap());
    let p_def = Params::resolve(NODE.params, None).unwrap();
    assert!(lsystem(&[vec![dummy], vec![], vec![], vec![]], &p_def).is_err());

    let p_open =
        Params::resolve(NODE.params, serde_json::json!({"premise": "F[F", "rules": "", "generations": 0.}).as_object())
            .unwrap();
    assert!(lsystem(&[vec![], vec![], vec![], vec![]], &p_open).is_err());
    let p_close =
        Params::resolve(NODE.params, serde_json::json!({"premise": "F]F", "rules": "", "generations": 0.}).as_object())
            .unwrap();
    assert!(lsystem(&[vec![], vec![], vec![], vec![]], &p_close).is_err());

    let p_bad_rule = Params::resolve(NODE.params, serde_json::json!({"rules": "invalid_rule"}).as_object()).unwrap();
    assert!(lsystem(&[vec![], vec![], vec![], vec![]], &p_bad_rule).is_err());
}

#[test]
fn test_lsystem_determinism_and_attributes() {
    let p = Params::resolve(
        NODE.params,
        serde_json::json!({
            "premise": "F+F",
            "rules": "",
            "generations": 0.,
            "random_scale": 0.5,
            "seed": 12345,
            "point_attributes": true
        })
        .as_object(),
    )
    .unwrap();

    let out1 = lsystem(&[vec![], vec![], vec![], vec![]], &p).unwrap();
    let out2 = lsystem(&[vec![], vec![], vec![], vec![]], &p).unwrap();

    assert_eq!(out1.p(), out2.p());
    assert_eq!(out1.point["width"].data, out2.point["width"].data);
    assert_eq!(out1.point["arc"].data, out2.point["arc"].data);
    assert_eq!(out1.point["up"].data, out2.point["up"].data);
    assert!(out1.point.contains_key("segs"));
    assert!(out1.point.contains_key("div"));
    assert!(out1.point.contains_key("gen"));
}

#[test]
fn test_turtle_3d_rotations_independent_axes() {
    let out = run_lsystem(serde_json::json!({
        "premise": "F+F&F",
        "rules": "",
        "generations": 0.,
        "step_size": 1.,
        "separate_angles": true,
        "yaw": 90.,
        "pitch": 90.,
        "roll": 90.
    }))
    .unwrap();
    assert_eq!(out.npoints, 4);
    let pts = out.p();
    let expected = [0., 0., 0., 0., 0., 1., -1., 0., 1., -1., -1., 1.];
    for (a, b) in pts.iter().zip(expected.iter()) {
        assert!((a - b).abs() < 1e-5, "expected {b}, got {a}");
    }
}

#[test]
fn test_turtle_initial_orientation() {
    // Pitch 90 initial orientation points heading to [-1, 0, 0]
    let out_yaw = run_lsystem(serde_json::json!({
        "premise": "F",
        "rules": "",
        "generations": 0.,
        "step_size": 1.,
        "initial_orientation": [90., 0., 0.]
    }))
    .unwrap();
    let p_yaw = &out_yaw.p()[3..6];
    assert!((p_yaw[0] - (-1.)).abs() < 1e-5);
    assert!(p_yaw[1].abs() < 1e-5);
    assert!(p_yaw[2].abs() < 1e-5);

    // Pitch 90 initial orientation points heading to [0, -1, 0]
    let out_pitch = run_lsystem(serde_json::json!({
        "premise": "F",
        "rules": "",
        "generations": 0.,
        "step_size": 1.,
        "initial_orientation": [0., 90., 0.]
    }))
    .unwrap();
    let p_pitch = &out_pitch.p()[3..6];
    assert!(p_pitch[0].abs() < 1e-5);
    assert!((p_pitch[1] - (-1.)).abs() < 1e-5);
    assert!(p_pitch[2].abs() < 1e-5);

    // Roll 90 followed by yaw +90 turn points heading to [0, -1, 0]
    let out_roll = run_lsystem(serde_json::json!({
        "premise": "+F",
        "rules": "",
        "generations": 0.,
        "step_size": 1.,
        "angle": 90.,
        "initial_orientation": [0., 0., 90.]
    }))
    .unwrap();
    let p_roll = &out_roll.p()[3..6];
    assert!(p_roll[0].abs() < 1e-5);
    assert!((p_roll[1] - (-1.)).abs() < 1e-5);
    assert!(p_roll[2].abs() < 1e-5);
}

#[test]
fn test_turtle_roll_and_inverse_rotations() {
    let out_roll = run_lsystem(serde_json::json!({
        "premise": "\\+F",
        "rules": "",
        "generations": 0.,
        "step_size": 1.,
        "separate_angles": true,
        "yaw": 90.,
        "pitch": 90.,
        "roll": 90.
    }))
    .unwrap();
    let last = &out_roll.p()[3..6];
    assert!(last[0].abs() < 1e-5);
    assert!((last[1] - (-1.)).abs() < 1e-5);
    assert!(last[2].abs() < 1e-5);

    let out_inv = run_lsystem(serde_json::json!({
        "premise": "+-&^\\/F",
        "rules": "",
        "generations": 0.,
        "step_size": 1.,
        "separate_angles": true,
        "yaw": 90.,
        "pitch": 90.,
        "roll": 90.
    }))
    .unwrap();
    let last_inv = &out_inv.p()[3..6];
    assert!(last_inv[0].abs() < 1e-5);
    assert!(last_inv[1].abs() < 1e-5);
    assert!((last_inv[2] - 1.).abs() < 1e-5);
}

#[test]
fn test_branch_restores_full_frame() {
    let out = run_lsystem(serde_json::json!({
        "premise": "F[+&\\!\"F]F+F&F",
        "rules": "",
        "generations": 0.,
        "angle": 90.,
        "step_size": 1.,
        "thickness": 1.,
        "thickness_scale": 0.5,
        "step_size_scale": 0.25
    }))
    .unwrap();
    let pts = out.p();
    // Points 3, 4, 5 correspond to continued trunk and turns: [0,0,2], [-1,0,2], [-1,-1,2]
    let continued = &pts[9..18];
    let expected = [0., 0., 2., -1., 0., 2., -1., -1., 2.];
    for (a, b) in continued.iter().zip(expected.iter()) {
        assert!((a - b).abs() < 1e-5, "expected {b}, got {a}");
    }
    let widths = out.point["width"].as_f32().unwrap();
    for &w in &widths[3..6] {
        assert!((w - 1.0).abs() < 1e-5);
    }
    // Branched segment length is step_size * step_size_scale = 0.25
    let b_dx = pts[6] - pts[3];
    let b_dy = pts[7] - pts[4];
    let b_dz = pts[8] - pts[5];
    let branch_len = (b_dx * b_dx + b_dy * b_dy + b_dz * b_dz).sqrt();
    assert!((branch_len - 0.25).abs() < 1e-5);
}

#[test]
fn test_planar_compatibility_without_separate_angles() {
    let out = run_lsystem(serde_json::json!({
        "premise": "F[+F][-F]F",
        "rules": "",
        "generations": 0.,
        "angle": 30.
    }))
    .unwrap();
    for i in 0..out.npoints {
        let y = out.p()[i * 3 + 1];
        assert!(y.abs() < 1e-5, "expected y=0 for planar grammar, got {y}");
    }
}

#[test]
fn test_3d_spatial_tree_starter() {
    let out = run_lsystem(serde_json::json!({
        "premise": "A",
        "rules": "A=F[&A]/[&A]/[&A]A",
        "generations": 4.,
        "separate_angles": true,
        "yaw": 33.,
        "pitch": 42.,
        "roll": 137.5,
        "step_size": 0.3,
        "thickness": 0.03
    }))
    .unwrap();
    assert!(out.npoints > 50);
    assert!(out.nprims() > 10);
    let pts = out.p();
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for chunk in pts.chunks_exact(3) {
        for k in 0..3 {
            min[k] = min[k].min(chunk[k]);
            max[k] = max[k].max(chunk[k]);
        }
    }
    // Extents in all 3 dimensions must be substantial (non-planar 3D tree)
    for k in 0..3 {
        assert!(max[k] - min[k] > 0.5, "dimension {k} extent too small: {}", max[k] - min[k]);
    }
}
