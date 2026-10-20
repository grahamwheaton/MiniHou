//! Selection nodes: `grow` (Select More / Less), `select_path` (shortest path), `select_loop` and `select_ring`.
//! They pass the geometry through and only write groups. The walks are the viewport's (`crate::select_topo`), so a
//! node reproduces what Alt / Ctrl+Alt / Ctrl clicks select. Spec: docs/RUST_SELECT_NODES.md.
use std::sync::Arc;

use super::{edge_tools, groups, model, Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, Geometry};
use crate::params::Params;
use crate::select_topo::{Domain, PathMethod, SelectTopology};

const DOMAINS: &[&str] = &["points", "edges", "primitives"];

fn domain(p: &Params) -> Result<Domain> {
    Ok(match p.e("domain", DOMAINS)?.as_str() { "points" => Domain::Points, "edges" => Domain::Edges, _ => Domain::Faces })
}
fn noun(d: Domain) -> &'static str { match d { Domain::Points => "point", Domain::Edges => "edge", Domain::Faces => "primitive" } }

/// Members of `pattern` in `d` (an empty pattern selects everything).
fn members(g: &Geometry, d: Domain, pattern: &str) -> Result<Vec<bool>> {
    match d {
        Domain::Points => groups::point_mask(g, pattern),
        Domain::Edges => edge_tools::edge_mask(g, pattern),
        Domain::Faces => if pattern.trim().is_empty() { Ok(vec![true; g.nprims()]) } else { super::group::mask(g, "prim", pattern) },
    }
}
/// The existing group `name` in `d`, if any.
fn existing(g: &Geometry, d: Domain, name: &str) -> Result<Option<Vec<bool>>> {
    Ok(match d {
        Domain::Points => g.point.get(&format!("group_{name}")).map(|a| super::group::flags(a)),
        Domain::Faces => g.prim.get(&format!("group_{name}")).map(|a| super::group::flags(a)),
        Domain::Edges => {
            let key = format!("edge_group_{name}");
            if g.vertex.contains_key(&key) { Some(edge_tools::values(g, &key)?.into_iter().map(|x| x != 0.).collect()) } else { None }
        }
    })
}
/// Writes group `name` in `d`; a point or primitive group of the same name in the other domain is replaced.
fn write(g: &mut Geometry, d: Domain, name: &str, mask: &[bool]) -> Result<()> {
    model::group_name(name).map_err(|_| Error::new(format!("invalid group name {name:?}")))?;
    match d {
        Domain::Edges => edge_tools::write_values(g, &format!("edge_group_{name}"), &mask.iter().map(|&b| b as u8 as f32).collect::<Vec<_>>(), true),
        _ => {
            let key = format!("group_{name}");
            let a = Arc::new(Attr::i32(1, mask.iter().map(|&b| b as i32).collect()));
            if d == Domain::Points { g.prim.remove(&key); g.point.insert(key, a); } else { g.point.remove(&key); g.prim.insert(key, a); }
            Ok(())
        }
    }
}
/// Writes `mask` as `name`, replacing the group or adding to it (`combine` = "union").
fn write_combined(g: &mut Geometry, d: Domain, name: &str, mut mask: Vec<bool>, p: &Params) -> Result<()> {
    if p.e("combine", &["replace", "union"])? == "union" {
        if let Some(old) = existing(g, d, name)? { for (m, o) in mask.iter_mut().zip(old) { *m |= o; } }
    }
    write(g, d, name, &mask)
}
fn to_mask(n: usize, ids: impl IntoIterator<Item = u32>) -> Vec<bool> {
    let mut m = vec![false; n];
    for i in ids { if let Some(x) = m.get_mut(i as usize) { *x = true; } }
    m
}
/// A plain group name (not a pattern, range or id list).
fn plain(name: &str) -> bool { model::group_name(name.trim()).is_ok() }

pub const GROW: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None, name: "grow", inputs: GEO, params: crate::params!(
    "group": ""; "domain": "primitives"; "mode": "grow"; "steps": 1; "face_step": true;
    "keep_selection": true; "result_group_name": ""; "output_group": false; "output_group_name": "grown"), cook: grow };
fn grow(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = GROW.one(inputs, 0)?;
    let d = domain(p)?;
    let pattern = p.s("group")?;
    let shrink = p.e("mode", &["grow", "shrink"])? == "shrink";
    let steps = p.i("steps")?;
    if !(0..=100_000).contains(&steps) { return Err(Error::new("grow: steps must be 0..100000")); }
    let before = members(&g, d, &pattern)?;
    let mut after = before.clone();
    if steps > 0 && after.iter().any(|x| *x) {
        SelectTopology::from_geometry(&g).grow(d, &mut after, steps as usize, p.b("face_step")?, shrink);
    }
    let mut out = (*g).clone();
    if p.b("keep_selection")? {
        let name = p.s("result_group_name")?;
        let name = if !name.trim().is_empty() { name.trim().to_owned() } else if plain(&pattern) { pattern.trim().to_owned() } else { "selection".to_owned() };
        write(&mut out, d, &name, &after)?;
    }
    if p.b("output_group")? {
        let delta: Vec<bool> = before.iter().zip(&after).map(|(&b, &a)| if shrink { b && !a } else { a && !b }).collect();
        write(&mut out, d, p.s("output_group_name")?.trim(), &delta)?;
    }
    Ok(out)
}

