//! Node `measure_area`: face area (prim attribute area) and total (detail total_area).

use std::sync::Arc;

use super::{Inputs, NodeSpec, GEO};
use crate::error::Result;
use crate::geometry::{Attr, Geometry};
use crate::params::Params;

pub const MEASURE_AREA: NodeSpec = NodeSpec {
    name: "measure_area",
    inputs: GEO,
    params: crate::params!("group": ""),
    cook: measure_area,
};

fn measure_area(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = MEASURE_AREA.one(inputs, 0)?;
    let mut out = (*g).clone();
    let group = p.s("group")?;

    if g.nprims() == 0 {
        out.point.remove("area");
        out.vertex.remove("area");
        out.prim.insert("area".into(), Arc::new(Attr::f32(1, vec![])));
        out.detail.insert("total_area".into(), serde_json::json!(0.0));
        return Ok(out);
    }

    let sel = if group.trim().is_empty() {
        vec![true; g.nprims()]
    } else {
        super::group::mask(&g, "prim", &group).unwrap_or_else(|_| vec![false; g.nprims()])
    };

    let face = crate::normals::prim_normals(g.p(), &g.topology);
    let mut areas = Vec::with_capacity(g.nprims());
    let mut total_area: f64 = 0.0;

    for i in 0..g.nprims() {
        if !sel[i] || !g.topology.is_closed(i) {
            areas.push(0.0f32);
            continue;
        }
        let s = g.topology.prim_offsets[i] as usize;
        let e = g.topology.prim_offsets[i + 1] as usize;
        if e - s < 3 {
            areas.push(0.0f32);
            continue;
        }
        let n = face[i];
        let norm = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let a = norm / 2.0;
        areas.push(a as f32);
        total_area += a;
    }

    out.point.remove("area");
    out.vertex.remove("area");
    out.prim.insert("area".into(), Arc::new(Attr::f32(1, areas)));
    out.detail.insert("total_area".into(), serde_json::json!(total_area));
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
    fn test_measure_area_unit_cube() {
        let cube = unit_cube();
        let p = Params::resolve(MEASURE_AREA.params, None).unwrap();
        let res = (MEASURE_AREA.cook)(&[vec![Arc::new(cube.clone())]], &p).unwrap();
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
        let p = Params::resolve(MEASURE_AREA.params, None).unwrap();
        let res = (MEASURE_AREA.cook)(&[vec![Arc::new(grid)]], &p).unwrap();
        let total = res.detail.get("total_area").unwrap().as_f64().unwrap();
        assert!((total - 4.0).abs() < 1e-5, "expected grid area 4.0, got {total}");
    }

    #[test]
    fn test_measure_area_line_3() {
        let line = line_3();
        let p = Params::resolve(MEASURE_AREA.params, None).unwrap();
        let res = (MEASURE_AREA.cook)(&[vec![Arc::new(line)]], &p).unwrap();
        let areas = res.prim.get("area").unwrap().as_f32().unwrap();
        assert_eq!(areas[0], 0.0);
        let total = res.detail.get("total_area").unwrap().as_f64().unwrap();
        assert_eq!(total, 0.0);
    }

    #[test]
    fn test_measure_area_empty_input_and_missing_group() {
        let empty = Geometry::default();
        let p = Params::resolve(MEASURE_AREA.params, None).unwrap();
        let res = (MEASURE_AREA.cook)(&[vec![Arc::new(empty)]], &p).unwrap();
        assert_eq!(res.prim.get("area").unwrap().rows(), 0);
        assert_eq!(res.detail.get("total_area").unwrap().as_f64().unwrap(), 0.0);

        let cube = unit_cube();
        let mut map = serde_json::Map::new();
        map.insert("group".into(), serde_json::json!("nonexistent_group"));
        let p_group = Params::resolve(MEASURE_AREA.params, Some(&map)).unwrap();
        let res_group = (MEASURE_AREA.cook)(&[vec![Arc::new(cube)]], &p_group).unwrap();
        let areas = res_group.prim.get("area").unwrap().as_f32().unwrap();
        for &a in areas {
            assert_eq!(a, 0.0);
        }
        assert_eq!(res_group.detail.get("total_area").unwrap().as_f64().unwrap(), 0.0);
    }

    #[test]
    fn test_measure_area_determinism() {
        let cube = unit_cube();
        let p = Params::resolve(MEASURE_AREA.params, None).unwrap();
        let res1 = (MEASURE_AREA.cook)(&[vec![Arc::new(cube.clone())]], &p).unwrap();
        let res2 = (MEASURE_AREA.cook)(&[vec![Arc::new(cube)]], &p).unwrap();
        assert_eq!(res1.prim.get("area"), res2.prim.get("area"));
        assert_eq!(res1.detail.get("total_area"), res2.detail.get("total_area"));
    }
}
