//! Node `measure_volume`: closed-mesh volume (signed, via divergence theorem), detail attribute volume (float).
use super::{Inputs, NodeSpec, GEO};
use crate::error::Result;
use crate::geometry::Geometry;
use crate::params::Params;

pub const MEASURE_VOLUME: NodeSpec = NodeSpec {
    name: "measure_volume",
    inputs: GEO,
    params: crate::params!("group": ""),
    cook: measure_volume,
};

fn measure_volume(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = MEASURE_VOLUME.one(inputs, 0)?;
    let mut out = (*g).clone();
    let group = p.s("group")?;
    if g.npoints == 0 || g.nprims() == 0 {
        out.detail.insert("volume".into(), serde_json::json!(0.0));
        return Ok(out);
    }
    let sel = if group.trim().is_empty() {
        vec![true; g.nprims()]
    } else {
        super::group::mask(&g, "prim", &group).unwrap_or_else(|_| vec![false; g.nprims()])
    };
    let face = crate::normals::prim_normals(g.p(), &g.topology);
    let centres = super::group::centroids64(&g);
    let mut volume = 0.0f64;
    for i in 0..g.nprims() {
        if !sel[i] || !g.topology.is_closed(i) {
            continue;
        }
        let s = g.topology.prim_offsets[i] as usize;
        let e = g.topology.prim_offsets[i + 1] as usize;
        if e - s < 3 {
            continue;
        }
        let c = [centres[i * 3], centres[i * 3 + 1], centres[i * 3 + 2]];
        let n = face[i];
        volume += (n[0] * c[0] + n[1] * c[1] + n[2] * c[2]) / 6.0;
    }
    out.detail.insert("volume".into(), serde_json::json!(volume));
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
    fn test_measure_volume_unit_cube() {
        let cube = unit_cube();
        let p = Params::resolve(MEASURE_VOLUME.params, None).unwrap();
        let res = (MEASURE_VOLUME.cook)(&[vec![Arc::new(cube.clone())]], &p).unwrap();
        assert_eq!(res.npoints, cube.npoints);
        assert_eq!(res.nprims(), cube.nprims());
        let vol = res.detail.get("volume").unwrap().as_f64().unwrap();
        assert!((vol - 1.0).abs() < 1e-5, "expected volume 1.0, got {vol}");
    }

    #[test]
    fn test_measure_volume_grid_2x2() {
        let grid = grid_2x2();
        let p = Params::resolve(MEASURE_VOLUME.params, None).unwrap();
        let res = (MEASURE_VOLUME.cook)(&[vec![Arc::new(grid)]], &p).unwrap();
        let vol = res.detail.get("volume").unwrap().as_f64().unwrap();
        assert!((vol - 0.0).abs() < 1e-5, "expected volume 0.0 for open grid, got {vol}");
    }

    #[test]
    fn test_measure_volume_line_3() {
        let line = line_3();
        let p = Params::resolve(MEASURE_VOLUME.params, None).unwrap();
        let res = (MEASURE_VOLUME.cook)(&[vec![Arc::new(line)]], &p).unwrap();
        let vol = res.detail.get("volume").unwrap().as_f64().unwrap();
        assert_eq!(vol, 0.0);
    }

    #[test]
    fn test_measure_volume_empty_input_and_missing_group() {
        let empty = Geometry::default();
        let p = Params::resolve(MEASURE_VOLUME.params, None).unwrap();
        let res = (MEASURE_VOLUME.cook)(&[vec![Arc::new(empty)]], &p).unwrap();
        assert_eq!(res.detail.get("volume").unwrap().as_f64().unwrap(), 0.0);

        let cube = unit_cube();
        let mut map = serde_json::Map::new();
        map.insert("group".into(), serde_json::json!("nonexistent_group"));
        let p_group = Params::resolve(MEASURE_VOLUME.params, Some(&map)).unwrap();
        let res_group = (MEASURE_VOLUME.cook)(&[vec![Arc::new(cube)]], &p_group).unwrap();
        assert_eq!(res_group.detail.get("volume").unwrap().as_f64().unwrap(), 0.0);
    }

    #[test]
    fn test_measure_volume_determinism() {
        let cube = unit_cube();
        let p = Params::resolve(MEASURE_VOLUME.params, None).unwrap();
        let res1 = (MEASURE_VOLUME.cook)(&[vec![Arc::new(cube.clone())]], &p).unwrap();
        let res2 = (MEASURE_VOLUME.cook)(&[vec![Arc::new(cube)]], &p).unwrap();
        assert_eq!(res1.detail.get("volume"), res2.detail.get("volume"));
    }
}