pub const SELECT_PATH: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None, name: "select_path", inputs: GEO, params: crate::params!(
    "domain": "points"; "start": 0; "end": 1; "waypoints": ""; "method": "shortest_distance"; "closed": false;
    "group_name": "path"; "combine": "replace"), cook: select_path };
/// The ordered elements of `text`: ids (and `p<a>-<b>` point pairs for edges) keep their order; group names and other
/// patterns add their members in element order.
fn ordered(g: &Geometry, t: &SelectTopology, d: Domain, text: &str) -> Result<Vec<u32>> {
    let mut out = Vec::new();
    for token in text.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()) {
        if let Ok(i) = token.parse::<u32>() { out.push(i); continue; }
        if d == Domain::Edges {
            if let Some((a, b)) = token.strip_prefix('p').and_then(|s| s.split_once('-')).and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?))) {
                out.push(t.edge_of(a, b).ok_or_else(|| Error::new(format!("select_path: edge {token:?} does not exist")))?);
                continue;
            }
        }
        out.extend(members(g, d, token)?.iter().enumerate().filter(|x| *x.1).map(|x| x.0 as u32));
    }
    Ok(out)
}
fn select_path(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = SELECT_PATH.one(inputs, 0)?;
    let d = domain(p)?;
    let method = match p.e("method", &["fewest_steps", "shortest_distance"])?.as_str() { "fewest_steps" => PathMethod::FewestSteps, _ => PathMethod::ShortestDistance };
    let t = SelectTopology::from_geometry(&g);
    let text = p.s("waypoints")?;
    let mut stops = if text.trim().is_empty() {
        let (a, b) = (p.i("start")?, p.i("end")?);
        if a < 0 || b < 0 { return Err(Error::new("select_path: start and end must be element numbers (>= 0)")); }
        vec![a as u32, b as u32]
    } else { ordered(&g, &t, d, &text)? };
    let n = t.count(d);
    if let Some(&bad) = stops.iter().find(|&&i| i as usize >= n) {
        return Err(Error::new(format!("select_path: there is no {} {bad} (the input has {n})", noun(d))));
    }
    if p.b("closed")? && stops.len() > 1 { stops.push(stops[0]); }
    let mut on = vec![false; n];
    if let [only] = stops.as_slice() { on[*only as usize] = true; }
    for pair in stops.windows(2) {
        let route = t.path(g.p(), d, pair[0], pair[1], method);
        if route.is_empty() {
            return Err(Error::new(format!("select_path: {} {} and {} are not connected", noun(d), pair[0], pair[1])));
        }
        for x in route { on[x as usize] = true; }
    }
    let mut out = (*g).clone();
    write_combined(&mut out, d, p.s("group_name")?.trim(), on, p)?;
    Ok(out)
}

pub const SELECT_LOOP: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None, name: "select_loop", inputs: GEO, params: crate::params!(
    "seeds": ""; "domain": "edges"; "mode": "edge_loop"; "direction": "both"; "group_name": "loop"; "combine": "replace"), cook: select_loop };
pub const SELECT_RING: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None, name: "select_ring", inputs: GEO, params: crate::params!(
    "seeds": ""; "domain": "edges"; "direction": "both"; "group_name": "ring"; "combine": "replace"), cook: select_ring };
fn select_loop(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let boundary = p.e("mode", &["edge_loop", "boundary_loop"])? == "boundary_loop";
    walk_node(&SELECT_LOOP, inputs, p, if boundary { Walk::Boundary } else { Walk::Loop })
}
fn select_ring(inputs: &Inputs, p: &Params) -> Result<Geometry> { walk_node(&SELECT_RING, inputs, p, Walk::Ring) }

