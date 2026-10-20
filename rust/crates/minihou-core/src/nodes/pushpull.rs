//! Bounded solid Push/Pull and named inset/outset, from pushpull.py.
use super::{GEO, Inputs, NodeSpec, model};
use super::{boolean, model::*, modeling_tools::twins};
use crate::{
    error::{Error, Result},
    geometry::{Attr, Geometry},
    params::Params,
};
use std::{collections::{BTreeMap, BTreeSet, HashMap}, sync::Arc};
pub const PUSHPULL: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "pushpull",
    inputs: GEO,
    params: crate::params!("group":"";"distance":0.25;"through":"auto"),
    cook: pushpull,
};
mod regions;
mod cut;
mod through;
pub(super) use through::apply_through;
use cut::{TAG, face_samples, exit_depth, coplanar_target, cut, drop_collinear, merge_coplanar};
pub use cut::ray_hit;
pub(super) use cut::auto_merge;
use regions::{extent_of, regions, solid, Region};
fn pushpull(i: &Inputs, p: &Params) -> Result<Geometry> {
    let g = PUSHPULL.one(i, 0)?;
    let through = match p.e("through", &["off", "auto", "force"])?.as_str() {
        "off" => Through::Off,
        "force" => Through::Force,
        _ => Through::Auto,
    };
    apply_through(&g, &p.s("group")?, p.f("distance")?, through)
}
/// SketchUp-style punch-through behaviour (Rust-only `through` parameter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Through {
    /// Python-compatible: exactly one convex face, the cutter ends at `distance`.
    Off,
    /// Punch through when the push reaches (or passes) the opposite surface; the
    /// excess is ignored. A pull landing on a parallel face plane merges into it.
    Auto,
    /// Always punch through to the opposite surface when one exists.
    Force,
}
#[cfg(test)]
pub(super) fn apply(g: &Geometry, group: &str, distance: f64) -> Result<Geometry> {
    apply_through(g, group, distance, Through::Off)
}
pub const INSET: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "inset",
    inputs: GEO,
    params: crate::params!("group":"";"amount":0.1;"mode":"connected";
        "depth":0.0;"use_boundary":true;"use_even_offset":true;"use_relative_offset":false;"use_edge_rail":false;
        "use_outset":false;"use_individual":false;"use_interpolate":true;
        "output_group":false;"output_group_name":"inset_ring";"output_inner_group":false;"output_inner_group_name":"inset_inner"),
    cook: inset,
};
pub const OUTSET: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "outset",
    inputs: GEO,
    params: crate::params!("group":"";"amount":0.1;"mode":"connected";
        "output_group":false;"output_group_name":"outset_ring";"output_inner_group":false;"output_inner_group_name":"outset_boundary"),
    cook: outset,
};
fn inset(i: &Inputs, p: &Params) -> Result<Geometry> {
    let g = INSET.one(i, 0)?;
    let individual = p.e("mode", &["connected", "individual"])? == "individual" || p.b("use_individual")?;
    let mode = if individual { "individual" } else { "connected" };
    let amount = if p.b("use_outset")? { -p.f("amount")? } else { p.f("amount")? };
    let depth = p.f("depth")?;
    let extended = !p.b("use_boundary")? || !p.b("use_even_offset")? || p.b("use_relative_offset")? || p.b("use_edge_rail")? || !p.b("use_interpolate")?;
    let mut out = if extended {
        let sel = model::prim_mask(&g, &p.s("group")?)?;
        let o = super::inset_ext::Opts { thickness: amount, depth, boundary: p.b("use_boundary")?, even: p.b("use_even_offset")?,
            relative: p.b("use_relative_offset")?, rail: p.b("use_edge_rail")?, individual, interpolate: p.b("use_interpolate")? };
        super::inset_ext::inset_ext(&g, &sel, &o)?
    } else {
        model::extrude(&g, &p.s("group")?, depth, amount, mode, false, None)?
    };
    out = auto_merge(&out, g.npoints)?;
    write_output_groups(&mut out, p)?;
    Ok(out)
}
fn write_output_groups(out: &mut Geometry, p: &Params) -> Result<()> {
    for (flag, name, src) in [("output_group", "output_group_name", "group_extrudeSide"), ("output_inner_group", "output_inner_group_name", "group_extrudeFront")] {
        if p.b(flag)? {
            let n = p.s(name)?;
            model::group_name(n.trim()).map_err(|_| Error::new(format!("invalid {name} {n:?}")))?;
            let mask: Vec<bool> = out.prim.get(src).map_or(vec![false; out.nprims()], |a| super::group::flags(a));
            model::write_group(out, "prim", n.trim(), &mask)?;
        }
    }
    Ok(())
}
fn outset(i: &Inputs, p: &Params) -> Result<Geometry> {
    let g = OUTSET.one(i, 0)?;
    let out = model::extrude(&g, &p.s("group")?, 0., -p.f("amount")?, &p.e("mode", &["connected", "individual"])?, false, None)?;
    let mut out = auto_merge(&out, g.npoints)?;
    write_output_groups(&mut out, p)?;
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    fn quad(s: f32) -> Geometry {
        Geometry::from_arrays(vec![0., 0., 0., s, 0., 0., s, s, 0., 0., s, 0.], vec![0, 1, 2, 3], vec![0, 4], None).unwrap()
    }
    #[test]
    fn inset_auto_merges_collapsed_inner_face() {
        // inset by half the width: the inner quad collapses to one point, merged with itself; ring becomes triangles
        let p = Params::resolve(INSET.params, serde_json::json!({"amount": std::f64::consts::SQRT_2, "use_even_offset": false}).as_object()).unwrap();
        let o = inset(&[vec![Arc::new(quad(2.))]], &p).unwrap();
        o.validate().unwrap();
        assert_eq!((o.npoints, o.nprims()), (5, 4));
    }
    #[test]
    fn outset_output_groups_are_optional() {
        let p = Params::resolve(OUTSET.params, None).unwrap();
        let o = outset(&[vec![Arc::new(quad(2.))]], &p).unwrap();
        assert!(!o.prim.contains_key("group_outset_ring"));
        let p = Params::resolve(OUTSET.params, serde_json::json!({"output_group": true, "output_inner_group": true}).as_object()).unwrap();
        let o = outset(&[vec![Arc::new(quad(2.))]], &p).unwrap();
        assert_eq!(o.prim["group_outset_ring"].as_i32().unwrap().iter().sum::<i32>(), 4);
        assert_eq!(o.prim["group_outset_boundary"].as_i32().unwrap().iter().sum::<i32>(), 1);
    }
    #[test]
    fn inset_has_no_height_and_python_groups() {
        let g = Geometry::from_arrays(
            vec![0., 0., 0., 2., 0., 0., 2., 2., 0., 0., 2., 0.],
            vec![0, 1, 2, 3],
            vec![0, 4],
            None,
        )
        .unwrap();
        let p = Params::resolve(INSET.params, None).unwrap();
        let o = inset(&[vec![Arc::new(g)]], &p).unwrap();
        assert_eq!((o.npoints, o.nprims()), (8, 5));
        assert!(o.p().chunks_exact(3).all(|p| p[2] == 0.));
        assert!(o.prim.contains_key("group_extrudeFront"));
        assert!(o.prim.contains_key("group_extrudeSide"));
    }
    #[test]
    fn pushpull_distance_cooks_live_preview_from_connected_box() {
        let box_params=Params::resolve(super::super::generate::BOX.params,None).unwrap();
        let g=(super::super::generate::BOX.cook)(&[vec![]],&box_params).unwrap();
        let zero=apply(&g,"0",0.).unwrap();
        let moved=apply(&g,"0",0.25).unwrap();
        assert_eq!(zero.p(),g.p());
        assert_ne!(moved.p(),g.p());
        moved.validate().unwrap();
    }
    fn mk(points: &[[f64; 3]], faces: &[Vec<usize>]) -> Geometry {
        let build = |faces: &[Vec<usize>]| {
            let mut o = vec![0];
            for f in faces {
                o.push(o.last().unwrap() + f.len() as i32);
            }
            Geometry::from_arrays(
                points.iter().flat_map(|p| p.map(|v| v as f32)).collect(),
                faces.iter().flatten().map(|&v| v as i32).collect(),
                o,
                None,
            )
            .unwrap()
        };
        let g = build(faces);
        if volume(&g) > 0. {
            return g;
        }
        let rev: Vec<Vec<usize>> = faces.iter().map(|f| f.iter().rev().copied().collect()).collect();
        build(&rev)
    }
    fn volume(g: &Geometry) -> f64 {
        let pos = positions(g);
        let (ts, _, _) = boolean::triangles(g);
        ts.iter().map(|t| dot(pos[t[0]], cross(pos[t[1]], pos[t[2]]))).sum::<f64>() / 6.
    }
    fn euler(g: &Geometry) -> i64 {
        let edges = crate::topo::edges(&g.topology, g.npoints);
        g.npoints as i64 - edges.len() as i64 + g.nprims() as i64
    }
    /// Closed, consistently wound, outward, finite, and with a given Euler characteristic.
    fn check(g: &Geometry, chi: i64) {
        g.validate().unwrap();
        solid(g, "test").unwrap();
        assert!(twins(g).iter().all(Option::is_some), "not closed");
        assert_eq!(euler(g), chi, "Euler characteristic");
        assert!(!g.prim.contains_key(TAG));
    }
    fn face_area(g: &Geometry, f: usize) -> f64 {
        let pos = positions(g);
        let q: Vec<_> = range(g, f).map(|v| pos[g.topology.vertex_point[v] as usize]).collect();
        let mut n = [0.; 3];
        for k in 0..q.len() {
            n = add(n, cross(q[k], q[(k + 1) % q.len()]));
        }
        len(n) / 2.
    }
    fn prims_where(g: &Geometry, f: impl Fn([f64; 3], [f64; 3]) -> bool) -> Vec<usize> {
        let pos = positions(g);
        let ns = crate::normals::prim_normals(g.p(), &g.topology);
        (0..g.nprims())
            .filter(|&p| {
                let r = range(g, p);
                let c = mul(
                    r.clone().map(|v| pos[g.topology.vertex_point[v] as usize]).fold([0.; 3], add),
                    1. / r.len() as f64,
                );
                f(c, crate::boolean_kernel::unit(ns[p]))
            })
            .collect()
    }
    fn group(ids: &[usize]) -> String {
        ids.iter().map(usize::to_string).collect::<Vec<_>>().join(" ")
    }
    fn unit_box() -> Geometry {
        let p = Params::resolve(super::super::generate::BOX.params, None).unwrap();
        (super::super::generate::BOX.cook)(&[vec![]], &p).unwrap()
    }
    /// Box with the +Y face inset; returns (geometry, inner face, inner face area).
    fn inset_top(g: &Geometry, amount: f64) -> (Geometry, usize, f64) {
        let top = prims_where(g, |_, n| n[1] > 0.99)[0];
        let o = model::extrude(g, &top.to_string(), 0., amount, "connected", false, None).unwrap();
        let mask = prim_mask(&o, "extrudeFront").unwrap();
        let inner = (0..o.nprims()).find(|&i| mask[i]).unwrap();
        let area = face_area(&o, inner);
        (o, inner, area)
    }
    #[test]
    fn through_hole_on_rotated_box_has_no_unpunched_band() {
        let (g, inner, area) = inset_top(&unit_box(), 0.25);
        let pos = positions(&g);
        let faces: Vec<Vec<usize>> = (0..g.nprims())
            .map(|f| range(&g, f).map(|v| g.topology.vertex_point[v] as usize).collect())
            .collect();
        for (ax, angle) in [([1., 1., 0.], 0.7_f64), ([0., 1., 1.], 0.4), ([1., 2., 3.], 1.1)] {
            let k = mul(ax, 1. / len(ax));
            let rot = |p: [f64; 3]| {
                let (c, s) = (angle.cos(), angle.sin());
                add(add(mul(p, c), mul(cross(k, p), s)), mul(k, dot(k, p) * (1. - c)))
            };
            let pts: Vec<_> = pos.iter().map(|&p| rot(p)).collect();
            let r = mk(&pts, &faces);
            let out = apply_through(&r, &inner.to_string(), -1., Through::Auto).unwrap();
            check(&out, 0);
            assert!((volume(&out) - (1. - area)).abs() < 1e-4, "volume {} axis {ax:?}", volume(&out));
        }
    }
    #[test]
    fn through_hole_on_box_snaps_to_far_face() {
        let (g, inner, area) = inset_top(&unit_box(), 0.25);
        let thickness = 1.;
        let off = apply_through(&g, &inner.to_string(), -0.5, Through::Off).unwrap();
        check(&off, 2); // pocket: still a sphere
        let out = apply_through(&g, &inner.to_string(), -thickness, Through::Auto).unwrap();
        check(&out, 0); // torus: through-hole
        assert!((volume(&out) - (1. - area * thickness)).abs() < 1e-4, "{}", volume(&out));
        // Neither the pushed face nor a coincident far cap survives inside the hole.
        let pos = positions(&out);
        let caps: Vec<_> = prims_where(&out, |_, n| n[1].abs() > 0.99)
            .into_iter()
            .filter(|&f| range(&out, f).all(|v| {
                let p = pos[out.topology.vertex_point[v] as usize];
                p[0].abs() < 0.4 && p[2].abs() < 0.4
            }))
            .collect();
        assert!(caps.is_empty(), "caps left in the hole: {caps:?}");
        // Snap tolerance: a hair short still punches; Force punches from any distance.
        let near = apply_through(&g, &inner.to_string(), -thickness * (1. - 2e-6), Through::Auto).unwrap();
        check(&near, 0);
        let forced = apply_through(&g, &inner.to_string(), -0.1, Through::Force).unwrap();
        check(&forced, 0);
        // New faces: material slot inherited, corner UVs invalid (NaN) rather than garbage.
        assert_eq!(out.nprims(), out.prim.values().next().map_or(out.nprims(), |a| a.rows()));
    }
    #[test]
    fn push_past_thickness_ignores_excess() {
        let (g, inner, area) = inset_top(&unit_box(), 0.25);
        let exact = apply_through(&g, &inner.to_string(), -1., Through::Auto).unwrap();
        let past = apply_through(&g, &inner.to_string(), -7.5, Through::Auto).unwrap();
        check(&past, 0);
        assert_eq!((past.npoints, past.nprims()), (exact.npoints, exact.nprims()));
        assert!((volume(&past) - (1. - area)).abs() < 1e-4);
        // Python-compatible Off mode also cuts through when far past.
        let legacy = apply_through(&g, &inner.to_string(), -3., Through::Off).unwrap();
        check(&legacy, 0);
        assert!((volume(&legacy) - volume(&past)).abs() < 1e-5);
        // Auto also drops the collinear split points the legacy cut leaves behind.
        assert!(past.npoints <= legacy.npoints);
    }
    fn wedge() -> Geometry {
        // Bottom z=0 square [0,2]^2; slanted top from z=1 (x=0) to z=2 (x=2).
        let p = [
            [0., 0., 0.], [2., 0., 0.], [2., 2., 0.], [0., 2., 0.],
            [0., 0., 1.], [2., 0., 2.], [2., 2., 2.], [0., 2., 1.],
        ];
        let f = vec![
            vec![0, 3, 2, 1], vec![4, 5, 6, 7], vec![0, 1, 5, 4],
            vec![1, 2, 6, 5], vec![2, 3, 7, 6], vec![3, 0, 4, 7],
        ];
        mk(&p, &f)
    }
    #[test]
    fn non_parallel_back_face_wedge() {
        let w = wedge();
        solid(&w, "wedge").unwrap();
        let bottom = prims_where(&w, |_, n| n[2] < -0.99)[0];
        let o = model::extrude(&w, &bottom.to_string(), 0., 0.25, "connected", false, None).unwrap();
        let inner = (0..o.nprims()).find(|&i| prim_mask(&o, "extrudeFront").unwrap()[i]).unwrap();
        // Between the shallow and deep exits: an ordinary (partially open) cut is not a punch.
        let pocket = apply_through(&o, &inner.to_string(), -0.8, Through::Auto).unwrap();
        check(&pocket, 2);
        let deepest = 2.;
        let hole = apply_through(&o, &inner.to_string(), -deepest, Through::Auto).unwrap();
        check(&hole, 0);
        let past = apply_through(&o, &inner.to_string(), -5., Through::Auto).unwrap();
        check(&past, 0);
        assert!((volume(&hole) - volume(&past)).abs() < 1e-5);
    }
    #[test]
    fn face_with_existing_hole_can_be_pushed() {
        let (g, inner, _) = inset_top(&unit_box(), 0.25);
        let holed = apply_through(&g, &inner.to_string(), -1., Through::Auto).unwrap();
        // The far (-Y) face now carries the hole; push all its pieces in by 0.2.
        let bottom = prims_where(&holed, |c, n| n[1] < -0.99 && c[1] < -0.49);
        assert!(!bottom.is_empty());
        let before = volume(&holed);
        let area: f64 = bottom.iter().map(|&f| face_area(&holed, f)).sum();
        let out = apply_through(&holed, &group(&bottom), -0.2, Through::Auto).unwrap();
        check(&out, 0);
        assert!((volume(&out) - (before - 0.2 * area)).abs() < 1e-4, "{} vs {}", volume(&out), before - 0.2 * area);
    }
    #[test]
    fn ngon_face_punches_through_octagonal_prism() {
        let mut p = vec![];
        for z in [0., 1.] {
            for i in 0..8 {
                let a = (i as f64 + 0.5) * std::f64::consts::TAU / 8.;
                p.push([2. * a.cos(), 2. * a.sin(), z]);
            }
        }
        let mut f = vec![(0..8).rev().collect::<Vec<_>>(), (8..16).collect()];
        for i in 0..8 {
            f.push(vec![i, (i + 1) % 8, (i + 1) % 8 + 8, i + 8]);
        }
        let g = mk(&p, &f);
        let top = prims_where(&g, |_, n| n[2] > 0.99)[0];
        let o = model::extrude(&g, &top.to_string(), 0., 0.3, "connected", false, None).unwrap();
        let inner = (0..o.nprims()).find(|&i| prim_mask(&o, "extrudeFront").unwrap()[i]).unwrap();
        assert_eq!(range(&o, inner).len(), 8);
        let area = face_area(&o, inner);
        let before = volume(&o);
        let out = apply_through(&o, &inner.to_string(), -1., Through::Auto).unwrap();
        check(&out, 0);
        assert!((volume(&out) - (before - area)).abs() < 1e-4);
    }
    fn step() -> Geometry {
        // Profile in XZ: lower step x in [1,2] at z=1, upper block x in [0,1] at z=2; depth y in [0,1].
        let prof = [[0., 0.], [2., 0.], [2., 1.], [1., 1.], [1., 2.], [0., 2.]];
        let mut p = vec![];
        for y in [0., 1.] {
            for q in prof {
                p.push([q[0], y, q[1]]);
            }
        }
        let mut f = vec![(0..6).collect::<Vec<_>>(), (6..12).rev().collect()];
        for i in 0..6 {
            f.push(vec![i, i + 6, (i + 1) % 6 + 6, (i + 1) % 6]);
        }
        mk(&p, &f)
    }
    #[test]
    fn pull_to_coplanar_merges_into_face() {
        let g = step();
        solid(&g, "step").unwrap();
        let low = prims_where(&g, |c, n| n[2] > 0.99 && c[2] < 1.5)[0];
        let out = apply_through(&g, &low.to_string(), 1. + 3e-6, Through::Auto).unwrap();
        check(&out, 2);
        assert!((volume(&out) - 4.).abs() < 1e-4);
        let tops = prims_where(&out, |_, n| n[2] > 0.99);
        assert_eq!(tops.len(), 1, "top faces merged");
        assert!((face_area(&out, tops[0]) - 2.).abs() < 1e-4);
        assert_eq!(out.nprims(), 6, "a clean box");
        // Off keeps the Python behaviour (no merge).
        let legacy = apply_through(&g, &low.to_string(), 1., Through::Off);
        if let Ok(legacy) = legacy {
            assert!(legacy.nprims() > 6);
        }
    }
    #[test]
    fn open_mesh_is_a_plain_push() {
        let g = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.],
            vec![0, 1, 2, 3],
            vec![0, 4],
            None,
        )
        .unwrap();
        for d in [-0.5, 0.5] {
            let out = apply_through(&g, "0", d, Through::Auto).unwrap();
            assert_eq!(out.nprims(), 5);
            assert!(out.p().iter().all(|x| x.is_finite()));
        }
        assert!(apply_through(&g, "0", 0.5, Through::Off).is_err());
    }
    #[test]
    fn two_selected_faces_push_together() {
        let b = unit_box();
        let top = prims_where(&b, |_, n| n[1] > 0.99)[0];
        let side = prims_where(&b, |_, n| n[1] < -0.99)[0];
        let o = model::extrude(&b, &group(&[top, side]), 0., 0.25, "individual", false, None).unwrap();
        let mask = prim_mask(&o, "extrudeFront").unwrap();
        let ids: Vec<_> = (0..o.nprims()).filter(|&i| mask[i]).collect();
        assert_eq!(ids.len(), 2);
        let area: f64 = ids.iter().map(|&f| face_area(&o, f)).sum();
        let pockets = apply_through(&o, &group(&ids), -0.3, Through::Auto).unwrap();
        check(&pockets, 2);
        assert!((volume(&pockets) - (1. - 0.3 * area)).abs() < 1e-4);
        assert!(apply_through(&o, &group(&ids), -0.3, Through::Off).is_err());
    }
    #[test]
    fn concave_face_pull_in_auto_mode() {
        // The step's front face is an L-shaped hexagon.
        let g = step();
        let front = prims_where(&g, |_, n| n[1] < -0.99)[0];
        assert!(apply_through(&g, &front.to_string(), 0.5, Through::Off).is_err());
        let out = apply_through(&g, &front.to_string(), 0.5, Through::Auto).unwrap();
        check(&out, 2);
        assert!((volume(&out) - 3. * 1.5).abs() < 1e-4);
    }
    #[test]
    fn default_through_param_is_auto_and_python_cases_match_off() {
        let p = Params::resolve(PUSHPULL.params, None).unwrap();
        assert_eq!(p.s("through").unwrap(), "auto");
    }
}
