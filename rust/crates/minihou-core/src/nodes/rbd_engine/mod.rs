//! Rigid-body worlds and pose caches for `rbd_solver`, ported from `minihou/rbd.py`.
//!
//! What is a straight port: input validation (`pieces`, `convex_part`, `link_table`, `validate_link_values`), the
//! world/frame semantics (integer frames with fixed substeps, fractional frames hold the preceding integer frame,
//! going back restarts from the initial state, never from a snapshot; byte-bounded pose cache; at most 8 resident
//! worlds), velocity damping, the 0.1 s settle time before links may break, legacy glue's one-break-per-substep rule,
//! contact exclusion between linked pairs until all their non-colliding links break, and the output geometry
//! (transformed points and normals, `rbd_v` / `rbd_w`, colliders merged, detail `rbd`).
//!
//! What is not: Python delegates the dynamics to MuJoCo 3.15.0, an optional dependency that is not in the Rust build.
//! The dynamics here are a native sequential-impulse solver: convex-convex contacts from SAT with face clipping
//! (`margin` is the contact distance), Coulomb friction (pair friction = max), restitution (pair = mean, applied above a
//! small approach speed), split position correction, and joints as velocity constraints (weld for glue, ball for pin,
//! ball + two angular rows + limit + damping for hinge, explicit Hooke spring + length stops for spring). Masses and
//! inertias are those of the closed (convex) part meshes at uniform density, like MuJoCo's `inertia="convex"`. Results
//! therefore behave like the Python solver (bodies fall, rest, stack, tumble, links hold and break) but are not
//! bit-identical to MuJoCo. `iterations` is the velocity iteration count per substep.


use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::geometry::Geometry;
use crate::timectx;

mod solver;

pub(super) const BACKEND: &str = solver::BACKEND;
pub(super) const BACKEND_VERSION: &str = solver::BACKEND_VERSION;
pub(crate) const SETTLE_TIME: f64 = solver::SETTLE_TIME;
pub use solver::clear_worlds;
pub(super) use solver::rbd_solver;

mod math;
use math::*;
mod input;
mod collision;
mod joints;
mod world;

