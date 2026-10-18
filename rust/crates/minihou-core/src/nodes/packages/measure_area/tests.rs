use super::*;
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
fn test_measure_area_unit_cube() {
    let cube = unit_cube();
    let p = Params::resolve(NODE.params, None).unwrap();
    let res = (NODE.cook)(&[vec![Arc::new(cube.clone())]], &p).unwrap();
    assert_eq!(res.nprims(), cube.nprims());
    let areas = res.prim.get("area").unwrap().as_f32().unwrap();
    for &a in areas {
        assert!((a - 1.0).abs() < 1e-5, "expected cube face area 1.0, got {a}");
    }
    let total = res.detail.get("total_area").unwrap().as_f64().unwrap();
    assert!((total - 6.0).abs() < 1e-5, "expected total area 6.0, got {total}");
}

#[test]
fn test_measure_area_grid_2x2() {
    let grid = grid_2x2();
    let p = Params::resolve(NODE.params, None).unwrap();
    let res = (NODE.cook)(&[vec![Arc::new(grid)]], &p).unwrap();
    let total = res.detail.get("total_area").unwrap().as_f64().unwrap();
    assert!((total - 4.0).abs() < 1e-5, "expected grid area 4.0, got {total}");
}

#[test]
fn test_measure_area_line_3() {
    let line = line_3();
    let p = Params::resolve(NODE.params, None).unwrap();
    let res = (NODE.cook)(&[vec![Arc::new(line)]], &p).unwrap();
    let areas = res.prim.get("area").unwrap().as_f32().unwrap();
    assert_eq!(areas[0], 0.0);
    let total = res.detail.get("total_area").unwrap().as_f64().unwrap();
    assert_eq!(total, 0.0);
}

#[test]
fn test_measure_area_empty_input_and_missing_group() {
    let empty = Geometry::default();
    let p = Params::resolve(NODE.params, None).unwrap();
    let res = (NODE.cook)(&[vec![Arc::new(empty)]], &p).unwrap();
    assert_eq!(res.prim.get("area").unwrap().rows(), 0);
    assert_eq!(res.detail.get("total_area").unwrap().as_f64().unwrap(), 0.0);

    let cube = unit_cube();
    let mut map = serde_json::Map::new();
    map.insert("group".into(), serde_json::json!("nonexistent_group"));
    let p_group = Params::resolve(NODE.params, Some(&map)).unwrap();
    let res_group = (NODE.cook)(&[vec![Arc::new(cube)]], &p_group).unwrap();
    let areas = res_group.prim.get("area").unwrap().as_f32().unwrap();
    for &a in areas {
        assert_eq!(a, 0.0);
    }
    assert_eq!(res_group.detail.get("total_area").unwrap().as_f64().unwrap(), 0.0);
}

#[test]
fn test_measure_area_determinism() {
    let cube = unit_cube();
    let p = Params::resolve(NODE.params, None).unwrap();
    let res1 = (NODE.cook)(&[vec![Arc::new(cube.clone())]], &p).unwrap();
    let res2 = (NODE.cook)(&[vec![Arc::new(cube)]], &p).unwrap();
    assert_eq!(res1.prim.get("area"), res2.prim.get("area"));
    assert_eq!(res1.detail.get("total_area"), res2.detail.get("total_area"));
}
