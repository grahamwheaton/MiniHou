use super::*;
use crate::geometry::{Attr, AttrData};
use serde_json::json;

fn quad() -> Geometry {
    let mut g = Geometry::from_arrays(
        vec![0.,0.,0., 1.,0.,0., 1.,1.,0., 0.,1.,0.],
        vec![0,1,2,3], vec![0,4], None).unwrap();
    g.point.insert("Cd".into(), Arc::new(Attr::f32(3, [1.,0.5,0.].repeat(4))));
    g.vertex.insert("uv".into(), Arc::new(Attr::f32(2, vec![0.,0., 1.,0., 1.,1., 0.,1.])));
    g.prim.insert("material_index".into(), Arc::new(Attr::i32(1, vec![2])));
    g.prim.insert("group_top".into(), Arc::new(Attr::i32(1, vec![1])));
    g.edge.insert("group_outline".into(), Arc::new(Attr::i32(1, vec![1; 4])));
    g
}
fn params(values: &serde_json::Value) -> Params {
    Params::resolve_node("array", NODE.params, values.as_object()).unwrap()
}
fn run(g: Geometry, values: serde_json::Value) -> Geometry {
    let out = (NODE.cook)(&[vec![Arc::new(g)]], &params(&values)).unwrap();
    out.validate().unwrap();
    out
}
fn run_err(g: Geometry, values: serde_json::Value) -> String {
    match Params::resolve_node("array", NODE.params, values.as_object()) {
        Err(e) => e.to_string(),
        Ok(p) => (NODE.cook)(&[vec![Arc::new(g)]], &p).err().unwrap().to_string(),
    }
}
fn bounds(g: &Geometry) -> ([f32; 3], [f32; 3]) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for q in g.p().chunks_exact(3) {
        for a in 0..3 { lo[a] = lo[a].min(q[a]); hi[a] = hi[a].max(q[a]); }
    }
    (lo, hi)
}
fn close(a: [f32; 3], b: [f32; 3]) -> bool { (0..3).all(|k| (a[k]-b[k]).abs() < 1e-4) }

#[test]
fn offset_spacing_three_copies_has_expected_points_and_bounds() {
    let out = run(quad(), json!({"spacing":"offset","count":3, "constant_offset":[2.,0.,0.]}));
    assert_eq!((out.npoints,out.nprims()), (12,3));
    assert_eq!(bounds(&out), ([0.,0.,0.],[5.,1.,0.]));
}
#[test]
fn relative_spacing_unit_cube_scales_bounds_with_count() {
    let mut p = Vec::new();
    for z in [0.,1.] { for y in [0.,1.] { for x in [0.,1.] { p.extend([x,y,z]); } } }
    let out = run(Geometry::points_only(p).unwrap(), json!({"count":4}));
    assert_eq!(out.npoints,32);
    assert_eq!(bounds(&out), ([0.,0.,0.],[4.,1.,1.]));
}
#[test]
fn endpoint_spacing_places_last_copy_at_endpoint() {
    let out = run(quad(), json!({"spacing":"endpoint","count":5, "endpoint":[8.,0.,0.]}));
    assert_eq!(out.npoints, 20);
    assert_eq!(bounds(&out), ([0.,0.,0.],[9.,1.,0.]));
    let one = run(quad(), json!({"spacing":"endpoint","count":1, "endpoint":[8.,0.,0.]}));
    assert_eq!(bounds(&one), ([0.,0.,0.],[1.,1.,0.]));
}
#[test]
fn legacy_fixed_count_graph_loads_with_same_geometry() {
    let old = run(quad(), json!({"fit_type":"fixed_count","count":3,"fit_length":4.0,
        "relative_offset":[0.,0.,0.], "constant_offset":[2.,0.,0.]}));
    let new = run(quad(), json!({"spacing":"offset","count":3, "constant_offset":[2.,0.,0.]}));
    assert_eq!(old.p(), new.p());
    let rel = run(quad(), json!({"fit_type":"fixed_count","count":3,"relative_offset":[1.,0.,0.],"constant_offset":[0.5,0.,0.]}));
    assert_eq!(bounds(&rel), ([0.,0.,0.],[4.,1.,0.]));
}
#[test]
fn legacy_fit_length_graph_becomes_endpoint_with_same_geometry() {
    let p = params(&json!({"fit_type":"fit_length","fit_length":3.0}));
    assert_eq!(p.s("spacing").unwrap(), "endpoint");
    let out = run(quad(), json!({"fit_type":"fit_length", "fit_length":3.0}));
    assert_eq!(out.npoints,16);
    assert_eq!(bounds(&out), ([0.,0.,0.],[4.,1.,0.]));
    let zero = run(quad(), json!({"fit_type":"fit_length", "fit_length":10.0, "relative_offset":[0.,0.,0.]}));
    assert_eq!(zero.npoints,4);
    // A spacing chosen later wins over the stale legacy key still stored in the document.
    let p = params(&json!({"fit_type":"fit_length","fit_length":3.0,"spacing":"offset"}));
    assert_eq!((p.s("spacing").unwrap(), p.f("legacy_fit_length").unwrap()), ("offset".into(), -1.0));
}
#[test]
fn removed_fit_curve_reports_error() {
    assert!(run_err(quad(), json!({"fit_type":"fit_curve"})).contains("removed"));
}
#[test]
fn removed_cap_and_object_inputs_are_gone() {
    assert_eq!(NODE.inputs.len(), 1);
}
#[test]
fn adjacent_seams_weld() {
    let joined = run(quad(), json!({"count":3,"merge_distance":0.001}));
    assert_eq!((joined.npoints,joined.nprims()), (8,3));
    assert_eq!(joined.edge["group_outline"].rows(), joined.topology.nedges());
}
#[test]
fn copies_preserve_groups_materials_and_offset_corner_uvs() {
    let out = run(quad(), json!({"count":3,"uv_offset_u":0.25,"uv_offset_v":0.5}));
    assert_eq!(out.point["Cd"].data, Attr::f32(3, [1.,0.5,0.].repeat(12)).data);
    assert_eq!(out.prim["material_index"].data, AttrData::I32(vec![2,2,2]));
    assert_eq!(out.prim["group_top"].data, AttrData::I32(vec![1,1,1]));
    let uv = out.vertex["uv"].as_f32().unwrap();
    assert_eq!(&uv[8..10], &[0.25,0.5]);
    assert_eq!(&uv[16..18], &[0.5,1.]);
}
#[test]
fn registered_array_output_can_feed_transform_node() {
    let spec = crate::nodes::get("array").unwrap();
    assert_eq!(spec.outputs, &["geometry"]);
    assert_eq!(spec.inputs[0].name, "geo");
    let out = run(quad(), json!({"count":2}));
    let transform = crate::nodes::get("transform").unwrap();
    let p = Params::resolve(transform.params, json!({"translate":[0.,0.,2.]}).as_object()).unwrap();
    let moved = (transform.cook)(&[vec![Arc::new(out)]], &p).unwrap();
    assert_eq!(bounds(&moved), ([0.,0.,2.],[2.,1.,2.]));
}
