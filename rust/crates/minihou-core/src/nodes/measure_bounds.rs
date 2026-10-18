//! Node `measure_bounds`: bounding box size and centre (detail attributes bbox_size vec3, bbox_center vec3).

use super::{Inputs, NodeSpec, GEO};
use crate::error::Result;
use crate::geometry::Geometry;
use crate::params::Params;

pub const MEASURE_BOUNDS: NodeSpec = NodeSpec {
    name: "measure_bounds",
    inputs: GEO,
    params: crate::params!("group": ""),
    cook: measure_bounds,
};

fn measure_bounds(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = MEASURE_BOUNDS.one(inputs, 0)?;
    let mut out = (*g).clone();
    let group = p.s("group")?;

    if g.npoints == 0 {
        out.detail.insert("bbox_size".into(), serde_json::json!([0.0, 0.0, 0.0]));
        out.detail.insert("bbox_center".into(), serde_json::json!([0.0, 0.0, 0.0]));
        return Ok(out);
    }

    let sel = if group.trim().is_empty() {
        vec![true; g.npoints]
    } else {
        super::group::mask(&g, "point", &group).unwrap_or_else(|_| vec![false; g.npoints])
    };

    if sel.iter().all(|&b| !b) {
        out.detail.insert("bbox_size".into(), serde_json::json!([0.0, 0.0, 0.0]));
        out.detail.insert("bbox_center".into(), serde_json::json!([0.0, 0.0, 0.0]));
        return Ok(out);
    }

    let p_arr = g.p();
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for (i, &selected) in sel.iter().enumerate() {
        if !selected {
            continue;
        }
        for k in 0..3 {
            let val = p_arr[i * 3 + k];
            if val < min[k] {
                min[k] = val;
            }
            if val > max[k] {
                max[k] = val;
            }
        }
    }

    let size = [
        (max[0] - min[0]) as f64,
        (max[1] - min[1]) as f64,
        (max[2] - min[2]) as f64,
    ];
    let center = [
        ((min[0] + max[0]) * 0.5) as f64,
        ((min[1] + max[1]) * 0.5) as f64,
        ((min[2] + max[2]) * 0.5) as f64,
    ];

    out.detail.insert("bbox_size".into(), serde_json::json!(size));
    out.detail.insert("bbox_center".into(), serde_json::json!(center));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use super::*;
    use crate::geometry::Geometry;
    use crate::params::Params;

    fn unit_cube() -> Geometry {
        let p = Params::resolve(super::super::generate::BOX.params, None).unwrap();
        (super::super::generate::BOX.cook)(&[vec![]], &p).unwrap()
    }

    fn grid_2x2() -> Geometry {
        let mut map = serde_json::Map::new();
        map.insert("size_x".into(), serde_json::json!(2.0));
        map.insert("size_y".into(), serde_json::json!(2.0));
        map.insert("columns".into(), serde_json::json!(2));
        map.insert("rows".into(), serde_json::json!(2));
        let p = Params::resolve(super::super::generate::GRID.params, Some(&map)).unwrap();
        (super::super::generate::GRID.cook)(&[vec![]], &p).unwrap()
    }

    fn line_3() -> Geometry {
        let mut map = serde_json::Map::new();
        map.insert("length".into(), serde_json::json!(3.0));
        let p = Params::resolve(super::super::generate::LINE.params, Some(&map)).unwrap();
        (super::super::generate::LINE.cook)(&[vec![]], &p).unwrap()
    }

    #[test]
    fn test_measure_bounds_unit_cube() {
        let cube = unit_cube();
        let p = Params::resolve(MEASURE_BOUNDS.params, None).unwrap();
        let res = (MEASURE_BOUNDS.cook)(&[vec![Arc::new(cube.clone())]], &p).unwrap();
        assert_eq!(res.npoints, cube.npoints);
        let size = res.detail.get("bbox_size").unwrap().as_array().unwrap();
        let center = res.detail.get("bbox_center").unwrap().as_array().unwrap();
        for s in size {
            assert!((s.as_f64().unwrap() - 1.0).abs() < 1e-5);
        }
        for c in center {
            assert!(c.as_f64().unwrap().abs() < 1e-5);
        }
    }

    #[test]
    fn test_measure_bounds_grid_2x2() {
        let grid = grid_2x2();
        let p = Params::resolve(MEASURE_BOUNDS.params, None).unwrap();
        let res = (MEASURE_BOUNDS.cook)(&[vec![Arc::new(grid)]], &p).unwrap();
        let size = res.detail.get("bbox_size").unwrap().as_array().unwrap();
        let sx = size[0].as_f64().unwrap();
        let sz = size[2].as_f64().unwrap();
        assert!((sx - 2.0).abs() < 1e-5 || (size[1].as_f64().unwrap() - 2.0).abs() < 1e-5);
        assert!((sz - 2.0).abs() < 1e-5 || (size[1].as_f64().unwrap() - 2.0).abs() < 1e-5);
    }

    #[test]
    fn test_measure_bounds_line_3() {
        let line = line_3();
        let p = Params::resolve(MEASURE_BOUNDS.params, None).unwrap();
        let res = (MEASURE_BOUNDS.cook)(&[vec![Arc::new(line)]], &p).unwrap();
        let size = res.detail.get("bbox_size").unwrap().as_array().unwrap();
        let max_dim = size.iter().map(|v| v.as_f64().unwrap()).fold(0.0f64, f64::max);
        assert!((max_dim - 3.0).abs() < 1e-5, "expected line length 3.0, got {max_dim}");
    }

    #[test]
    fn test_measure_bounds_empty_input_and_missing_group() {
        let empty = Geometry::default();
        let p = Params::resolve(MEASURE_BOUNDS.params, None).unwrap();
        let res = (MEASURE_BOUNDS.cook)(&[vec![Arc::new(empty)]], &p).unwrap();
        let size = res.detail.get("bbox_size").unwrap().as_array().unwrap();
        let center = res.detail.get("bbox_center").unwrap().as_array().unwrap();
        for s in size { assert_eq!(s.as_f64().unwrap(), 0.0); }
        for c in center { assert_eq!(c.as_f64().unwrap(), 0.0); }

        let cube = unit_cube();
        let mut map = serde_json::Map::new();
        map.insert("group".into(), serde_json::json!("nonexistent_group"));
        let p_group = Params::resolve(MEASURE_BOUNDS.params, Some(&map)).unwrap();
        let res_group = (MEASURE_BOUNDS.cook)(&[vec![Arc::new(cube)]], &p_group).unwrap();
        let size_grp = res_group.detail.get("bbox_size").unwrap().as_array().unwrap();
        for s in size_grp { assert_eq!(s.as_f64().unwrap(), 0.0); }
    }

    #[test]
    fn test_measure_bounds_determinism() {
        let cube = unit_cube();
        let p = Params::resolve(MEASURE_BOUNDS.params, None).unwrap();
        let res1 = (MEASURE_BOUNDS.cook)(&[vec![Arc::new(cube.clone())]], &p).unwrap();
        let res2 = (MEASURE_BOUNDS.cook)(&[vec![Arc::new(cube)]], &p).unwrap();
        assert_eq!(res1.detail.get("bbox_size"), res2.detail.get("bbox_size"));
        assert_eq!(res1.detail.get("bbox_center"), res2.detail.get("bbox_center"));
    }
}
