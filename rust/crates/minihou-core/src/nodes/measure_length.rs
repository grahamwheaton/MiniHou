//! Node `measure_length`: curve or polyline length per primitive (prim attribute length) and total (detail total_length).

use std::sync::Arc;

use super::{Inputs, NodeSpec, GEO};
use crate::error::Result;
use crate::geometry::{Attr, Geometry};
use crate::params::Params;

pub const MEASURE_LENGTH: NodeSpec = NodeSpec {
    name: "measure_length",
    inputs: GEO,
    params: crate::params!("group": ""),
    cook: measure_length,
};

fn measure_length(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = MEASURE_LENGTH.one(inputs, 0)?;
    let mut out = (*g).clone();
    let group = p.s("group")?;

    if g.nprims() == 0 {
        out.point.remove("length");
        out.vertex.remove("length");
        out.prim.insert("length".into(), Arc::new(Attr::f32(1, vec![])));
        out.detail.insert("total_length".into(), serde_json::json!(0.0));
        return Ok(out);
    }

    let sel = if group.trim().is_empty() {
        vec![true; g.nprims()]
    } else {
        super::group::mask(&g, "prim", &group).unwrap_or_else(|_| vec![false; g.nprims()])
    };

    let p_arr = g.p();
    let mut lengths = Vec::with_capacity(g.nprims());
    let mut total_length: f64 = 0.0;

    for i in 0..g.nprims() {
        if !sel[i] {
            lengths.push(0.0f32);
            continue;
        }
        let s = g.topology.prim_offsets[i] as usize;
        let e = g.topology.prim_offsets[i + 1] as usize;
        let ids = &g.topology.vertex_point[s..e];
        let mut len = 0.0f64;
        let n = if g.topology.is_closed(i) {
            ids.len()
        } else {
            ids.len().saturating_sub(1)
        };
        for j in 0..n {
            let a = ids[j] as usize * 3;
            let b = ids[(j + 1) % ids.len()] as usize * 3;
            let dx = p_arr[a] as f64 - p_arr[b] as f64;
            let dy = p_arr[a + 1] as f64 - p_arr[b + 1] as f64;
            let dz = p_arr[a + 2] as f64 - p_arr[b + 2] as f64;
            len += (dx * dx + dy * dy + dz * dz).sqrt();
        }
        lengths.push(len as f32);
        total_length += len;
    }

    out.point.remove("length");
    out.vertex.remove("length");
    out.prim.insert("length".into(), Arc::new(Attr::f32(1, lengths)));
    out.detail.insert("total_length".into(), serde_json::json!(total_length));
    Ok(out)
}

#[cfg(test)]
mod tests {
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
    fn test_measure_length_unit_cube() {
        let cube = unit_cube();
        let p = Params::resolve(MEASURE_LENGTH.params, None).unwrap();
        let res = (MEASURE_LENGTH.cook)(&[vec![Arc::new(cube.clone())]], &p).unwrap();
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
        let p = Params::resolve(MEASURE_LENGTH.params, None).unwrap();
        let res = (MEASURE_LENGTH.cook)(&[vec![Arc::new(grid)]], &p).unwrap();
        let total = res.detail.get("total_length").unwrap().as_f64().unwrap();
        assert!(total > 0.0);
    }

    #[test]
    fn test_measure_length_line_3() {
        let line = line_3();
        let p = Params::resolve(MEASURE_LENGTH.params, None).unwrap();
        let res = (MEASURE_LENGTH.cook)(&[vec![Arc::new(line)]], &p).unwrap();
        let lengths = res.prim.get("length").unwrap().as_f32().unwrap();
        assert_eq!(lengths.len(), 1);
        assert!((lengths[0] - 3.0).abs() < 1e-5, "expected primitive length 3.0, got {}", lengths[0]);
        let total = res.detail.get("total_length").unwrap().as_f64().unwrap();
        assert!((total - 3.0).abs() < 1e-5);
    }

    #[test]
    fn test_measure_length_empty_input_and_missing_group() {
        let empty = Geometry::default();
        let p = Params::resolve(MEASURE_LENGTH.params, None).unwrap();
        let res = (MEASURE_LENGTH.cook)(&[vec![Arc::new(empty)]], &p).unwrap();
        assert_eq!(res.prim.get("length").unwrap().rows(), 0);
        assert_eq!(res.detail.get("total_length").unwrap().as_f64().unwrap(), 0.0);

        let line = line_3();
        let mut map = serde_json::Map::new();
        map.insert("group".into(), serde_json::json!("nonexistent_group"));
        let p_group = Params::resolve(MEASURE_LENGTH.params, Some(&map)).unwrap();
        let res_group = (MEASURE_LENGTH.cook)(&[vec![Arc::new(line)]], &p_group).unwrap();
        let lengths = res_group.prim.get("length").unwrap().as_f32().unwrap();
        assert_eq!(lengths[0], 0.0);
        assert_eq!(res_group.detail.get("total_length").unwrap().as_f64().unwrap(), 0.0);
    }

    #[test]
    fn test_measure_length_determinism() {
        let line = line_3();
        let p = Params::resolve(MEASURE_LENGTH.params, None).unwrap();
        let res1 = (MEASURE_LENGTH.cook)(&[vec![Arc::new(line.clone())]], &p).unwrap();
        let res2 = (MEASURE_LENGTH.cook)(&[vec![Arc::new(line)]], &p).unwrap();
        assert_eq!(res1.prim.get("length"), res2.prim.get("length"));
        assert_eq!(res1.detail.get("total_length"), res2.detail.get("total_length"));
    }
}
