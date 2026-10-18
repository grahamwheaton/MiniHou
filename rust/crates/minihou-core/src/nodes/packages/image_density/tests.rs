use super::*;
use serde_json::{json, Value};

fn grid() -> Geometry {
    let mut g = Geometry::from_arrays(
        vec![-1.,-1.,0., 1.,-1.,0., -1.,1.,0., 1.,1.,0.],
        vec![0,1,3,2], vec![0,4], None).unwrap();
    g.point.insert("Cd".into(), Arc::new(Attr::f32(3, vec![0.,0.,0., 1.,1.,1., 0.,0.,0., 1.,1.,1.])));
    g.point.insert("uv".into(), Arc::new(Attr::f32(2, vec![0.,0., 1.,0., 0.,1., 1.,1.])));
    g
}
fn run(g: &Geometry, options: Value) -> Result<Geometry> {
    let params = Params::resolve(NODE.params, options.as_object())?;
    cook(&[vec![Arc::new(g.clone())]], &params)
}
fn density(g: &Geometry, name: &str) -> Vec<f32> { g.point[name].as_f32().unwrap().to_vec() }

#[test]
fn image_density_half_dark_and_preserves_attributes() {
    let g = grid();
    let out = run(&g, json!({})).unwrap();
    assert_eq!(density(&out, "density"), vec![1.,0.,1.,0.]);
    assert_eq!(out.point["Cd"], g.point["Cd"]);
    assert_eq!(out.point["uv"], g.point["uv"]);
    assert_eq!(out.topology, g.topology);
    assert_eq!(density(&run(&g, json!({"invert":false})).unwrap(), "density"), vec![0.,1.,0.,1.]);
}

#[test]
fn image_density_channels_contrast_threshold_and_nan() {
    let mut g = grid();
    g.point.insert("Cd".into(), Arc::new(Attr::f32(3, vec![0.2,0.4,0.8, 0.8,0.1,0.3, f32::NAN,0.,0., 2.,2.,2.])));
    g.point.insert("Alpha".into(), Arc::new(Attr::f32(1, vec![0.,1.,0.25,1.])));
    assert_eq!(density(&run(&g, json!({"channel":"alpha","invert":false})).unwrap(), "density"), vec![0.,1.,0.25,1.]);
    let red = run(&g, json!({"channel":"red","invert":false,"contrast":2.0,"threshold":0.5,"output_attribute":"red_density"})).unwrap();
    assert_eq!(density(&red, "red_density"), vec![0.,1.,0.,1.]);
    assert_eq!(density(&run(&g, json!({"channel":"blue","invert":false,"brightness":0.2})).unwrap(), "density"), vec![1.,0.5,0.2,1.]);
    let mut grey = grid();
    grey.point.insert("Cd".into(), Arc::new(Attr::f32(1, vec![0.,0.5,1.,0.])));
    assert_eq!(density(&run(&grey, json!({"channel":"blue","invert":false})).unwrap(), "density"), vec![0.,0.5,1.,0.]);
}

#[test]
fn image_density_missing_cd_is_clear() {
    let mut g = grid(); g.point.remove("Cd");
    assert!(run(&g, json!({})).unwrap_err().0.contains("'Cd'"));
}

#[test]
fn image_density_demo_graph_recooks_after_parameter_change() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../minihou-app/src/test_data/image_density_scatter.minihou");
    let mut graph: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let image = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/image_file_test.png");
    graph["nodes"]["image"]["params"]["file"] = json!(image);
    let cooked = crate::graph::Graph::from_value(&graph).unwrap();
    let baseline = cooked.cook(Some("scatter_points")).unwrap();
    assert!(baseline.npoints > 0 && baseline.npoints < 200);
    assert_eq!(cooked.cook(None).unwrap().npoints, baseline.npoints);
    graph["nodes"]["density"]["params"]["brightness"] = json!(1.0);
    let changed = crate::graph::Graph::from_value(&graph).unwrap();
    assert_eq!(changed.cook(Some("scatter_points")).unwrap().npoints, 0);
    assert_eq!(changed.cook(None).unwrap().npoints, 0);
}
