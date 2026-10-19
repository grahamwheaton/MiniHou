use super::*;
use crate::geometry::Attr;
use serde_json::{json, Value};

fn quad() -> Geometry {
    let mut g = Geometry::from_arrays(vec![0.,0.,0., 1.,0.,0., 1.,1.,0., 0.,1.,0.], vec![0,1,2,3], vec![0,4], None).unwrap();
    g.vertex.insert("uv".into(), Arc::new(Attr::f32(2, vec![0.,0., 1.,0., 1.,1., 0.,1.])));
    g.vertex.insert("Second".into(), Arc::new(Attr::f32(3, vec![0.,0.,2., 1.,0.,2., 1.,1.,2., 0.,1.,2.])));
    g.prim.insert("material_index".into(), Arc::new(Attr::i32(1, vec![1])));
    g.detail.insert("materials".into(), json!(["", "Wood"]));
    g.detail.insert("material_bindings".into(), json!({"version":1,"slots":[null,"doc:wood"]}));
    g
}

fn cube() -> Geometry {
    let mut g = Geometry::from_arrays(
        vec![-1.,-1.,-1., 1.,-1.,-1., 1.,1.,-1., -1.,1.,-1., -1.,-1.,1., 1.,-1.,1., 1.,1.,1., -1.,1.,1.],
        vec![0,3,2,1, 4,5,6,7, 0,1,5,4, 1,2,6,5, 2,3,7,6, 3,0,4,7],
        vec![0,4,8,12,16,20,24], None).unwrap();
    g.vertex.insert("uv".into(), Arc::new(Attr::f32(2, (0..6).flat_map(|_| [0.,0.,1.,0.,1.,1.,0.,1.]).collect())));
    g.vertex.insert("Second".into(), Arc::new(Attr::f32(2, (0..6).flat_map(|_| [0.,0.,1.,0.,1.,1.,0.,1.]).collect())));
    g.prim.insert("material_index".into(), Arc::new(Attr::i32(1, vec![1;6])));
    g.detail.insert("materials".into(), json!(["", "Wood"]));
    g.detail.insert("material_bindings".into(), json!({"version":1,"slots":[null,"doc:wood"]}));
    g
}

fn cook(name: &str, inputs: Vec<Vec<Arc<Geometry>>>, options: Value) -> Geometry {
    let node = get(name).unwrap();
    let params = Params::resolve(node.params, options.as_object()).unwrap();
    (node.cook)(&inputs, &params).unwrap()
}

fn inherited(g: &Geometry) {
    g.validate().unwrap();
    assert_eq!(g.detail["material_bindings"]["slots"][1], "doc:wood");
    assert!(g.prim["material_index"].as_i32().unwrap().iter().all(|&i| i == 1));
    for name in ["uv", "Second"] {
        assert!(g.vertex.contains_key(name));
        assert!(g.vertex[name].as_f32().unwrap().iter().all(|v| v.is_finite()));
    }
}

#[test]
fn copy_and_blast_keep_corner_sets_and_slots() {
    let src = quad();
    let targets = Geometry::points_only(vec![0.,0.,0., 2.,0.,0.]).unwrap();
    let copied = cook("copy_to_points", vec![vec![Arc::new(src.clone())],vec![Arc::new(targets)]], json!({"pack_instances":false}));
    assert_eq!(copied.nprims(), 2);
    inherited(&copied);
    assert_eq!(copied.vertex["uv"].as_f32().unwrap()[..8], copied.vertex["uv"].as_f32().unwrap()[8..]);
    let kept = cook("blast", vec![vec![Arc::new(copied)]], json!({"group":"0","delete_non_selected":true}));
    assert_eq!(kept.nprims(), 1);
    inherited(&kept);
}

#[test]
fn subdivision_inherits_slot_and_interpolates_named_uvs() {
    let out = cook("subdivide", vec![vec![Arc::new(quad())]], json!({"depth":1,"algorithm":"linear"}));
    assert_eq!(out.nprims(), 4);
    inherited(&out);
    let uv = out.vertex["uv"].as_f32().unwrap();
    assert!(uv.chunks_exact(2).any(|p| p == [0.5,0.5]));
    let second = out.vertex["Second"].as_f32().unwrap();
    assert!(second.chunks_exact(3).any(|p| p == [0.5,0.5,2.]));
}