#[derive(Clone, Copy, PartialEq)]
enum Walk { Loop, Ring, Boundary }
fn walk_node(spec: &NodeSpec, inputs: &Inputs, p: &Params, kind: Walk) -> Result<Geometry> {
    let g = spec.one(inputs, 0)?;
    let d = domain(p)?;
    let side = match p.e("direction", &["both", "forward", "backward"])?.as_str() { "forward" => Some(1), "backward" => Some(0), _ => None };
    let seeds_text = p.s("seeds")?;
    let t = SelectTopology::from_geometry(&g);
    // no seeds: an empty group rather than every edge's loop
    let seeds: Vec<u32> = if seeds_text.trim().is_empty() { Vec::new() } else {
        edge_tools::edge_mask(&g, &seeds_text)?.iter().enumerate().filter(|x| *x.1).map(|x| x.0 as u32).collect()
    };
    let mut edges = Vec::new();
    for &s in &seeds {
        edges.extend(match kind {
            Walk::Boundary => t.boundary_walk(s, side),
            // a face loop is the polygons of the ring, as Alt+click in face mode
            Walk::Loop if d != Domain::Faces => t.walk_side(g.p(), s, false, side),
            _ => t.walk_side(g.p(), s, true, side),
        });
    }
    let mask = match d {
        Domain::Edges => to_mask(t.count(d), edges),
        Domain::Points => to_mask(t.count(d), t.points_of_edges(&edges)),
        Domain::Faces => to_mask(t.count(d), t.faces_of_edges(&edges)),
    };
    let mut out = (*g).clone();
    write_combined(&mut out, d, p.s("group_name")?.trim(), mask, p)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `nx` by `ny` quads in z = 0; point (i, j) is j * (nx + 1) + i, primitive (i, j) is j * nx + i.
    fn grid(nx: i32, ny: i32) -> Geometry {
        let w = nx + 1;
        let p = (0..=ny).flat_map(|j| (0..=nx).flat_map(move |i| [i as f32, j as f32, 0.])).collect();
        let vp: Vec<i32> = (0..ny).flat_map(|j| (0..nx).flat_map(move |i| [j * w + i, j * w + i + 1, (j + 1) * w + i + 1, (j + 1) * w + i])).collect();
        let offs = (0..=nx * ny).map(|k| k * 4).collect();
        Geometry::from_arrays(p, vp, offs, None).unwrap()
    }
    fn cube() -> Geometry {
        let p = (0..8).flat_map(|c| [(c & 1) as f32, ((c >> 1) & 1) as f32, ((c >> 2) & 1) as f32]).collect();
        let vp = vec![0, 2, 3, 1, 4, 5, 7, 6, 0, 1, 5, 4, 2, 6, 7, 3, 0, 4, 6, 2, 1, 3, 7, 5];
        Geometry::from_arrays(p, vp, (0..=6).map(|k| k * 4).collect(), None).unwrap()
    }
    fn with_prims(mut g: Geometry, name: &str, on: &[usize]) -> Geometry {
        let mut m = vec![0; g.nprims()];
        for &i in on { m[i] = 1; }
        g.prim.insert(format!("group_{name}"), Arc::new(Attr::i32(1, m)));
        g
    }
    fn run(spec: &NodeSpec, g: Geometry, params: serde_json::Value) -> Result<Geometry> {
        let p = Params::resolve(spec.params, params.as_object())?;
        (spec.cook)(&[vec![Arc::new(g)]], &p)
    }
    fn prims(g: &Geometry, name: &str) -> Vec<usize> {
        super::super::group::flags(&g.prim[&format!("group_{name}")]).iter().enumerate().filter(|x| *x.1).map(|x| x.0).collect()
    }
    fn points(g: &Geometry, name: &str) -> Vec<usize> {
        super::super::group::flags(&g.point[&format!("group_{name}")]).iter().enumerate().filter(|x| *x.1).map(|x| x.0).collect()
    }
    /// Edge group members as sorted point pairs.
    fn edges(g: &Geometry, name: &str) -> Vec<(i64, i64)> {
        let e = crate::topo::edges(&g.topology, g.npoints);
        edge_tools::values(g, &format!("edge_group_{name}")).unwrap().iter().enumerate().filter(|x| *x.1 != 0.)
            .map(|(i, _)| (e.points[2 * i], e.points[2 * i + 1])).collect()
    }

    #[test]
    fn grow_quad_grid_by_vertex_and_edge_with_delta_group() {
        let g = with_prims(grid(5, 5), "sel", &[12]);
        let out = run(&GROW, g.clone(), json!({"group": "sel", "output_group": true})).unwrap();
        assert_eq!(prims(&out, "sel"), vec![6, 7, 8, 11, 12, 13, 16, 17, 18]);
        assert_eq!(prims(&out, "grown"), vec![6, 7, 8, 11, 13, 16, 17, 18]);
        let out = run(&GROW, g.clone(), json!({"group": "sel", "face_step": false, "result_group_name": "more"})).unwrap();
        assert_eq!(prims(&out, "more"), vec![7, 11, 12, 13, 17]);
        assert_eq!(prims(&out, "sel"), vec![12], "the input group is left alone when the result has its own name");
        // the geometry passes through untouched
        assert_eq!(out.topology.vertex_point, g.topology.vertex_point);
        assert_eq!(out.p(), g.p());
        // two steps by edges: a diamond of 13
        let out = run(&GROW, g, json!({"group": "sel", "face_step": false, "steps": 2})).unwrap();
        assert_eq!(prims(&out, "sel").len(), 13);
    }

    #[test]
    fn shrink_is_select_less_with_removed_delta() {
        let g = with_prims(grid(5, 5), "sel", &[6, 7, 8, 11, 12, 13, 16, 17, 18]);
        let out = run(&GROW, g, json!({"group": "sel", "mode": "shrink", "output_group": true, "output_group_name": "removed", "result_group_name": "core"})).unwrap();
        assert_eq!(prims(&out, "core"), vec![12]);
        assert_eq!(prims(&out, "removed"), vec![6, 7, 8, 11, 13, 16, 17, 18]);
    }

    #[test]
    fn grow_cube_open_boundary_and_full_selection() {
        // a cube face grown by edges reaches the four sides, then everything
        let out = run(&GROW, with_prims(cube(), "s", &[0]), json!({"group": "s", "face_step": false})).unwrap();
        assert_eq!(prims(&out, "s"), vec![0, 2, 3, 4, 5]);
        let out = run(&GROW, with_prims(cube(), "s", &[0]), json!({"group": "s", "steps": 50})).unwrap();
        assert_eq!(prims(&out, "s").len(), 6);
        // a closed cube with every face selected stays full under shrink; one side missing erodes its neighbours
        let out = run(&GROW, with_prims(cube(), "s", &[0, 1, 2, 3, 4, 5]), json!({"group": "s", "mode": "shrink"})).unwrap();
        assert_eq!(prims(&out, "s").len(), 6);
        let out = run(&GROW, with_prims(cube(), "s", &[1, 2, 3, 4, 5]), json!({"group": "s", "mode": "shrink", "face_step": false})).unwrap();
        assert_eq!(prims(&out, "s"), vec![1]);
        // an open grid fully selected keeps its border
        let out = run(&GROW, grid(3, 3), json!({"mode": "shrink"})).unwrap();
        assert_eq!(prims(&out, "selection").len(), 9);
    }

    #[test]
    fn grow_empty_zero_steps_isolated_and_nonmanifold() {
        // empty selection: nothing grows, the groups are written empty
        let out = run(&GROW, with_prims(grid(2, 2), "s", &[]), json!({"group": "s", "output_group": true, "steps": 3})).unwrap();
        assert!(prims(&out, "s").is_empty() && prims(&out, "grown").is_empty());
        // zero steps copy the selection
        let out = run(&GROW, with_prims(grid(2, 2), "s", &[0]), json!({"group": "s", "steps": 0, "output_group": true})).unwrap();
        assert_eq!(prims(&out, "s"), vec![0]);
        assert!(prims(&out, "grown").is_empty());
        assert!(run(&GROW, grid(1, 1), json!({"steps": -1})).is_err());
        // an isolated point (no edges) neither grows nor erodes
        let mut g = Geometry::from_arrays(vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 5., 5., 5.], vec![0, 1, 2], vec![0, 3], None).unwrap();
        g.point.insert("group_p".into(), Arc::new(Attr::i32(1, vec![0, 0, 0, 1])));
        let out = run(&GROW, g.clone(), json!({"group": "p", "domain": "points"})).unwrap();
        assert_eq!(points(&out, "p"), vec![3]);
        let out = run(&GROW, g, json!({"group": "p", "domain": "points", "mode": "shrink"})).unwrap();
        assert_eq!(points(&out, "p"), vec![3]);
        // three triangles on one edge (non-manifold fin): growing across it by edges reaches both others
        let g = Geometry::from_arrays(vec![0., 0., 0., 1., 0., 0., 0.5, 1., 0., 0.5, -1., 0., 0.5, 0., 1.], vec![0, 1, 2, 1, 0, 3, 0, 1, 4], vec![0, 3, 6, 9], None).unwrap();
        let out = run(&GROW, with_prims(g, "s", &[0]), json!({"group": "s", "face_step": false})).unwrap();
        assert_eq!(prims(&out, "s"), vec![0, 1, 2]);
    }

    #[test]
    fn grow_points_and_edges_domains() {
        let mut g = grid(2, 2);
        g.point.insert("group_c".into(), Arc::new(Attr::i32(1, vec![0, 0, 0, 0, 1, 0, 0, 0, 0])));
        let out = run(&GROW, g.clone(), json!({"group": "c", "domain": "points", "face_step": false})).unwrap();
        assert_eq!(points(&out, "c"), vec![1, 3, 4, 5, 7]);
        let out = run(&GROW, g.clone(), json!({"group": "c", "domain": "points"})).unwrap();
        assert_eq!(points(&out, "c").len(), 9);
        let out = run(&GROW, g, json!({"group": "p0-1", "domain": "edges", "face_step": false, "result_group_name": "e", "output_group": true})).unwrap();
        assert_eq!(edges(&out, "e"), vec![(0, 1), (0, 3), (1, 2), (1, 4)]);
        assert_eq!(edges(&out, "grown"), vec![(0, 3), (1, 2), (1, 4)]);
    }

    #[test]
    fn path_methods_differ_on_a_weighted_shortcut() {
        // 0-1-2-3-4 along y = 0 (length 4, four steps) or a two-edge detour 0-5-4 through point 5 (length 7.2, two steps)
        let p = vec![0., 0., 0., 1., 0., 0., 2., 0., 0., 3., 0., 0., 4., 0., 0., 2., 3., 0.];
        let g = Geometry::from_arrays(p, vec![0, 1, 1, 2, 2, 3, 3, 4, 0, 5, 5, 4], vec![0, 2, 4, 6, 8, 10, 12], Some(vec![false; 6])).unwrap();
        let out = run(&SELECT_PATH, g.clone(), json!({"start": 0, "end": 4})).unwrap();
        assert_eq!(points(&out, "path"), vec![0, 1, 2, 3, 4]);
        let out = run(&SELECT_PATH, g, json!({"start": 0, "end": 4, "method": "fewest_steps"})).unwrap();
        assert_eq!(points(&out, "path"), vec![0, 4, 5]);
        // on a mesh: a finely divided bottom border against a coarse three-edge way round the top
        let mut p: Vec<f32> = (0..5).flat_map(|i| [i as f32, 0., 0.]).collect();
        p.extend([0., 3., 0., 4., 3., 0.]);
        let g = Geometry::from_arrays(p, vec![0, 1, 2, 3, 4, 6, 5], vec![0, 7], None).unwrap();
        let out = run(&SELECT_PATH, g.clone(), json!({"start": 0, "end": 4})).unwrap();
        assert_eq!(points(&out, "path"), vec![0, 1, 2, 3, 4]);
        let out = run(&SELECT_PATH, g, json!({"start": 0, "end": 4, "method": "fewest_steps"})).unwrap();
        assert_eq!(points(&out, "path"), vec![0, 4, 5, 6]);
    }

    #[test]
    fn path_waypoints_closed_union_and_errors() {
        let g = grid(4, 4); // w = 5
        let out = run(&SELECT_PATH, g.clone(), json!({"waypoints": "0 4 24"})).unwrap();
        assert_eq!(points(&out, "path"), vec![0, 1, 2, 3, 4, 9, 14, 19, 24]);
        // closed: the last waypoint routes back to the first, here round the whole border
        let out = run(&SELECT_PATH, g.clone(), json!({"waypoints": "0 4 24 20", "closed": true})).unwrap();
        assert_eq!(points(&out, "path"), (0..25).filter(|i| i % 5 == 0 || i % 5 == 4 || *i < 5 || *i >= 20).collect::<Vec<_>>());
        let out = run(&SELECT_PATH, g.clone(), json!({"waypoints": "0 4 24 20"})).unwrap();
        assert_eq!(points(&out, "path").len(), 13, "open: the left side is not walked");
        // faces: a straight row of primitives; edges by point pairs
        let out = run(&SELECT_PATH, g.clone(), json!({"domain": "primitives", "start": 0, "end": 3})).unwrap();
        assert_eq!(prims(&out, "path"), vec![0, 1, 2, 3]);
        let out = run(&SELECT_PATH, g.clone(), json!({"domain": "edges", "waypoints": "p0-1 p3-4"})).unwrap();
        assert_eq!(edges(&out, "path"), vec![(0, 1), (1, 2), (2, 3), (3, 4)]);
        // union with an existing group
        let mut h = g.clone();
        h.point.insert("group_path".into(), Arc::new(Attr::i32(1, (0..25).map(|i| (i == 24) as i32).collect())));
        let out = run(&SELECT_PATH, h, json!({"start": 0, "end": 1, "combine": "union"})).unwrap();
        assert_eq!(points(&out, "path"), vec![0, 1, 24]);
        // out of range and disconnected endpoints are clear errors
        let e = run(&SELECT_PATH, g, json!({"start": 0, "end": 99})).unwrap_err().to_string();
        assert!(e.contains("no point 99"), "{e}");
        let two = Geometry::from_arrays(vec![0., 0., 0., 1., 0., 0., 5., 0., 0., 6., 0., 0.], vec![0, 1, 2, 3], vec![0, 2, 4], Some(vec![false, false])).unwrap();
        let e = run(&SELECT_PATH, two, json!({"start": 0, "end": 3})).unwrap_err().to_string();
        assert!(e.contains("not connected"), "{e}");
    }

    #[test]
    fn loops_rings_and_boundaries() {
        let g = grid(4, 3); // w = 5
        // interior horizontal edge (6, 7): the whole row j = 1, border to border
        let out = run(&SELECT_LOOP, g.clone(), json!({"seeds": "p6-7"})).unwrap();
        assert_eq!(edges(&out, "loop"), (0..4).map(|i| (5 + i, 6 + i)).collect::<Vec<_>>());
        let out = run(&SELECT_LOOP, g.clone(), json!({"seeds": "p6-7", "direction": "forward"})).unwrap();
        assert_eq!(edges(&out, "loop"), vec![(6, 7), (7, 8), (8, 9)]);
        let out = run(&SELECT_LOOP, g.clone(), json!({"seeds": "p6-7", "domain": "points"})).unwrap();
        assert_eq!(points(&out, "loop"), vec![5, 6, 7, 8, 9]);
        // ring: every horizontal edge of column 1; face loop: the column's quads
        let out = run(&SELECT_RING, g.clone(), json!({"seeds": "p6-7"})).unwrap();
        assert_eq!(edges(&out, "ring"), (0..4).map(|j| (5 * j + 1, 5 * j + 2)).collect::<Vec<_>>());
        let out = run(&SELECT_RING, g.clone(), json!({"seeds": "p6-7", "domain": "primitives"})).unwrap();
        assert_eq!(prims(&out, "ring"), vec![1, 5, 9]);
        // boundary loop runs around the whole open border, an interior seed gives nothing
        let out = run(&SELECT_LOOP, g.clone(), json!({"seeds": "p0-1", "mode": "boundary_loop"})).unwrap();
        assert_eq!(edges(&out, "loop").len(), 2 * 4 + 2 * 3);
        let out = run(&SELECT_LOOP, g.clone(), json!({"seeds": "p6-7", "mode": "boundary_loop"})).unwrap();
        assert!(edges(&out, "loop").is_empty());
        // no seeds: empty group
        let out = run(&SELECT_RING, g, json!({})).unwrap();
        assert!(edges(&out, "ring").is_empty());
    }

    #[test]
    fn cube_loop_stops_at_poles_ring_closes_and_ngons_stop_rings() {
        let out = run(&SELECT_LOOP, cube(), json!({"seeds": "p0-1"})).unwrap();
        assert_eq!(edges(&out, "loop"), vec![(0, 1)], "valence-3 corners are poles");
        let out = run(&SELECT_RING, cube(), json!({"seeds": "p0-1"})).unwrap();
        assert_eq!(edges(&out, "ring"), vec![(0, 1), (2, 3), (4, 5), (6, 7)]);
        // a closed loop around a tube of quads
        let n = 6;
        let p: Vec<f32> = (0..3).flat_map(|j| (0..n).flat_map(move |i| { let a = i as f32 * std::f32::consts::TAU / n as f32; [a.cos(), a.sin(), j as f32] })).collect();
        let vp: Vec<i32> = (0..2).flat_map(|j| (0..n).flat_map(move |i| [j * n + i, j * n + (i + 1) % n, (j + 1) * n + (i + 1) % n, (j + 1) * n + i])).collect();
        let tube = Geometry::from_arrays(p, vp, (0..=2 * n).map(|k| k * 4).collect(), None).unwrap();
        let out = run(&SELECT_LOOP, tube, json!({"seeds": format!("p{}-{}", n, n + 1)})).unwrap();
        assert_eq!(edges(&out, "loop").len(), n as usize);
        // a strip ending in a pentagon: the ring stops at its edge
        let p: Vec<f32> = (0..2).flat_map(|j| (0..4).flat_map(move |i| [i as f32, j as f32, 0.])).chain([3.5, 0.5, 0.]).collect();
        let strip = Geometry::from_arrays(p, vec![0, 1, 5, 4, 1, 2, 6, 5, 2, 3, 8, 7, 6], vec![0, 4, 8, 13], None).unwrap();
        let out = run(&SELECT_RING, strip, json!({"seeds": "p0-4", "combine": "union"})).unwrap();
        assert_eq!(edges(&out, "ring"), vec![(0, 4), (1, 5), (2, 6)]);
    }

    #[test]
    fn test_selection_nodes_empty_and_single_element() {
        let empty = Geometry::points_only(vec![]).unwrap();
        // grow on empty geometry does not panic across domains
        for domain in ["points", "edges", "primitives"] {
            let out = run(&GROW, empty.clone(), json!({"domain": domain})).unwrap();
            assert_eq!(out.npoints, 0);
            assert_eq!(out.nprims(), 0);
        }
        // select_path on empty geometry gives a clear out-of-range error
        let err = run(&SELECT_PATH, empty.clone(), json!({"start": 0, "end": 0})).unwrap_err().to_string();
        assert!(err.contains("there is no point 0"), "{err}");

        // select_loop and select_ring on empty geometry write empty groups safely
        let out_l = run(&SELECT_LOOP, empty.clone(), json!({})).unwrap();
        let out_r = run(&SELECT_RING, empty, json!({})).unwrap();
        assert_eq!(out_l.npoints, 0);
        assert_eq!(out_r.npoints, 0);

        // single point geometry
        let single_pt = Geometry::points_only(vec![1.0, 2.0, 3.0]).unwrap();
        let out_pt = run(&GROW, single_pt.clone(), json!({"domain": "points"})).unwrap();
        assert_eq!(points(&out_pt, "selection"), vec![0]);
        // single point path to self succeeds
        let out_path = run(&SELECT_PATH, single_pt.clone(), json!({"start": 0, "end": 0})).unwrap();
        assert_eq!(points(&out_path, "path"), vec![0]);

        // single triangle geometry
        let tri = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 0., 1., 0.],
            vec![0, 1, 2],
            vec![0, 3],
            None,
        ).unwrap();
        let out_tri = run(&GROW, tri.clone(), json!({"domain": "primitives"})).unwrap();
        assert_eq!(prims(&out_tri, "selection"), vec![0]);
        let out_tri_path = run(&SELECT_PATH, tri.clone(), json!({"start": 0, "end": 2})).unwrap();
        assert_eq!(points(&out_tri_path, "path"), vec![0, 2]);
    }

    #[test]
    fn test_selection_nodes_degenerate_and_nonmanifold() {
        // Collinear zero-area triangle and duplicate points
        let degen = Geometry::from_arrays(
            vec![0., 0., 0., 0., 0., 0., 0., 0., 0.],
            vec![0, 1, 2],
            vec![0, 3],
            None,
        ).unwrap();
        let out_grow = run(&GROW, degen.clone(), json!({"domain": "primitives"})).unwrap();
        assert_eq!(prims(&out_grow, "selection"), vec![0]);

        // Duplicate points forming two separate edges
        let dup = Geometry::from_arrays(
            vec![0., 0., 0., 0., 0., 0., 1., 1., 1., 1., 1., 1.],
            vec![0, 1, 2, 3],
            vec![0, 2, 4],
            Some(vec![false, false]),
        ).unwrap();
        let out_dup = run(&GROW, dup, json!({"domain": "points"})).unwrap();
        assert_eq!(points(&out_dup, "selection"), vec![0, 1, 2, 3]);
    }

    #[test]
    fn test_selection_nodes_nan_inf_and_huge_params() {
        let g = grid(2, 2);
        // huge steps > 100_000 is rejected
        assert!(run(&GROW, g.clone(), json!({"steps": 100_001})).is_err());
        // negative steps is rejected
        assert!(run(&GROW, g.clone(), json!({"steps": -10})).is_err());
        // negative start/end in select_path is rejected
        assert!(run(&SELECT_PATH, g.clone(), json!({"start": -1, "end": 2})).is_err());
        assert!(run(&SELECT_PATH, g.clone(), json!({"start": 0, "end": -2})).is_err());

        // Geometry with NaN / Inf in positions
        let nan_geo = Geometry::from_arrays(
            vec![f32::NAN, 0., 0., f32::INFINITY, 1., 0., 0., 2., 0.],
            vec![0, 1, 2],
            vec![0, 3],
            None,
        ).unwrap();
        // grow does not panic on NaN coords (it operates on graph topology)
        let out_nan = run(&GROW, nan_geo.clone(), json!({"domain": "points"})).unwrap();
        assert_eq!(points(&out_nan, "selection"), vec![0, 1, 2]);

        // select_path methods handle NaN gracefully (either returning route or clear disconnected error, no panic or hang)
        let _res1 = run(&SELECT_PATH, nan_geo.clone(), json!({"start": 0, "end": 2, "method": "fewest_steps"}));
        let _res2 = run(&SELECT_PATH, nan_geo.clone(), json!({"start": 0, "end": 2, "method": "shortest_distance"}));
        let _res3 = run(&SELECT_PATH, nan_geo, json!({"start": 0, "end": 2, "method": "most_direct"}));
    }

    #[test]
    fn test_selection_nodes_missing_groups_and_invalid_enums() {
        let g = grid(2, 2);
        // nonexistent group pattern in grow gives a clear error
        let err = run(&GROW, g.clone(), json!({"group": "group_does_not_exist"})).unwrap_err().to_string();
        assert!(err.contains("no group"), "{err}");

        // invalid enums give clean errors
        assert!(run(&GROW, g.clone(), json!({"domain": "vertices"})).is_err());
        assert!(run(&GROW, g.clone(), json!({"mode": "invert"})).is_err());
        assert!(run(&SELECT_PATH, g.clone(), json!({"domain": "faces"})).is_err());
        assert!(run(&SELECT_PATH, g.clone(), json!({"method": "dijkstra"})).is_err());
        assert!(run(&SELECT_PATH, g.clone(), json!({"combine": "intersect"})).is_err());
        assert!(run(&SELECT_LOOP, g.clone(), json!({"mode": "ring"})).is_err());
        assert!(run(&SELECT_RING, g.clone(), json!({"direction": "diagonal"})).is_err());
    }

    #[test]
    fn test_selection_nodes_determinism_and_attribute_preservation() {
        let mut g = grid(3, 3);
        g.point.insert("Cd".into(), Arc::new(Attr::f32(3, vec![1.0; g.npoints * 3])));
        g.point.insert("N".into(), Arc::new(Attr::f32(3, vec![0.0, 0.0, 1.0].repeat(g.npoints))));
        g.vertex.insert("uv".into(), Arc::new(Attr::f32(2, vec![0.5; g.nvertices() * 2])));
        g.prim.insert("material_index".into(), Arc::new(Attr::i32(1, vec![42; g.nprims()])));
        g.prim.insert("group_custom".into(), Arc::new(Attr::i32(1, vec![1; g.nprims()])));

        // Run grow twice and verify deterministic identical output and preserved attributes
        let out1 = run(&GROW, g.clone(), json!({"group": "0", "steps": 2})).unwrap();
        let out2 = run(&GROW, g.clone(), json!({"group": "0", "steps": 2})).unwrap();
        assert_eq!(out1.p(), out2.p());
        assert_eq!(out1.point["Cd"].data, g.point["Cd"].data);
        assert_eq!(out1.point["N"].data, g.point["N"].data);
        assert_eq!(out1.vertex["uv"].data, g.vertex["uv"].data);
        assert_eq!(out1.prim["material_index"].data, g.prim["material_index"].data);
        assert_eq!(out1.prim["group_custom"].data, g.prim["group_custom"].data);
        assert_eq!(prims(&out1, "selection"), prims(&out2, "selection"));

        // Run select_path determinism
        let p1 = run(&SELECT_PATH, g.clone(), json!({"start": 0, "end": 15})).unwrap();
        let p2 = run(&SELECT_PATH, g.clone(), json!({"start": 0, "end": 15})).unwrap();
        assert_eq!(points(&p1, "path"), points(&p2, "path"));
        assert_eq!(p1.point["Cd"].data, g.point["Cd"].data);

        // Run select_loop and select_ring determinism
        let l1 = run(&SELECT_LOOP, g.clone(), json!({"seeds": "p0-1"})).unwrap();
        let l2 = run(&SELECT_LOOP, g.clone(), json!({"seeds": "p0-1"})).unwrap();
        assert_eq!(edges(&l1, "loop"), edges(&l2, "loop"));
        assert_eq!(l1.vertex["uv"].data, g.vertex["uv"].data);

        let r1 = run(&SELECT_RING, g.clone(), json!({"seeds": "p0-1"})).unwrap();
        let r2 = run(&SELECT_RING, g, json!({"seeds": "p0-1"})).unwrap();
        assert_eq!(edges(&r1, "ring"), edges(&r2, "ring"));
    }

    #[test]
    fn test_selection_nodes_topology_euler_unchanged() {
        // Selection nodes only write groups; topology and Euler characteristic are invariant
        let c = cube();
        let euler_char = |geo: &Geometry| {
            let v = geo.npoints as i32;
            let e = crate::topo::edges(&geo.topology, geo.npoints).len() as i32;
            let f = geo.nprims() as i32;
            v - e + f
        };
        assert_eq!(euler_char(&c), 2); // closed genus 0 surface V - E + F = 8 - 12 + 6 = 2

        let out_grow = run(&GROW, c.clone(), json!({"group": "0", "steps": 3})).unwrap();
        assert_eq!(euler_char(&out_grow), 2);
        assert_eq!(out_grow.topology.vertex_point, c.topology.vertex_point);
        assert_eq!(out_grow.topology.prim_offsets, c.topology.prim_offsets);

        let out_path = run(&SELECT_PATH, c.clone(), json!({"start": 0, "end": 7})).unwrap();
        assert_eq!(euler_char(&out_path), 2);

        let out_loop = run(&SELECT_LOOP, c.clone(), json!({"seeds": "p0-1"})).unwrap();
        assert_eq!(euler_char(&out_loop), 2);

        let out_ring = run(&SELECT_RING, c, json!({"seeds": "p0-1"})).unwrap();
        assert_eq!(euler_char(&out_ring), 2);

        // Grid Euler characteristic V - E + F = 16 - 24 + 9 = 1
        let g = grid(3, 3);
        assert_eq!(euler_char(&g), 1);
        let out_g = run(&GROW, g.clone(), json!({"steps": 2})).unwrap();
        assert_eq!(euler_char(&out_g), 1);
    }
}
