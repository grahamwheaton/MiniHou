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
    map.insert("columns".into(), serde_json::json!(3));
    map.insert("rows".into(), serde_json::json!(3));
    let p = Params::resolve(crate::nodes::generate::GRID.params, Some(&map)).unwrap();
    (crate::nodes::generate::GRID.cook)(&[vec![]], &p).unwrap()
}

#[test]
fn test_cube_unchanged_no_coplanar_faces() {
    let cube = Arc::new(unit_cube());
    assert_eq!(cube.nprims(), 6);
    let params = Params::resolve(NODE.params, None).unwrap();
    let out = (NODE.cook)(&[vec![cube]], &params).unwrap();
    assert_eq!(out.nprims(), 6, "cube with only 90-degree faces must remain unchanged");
    assert_eq!(out.npoints, 8);
}

#[test]
fn test_flat_grid_interior_edges_dissolved_gives_one_face() {
    let grid = Arc::new(grid_2x2());
    assert_eq!(grid.nprims(), 4, "2x2 grid starts with 4 faces");
    let params = Params::resolve(NODE.params, None).unwrap();
    let out = (NODE.cook)(&[vec![grid]], &params).unwrap();
    assert_eq!(out.nprims(), 1, "flat grid with interior edges dissolved must give one face");
    assert_eq!(out.topology.vertex_point.len(), 8, "perimeter of 2x2 grid has 8 vertices");
}

#[test]
fn test_cube_with_split_edge_merges_back() {
    let cube = unit_cube();
    assert_eq!(cube.nprims(), 6);

    // Split the first quad [v0, v1, v2, v3] into two coplanar triangles [v0, v1, v2] and [v0, v2, v3]
    let vp = &cube.topology.vertex_point;
    let offs = &cube.topology.prim_offsets;

    let v0 = vp[offs[0] as usize];
    let v1 = vp[offs[0] as usize + 1];
    let v2 = vp[offs[0] as usize + 2];
    let v3 = vp[offs[0] as usize + 3];

    let mut new_vp = Vec::new();
    let mut new_offs = vec![0i32];

    // Triangle 1
    new_vp.push(v0);
    new_vp.push(v1);
    new_vp.push(v2);
    new_offs.push(new_vp.len() as i32);

    // Remaining 5 quads of the cube
    for prim in 1..cube.nprims() {
        let s = offs[prim] as usize;
        let e = offs[prim + 1] as usize;
        new_vp.extend_from_slice(&vp[s..e]);
        new_offs.push(new_vp.len() as i32);
    }

    // Triangle 2
    new_vp.push(v0);
    new_vp.push(v2);
    new_vp.push(v3);
    new_offs.push(new_vp.len() as i32);

    let split_cube = Geometry::from_arrays(cube.p().to_vec(), new_vp, new_offs, None).unwrap();
    assert_eq!(split_cube.nprims(), 7, "cube with split quad has 7 faces");

    let params = Params::resolve(NODE.params, None).unwrap();
    let out = (NODE.cook)(&[vec![Arc::new(split_cube)]], &params).unwrap();
    assert_eq!(out.nprims(), 6, "two collinear faces on cube face must merge back into one face");
}