#[test]
fn extrude_and_loop_cut_inherit_face_slot() {
    let extruded = cook("polyextrude", vec![vec![Arc::new(quad())]], json!({"distance":0.5}));
    assert!(extruded.nprims() > 1);
    inherited(&extruded);
    let split = cook("loop_cut", vec![vec![Arc::new(quad())]], json!({"mode":"points","split_points":"0-2"}));
    assert_eq!(split.nprims(), 2);
    inherited(&split);
}

#[test]
fn bevel_keeps_parent_slot_and_corner_sets() {
    let out = cook("polybevel", vec![vec![Arc::new(quad())],vec![]], json!({"edges":"0-1","distance":0.1}));
    assert!(out.nprims() >= 1);
    inherited(&out);
}

#[test]
fn explicit_material_assignment_keeps_binding_table_parallel() {
    let out = cook("set_material", vec![vec![Arc::new(quad())]], json!({"material":"Stone"}));
    out.validate().unwrap();
    assert_eq!(out.detail["materials"], json!(["", "Wood", "Stone"]));
    assert_eq!(out.detail["material_bindings"]["slots"], json!([null,"doc:wood",null]));
    assert_eq!(out.prim["material_index"].as_i32().unwrap(), &[2]);
}

#[test]
fn source_less_cap_is_neutral_with_invalid_uvs() {
    let mut g = quad();
    let first_prim = g.nprims();
    let first_corner = g.nvertices();
    let mut vp = g.topology.vertex_point.clone();
    vp.extend([0,3,2,1]);
    g.topology = Arc::new(crate::geometry::Topology {vertex_point:vp,prim_offsets:vec![0,4,8],closed:None});
    for attr in g.vertex.values_mut() {
        let mut v = attr.as_f32().unwrap().to_vec();
        v.extend(vec![0.; attr.width * 4]);
        *attr = Arc::new(Attr::f32(attr.width,v));
    }
    g.prim.insert("material_index".into(), Arc::new(Attr::i32(1,vec![1,0])));
    material_tables::neutral_new_faces(&mut g,first_prim,first_corner);
    assert_eq!(g.prim["material_index"].as_i32().unwrap(), &[1,0]);
    assert!(g.vertex["uv"].as_f32().unwrap()[8..].iter().all(|v| v.is_nan()));
    assert!(g.vertex["Second"].as_f32().unwrap()[12..].iter().all(|v| v.is_nan()));
    g.validate().unwrap();
}

#[test]
fn polyfill_new_cap_has_neutral_slot() {
    let mut g = cube();
    let mut topo = (*g.topology).clone();
    topo.vertex_point.truncate(20);
    topo.prim_offsets.truncate(6);
    g.topology = Arc::new(topo);
    for attr in g.vertex.values_mut() {
        let mut data = attr.as_f32().unwrap().to_vec();
        data.truncate(20 * attr.width);
        *attr = Arc::new(Attr::f32(attr.width, data));
    }
    g.prim.insert("material_index".into(), Arc::new(Attr::i32(1,vec![1;5])));
    let out = cook("polyfill", vec![vec![Arc::new(g)]], json!({}));
    assert_eq!(out.nprims(), 6);
    let cap = out.nprims() - 1;
    assert_eq!(out.prim["material_index"].as_i32().unwrap()[cap], 0);
    let start = out.topology.prim_offsets[cap] as usize * 2;
    assert!(out.vertex["uv"].as_f32().unwrap()[start..].iter().all(|v| v.is_nan()));
}

#[test]
fn boolean_preserves_source_faces_and_slots() {
    let a = cube();
    let mut b = cube();
    let mut p = b.p().to_vec();
    for xyz in p.chunks_exact_mut(3) { xyz[0] += 0.5; }
    b.set_p(p);
    let out = cook("boolean", vec![vec![Arc::new(a)],vec![Arc::new(b)]], json!({"operation":"subtract"}));
    assert!(out.nprims() > 0);
    inherited(&out);
}

#[test]
fn boolean_marks_missing_named_uv_on_cutting_surface() {
    let a = cube();
    let mut b = cube();
    let mut p = b.p().to_vec();
    for xyz in p.chunks_exact_mut(3) { xyz[0] += 0.5; }
    b.set_p(p);
    b.vertex.remove("Second");
    let out = cook("boolean", vec![vec![Arc::new(a)],vec![Arc::new(b)]], json!({"operation":"subtract"}));
    let from_b = out.prim["group_from_b"].as_i32().unwrap();
    assert!(from_b.iter().any(|&v| v == 1));
    let second = out.vertex["Second"].as_f32().unwrap();
    for (face, &b_face) in from_b.iter().enumerate() {
        let start = out.topology.prim_offsets[face] as usize * 2;
        let end = out.topology.prim_offsets[face + 1] as usize * 2;
        assert!(second[start..end].iter().all(|v| if b_face == 1 { v.is_nan() } else { v.is_finite() }));
    }
}

