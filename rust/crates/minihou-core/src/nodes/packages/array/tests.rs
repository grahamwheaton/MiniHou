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
fn endpoint_spacing_spreads_evenly_in_any_direction() {
    let out = run(Geometry::points_only(vec![0.,0.,0.]).unwrap(),
        json!({"spacing":"endpoint","count":3, "endpoint":[0.,-2.,4.]}));
    assert_eq!(out.p(), &[0.,0.,0., 0.,-1.,2., 0.,-2.,4.]);
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
fn migration_rule_old_graphs_match_current_line_graphs() {
    // Fixed Count keeps its offsets and count.
    let old = run(quad(), json!({"fit_type":"fixed_count","count":4,"relative_offset":[1.,0.,0.],"constant_offset":[0.5,0.,0.]}));
    let new = run(quad(), json!({"spacing":"relative","count":4,"relative_offset":[1.,0.,0.],"constant_offset":[0.5,0.,0.]}));
    assert_eq!((old.p(), old.topology.vertex_point.clone()), (new.p(), new.topology.vertex_point.clone()));
    // Fit Length 3 with a unit step is four copies whose last copy sits at the endpoint (3, 0, 0).
    let old = run(quad(), json!({"fit_type":"fit_length","fit_length":3.0}));
    let new = run(quad(), json!({"spacing":"endpoint","count":4,"endpoint":[3.,0.,0.]}));
    assert_eq!(params(&json!({"fit_type":"fit_length","fit_length":3.0})).s("spacing").unwrap(), "endpoint");
    assert_eq!((old.p(), old.topology.vertex_point.clone()), (new.p(), new.topology.vertex_point.clone()));
    let old = run(quad(), json!({"fit_type":"fit_length","fit_length":5.0,"relative_offset":[0.,1.,0.],"constant_offset":[0.,1.,0.]}));
    let new = run(quad(), json!({"spacing":"endpoint","count":3,"endpoint":[0.,4.,0.]}));
    assert_eq!(old.p(), new.p());
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
#[test]
fn grid_counts_and_positions_per_axis() {
    let pt = || Geometry::points_only(vec![0.,0.,0.]).unwrap();
    let out = run(pt(), json!({"mode":"grid","spacing":"offset","count_x":3,"count_y":2,"count_z":2,"constant_offset":[1.,2.,3.]}));
    assert_eq!(out.npoints, 12);
    assert_eq!(&out.p()[..9], &[0.,0.,0., 1.,0.,0., 2.,0.,0.]);
    assert_eq!(&out.p()[9..12], &[0.,2.,0.]);
    assert_eq!(&out.p()[33..36], &[2.,2.,3.]);
    let end = run(pt(), json!({"mode":"grid","spacing":"endpoint","count_x":5,"count_y":3,"count_z":1,"endpoint":[8.,4.,9.]}));
    assert_eq!(end.npoints, 15);
    assert_eq!(bounds(&end), ([0.,0.,0.],[8.,4.,0.]));
    let rel = run(quad(), json!({"mode":"grid","count_x":2,"count_y":3,"count_z":1,"relative_offset":[1.,1.,1.]}));
    assert_eq!((rel.nprims(), bounds(&rel)), (6, ([0.,0.,0.],[2.,3.,0.])));
    // Neighbours along both axes weld: a 2 x 2 grid of unit quads shares 9 points.
    let welded = run(quad(), json!({"mode":"grid","count_x":2,"count_y":2,"count_z":1,"relative_offset":[1.,1.,1.],"merge_distance":0.001}));
    assert_eq!(welded.npoints, 9);
    assert!(run_err(pt(), json!({"mode":"grid","count_x":100,"count_y":100,"count_z":2})).contains("at most"));
}
fn near(a: &[f32], b: &[f32]) -> bool { a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5) }
#[test]
fn radial_count_places_copies_at_even_angles() {
    let pt = Geometry::points_only(vec![0.,0.,0.]).unwrap();
    let out = run(pt.clone(), json!({"mode":"radial","count":4,"radius":2.0}));
    assert!(near(out.p(), &[2.,0.,0., 0.,2.,0., -2.,0.,0., 0.,-2.,0.]), "{:?}", out.p());
    // A partial angle reaches its end: 90 degrees over three copies is 0, 45 and 90.
    let arc = run(pt.clone(), json!({"mode":"radial","count":3,"radius":1.0,"angle":90.0}));
    let h = std::f32::consts::FRAC_1_SQRT_2;
    assert!(near(arc.p(), &[1.,0.,0., h,h,0., 0.,1.,0.]));
    // About X the circle starts on +Y.
    let x = run(pt, json!({"mode":"radial","count":2,"radius":1.0,"axis":"x"}));
    assert!(near(x.p(), &[0.,1.,0., 0.,-1.,0.]));
}
#[test]
fn radial_distance_mode_derives_the_count() {
    let pt = Geometry::points_only(vec![0.,0.,0.]).unwrap();
    let tau = std::f64::consts::TAU;
    let out = run(pt.clone(), json!({"mode":"radial","radial_mode":"distance","radius":1.0,"distance":tau / 8.0}));
    assert_eq!(out.npoints, 8);
    let half = run(pt, json!({"mode":"radial","radial_mode":"distance","radius":1.0,"angle":180.0,"distance":tau / 8.0}));
    assert_eq!(half.npoints, 5);
}
#[test]
fn radial_align_rotation_turns_copies_and_can_be_off() {
    let seg = Geometry::points_only(vec![0.,0.,0., 1.,0.,0.]).unwrap();
    let on = run(seg.clone(), json!({"mode":"radial","count":4,"radius":2.0}));
    // Second copy at 90 degrees: the segment pointing along +X now points along +Y.
    assert!(near(&on.p()[6..12], &[0.,2.,0., 0.,3.,0.]), "{:?}", on.p());
    let off = run(seg, json!({"mode":"radial","count":4,"radius":2.0,"align_rotation":false}));
    assert!(near(&off.p()[6..12], &[0.,2.,0., 1.,2.,0.]));
}
fn random_params(seed: i64, count: i64) -> serde_json::Value {
    json!({"spacing":"offset","count":count,"constant_offset":[10.,0.,0.],"random_seed":seed,
        "random_position":[0.5,0.25,0.],"random_rotation":[0.,0.,30.],"random_scale":0.5,"random_uv":0.1})
}
#[test]
fn random_same_seed_same_result_different_seed_differs() {
    let a = run(quad(), random_params(3, 4));
    let b = run(quad(), random_params(3, 4));
    let c = run(quad(), random_params(4, 4));
    assert_eq!(a.p(), b.p());
    assert_eq!(a.vertex["uv"].data, b.vertex["uv"].data);
    assert_ne!(a.p(), c.p());
    // Copy 0 is randomised too.
    let plain = run(quad(), json!({"spacing":"offset","count":4,"constant_offset":[10.,0.,0.]}));
    assert_ne!(&a.p()[..12], &plain.p()[..12]);
}
#[test]
fn random_stays_within_ranges() {
    let pt = Geometry::points_only(vec![0.,0.,0., 1.,0.,0.]).unwrap();
    let out = run(pt, json!({"spacing":"offset","count":50,"constant_offset":[10.,0.,0.],"random_seed":9,
        "random_position":[0.5,0.25,0.],"random_scale":0.5}));
    for k in 0..50 {
        let a = &out.p()[6*k..6*k+3];
        let b = &out.p()[6*k+3..6*k+6];
        assert!((a[0] - 10. * k as f32).abs() <= 0.5 + 1e-4 && a[1].abs() <= 0.25 + 1e-6 && a[2] == 0.);
        let len = b[0] - a[0];
        assert!((0.5 - 1e-4..=1.5 + 1e-4).contains(&len), "uniform scale {len}");
        assert!((b[1] - a[1]).abs() < 1e-5);
    }
    let rot = run(Geometry::points_only(vec![1.,0.,0.]).unwrap(),
        json!({"spacing":"offset","count":40,"random_rotation":[0.,0.,30.],"random_seed":1}));
    for q in rot.p().chunks_exact(3) {
        let angle = (q[1] as f64).atan2(q[0] as f64).to_degrees();
        assert!(angle.abs() <= 30. + 1e-3, "{angle}");
    }
}
#[test]
fn random_copies_are_stable_when_the_count_grows() {
    let few = run(quad(), random_params(7, 3));
    let many = run(quad(), random_params(7, 6));
    assert_eq!(few.p(), &many.p()[..few.p().len()]);
    let uv = few.vertex["uv"].as_f32().unwrap();
    assert_eq!(uv, &many.vertex["uv"].as_f32().unwrap()[..uv.len()]);
}
#[test]
fn pack_makes_one_instance_per_copy_and_unpacks_to_the_plain_array() {
    let values = |pack: bool| json!({"mode":"radial","count":5,"radius":3.0,"pack":pack,"random_seed":2,
        "random_rotation":[10.,20.,30.],"random_scale":0.3,"uv_offset_u":0.5,"merge_distance":0.01});
    let inst = run(quad(), values(true));
    assert_eq!((crate::nodes::packed::instance_count(&inst), inst.packed.len(), inst.npoints), (5, 1, 5));
    let real = crate::nodes::packed::unpack(&inst, None, false).unwrap();
    // The plain array with UV offsets and welding off matches the unpacked instances.
    let plain = run(quad(), json!({"mode":"radial","count":5,"radius":3.0,"random_seed":2,
        "random_rotation":[10.,20.,30.],"random_scale":0.3}));
    assert_eq!((real.npoints, real.nprims()), (plain.npoints, plain.nprims()));
    assert!(near(real.p(), plain.p()));
    // UV offsets are off while packed.
    assert_eq!(real.vertex["uv"].as_f32().unwrap(), &quad().vertex["uv"].as_f32().unwrap().repeat(5)[..]);
}
#[test]
fn optional_copy_index_output() {
    assert!(!run(quad(), json!({"count":3})).point.contains_key("copy_index"));
    let out = run(quad(), json!({"count":3,"output_copy_index":true}));
    assert_eq!(out.point["copy_index"].data, AttrData::I32(vec![0,0,0,0, 1,1,1,1, 2,2,2,2]));
    let inst = run(quad(), json!({"count":3,"pack":true,"output_copy_index":true}));
    assert_eq!(inst.point["copy_index"].data, AttrData::I32(vec![0,1,2]));
}
#[test]
fn optional_copy_fraction_output() {
    assert!(!run(quad(), json!({"count":3})).point.contains_key("copy_fraction"));
    let out = run(Geometry::points_only(vec![0.,0.,0.]).unwrap(), json!({"count":5,"output_copy_fraction":true}));
    assert_eq!(out.point["copy_fraction"].data, AttrData::F32(vec![0.,0.25,0.5,0.75,1.]));
    let one = run(Geometry::points_only(vec![0.,0.,0.]).unwrap(), json!({"count":1,"output_copy_fraction":true}));
    assert_eq!(one.point["copy_fraction"].data, AttrData::F32(vec![0.]));
}
#[test]
fn optional_array_copies_group_numbers_each_copy() {
    assert!(!run(quad(), json!({"count":3})).prim.contains_key("array_copies"));
    let out = run(quad(), json!({"mode":"grid","count_x":2,"count_y":2,"count_z":1,"output_copies_group":true,"merge_distance":0.001}));
    assert_eq!(out.prim["array_copies"].data, AttrData::I32(vec![0,1,2,3]));
}
