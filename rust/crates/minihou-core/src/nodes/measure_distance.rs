//! Node `measure_distance`: distance between two points or two element groups (detail attribute distance; points from a seed index or the first/last point).

use super::{Inputs, NodeSpec, GEO};
use crate::error::Result;
use crate::geometry::Geometry;
use crate::params::Params;

pub const MEASURE_DISTANCE: NodeSpec = NodeSpec {
    name: "measure_distance",
    inputs: GEO,
    params: crate::params!(
        "group": "";
        "group2": "";
        "point1": 0;
        "point2": -1;
    ),
    cook: measure_distance,
};

fn measure_distance(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = MEASURE_DISTANCE.one(inputs, 0)?;
    let mut out = (*g).clone();
    let group1 = p.s("group")?;
    let group2 = p.s("group2")?;
    let p1 = p.i("point1")?;
    let p2 = p.i("point2")?;

    if g.npoints == 0 {
        out.detail.insert("distance".into(), serde_json::json!(0.0));
        return Ok(out);
    }

    let dist = if !group1.trim().is_empty() && !group2.trim().is_empty() {
        let sel1 = super::group::mask(&g, "point", &group1).unwrap_or_else(|_| vec![false; g.npoints]);
        let sel2 = super::group::mask(&g, "point", &group2).unwrap_or_else(|_| vec![false; g.npoints]);
        let pts1: Vec<usize> = sel1.iter().enumerate().filter_map(|(i, &b)| if b { Some(i) } else { None }).collect();
        let pts2: Vec<usize> = sel2.iter().enumerate().filter_map(|(i, &b)| if b { Some(i) } else { None }).collect();
        if pts1.is_empty() || pts2.is_empty() {
            0.0
        } else {
            let p_arr = g.p();
            let mut min_dist_sq = f64::INFINITY;
            for &a in &pts1 {
                for &b in &pts2 {
                    let dx = p_arr[a * 3] as f64 - p_arr[b * 3] as f64;
                    let dy = p_arr[a * 3 + 1] as f64 - p_arr[b * 3 + 1] as f64;
                    let dz = p_arr[a * 3 + 2] as f64 - p_arr[b * 3 + 2] as f64;
                    let d2 = dx * dx + dy * dy + dz * dz;
                    if d2 < min_dist_sq {
                        min_dist_sq = d2;
                    }
                }
            }
            if min_dist_sq.is_finite() { min_dist_sq.sqrt() } else { 0.0 }
        }
    } else if !group1.trim().is_empty() {
        let sel = super::group::mask(&g, "point", &group1).unwrap_or_else(|_| vec![false; g.npoints]);
        let pts: Vec<usize> = sel.iter().enumerate().filter_map(|(i, &b)| if b { Some(i) } else { None }).collect();
        if pts.len() < 2 {
            0.0
        } else {
            let n = pts.len() as i64;
            let idx1 = if p1 < 0 { (n + p1).clamp(0, n - 1) } else { p1.clamp(0, n - 1) } as usize;
            let idx2 = if p2 < 0 { (n + p2).clamp(0, n - 1) } else { p2.clamp(0, n - 1) } as usize;
            let a = pts[idx1];
            let b = pts[idx2];
            let p_arr = g.p();
            let dx = p_arr[a * 3] as f64 - p_arr[b * 3] as f64;
            let dy = p_arr[a * 3 + 1] as f64 - p_arr[b * 3 + 1] as f64;
            let dz = p_arr[a * 3 + 2] as f64 - p_arr[b * 3 + 2] as f64;
            (dx * dx + dy * dy + dz * dz).sqrt()
        }
    } else if g.npoints < 2 {
        0.0
    } else {
        let n = g.npoints as i64;
        let idx1 = if p1 < 0 { (n + p1).clamp(0, n - 1) } else { p1.clamp(0, n - 1) } as usize;
        let idx2 = if p2 < 0 { (n + p2).clamp(0, n - 1) } else { p2.clamp(0, n - 1) } as usize;
        let p_arr = g.p();
        let dx = p_arr[idx1 * 3] as f64 - p_arr[idx2 * 3] as f64;
        let dy = p_arr[idx1 * 3 + 1] as f64 - p_arr[idx2 * 3 + 1] as f64;
        let dz = p_arr[idx1 * 3 + 2] as f64 - p_arr[idx2 * 3 + 2] as f64;
        (dx * dx + dy * dy + dz * dz).sqrt()
    };

    out.detail.insert("distance".into(), serde_json::json!(dist));
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
    fn test_measure_distance_unit_cube() {
        let cube = unit_cube();
        let mut map = serde_json::Map::new();
        map.insert("point1".into(), serde_json::json!(0));
        map.insert("point2".into(), serde_json::json!(1));
        let p = Params::resolve(MEASURE_DISTANCE.params, Some(&map)).unwrap();
        let res = (MEASURE_DISTANCE.cook)(&[vec![Arc::new(cube.clone())]], &p).unwrap();
        assert_eq!(res.npoints, cube.npoints);
        let dist = res.detail.get("distance").unwrap().as_f64().unwrap();
        assert!((dist - 1.0).abs() < 1e-5, "expected unit edge distance 1.0, got {dist}");
    }

    #[test]
    fn test_measure_distance_grid_2x2() {
        let grid = grid_2x2();
        let p = Params::resolve(MEASURE_DISTANCE.params, None).unwrap();
        let res = (MEASURE_DISTANCE.cook)(&[vec![Arc::new(grid)]], &p).unwrap();
        let dist = res.detail.get("distance").unwrap().as_f64().unwrap();
        assert!(dist > 0.0);
    }

    #[test]
    fn test_measure_distance_line_3() {
        let line = line_3();
        let p = Params::resolve(MEASURE_DISTANCE.params, None).unwrap();
        let res = (MEASURE_DISTANCE.cook)(&[vec![Arc::new(line)]], &p).unwrap();
        let dist = res.detail.get("distance").unwrap().as_f64().unwrap();
        assert!((dist - 3.0).abs() < 1e-5, "expected line distance 3.0, got {dist}");
    }

    #[test]
    fn test_measure_distance_empty_input_and_missing_group() {
        let empty = Geometry::default();
        let p = Params::resolve(MEASURE_DISTANCE.params, None).unwrap();
        let res = (MEASURE_DISTANCE.cook)(&[vec![Arc::new(empty)]], &p).unwrap();
        assert_eq!(res.detail.get("distance").unwrap().as_f64().unwrap(), 0.0);

        let line = line_3();
        let mut map = serde_json::Map::new();
        map.insert("group".into(), serde_json::json!("nonexistent_group"));
        let p_group = Params::resolve(MEASURE_DISTANCE.params, Some(&map)).unwrap();
        let res_group = (MEASURE_DISTANCE.cook)(&[vec![Arc::new(line)]], &p_group).unwrap();
        assert_eq!(res_group.detail.get("distance").unwrap().as_f64().unwrap(), 0.0);
    }

    #[test]
    fn test_measure_distance_determinism() {
        let line = line_3();
        let p = Params::resolve(MEASURE_DISTANCE.params, None).unwrap();
        let res1 = (MEASURE_DISTANCE.cook)(&[vec![Arc::new(line.clone())]], &p).unwrap();
        let res2 = (MEASURE_DISTANCE.cook)(&[vec![Arc::new(line)]], &p).unwrap();
        assert_eq!(res1.detail.get("distance"), res2.detail.get("distance"));
    }
}