pub(super) use input::{link_table, pieces, Link, Piece, PieceValues};
pub(crate) use input::check_static_inputs;
pub(super) use world::World;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;

    fn node(t: &str, params: Value, inputs: Value) -> Value {
        json!({"type": t, "params": params, "inputs": inputs})
    }

    fn drop_graph(settings: Value) -> Value {
        json!({"nodes": {
            "box": node("box", json!({"center": [0, 0, 3]}), json!({})),
            "body": node("rbd_configure", json!({}), json!({"geo": "box"})),
            "floor": node("box", json!({"size": [20, 20, 1], "center": [0, 0, -0.5]}), json!({})),
            "sim": node("rbd_solver", settings, json!({"bodies": "body", "colliders": "floor"})),
        }, "display": "sim"})
    }

    fn cook(g: &Value, frame: f64) -> Result<Geometry> {
        timectx::at(Some(frame), None, || Graph::from_value(g).unwrap().cook(None).map(|g| (*g).clone()))
    }

    fn pts(g: &Geometry, n: usize) -> Vec<V3> {
        g.p().chunks(3).take(n).map(|q| [q[0] as f64, q[1] as f64, q[2] as f64]).collect()
    }

    #[test]
    fn ground_contact_rigidity_and_replay() {
        let g = drop_graph(json!({}));
        let initial = cook(&g, 1.0).unwrap();
        let result = cook(&g, 90.0).unwrap();
        let (a, b) = (pts(&initial, 8), pts(&result, 8));
        let zmin = b.iter().map(|p| p[2]).fold(f64::INFINITY, f64::min);
        assert!(zmin > -0.005 && zmin < 0.015, "rest height {zmin}");
        for i in 0..8 {
            for j in 0..8 {
                assert!((norm(sub(a[i], a[j])) - norm(sub(b[i], b[j]))).abs() < 1e-5);
            }
        }
        assert_eq!(result.npoints, 16, "colliders merged");
        assert_eq!(result.detail["rbd"]["frame"], json!(90));
        // stepping frame by frame gives the same result as jumping; fractional frames hold
        let direct = cook(&g, 60.0).unwrap();
        for f in 5..=60 {
            cook(&g, f as f64).unwrap();
        }
        assert_eq!(cook(&g, 60.0).unwrap().p(), direct.p());
        assert_eq!(cook(&g, 60.9).unwrap().p(), direct.p());
        clear_worlds();
        cook(&g, 4.0).unwrap();
        assert_eq!(cook(&g, 60.0).unwrap().p(), direct.p());
    }

    #[test]
    fn spin_damping_and_velocity_attributes() {
        let mut g = drop_graph(json!({"gravity": [0, 0, 0], "include_colliders": false}));
        g["nodes"]["body"]["params"] = json!({"velocity": [3, 0, 0], "mass": 7, "linear_damping": 0});
        let fast = cook(&g, 25.0).unwrap();
        let mean_x = pts(&fast, 8).iter().map(|p| p[0]).sum::<f64>() / 8.0;
        assert!(mean_x > 2.9, "{mean_x}");
        assert_eq!(fast.npoints, 8);
        assert!((fast.prim["rbd_v"].as_f32().unwrap()[0] - 3.0).abs() < 1e-4);
        g["nodes"]["body"]["params"]["linear_damping"] = json!(2);
        let slow = cook(&g, 25.0).unwrap();
        assert!(pts(&slow, 8).iter().map(|p| p[0]).sum::<f64>() / 8.0 < 1.4);
        g["nodes"]["body"]["params"] = json!({"angular_velocity": [0, 0, 2]});
        let spun = cook(&g, 10.0).unwrap();
        assert!(spun.prim["rbd_w"].as_f32().unwrap()[2] > 1.5);
    }

    #[test]
    fn rejects_bad_geometry_and_animated_inputs() {
        for kind in ["torus", "grid"] {
            let mut g = drop_graph(json!({}));
            g["nodes"]["box"] = node(kind, json!({}), json!({}));
            let e = cook(&g, 1.0).unwrap_err();
            assert!(e.0.contains("concave") || e.0.contains("open") || e.0.contains("solid"), "{e}");
        }
        let graph = Graph::from_value(&json!({"nodes": {
            "box": node("box", json!({}), json!({})),
            "s": node("solver", json!({"body": "transform"}), json!({"geo": "box"})),
            "body": node("rbd_configure", json!({}), json!({"geo": "s"})),
            "sim": node("rbd_solver", json!({}), json!({"bodies": "body"})),
        }, "display": "sim"})).unwrap();
        let e = check_static_inputs(&graph, "sim").unwrap_err();
        assert!(e.0.contains("static") && e.0.contains("'sim' (rbd_solver)"), "{e}");
    }

    fn wall(velocity: V3) -> Value {
        let mut nodes = serde_json::Map::new();
        let mut names = Vec::new();
        for r in 0..6 {
            for c in 0..8 {
                let name = format!("b{r}_{c}");
                nodes.insert(name.clone(), node("box", json!({"size": [0.9, 0.4, 0.45], "center": [-3.5 + c as f64, 0, 0.225 + 0.46 * r as f64]}), json!({})));
                names.push(name);
            }
        }
        nodes.insert("row".into(), json!({"type": "merge", "params": {}, "inputs": {"geos": names}}));
        nodes.insert("bricks".into(), node("rbd_configure", json!({"mass": 1, "friction": 0.65}), json!({"geo": "row"})));
        nodes.insert("ball".into(), node("sphere", json!({"radius": 0.45, "rows": 8, "columns": 16, "center": [0.2, -5, 1.4]}), json!({})));
        nodes.insert("projectile".into(), node("rbd_configure", json!({"id_offset": 10000, "mass": 12, "velocity": velocity,
            "angular_velocity": [2, 0, 0], "restitution": 0.15}), json!({"geo": "ball"})));
        nodes.insert("bodies".into(), json!({"type": "merge", "params": {}, "inputs": {"geos": ["bricks", "projectile"]}}));
        nodes.insert("floor".into(), node("box", json!({"size": [80, 80, 0.5], "center": [0, 0, -0.25]}), json!({})));
        nodes.insert("ground".into(), node("rbd_configure", json!({"active": false, "friction": 0.8}), json!({"geo": "floor"})));
        nodes.insert("sim".into(), node("rbd_solver", json!({"substeps": 12}), json!({"bodies": "bodies", "colliders": "ground"})));
        json!({"nodes": nodes, "display": "sim"})
    }

    fn centers(g: &Geometry) -> Vec<V3> {
        pts(g, 384).chunks(8).map(|c| c.iter().fold([0.0; 3], |s, p| add(s, *p)).map(|x| x / 8.0)).collect()
    }

    #[test]
    fn brick_wall_projectile_knockdown() {
        let g = wall([0.0, 12.0, 0.0]);
        let c0 = centers(&cook(&g, 1.0).unwrap());
        let result = cook(&g, 72.0).unwrap();
        let c = centers(&result);
        let moved = c.iter().zip(&c0).filter(|(a, b)| norm(sub(**a, **b)) > 0.3).count();
        assert!(moved > 8, "moved {moved}");
        assert!(c.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max) > 1.0);
        assert!(result.p().chunks(3).map(|q| q[2] as f64).fold(f64::INFINITY, f64::min) > -0.501);
        assert!(result.prim["rbd_w"].as_f32().unwrap()[..288 * 3].iter().map(|x| x.abs()).fold(0.0, f32::max) > 0.1);
        // a wall with no shot stays upright: the knockdown is caused by impact
        let quiet = centers(&cook(&wall([0.0; 3]), 72.0).unwrap());
        let drift = quiet.iter().zip(&c0).map(|(a, b)| norm(sub(*a, *b))).fold(0.0, f64::max);
        assert!(drift < 0.1, "quiet wall drift {drift}");
    }

    #[test]
    fn bounce_control() {
        let mut peaks = Vec::new();
        for bounce in [0.0, 0.8] {
            let mut g = drop_graph(json!({"substeps": 24}));
            g["nodes"]["body"]["params"] = json!({"restitution": bounce});
            g["nodes"]["ground"] = node("rbd_configure", json!({"active": false, "restitution": bounce}), json!({"geo": "floor"}));
            g["nodes"]["sim"]["inputs"]["colliders"] = json!("ground");
            let heights: Vec<f64> = (22..45).map(|f| pts(&cook(&g, f as f64).unwrap(), 8).iter().map(|p| p[2]).sum::<f64>() / 8.0).collect();
            peaks.push(heights.iter().copied().fold(f64::NEG_INFINITY, f64::max));
        }
        assert!(peaks[1] > peaks[0] + 0.25, "{peaks:?}");
    }

    #[test]
    fn spring_equilibrium_and_hinge_stop() {
        let spring = json!({"nodes": {
            "b": node("box", json!({"size": [0.2, 0.2, 0.2], "center": [0, 0, 1]}), json!({})),
            "body": node("rbd_configure", json!({}), json!({"geo": "b"})),
            "s": node("rbd_constraint", json!({"kind": "spring", "body_a": 0, "world": true, "anchor": [0, 0, 1], "anchor_b": [0, 0, 2],
                "stiffness": 100, "damping": 5, "rest_length": 1}), json!({})),
            "sim": node("rbd_solver", json!({"substeps": 20, "iterations": 100}), json!({"bodies": "body", "constraints": "s"})),
        }, "display": "sim"});
        let z = pts(&cook(&spring, 240.0).unwrap(), 8).iter().map(|p| p[2]).sum::<f64>() / 8.0;
        assert!((2.0 - z - 1.0981).abs() < 0.005, "spring length {}", 2.0 - z);
        let hinge = json!({"nodes": {
            "bar": node("box", json!({"size": [2, 0.2, 0.2], "center": [1, 0, 3]}), json!({})),
            "body": node("rbd_configure", json!({}), json!({"geo": "bar"})),
            "h": node("rbd_constraint", json!({"kind": "hinge", "body_a": 0, "world": true, "anchor": [0, 0, 3], "axis": [0, 1, 0],
                "limited": true, "lower": -25, "upper": 35, "damping": 0.5}), json!({})),
            "sim": node("rbd_solver", json!({"substeps": 20, "iterations": 100}), json!({"bodies": "body", "constraints": "h"})),
        }, "display": "sim"});
        let p = pts(&cook(&hinge, 120.0).unwrap(), 8);
        let c = p.iter().fold([0.0; 3], |s, q| add(s, *q)).map(|x| x / 8.0);
        let angle = (-(c[2] - 3.0)).atan2(c[0]).to_degrees();
        assert!((angle - 35.0).abs() < 0.8, "hinge stop at {angle}");
        assert!(c[1].abs() < 1e-3);
    }

    #[test]
    fn glue_welds_and_breaks() {
        let pair = |world_strength: f64| json!({"nodes": {
            "a": node("box", json!({"center": [0, 0, 3]}), json!({})),
            "b": node("box", json!({"center": [1, 0, 3]}), json!({})),
            "m": {"type": "merge", "params": {}, "inputs": {"geos": ["a", "b"]}},
            "body": node("rbd_configure", json!({}), json!({"geo": "m"})),
            "g": node("rbd_constraint", json!({"kind": "glue", "body_a": 0, "body_b": 1}), json!({})),
            "w": node("rbd_constraint", json!({"kind": "glue", "body_a": 0, "world": true, "strength": world_strength}), json!({})),
            "links": {"type": "merge", "params": {}, "inputs": {"geos": ["g", "w"]}},
            "sim": node("rbd_solver", json!({"substeps": 20, "iterations": 80}), json!({"bodies": "body", "constraints": "links"})),
        }, "display": "sim"});
        // unbreakable world weld on A, A-B glue: the cantilevered pair holds its pose
        let held = cook(&pair(0.0), 48.0).unwrap();
        let (p0, p) = (pts(&cook(&pair(0.0), 1.0).unwrap(), 16), pts(&held, 16));
        let drift = p.iter().zip(&p0).map(|(a, b)| norm(sub(*a, *b))).fold(0.0, f64::max);
        assert!(drift < 0.05, "glued drift {drift}");
        assert_eq!(held.detail["rbd"]["glue"]["links"], json!(2));
        // a weak world weld breaks: both fall, still glued to each other
        let fell = cook(&pair(5.0), 48.0).unwrap();
        assert_eq!(fell.detail["rbd"]["glue"]["broken_links"], json!([1]));
        let p = pts(&fell, 16);
        assert!(p.iter().map(|q| q[2]).fold(f64::NEG_INFINITY, f64::max) < 0.0);
        for i in 0..16 {
            for j in 0..16 {
                assert!((norm(sub(p[i], p[j])) - norm(sub(p0[i], p0[j]))).abs() < 0.02);
            }
        }
    }

    #[test]
    fn pin_holds_until_it_breaks() {
        let bar = |strength: f64| json!({"nodes": {
            "bar": node("box", json!({"size": [2, 0.2, 0.2], "center": [1, 0, 3]}), json!({})),
            "body": node("rbd_configure", json!({"id_offset": 0}), json!({"geo": "bar"})),
            "pin": node("rbd_constraint", json!({"kind": "pin", "body_a": 0, "world": true, "anchor": [0, 0, 3], "strength": strength}), json!({})),
            "sim": node("rbd_solver", json!({"substeps": 20, "iterations": 60}), json!({"bodies": "body", "constraints": "pin"})),
        }, "display": "sim"});
        let held = cook(&bar(0.0), 48.0).unwrap();
        // the pinned end stays at the pivot while the bar swings down
        let p = pts(&held, 8);
        let near = p.iter().map(|q| norm(sub(*q, [0.0, 0.0, 3.0]))).fold(f64::INFINITY, f64::min);
        assert!(near < 0.25, "pivot drift {near}");
        assert!(p.iter().map(|q| q[2]).fold(f64::INFINITY, f64::min) < 2.5);
        assert_eq!(held.detail["rbd"]["pin"]["broken"], json!(0));
        let broke = cook(&bar(5.0), 48.0).unwrap();
        assert_eq!(broke.detail["rbd"]["pin"]["broken"], json!(1));
        assert!(pts(&broke, 8).iter().map(|q| q[2]).fold(f64::NEG_INFINITY, f64::max) < 2.5);
    }
}
