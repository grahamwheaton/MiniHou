use super::*;
use std::sync::Arc;
use crate::{geometry::Attr, params::Params};

fn box_with_group() -> Geometry {
    let mut g = Geometry::from_arrays(
        vec![0.,0.,0., 1.,0.,0., 1.,1.,0., 0.,1.,0., 0.,0.,1., 1.,0.,1., 1.,1.,1., 0.,1.,1.],
        vec![0,3,2,1, 4,5,6,7, 0,1,5,4, 1,2,6,5, 2,3,7,6, 3,0,4,7],
        vec![0,4,8,12,16,20,24], None,
    ).unwrap();
    g.prim.insert("group_pick".into(), Arc::new(Attr::i32(1, vec![1,0,0,0,0,0])));
    g.prim.insert("group_empty".into(), Arc::new(Attr::i32(1, vec![0;6])));
    g
}

fn run(g: &Geometry, value: serde_json::Value) -> (Geometry, Geometry) {
    let p = Params::resolve(NODE.params, value.as_object()).unwrap();
    pieces(g, &p).unwrap()
}

#[test]
fn split_box_face_group_and_invert() {
    let g = box_with_group();
    let (picked, rest) = run(&g, serde_json::json!({"group":"pick", "group_type":"faces"}));
    assert_eq!((picked.nprims(), rest.nprims()), (1, 5));
    assert_eq!(picked.topology.vertex_point.len(), 4);
    assert_eq!(rest.topology.vertex_point.len(), 20);
    assert_eq!(group::flags(&picked.prim["group_pick"]), vec![true]);
    let (inverted, other) = run(&g, serde_json::json!({"group":"pick", "invert_selection":true}));
    assert_eq!(inverted.topology.vertex_point, rest.topology.vertex_point);
    assert_eq!(other.topology.vertex_point, picked.topology.vertex_point);
    let p = Params::resolve(NODE.params, serde_json::json!({"group":"pick"}).as_object()).unwrap();
    assert_eq!(cook(&[vec![Arc::new(g)]], &p).unwrap().nprims(), 1);
}

#[test]
fn split_node_outputs_selected_and_remainder_geometry() {
    let g = box_with_group();
    let p = Params::resolve(NODE.params, serde_json::json!({"group":"pick"}).as_object()).unwrap();
    let outputs = NODE.cook_outputs(&[vec![Arc::new(g.clone())]], &p).unwrap();
    assert_eq!(NODE.outputs, &["geometry", "remainder"]);
    assert_eq!(outputs.0.len(), 2);
    assert_eq!((outputs[0].nprims(), outputs[1].nprims()), (1, 5));
    assert_eq!(outputs[0].topology.vertex_point, vec![0, 3, 2, 1]);
    assert_eq!(outputs[1].topology.vertex_point, g.topology.vertex_point[4..]);
    assert_eq!(group::flags(&outputs[0].prim["group_pick"]), vec![true]);
    assert_eq!(group::flags(&outputs[1].prim["group_pick"]), vec![false; 5]);
    outputs[0].validate().unwrap();
    outputs[1].validate().unwrap();
}

#[test]
fn empty_group_returns_input_unchanged() {
    let g = box_with_group();
    for group_name in ["", "empty"] {
        let (selected, rest) = run(&g, serde_json::json!({"group":group_name, "invert_selection":true}));
        assert_eq!(selected.topology.vertex_point, g.topology.vertex_point);
        assert_eq!(selected.point.len(), g.point.len());
        assert_eq!(selected.prim.len(), g.prim.len());
        assert_eq!(rest.nprims(), 0);
    }
}

#[test]
fn unused_groups_are_removed_per_piece() {
    let g = box_with_group();
    let (picked, rest) = run(&g, serde_json::json!({"group":"pick", "delete_unused_groups":true}));
    assert!(picked.prim.contains_key("group_pick"));
    assert!(!picked.prim.contains_key("group_empty"));
    assert!(!rest.prim.contains_key("group_pick"));
}

#[test]
fn extract_selected_point_and_edge_without_a_whole_face() {
    let g = box_with_group();
    let points = Params::resolve(NODE.params, serde_json::json!({"group":"0", "group_type":"points", "extract_elements":true}).as_object()).unwrap();
    let picked = cook(&[vec![Arc::new(g.clone())]], &points).unwrap();
    assert_eq!(picked.npoints, 1);
    assert_eq!(picked.nprims(), 0);
    let outputs = NODE.cook_outputs(&[vec![Arc::new(g.clone())]], &points).unwrap();
    assert_eq!(outputs.0.len(), 2);
    assert_eq!(outputs[0].npoints, 1);
    assert_eq!(outputs[0].nprims(), 0);
    assert_eq!(outputs[1].topology.vertex_point, g.topology.vertex_point);
    let edges = Params::resolve(NODE.params, serde_json::json!({"group":"p0-1", "group_type":"edges", "extract_elements":true}).as_object()).unwrap();
    let picked = cook(&[vec![Arc::new(g.clone())]], &edges).unwrap();
    assert_eq!(picked.npoints, 2);
    assert_eq!(picked.nprims(), 1);
    assert!(!picked.topology.is_closed(0));
    picked.validate().unwrap();
    let outputs = NODE.cook_outputs(&[vec![Arc::new(g.clone())]], &edges).unwrap();
    assert_eq!(outputs.0.len(), 2);
    assert_eq!(outputs[0].npoints, 2);
    assert_eq!(outputs[0].nprims(), 1);
    assert_eq!(outputs[1].topology.vertex_point, g.topology.vertex_point);
    outputs[0].validate().unwrap();
    outputs[1].validate().unwrap();
}
