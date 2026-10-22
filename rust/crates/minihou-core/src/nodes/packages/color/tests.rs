use super::*;
use std::sync::Arc;

fn quad_geo() -> Geometry {
    Geometry::from_arrays(
        vec![
            0., 0., 0.,
            1., 0., 0.,
            1., 1., 0.,
            0., 1., 1.,
        ],
        vec![0, 1, 2, 3],
        vec![0, 4],
        None,
    ).unwrap()
}

fn run_color(geo: &Geometry, params: serde_json::Value) -> Result<Geometry> {
    let p = Params::resolve(NODE.params, params.as_object())?;
    color_node(&[vec![Arc::new(geo.clone())]], &p)
}

#[test]
fn package_registers_with_defaults() {
    let spec = crate::nodes::get("color").unwrap();
    assert_eq!(spec.name, NODE.name);
    let defaults = Params::resolve(NODE.params, None).unwrap();
    assert_eq!(defaults.e("domain", &["points", "edges", "faces"]).unwrap(), "points");
}

#[test]
fn test_color_domain_options_meta() {
    let raw = include_str!("param_meta.json");
    let meta: serde_json::Value = serde_json::from_str(raw).unwrap();
    let options = meta["domain"]["options"].as_array().unwrap();
    let opt_strs: Vec<&str> = options.iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(opt_strs, vec!["points", "edges", "faces"]);
}

#[test]
fn test_color_points_domain() {
    let g = quad_geo();
    let out_pts = run_color(&g, serde_json::json!({
        "domain": "points",
        "color": [1.0, 0.0, 0.0]
    })).unwrap();
    assert!(out_pts.point.contains_key("Cd"));
    assert_eq!(out_pts.point["Cd"].rows(), 4);
    assert_eq!(out_pts.point["Cd"].as_f32().unwrap(), &[1., 0., 0., 1., 0., 0., 1., 0., 0., 1., 0., 0.]);
    assert!(!out_pts.edge.contains_key("Cd"));
    assert!(!out_pts.prim.contains_key("Cd"));
}

#[test]
fn test_color_edges_domain() {
    let g = quad_geo();
    let out_edges = run_color(&g, serde_json::json!({
        "domain": "edges",
        "color": [0.0, 1.0, 0.0]
    })).unwrap();
    assert!(out_edges.edge.contains_key("Cd"));
    assert_eq!(out_edges.edge["Cd"].rows(), 4);
    assert_eq!(out_edges.edge["Cd"].as_f32().unwrap(), &[0., 1., 0., 0., 1., 0., 0., 1., 0., 0., 1., 0.]);
    assert!(!out_edges.point.contains_key("Cd"));
    assert!(!out_edges.prim.contains_key("Cd"));
}

#[test]
fn test_color_faces_domain() {
    let g = quad_geo();
    let out_faces = run_color(&g, serde_json::json!({
        "domain": "faces",
        "color": [0.0, 0.0, 1.0]
    })).unwrap();
    assert!(out_faces.prim.contains_key("Cd"));
    assert_eq!(out_faces.prim["Cd"].rows(), 1);
    assert_eq!(out_faces.prim["Cd"].as_f32().unwrap(), &[0., 0., 1.]);
    assert!(!out_faces.point.contains_key("Cd"));
    assert!(!out_faces.edge.contains_key("Cd"));
}

#[test]
fn test_color_edges_with_group_and_ramp() {
    let g = quad_geo();
    // Edge color with group: only edge 0
    let out_grouped = run_color(&g, serde_json::json!({
        "domain": "edges",
        "color": [1.0, 0.5, 0.25],
        "group": "0"
    })).unwrap();
    assert_eq!(out_grouped.edge["Cd"].rows(), 4);
    let cd = out_grouped.edge["Cd"].as_f32().unwrap();
    assert_eq!(&cd[0..3], &[1.0, 0.5, 0.25]);
    assert_eq!(&cd[3..6], &[0.0, 0.0, 0.0]);

    // Edge color with ramp mode along P.z
    let out_ramp = run_color(&g, serde_json::json!({
        "domain": "edges",
        "mode": "ramp",
        "ramp_attrib": "P.z",
        "color": [0.0, 0.0, 0.0],
        "color2": [1.0, 1.0, 1.0]
    })).unwrap();
    assert_eq!(out_ramp.edge["Cd"].rows(), 4);
    out_ramp.validate().unwrap();
}
