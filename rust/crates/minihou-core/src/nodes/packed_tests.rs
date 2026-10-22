use super::*;

/// Unit cube at `offset` (8 points, 6 quads) with a point attribute and a prim group.
pub(crate) fn cube(offset: [f32; 3]) -> Geometry {
    let p: Vec<f32> = (0..8).flat_map(|c| [0, 1, 2].map(|k| ((c >> k) & 1) as f32 + offset[k])).collect();
    let faces = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
    let vp: Vec<i32> = faces.iter().flatten().copied().collect();
    let mut g = Geometry::from_arrays(p, vp, (0..=6).map(|i| i * 4).collect(), None).unwrap();
    g.point.insert("id".into(), Arc::new(Attr::i32(1, (0..8).collect())));
    g.prim.insert("group_top".into(), Arc::new(Attr::i32(1, vec![0, 0, 0, 1, 0, 0])));
    g
}

pub(crate) fn assert_same(a: &Geometry, b: &Geometry) {
    assert_eq!((a.npoints, a.nprims()), (b.npoints, b.nprims()));
    assert_eq!(a.topology.vertex_point, b.topology.vertex_point);
    assert_eq!(a.topology.prim_offsets, b.topology.prim_offsets);
    for (x, y) in a.p().iter().zip(b.p()) {
        assert!((x - y).abs() < 1e-5, "{x} vs {y}");
    }
    for (ta, tb) in [(&a.point, &b.point), (&a.vertex, &b.vertex), (&a.prim, &b.prim)] {
        let names = |t: &AttrTable| t.keys().filter(|k| *k != "P").cloned().collect::<Vec<_>>();
        assert_eq!(names(ta), names(tb));
        for (k, x) in ta.iter().filter(|(k, _)| *k != "P") {
            match (&x.data, &tb[k].data) {
                (AttrData::F32(u), AttrData::F32(v)) => assert!(u.iter().zip(v).all(|(p, q)| (p - q).abs() < 1e-5), "{k}"),
                (u, v) => assert_eq!(u, v, "{k}"),
            }
        }
    }
}

#[test]
fn pack_all_makes_one_instance_and_round_trips() {
    let g = cube([2.0, 0.0, 0.0]);
    let packed = pack(&g, PackMode::All, None, true).unwrap();
    assert_eq!((packed.npoints, packed.nprims(), instance_count(&packed), packed.packed.len()), (1, 0, 1, 1));
    assert_eq!(&packed.p()[..3], &[2.5, 0.5, 0.5]);
    assert_eq!(instance_bounds(&packed, 0), Some(([2.0, 0.0, 0.0], [3.0, 1.0, 1.0])));
    let back = unpack(&packed, None, false).unwrap();
    assert!(back.packed.is_empty() && !back.point.contains_key(SOURCE));
    assert_same(&back, &g);
}

#[test]
fn pack_by_group_keeps_the_rest_unpacked() {
    let g = cube([0.0; 3]);
    let mask: Vec<bool> = (0..6).map(|i| i == 3).collect();
    let packed = pack(&g, PackMode::Group, Some(&mask), true).unwrap();
    assert_eq!((packed.nprims(), instance_count(&packed)), (5, 1));
    assert_eq!((packed.packed[0].nprims(), packed.packed[0].npoints), (1, 4));
    let back = unpack(&packed, None, false).unwrap();
    // the five other faces keep all 8 points; the packed face comes back with its own 4
    assert_eq!((back.nprims(), back.npoints, instance_count(&back)), (6, 12, 0));
    let face = &back.p()[8 * 3..];
    assert!(face.chunks_exact(3).all(|q| (q[1] - 1.0).abs() < 1e-6));
}

#[test]
fn pack_by_connectivity_packs_each_piece() {
    let parts = vec![cube([0.0; 3]), cube([3.0, 0.0, 0.0]), cube([6.0, 0.0, 0.0])];
    let g = merge_all(parts).unwrap();
    let packed = pack(&g, PackMode::Connectivity, None, true).unwrap();
    assert_eq!((packed.npoints, packed.nprims(), instance_count(&packed), packed.packed.len()), (3, 0, 3, 3));
    assert!(packed.packed.iter().all(|s| s.npoints == 8 && s.nprims() == 6));
    assert_same(&unpack(&packed, None, false).unwrap(), &g);
}

