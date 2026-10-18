use super::*;
use std::sync::Arc;
use crate::geometry::Geometry;
use crate::params::Params;

fn unit_cube() -> Geometry {
    let p = Params::resolve(crate::nodes::generate::BOX.params, None).unwrap();
    (crate::nodes::generate::BOX.cook)(&[vec![]], &p).unwrap()
}

fn grid_2x2() -> Geometry {
    let mut map = serde_json::Map::new();
    map.insert("size_x".into(), serde_json::json!(2.0));
    map.insert("size_y".into(), serde_json::json!(2.0));
    map.insert("columns".into(), serde_json::json!(2));
    map.insert("rows".into(), serde_json::json!(2));
    let p = Params::resolve(crate::nodes::generate::GRID.params, Some(&map)).unwrap();
    (crate::nodes::generate::GRID.cook)(&[vec![]], &p).unwrap()
}

fn line_3() -> Geometry {
    let mut map = serde_json::Map::new();
    map.insert("length".into(), serde_json::json!(3.0));
    let p = Params::resolve(crate::nodes::generate::LINE.params, Some(&map)).unwrap();
    (crate::nodes::generate::LINE.cook)(&[vec![]], &p).unwrap()
}

#[test]
fn test_measure_distance_unit_cube() {
    let cube = unit_cube();
    let mut map = serde_json::Map::new();
    map.insert("point1".into(), serde_json::json!(0));
    map.insert("point2".into(), serde_json::json!(1));
    let p = Params::resolve(NODE.params, Some(&map)).unwrap();
    let res = (NODE.cook)(&[vec![Arc::new(cube.clone())]], &p).unwrap();
    assert_eq!(res.npoints, cube.npoints);
    let dist = res.detail.get("distance").unwrap().as_f64().unwrap();
    assert!((dist - 1.0).abs() < 1e-5, "expected unit edge distance 1.0, got {dist}");
}

#[test]
fn test_measure_distance_grid_2x2() {
    let grid = grid_2x2();
    let p = Params::resolve(NODE.params, None).unwrap();
    let res = (NODE.cook)(&[vec![Arc::new(grid)]], &p).unwrap();
    let dist = res.detail.get("distance").unwrap().as_f64().unwrap();
    assert!(dist > 0.0);
}

#[test]
fn test_measure_distance_line_3() {
    let line = line_3();
    let p = Params::resolve(NODE.params, None).unwrap();
    let res = (NODE.cook)(&[vec![Arc::new(line)]], &p).unwrap();
    let dist = res.detail.get("distance").unwrap().as_f64().unwrap();
    assert!((dist - 3.0).abs() < 1e-5, "expected line distance 3.0, got {dist}");
}

#[test]
fn test_measure_distance_two_groups_min_distance() {
    let mut geo = Geometry::points_only(vec![
        0.0, 0.0, 0.0,
        0.0, 10.0, 0.0,
        2.0, 0.0, 0.0,
        5.0, 0.0, 0.0,
    ]).unwrap();
    geo.point.insert("group_g1".into(), Arc::new(crate::geometry::Attr::i32(1, vec![1, 1, 0, 0])));
    geo.point.insert("group_g2".into(), Arc::new(crate::geometry::Attr::i32(1, vec![0, 0, 1, 1])));

    let mut map = serde_json::Map::new();
    map.insert("group".into(), serde_json::json!("g1"));
    map.insert("group2".into(), serde_json::json!("g2"));
    let p = Params::resolve(NODE.params, Some(&map)).unwrap();
    let res = (NODE.cook)(&[vec![Arc::new(geo)]], &p).unwrap();
    let dist = res.detail.get("distance").unwrap().as_f64().unwrap();
    assert!((dist - 2.0).abs() < 1e-5, "expected min distance 2.0, got {dist}");
}

#[test]
fn test_measure_distance_empty_input_and_missing_group() {
    let empty = Geometry::default();
    let p = Params::resolve(NODE.params, None).unwrap();
    let res = (NODE.cook)(&[vec![Arc::new(empty)]], &p).unwrap();
    assert_eq!(res.detail.get("distance").unwrap().as_f64().unwrap(), 0.0);

    let line = line_3();
    let mut map = serde_json::Map::new();
    map.insert("group".into(), serde_json::json!("nonexistent_group"));
    let p_group = Params::resolve(NODE.params, Some(&map)).unwrap();
    let res_group = (NODE.cook)(&[vec![Arc::new(line)]], &p_group).unwrap();
    assert_eq!(res_group.detail.get("distance").unwrap().as_f64().unwrap(), 0.0);
}

#[test]
fn test_measure_distance_determinism() {
    let line = line_3();
    let p = Params::resolve(NODE.params, None).unwrap();
    let res1 = (NODE.cook)(&[vec![Arc::new(line.clone())]], &p).unwrap();
    let res2 = (NODE.cook)(&[vec![Arc::new(line)]], &p).unwrap();
    assert_eq!(res1.detail.get("distance"), res2.detail.get("distance"));
}
