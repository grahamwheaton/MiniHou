use super::*;
use crate::geometry::Attr;
use std::collections::BTreeSet;

fn cube() -> Geometry {
    Geometry::from_arrays(
        vec![0.,0.,0., 1.,0.,0., 1.,1.,0., 0.,1.,0.,
             0.,0.,1., 1.,0.,1., 1.,1.,1., 0.,1.,1.],
        vec![0,3,2,1, 4,5,6,7, 0,1,5,4, 1,2,6,5, 2,3,7,6, 3,0,4,7],
        vec![0,4,8,12,16,20,24], None,
    ).unwrap()
}

fn grid() -> Geometry {
    Geometry::from_arrays(
        vec![0.,0.,0., 1.,0.,0., 1.,1.,0., 0.,1.,0., 2.,0.,0., 2.,1.,0.],
        vec![0,1,2,3, 1,4,5,2], vec![0,4,8], None,
    ).unwrap()
}

fn run(g: &Geometry, params: serde_json::Value) -> Geometry {
    let p = Params::resolve(NODE.params, params.as_object()).unwrap();
    cook(&[vec![Arc::new(g.clone())]], &p).unwrap()
}

fn face_points(g: &Geometry, face: usize) -> BTreeSet<i32> {
    g.topology.vertex_point[g.topology.prim_offsets[face] as usize..g.topology.prim_offsets[face + 1] as usize]
        .iter().copied().collect()
}

#[test]
fn default_angle_separates_all_twelve_cube_edges() {
    let g = cube();
    assert_eq!(g.nedges(), 12);
    let out = run(&g, serde_json::json!({}));
    assert_eq!(out.nprims(), 6);
    assert_eq!(out.npoints, 24);
    assert_eq!(out.nedges(), 24);
    for a in 0..6 {
        for b in a + 1..6 {
            assert!(face_points(&out, a).is_disjoint(&face_points(&out, b)));
        }
    }
    for (v, &point) in out.topology.vertex_point.iter().enumerate() {
        assert_eq!(&out.p()[3 * point as usize..3 * point as usize + 3],
            &g.p()[3 * g.topology.vertex_point[v] as usize..3 * g.topology.vertex_point[v] as usize + 3]);
    }
    out.validate().unwrap();
}

#[test]
fn default_angle_leaves_flat_grid_welded() {
    let g = grid();
    let out = run(&g, serde_json::json!({}));
    assert_eq!(out.npoints, g.npoints);
    assert_eq!(out.topology.vertex_point, g.topology.vertex_point);
    assert_eq!(out.nedges(), g.nedges());
    out.validate().unwrap();
}

#[test]
fn selected_edges_split_only_grouped_edge() {
    let mut g = grid();
    let pairs = g.topology.edge_pairs();
    let shared = pairs.iter().position(|&pair| pair == (1, 2)).unwrap();
    let other = pairs.iter().position(|&pair| pair == (0, 1)).unwrap();
    let mut group = vec![0; pairs.len()];
    group[shared] = 1;
    g.edge.insert("group_cut".into(), Arc::new(Attr::i32(1, group)));
    g.edge.insert("weight".into(), Arc::new(Attr::f32(1, (0..pairs.len()).map(|i| i as f32).collect())));
    g.point.insert("id".into(), Arc::new(Attr::i32(1, (0..g.npoints as i32).collect())));
    let out = run(&g, serde_json::json!({"mode":"selected_edges", "group":"cut"}));
    assert_eq!(out.npoints, g.npoints + 2);
    assert!(face_points(&out, 0).is_disjoint(&face_points(&out, 1)));
    assert_eq!(out.nedges(), g.nedges() + 1);
    assert_eq!(out.edge["group_cut"].as_i32().unwrap().iter().filter(|&&x| x != 0).count(), 2);
    assert_eq!(out.edge["weight"].as_f32().unwrap()[other], other as f32);
    assert_eq!(out.point["id"].as_i32().unwrap().len(), out.npoints);
    out.validate().unwrap();

    let untouched = run(&g, serde_json::json!({"mode":"selected_edges", "group":""}));
    assert_eq!(untouched.topology.vertex_point, g.topology.vertex_point);
}