#[test]
fn voronoi_fracture_marks_new_interiors_neutral() {
    let seeds = Geometry::points_only(vec![-0.5,0.,0., 0.5,0.,0.]).unwrap();
    let out = cook("voronoi_fracture", vec![vec![Arc::new(cube())],vec![Arc::new(seeds)]], json!({}));
    out.validate().unwrap();
    let inside = out.prim["group_inside"].as_i32().unwrap();
    assert!(inside.iter().any(|&v| v == 0));
    assert!(inside.iter().any(|&v| v == 1));
    let slots = out.prim["material_index"].as_i32().unwrap();
    let uv = out.vertex["uv"].as_f32().unwrap();
    for (face, &interior) in inside.iter().enumerate() {
        let start = out.topology.prim_offsets[face] as usize * 2;
        let end = out.topology.prim_offsets[face + 1] as usize * 2;
        if interior == 1 {
            assert_eq!(slots[face], 0);
            assert!(uv[start..end].iter().all(|v| v.is_nan()));
        } else {
            assert_eq!(slots[face], 1);
            assert!(uv[start..end].iter().all(|v| v.is_finite()));
        }
    }
}

#[test]
fn voronoi_split_has_only_source_faces() {
    let seeds = Geometry::points_only(vec![-0.5,0.,0., 0.5,0.,0.]).unwrap();
    let out = cook("voronoi_split", vec![vec![Arc::new(cube())],vec![Arc::new(seeds)]], json!({}));
    assert!(out.nprims() > 6);
    inherited(&out);
}

#[test]
fn boolean_fracture_keeps_source_slots_on_both_sides() {
    let a = cube();
    let mut b = cube();
    let mut p = b.p().to_vec();
    for xyz in p.chunks_exact_mut(3) { xyz[0] += 0.5; }
    b.set_p(p);
    let out = cook("boolean_fracture", vec![vec![Arc::new(a)],vec![Arc::new(b)]], json!({}));
    assert!(out.nprims() > 0);
    inherited(&out);
}

#[test]
fn mountain_displaces_along_normals_with_houdini_options() {
    let q = quad();
    let out = cook("mountain", vec![vec![Arc::new(q.clone())]], json!({
        "amplitude": [0.0, 0.0, 0.5],
        "noise_type": "simplex",
        "fractal_type": "terrain",
        "blend": 0.8
    }));
    out.validate().unwrap();
    assert_eq!(out.npoints, q.npoints);
    let orig_p = q.p();
    let new_p = out.p();
    // Normal of planar quad is along +Z (or -Z) so Z positions change
    assert!(new_p.iter().zip(orig_p).any(|(a, b)| (a - b).abs() > 1e-4));
}

#[test]
fn noise_houdini_parity_features() {
    let q = quad();
    // Test vector mode writing to Cd with remapping, clamping and channel masking
    let out = cook("noise", vec![vec![Arc::new(q.clone())]], json!({
        "attrib": "Cd",
        "mode": "vector",
        "channel_z": false,
        "range_values": "positive",
        "enable_remap": true,
        "remap_min": 0.2,
        "remap_max": 0.8,
        "clamp_min": true,
        "min_value": 0.1,
        "operation": "set"
    }));
    out.validate().unwrap();
    assert!(out.point.contains_key("Cd"));
    let cd = out.point["Cd"].as_f32().unwrap();
    assert_eq!(cd.len(), q.npoints * 3);
    for chunk in cd.chunks_exact(3) {
        assert!(chunk[0] >= 0.1);
        assert_eq!(chunk[2], 0.0); // channel_z was false
    }

    // Test primitive class noise
    let out_prim = cook("noise", vec![vec![Arc::new(q.clone())]], json!({
        "attrib": "density",
        "class": "primitive",
        "mode": "scalar",
        "operation": "set",
        "range_values": "positive"
    }));
    out_prim.validate().unwrap();
    assert!(out_prim.prim.contains_key("density"));
    assert_eq!(out_prim.prim["density"].as_f32().unwrap().len(), q.nprims());
}
