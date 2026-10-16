//! Geometry-driven rigid bodies, ported from `minihou/nodes/rbd.py`: `rbd_configure` (per-piece body attributes),
//! `rbd_constraint` (one physical link as a two-point curve) and `rbd_solver` (see [`super::rbd_engine`]).

use std::sync::Arc;

use super::sim_common::{float, vec3};
use super::{InputSpec, Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::Params;

pub const RBD_CONFIGURE: NodeSpec = NodeSpec {
    name: "rbd_configure",
    inputs: GEO,
    params: crate::params!("piece_attrib": ""; "id_offset": 0; "active": true; "mass": 1.0; "friction": 0.6; "restitution": 0.0;
        "linear_damping": 0.05; "angular_damping": 0.1; "velocity": [0.0, 0.0, 0.0]; "angular_velocity": [0.0, 0.0, 0.0];
        "compound": false),
    cook: rbd_configure,
};

pub const RBD_CONSTRAINT: NodeSpec = NodeSpec {
    name: "rbd_constraint",
    inputs: &[],
    params: crate::params!("kind": "glue"; "body_a": 0; "body_b": 1; "world": false; "anchor": [0.0, 0.0, 0.0];
        "anchor_b": [0.0, 0.0, 1.0]; "axis": [0.0, 1.0, 0.0]; "limited": false; "lower": 0.0; "upper": 90.0; "stiffness": 100.0;
        "damping": 1.0; "rest_length": -1.0; "strength": 0.0; "break_torque": 0.0; "collide_connected": false),
    cook: rbd_constraint,
};

pub const RBD_SOLVER: NodeSpec = NodeSpec {
    name: "rbd_solver",
    inputs: &[
        InputSpec { name: "bodies", multi: false, optional: false },
        InputSpec { name: "colliders", multi: false, optional: true },
        InputSpec { name: "constraints", multi: false, optional: true },
    ],
    params: crate::params!("gravity": [0.0, 0.0, -9.81]; "start_frame": 1; "substeps": 10; "iterations": 50; "margin": 0.001;
        "reset": 0; "cache_mb": 32; "include_colliders": true),
    cook: super::rbd_engine::rbd_solver,
};

pub(super) const CONSTRAINT_KINDS: [&str; 4] = ["glue", "pin", "hinge", "spring"];

/// The values of one link (`validate_link_values` in `minihou/rbd.py`).
#[derive(Clone, Debug)]
pub(super) struct LinkValues {
    pub kind: usize,
    pub anchor: [f64; 3],
    pub anchor_b: [f64; 3],
    pub axis: [f64; 3],
    pub limited: bool,
    pub lower: f64,
    pub upper: f64,
    pub stiffness: f64,
    pub damping: f64,
    pub rest_length: f64,
    pub strength: f64,
    pub break_torque: f64,
}

pub(super) fn validate_link_values(v: &LinkValues) -> Result<()> {
    let kind = CONSTRAINT_KINDS.get(v.kind).copied().ok_or_else(|| Error::new("unknown RBD constraint kind"))?;
    for (key, a) in [("anchor", v.anchor), ("anchor_b", v.anchor_b), ("axis", v.axis)] {
        if a.iter().any(|x| !x.is_finite()) {
            return Err(Error::new(format!("constraint {key} must be a finite three-component vector")));
        }
    }
    for (key, x) in [("lower", v.lower), ("upper", v.upper), ("stiffness", v.stiffness), ("damping", v.damping),
                     ("rest_length", v.rest_length), ("strength", v.strength), ("break_torque", v.break_torque)] {
        if !x.is_finite() {
            return Err(Error::new(format!("constraint {key} must be finite")));
        }
    }
    if [v.stiffness, v.damping, v.strength, v.break_torque].iter().any(|&x| x < 0.0) {
        return Err(Error::new("constraint stiffness, damping and break thresholds must be non-negative"));
    }
    let norm = |a: [f64; 3]| (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    if kind == "hinge" && norm(v.axis) < 1e-9 {
        return Err(Error::new("hinge axis cannot be zero"));
    }
    if v.limited && kind != "hinge" && kind != "spring" {
        return Err(Error::new("limits apply only to hinge or spring; disable Limited for pin/glue"));
    }
    if v.limited {
        if v.lower >= v.upper {
            return Err(Error::new("constraint Upper must exceed Lower"));
        }
        if kind == "hinge" && !(v.lower <= 0.0 && 0.0 <= v.upper) {
            return Err(Error::new("hinge limits must include the initial zero angle"));
        }
        if kind == "spring" && v.lower < 0.0 {
            return Err(Error::new("spring minimum length cannot be negative"));
        }
    }
    if kind == "spring" && v.rest_length < 0.0 && v.rest_length != -1.0 {
        return Err(Error::new("spring Rest Length must be -1 (initial distance) or non-negative"));
    }
    if kind == "spring" && norm([v.anchor[0] - v.anchor_b[0], v.anchor[1] - v.anchor_b[1], v.anchor[2] - v.anchor_b[2]]) < 1e-9 {
        return Err(Error::new("spring anchors must start at different positions"));
    }
    if (kind == "pin" || kind == "spring") && v.break_torque != 0.0 {
        return Err(Error::new("pin and spring have no direct torque threshold; use Strength in newtons"));
    }
    Ok(())
}

fn rbd_configure(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = RBD_CONFIGURE.one(inputs, 0)?;
    let piece_attrib = p.s("piece_attrib")?;
    let id_offset = p.i("id_offset")?;
    let active = p.b("active")?;
    let mass = float(p, "mass", Some(0.000001), None)?;
    let friction = float(p, "friction", Some(0.0), Some(10.0))?;
    let restitution = float(p, "restitution", Some(0.0), Some(1.0))?;
    let linear_damping = float(p, "linear_damping", Some(0.0), Some(100.0))?;
    let angular_damping = float(p, "angular_damping", Some(0.0), Some(100.0))?;
    let velocity = vec3(p, "velocity")?;
    let angular_velocity = vec3(p, "angular_velocity")?;
    let compound = p.b("compound")?;
    let ids: Vec<i32> = if piece_attrib.is_empty() {
        super::attrib_tools::piece_labels(&geo).1
    } else {
        match geo.prim.get(&piece_attrib).map(|a| (a.width, &a.data)) {
            Some((1, AttrData::I32(v))) => v.clone(),
            _ => return Err(Error::new("piece_attrib must name a scalar primitive integer attribute")),
        }
    };
    let ids: Vec<i64> = ids.iter().map(|&i| i as i64 + id_offset).collect();
    if ids.iter().any(|&i| i < -(1i64 << 31) || i >= 1i64 << 31) {
        return Err(Error::new("piece IDs exceed int32 range"));
    }
    let n = geo.nprims();
    let mut out = (*geo).clone();
    out.prim.insert("rbd_id".into(), Arc::new(Attr::i32(1, ids.iter().map(|&i| i as i32).collect())));
    if compound {
        out.prim.insert("rbd_compound".into(), Arc::new(Attr::i32(1, vec![1; n])));
    }
    out.prim.insert("rbd_active".into(), Arc::new(Attr::i32(1, vec![active as i32; n])));
    for (name, value) in [("mass", mass), ("friction", friction), ("restitution", restitution),
                          ("linear_damping", linear_damping), ("angular_damping", angular_damping)] {
        out.prim.insert(format!("rbd_{name}"), Arc::new(Attr::f32(1, vec![value as f32; n])));
    }
    for (name, value) in [("v", velocity), ("w", angular_velocity)] {
        out.prim.insert(format!("rbd_{name}"), Arc::new(Attr::f32(3, (0..n).flat_map(|_| value.map(|x| x as f32)).collect())));
    }
    Ok(out)
}

fn rbd_constraint(_inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let kind = p.e("kind", &CONSTRAINT_KINDS)?;
    let body_a = p.i("body_a")?;
    let body_b = p.i("body_b")?;
    let world = p.b("world")?;
    let v = LinkValues {
        kind: CONSTRAINT_KINDS.iter().position(|k| *k == kind).unwrap(),
        anchor: vec3(p, "anchor")?,
        anchor_b: vec3(p, "anchor_b")?,
        axis: vec3(p, "axis")?,
        limited: p.b("limited")?,
        lower: float(p, "lower", None, None)?,
        upper: float(p, "upper", None, None)?,
        stiffness: float(p, "stiffness", Some(0.0), None)?,
        damping: float(p, "damping", Some(0.0), None)?,
        rest_length: float(p, "rest_length", Some(-1.0), None)?,
        strength: float(p, "strength", Some(0.0), None)?,
        break_torque: float(p, "break_torque", Some(0.0), None)?,
    };
    let collide_connected = p.b("collide_connected")?;
    validate_link_values(&v)?;
    if !world && body_a == body_b {
        return Err(Error::new("a constraint needs two different body IDs (or enable World)"));
    }
    if [body_a, body_b].iter().any(|&i| i < -(1i64 << 31) || i >= 1i64 << 31) {
        return Err(Error::new("constraint body IDs must fit int32"));
    }
    let second = if kind == "spring" { v.anchor_b } else { v.anchor };
    let pts: Vec<f32> = v.anchor.iter().chain(second.iter()).map(|&x| x as f32).collect();
    let mut out = Geometry::from_arrays(pts, vec![0, 1], vec![0, 2], Some(vec![false]))?;
    let ints = [("rbd_schema", 1), ("rbd_a", body_a), ("rbd_b", body_b), ("rbd_constraint", v.kind as i64),
                ("rbd_world", world as i64), ("limited", v.limited as i64), ("collide_connected", collide_connected as i64)];
    for (name, value) in ints {
        out.prim.insert(name.into(), Arc::new(Attr::i32(1, vec![value as i32])));
    }
    for (name, value) in [("anchor", v.anchor), ("anchor_b", v.anchor_b), ("axis", v.axis)] {
        out.prim.insert(name.into(), Arc::new(Attr::f32(3, value.map(|x| x as f32).to_vec())));
    }
    for (name, value) in [("lower", v.lower), ("upper", v.upper), ("stiffness", v.stiffness), ("damping", v.damping),
                          ("rest_length", v.rest_length), ("strength", v.strength), ("break_torque", v.break_torque)] {
        out.prim.insert(name.into(), Arc::new(Attr::f32(1, vec![value as f32])));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn cook(spec: &NodeSpec, geo: Vec<Geometry>, params: Value) -> Result<Geometry> {
        let mut inputs: Vec<Vec<Arc<Geometry>>> = geo.into_iter().map(|g| vec![Arc::new(g)]).collect();
        inputs.resize(spec.inputs.len(), Vec::new());
        let p = Params::resolve(spec.params, params.as_object())?;
        (spec.cook)(&inputs, &p)
    }

    fn boxes() -> Geometry {
        let b = super::super::get("box").unwrap();
        let one = (b.cook)(&[vec![]], &Params::resolve(b.params, None).unwrap()).unwrap();
        let m = super::super::get("merge").unwrap();
        let mut moved = one.clone();
        moved.set_p(one.p().chunks(3).flat_map(|q| [q[0] + 3.0, q[1], q[2]]).collect());
        (m.cook)(&[vec![Arc::new(one), Arc::new(moved)]], &Params::resolve(m.params, None).unwrap()).unwrap()
    }

    #[test]
    fn configure_writes_per_piece_attributes() {
        let g = cook(&RBD_CONFIGURE, vec![boxes()], json!({"id_offset": 5, "mass": 2, "velocity": [1, 2, 3], "compound": true})).unwrap();
        let ids = match &g.prim["rbd_id"].data { AttrData::I32(v) => v.clone(), _ => panic!() };
        assert_eq!(ids, [5, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 6]);
        assert_eq!(g.prim["rbd_mass"].as_f32().unwrap()[0], 2.0);
        assert_eq!(&g.prim["rbd_v"].as_f32().unwrap()[..3], &[1.0, 2.0, 3.0]);
        assert_eq!(g.prim["rbd_compound"].rows(), 12);
        assert!(cook(&RBD_CONFIGURE, vec![boxes()], json!({"piece_attrib": "nope"})).unwrap_err().0.contains("piece_attrib"));
        assert!(cook(&RBD_CONFIGURE, vec![boxes()], json!({"friction": 11})).unwrap_err().0.contains("above maximum"));
    }

    #[test]
    fn constraint_geometry_and_validation() {
        let g = cook(&RBD_CONSTRAINT, vec![], json!({"kind": "spring", "anchor": [0, 0, 1], "anchor_b": [0, 0, 3]})).unwrap();
        assert_eq!(g.p(), &[0., 0., 1., 0., 0., 3.]);
        assert_eq!(g.prim["rbd_constraint"].data, AttrData::I32(vec![3]));
        assert_eq!(g.prim["rest_length"].as_f32().unwrap(), &[-1.0]);
        let pin = cook(&RBD_CONSTRAINT, vec![], json!({"kind": "pin", "anchor": [1, 2, 3]})).unwrap();
        assert_eq!(pin.p(), &[1., 2., 3., 1., 2., 3.]);
        for (params, msg) in [
            (json!({"body_b": 0}), "two different"),
            (json!({"kind": "pin", "limited": true}), "limits apply"),
            (json!({"kind": "hinge", "axis": [0, 0, 0]}), "axis cannot be zero"),
            (json!({"kind": "hinge", "limited": true, "lower": 10}), "include the initial zero"),
            (json!({"kind": "spring", "anchor_b": [0, 0, 0]}), "different positions"),
            (json!({"kind": "pin", "break_torque": 1}), "no direct torque"),
            (json!({"kind": "spring", "rest_length": -0.5}), "Rest Length"),
            (json!({"body_a": 3000000000i64}), "int32"),
        ] {
            let e = cook(&RBD_CONSTRAINT, vec![], params).unwrap_err();
            assert!(e.0.contains(msg), "{e} / {msg}");
        }
    }
}
