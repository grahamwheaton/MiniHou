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
fn run(g: Geometry, object: Option<Geometry>, start: Option<Geometry>, end: Option<Geometry>,
       values: serde_json::Value) -> Geometry {
    let p = Params::resolve(NODE.params, values.as_object()).unwrap();
    let inputs: Vec<Vec<Arc<Geometry>>> = [Some(g), object, start, end].into_iter()
        .map(|g| g.map(|g| vec![Arc::new(g)]).unwrap_or_default()).collect();
    let out = (NODE.cook)(&inputs, &p).unwrap();
    out.validate().unwrap();
    out
}
fn bounds(g: &Geometry) -> ([f32; 3], [f32; 3]) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for q in g.p().chunks_exact(3) {
        for a in 0..3 { lo[a] = lo[a].min(q[a]); hi[a] = hi[a].max(q[a]); }
    }
    (lo, hi)
}

#[test]
fn fixed_count_three_constant_x_has_expected_points_and_bounds() {
    let out = run(quad(), None, None, None, json!({"count":3,
        "relative_offset":[0.,0.,0.], "constant_offset":[2.,0.,0.]}));
    assert_eq!((out.npoints,out.nprims()), (12,3));
    assert_eq!(bounds(&out), ([0.,0.,0.],[5.,1.,0.]));
}
#[test]
fn relative_offset_unit_cube_scales_bounds_with_count() {
    let mut p = Vec::new();
    for z in [0.,1.] { for y in [0.,1.] { for x in [0.,1.] { p.extend([x,y,z]); } } }
    let out = run(Geometry::points_only(p).unwrap(), None, None, None, json!({"count":4}));
    assert_eq!(out.npoints,32);
    assert_eq!(bounds(&out), ([0.,0.,0.],[4.,1.,1.]));
}
#[test]
fn fit_length_uses_translation_distance() {
    let out = run(quad(), None, None, None, json!({"fit_type":"fit_length",
        "fit_length":3.0}));
    assert_eq!(out.npoints,16);
    let zero = run(quad(), None, None, None, json!({"fit_type":"fit_length",
        "fit_length":10.0, "relative_offset":[0.,0.,0.]}));
    assert_eq!(zero.npoints,4);
}
#[test]
fn translated_second_input_adds_object_offset_each_step() {
    let object = Geometry::points_only(vec![2.,0.,0.]).unwrap();
    let out = run(quad(), Some(object), None, None, json!({"count":3,
        "relative_offset":[0.,0.,0.], "constant_offset":[0.5,0.,0.]}));
    assert_eq!(bounds(&out), ([0.,0.,0.],[6.,1.,0.]));
}
#[test]
fn merge_first_last_drops_expected_vertex_count() {
    let source = Geometry::points_only(vec![1.,0.,0.]).unwrap();
    let pivot = Arc::new(Geometry::points_only(vec![0.,0.,0.]).unwrap());
    let object = packed::instance_geometry(&[pivot], &[0], vec![0.,0.,0.],
        &[[[-1.,0.,0.],[0.,-1.,0.],[0.,0.,1.]]], Default::default()).unwrap();
    let options = json!({"count":3, "relative_offset":[0.,0.,0.],
        "merge_distance":0.001});
    let open = run(source.clone(), Some(object.clone()), None, None, options.clone());
    let closed = run(source, Some(object), None, None,
        json!({"count":3, "relative_offset":[0.,0.,0.],
            "merge_distance":0.001, "merge_first_last":true}));
    assert_eq!((open.npoints, closed.npoints), (3,2));
}
#[test]
fn adjacent_seams_and_caps_weld_and_caps_sit_outside_array() {
    let joined = run(quad(), None, None, None, json!({"count":3,"merge_distance":0.001}));
    assert_eq!((joined.npoints,joined.nprims()), (8,3));
    assert_eq!(joined.edge["group_outline"].rows(), joined.topology.nedges());
    let cap = Geometry::points_only(vec![0.,0.,0.]).unwrap();
    let out = run(quad(), None, Some(cap.clone()), Some(cap), json!({"count":2}));
    assert_eq!(out.npoints,10);
    assert_eq!(bounds(&out), ([-1.,0.,0.],[2.,1.,0.]));
}
#[test]
fn copies_preserve_groups_materials_and_offset_corner_uvs() {
    let out = run(quad(), None, None, None, json!({"count":3,"uv_offset_u":0.25,"uv_offset_v":0.5}));
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
    let out = run(quad(), None, None, None, json!({"count":2}));
    let transform = crate::nodes::get("transform").unwrap();
    let p = Params::resolve(transform.params, json!({"translate":[0.,0.,2.]}).as_object()).unwrap();
    let moved = (transform.cook)(&[vec![Arc::new(out)]], &p).unwrap();
    assert_eq!(bounds(&moved), ([0.,0.,2.],[2.,1.,2.]));
}
#[test]
fn deferred_curve_mode_reports_error() {
    let p = Params::resolve(NODE.params, json!({"fit_type":"fit_curve"}).as_object()).unwrap();
    let err = (NODE.cook)(&[vec![Arc::new(quad())]], &p).err().unwrap();
    assert!(err.to_string().contains("deferred"));
}
