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
    let p = Params::resolve(crate::nodes::registry().iter().find(|spec| spec.name == "line").unwrap().params, Some(&map)).unwrap();
    (crate::nodes::registry().iter().find(|spec| spec.name == "line").unwrap().cook)(&[vec![]], &p).unwrap()
}

#[test]
fn test_measure_length_unit_cube() {
    let cube = unit_cube();
    let p = Params::resolve(NODE.params, None).unwrap();
    let res = (NODE.cook)(&[vec![Arc::new(cube.clone())]], &p).unwrap();
    assert_eq!(res.nprims(), cube.nprims());
    let lengths = res.prim.get("length").unwrap().as_f32().unwrap();
    for &l in lengths {
        assert!((l - 4.0).abs() < 1e-5, "expected cube face perimeter 4.0, got {l}");
    }
    let total = res.detail.get("total_length").unwrap().as_f64().unwrap();
    assert!((total - 24.0).abs() < 1e-5);
}

#[test]
fn test_measure_length_grid_2x2() {
    let grid = grid_2x2();
    let p = Params::resolve(NODE.params, None).unwrap();
    let res = (NODE.cook)(&[vec![Arc::new(grid)]], &p).unwrap();
    let total = res.detail.get("total_length").unwrap().as_f64().unwrap();
    assert!(total > 0.0);
}

#[test]
fn test_measure_length_line_3() {
    let line = line_3();
    let p = Params::resolve(NODE.params, None).unwrap();
    let res = (NODE.cook)(&[vec![Arc::new(line)]], &p).unwrap();
    let lengths = res.prim.get("length").unwrap().as_f32().unwrap();
    assert_eq!(lengths.len(), 1);
    assert!((lengths[0] - 3.0).abs() < 1e-5, "expected primitive length 3.0, got {}", lengths[0]);
    let total = res.detail.get("total_length").unwrap().as_f64().unwrap();
    assert!((total - 3.0).abs() < 1e-5);
}

#[test]
fn test_measure_length_empty_input_and_missing_group() {
    let empty = Geometry::default();
    let p = Params::resolve(NODE.params, None).unwrap();
    let res = (NODE.cook)(&[vec![Arc::new(empty)]], &p).unwrap();
    assert_eq!(res.prim.get("length").unwrap().rows(), 0);
    assert_eq!(res.detail.get("total_length").unwrap().as_f64().unwrap(), 0.0);

    let line = line_3();
    let mut map = serde_json::Map::new();
    map.insert("group".into(), serde_json::json!("nonexistent_group"));
    let p_group = Params::resolve(NODE.params, Some(&map)).unwrap();
    let res_group = (NODE.cook)(&[vec![Arc::new(line)]], &p_group).unwrap();
    let lengths = res_group.prim.get("length").unwrap().as_f32().unwrap();
    assert_eq!(lengths[0], 0.0);
    assert_eq!(res_group.detail.get("total_length").unwrap().as_f64().unwrap(), 0.0);
}

#[test]
fn test_measure_length_determinism() {
    let line = line_3();
    let p = Params::resolve(NODE.params, None).unwrap();
    let res1 = (NODE.cook)(&[vec![Arc::new(line.clone())]], &p).unwrap();
    let res2 = (NODE.cook)(&[vec![Arc::new(line)]], &p).unwrap();
    assert_eq!(res1.prim.get("length"), res2.prim.get("length"));
    assert_eq!(res1.detail.get("total_length"), res2.detail.get("total_length"));
}