#[test]
fn nested_pack_and_unpack() {
    let g = cube([1.0, 2.0, 3.0]);
    let inner = pack(&g, PackMode::All, None, true).unwrap();
    let mut outer = pack(&inner, PackMode::All, None, false).unwrap();
    // move and rotate the outer instance; the nested one follows
    let xf = [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
    outer.set_p(vec![10.0, 0.0, 0.0]);
    transform_instances(&mut outer, &xf, &[]);
    let one = unpack(&outer, None, false).unwrap();
    assert_eq!(instance_count(&one), 1);
    let all = unpack(&outer, None, true).unwrap();
    assert_eq!((instance_count(&all), all.npoints), (0, 8));
    assert_same(&unpack(&one, None, false).unwrap(), &all);
    for (q, o) in all.p().chunks_exact(3).zip(g.p().chunks_exact(3)) {
        let w = math::apply(&xf, [o[0] as f64, o[1] as f64, o[2] as f64]);
        assert!((0..3).all(|k| (q[k] as f64 - w[k] - [10.0, 0.0, 0.0][k]).abs() < 1e-5));
    }
    let round = unpack(&pack(&inner, PackMode::All, None, true).unwrap(), None, true).unwrap();
    assert_same(&round, &g);
}

#[test]
fn unpack_selected_leaves_other_instances_packed() {
    let src = Arc::new(cube([0.0; 3]));
    let inst = instance_geometry(&[src.clone()], &[0, 0, 0], vec![0., 0., 0., 5., 0., 0., 9., 0., 0.], &[IDENTITY; 3], AttrTable::new()).unwrap();
    let out = unpack(&inst, Some(&[false, true, false]), false).unwrap();
    assert_eq!((instance_count(&out), out.nprims(), out.packed.len()), (2, 6, 1));
    assert!(Arc::ptr_eq(&out.packed[0], &src));
}

#[test]
fn thousand_instances_share_one_source() {
    let src = Arc::new(cube([0.0; 3]));
    let n = 1000;
    let p: Vec<f32> = (0..n).flat_map(|i| [i as f32, 0.0, 0.0]).collect();
    let inst = instance_geometry(&vec![src.clone(); n], &vec![0; n], p, &vec![IDENTITY; n], AttrTable::new()).unwrap();
    assert_eq!((inst.packed.len(), instance_count(&inst)), (1, n));
    assert!(Arc::ptr_eq(&inst.packed[0], &src));
    // per instance: P 12 + xform 36 + source 4 bytes; the cube is counted once; + the empty prim_offsets entry
    let bytes = memory_bytes(&inst);
    assert_eq!(bytes, n * 52 + memory_bytes(&src) + 4);
    assert!(bytes * 4 < memory_bytes(&unpack(&inst, None, false).unwrap()));
}

#[test]
fn merge_dedups_shared_sources_and_renumbers() {
    let a = Arc::new(cube([0.0; 3]));
    let b = Arc::new(cube([5.0; 3]));
    let one = instance_geometry(&[a.clone()], &[0], vec![0.0; 3], &[IDENTITY], AttrTable::new()).unwrap();
    let two = instance_geometry(&[b.clone(), a.clone()], &[0, 1], vec![0.0; 6], &[IDENTITY; 2], AttrTable::new()).unwrap();
    let m = merge_all(vec![cube([9.0; 3]), one, two]).unwrap();
    assert_eq!((m.packed.len(), instance_count(&m)), (2, 3));
    assert!(Arc::ptr_eq(&m.packed[source_of(&m, 8).unwrap()], &a));
    assert!(Arc::ptr_eq(&m.packed[source_of(&m, 9).unwrap()], &b));
    assert!(Arc::ptr_eq(&m.packed[source_of(&m, 10).unwrap()], &a));
}

#[test]
fn display_geometry_places_instances_and_leaves_plain_geometry_borrowed() {
    let g = cube([0.0; 3]);
    assert!(matches!(display_geometry(&g).unwrap(), std::borrow::Cow::Borrowed(_)));
    let src = Arc::new(g);
    let inst = instance_geometry(&[src], &[0, 0], vec![0., 0., 0., 4., 0., 0.], &[IDENTITY; 2], AttrTable::new()).unwrap();
    let shown = display_geometry(&inst).unwrap();
    assert_eq!((shown.npoints, shown.nprims(), instance_count(&shown)), (16, 12, 0));
    assert_eq!(crate::preview::bounds(shown.p()), ([0.0; 3], [5.0, 1.0, 1.0]));
}

#[test]
fn estimated_display_counts_include_nested_sources_and_detect_overflow() {
    let mut source = Arc::new(cube([0.0; 3]));
    let two = instance_geometry(&[source.clone()], &[0, 0], vec![0.; 6], &[IDENTITY; 2], AttrTable::new()).unwrap();
    assert_eq!(estimated_display_counts(&two), Some((16, 24)));
    source = Arc::new(two);
    for _ in 0..7 {
        let inst = instance_geometry(&[source.clone()], &vec![0; 1000], vec![0.; 3000], &vec![IDENTITY; 1000], AttrTable::new()).unwrap();
        source = Arc::new(inst);
    }
    assert_eq!(estimated_display_counts(&source), None);
}

#[test]
fn bounds_proxy_keeps_ordinary_geometry_and_nested_transforms() {
    let inner = instance_geometry(&[Arc::new(cube([0.0; 3]))], &[0], vec![3., 0., 0.], &[IDENTITY], AttrTable::new()).unwrap();
    let outer = instance_geometry(&[Arc::new(inner)], &[0], vec![10., 0., 0.], &[IDENTITY], AttrTable::new()).unwrap();
    let mixed = merge_all(vec![cube([0.0; 3]), outer]).unwrap();
    let shown = bounds_proxy_geometry(&mixed).unwrap();
    assert_eq!((shown.npoints, shown.nprims(), instance_count(&shown)), (16, 12, 0));
    assert_eq!(crate::preview::bounds(shown.p()), ([0.; 3], [14., 1., 1.]));
    assert_eq!((mixed.npoints, mixed.nprims(), instance_count(&mixed)), (9, 6, 1));
    assert_eq!(shown.prim["material_index"].as_i32().unwrap()[6..], [-1; 6]);
    let scaled = instance_geometry(&[Arc::new(cube([0.0; 3]))], &[0], vec![10.,0.,0.],
        &[[[-2.,0.,0.], [0.,3.,0.], [0.,0.,1.]]], AttrTable::new()).unwrap();
    let proxy = bounds_proxy_geometry(&scaled).unwrap();
    assert_eq!(crate::preview::bounds(proxy.p()), ([8.,0.,0.], [10.,3.,1.]));
}
