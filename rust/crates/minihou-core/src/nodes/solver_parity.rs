//! Python oracles for the time / physics family (goals, solver, rbd attribute nodes). Regenerate only with
//! `tools/dump_rust_solver_fixtures.py`; Python is the reference.

use serde_json::Value;

use crate::geometry::{AttrData, Geometry};
use crate::graph::Graph;
use crate::timectx;

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol * (1.0 + a.abs().max(b.abs()))
}

/// JSON equality with numbers compared within `tol` (relative).
fn same(a: &Value, b: &Value, tol: f64, path: &str) {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            assert!(close(x.as_f64().unwrap(), y.as_f64().unwrap(), tol), "{path}: {x} != {y}")
        }
        (Value::Array(x), Value::Array(y)) => {
            assert_eq!(x.len(), y.len(), "{path}: length");
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                same(p, q, tol, &format!("{path}[{i}]"));
            }
        }
        (Value::Object(x), Value::Object(y)) => {
            let (kx, ky): (Vec<_>, Vec<_>) = (x.keys().collect(), y.keys().collect());
            let (mut sx, mut sy) = (kx.clone(), ky.clone());
            sx.sort();
            sy.sort();
            assert_eq!(sx, sy, "{path}: keys");
            for k in kx {
                same(&x[k], &y[k], tol, &format!("{path}.{k}"));
            }
        }
        _ => assert_eq!(a, b, "{path}"),
    }
}

fn compare(name: &str, got: &Geometry, want: &Value, tol: f64) {
    let ints = |v: &Value| v.as_array().unwrap().iter().map(|x| x.as_i64().unwrap() as i32).collect::<Vec<_>>();
    assert_eq!(got.topology.vertex_point, ints(&want["vp"]), "{name}: vertex_point");
    assert_eq!(got.topology.prim_offsets, ints(&want["offsets"]), "{name}: offsets");
    let closed: Vec<bool> = want["closed"].as_array().unwrap().iter().map(|x| x.as_bool().unwrap()).collect();
    assert_eq!(got.topology.closed_mask(), closed, "{name}: closed");
    for (dom, table) in [("point", &got.point), ("vertex", &got.vertex), ("prim", &got.prim)] {
        let w = want[dom].as_object().unwrap();
        let mut names: Vec<&String> = table.keys().collect();
        let mut expected: Vec<&String> = w.keys().collect();
        names.sort();
        expected.sort();
        assert_eq!(names, expected, "{name}: {dom} attribute names");
        for (attr, a) in table {
            let e = &w[attr];
            assert_eq!(a.width as u64, e["width"].as_u64().unwrap(), "{name}: {dom}/{attr} width");
            assert_eq!(a.dtype(), e["dtype"].as_str().unwrap(), "{name}: {dom}/{attr} dtype");
            let data = e["data"].as_array().unwrap();
            match &a.data {
                AttrData::F32(v) => {
                    assert_eq!(v.len(), data.len());
                    for (i, (x, y)) in v.iter().zip(data).enumerate() {
                        assert!(close(*x as f64, y.as_f64().unwrap(), tol), "{name}: {dom}/{attr}[{i}] {x} != {y}");
                    }
                }
                AttrData::I32(v) => assert_eq!(*v, ints(&e["data"]), "{name}: {dom}/{attr}"),
            }
        }
    }
    same(&Value::Object(got.detail.clone()), &want["detail"], tol.max(1e-9), &format!("{name}: detail"));
}

#[test]
fn solver_family_matches_python() {
    let cases: Value = serde_json::from_str(include_str!("test_data/solver_parity.json")).unwrap();
    for c in cases.as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let graph = Graph::from_value(&c["graph"]).unwrap_or_else(|e| panic!("{name}: {e}"));
        let frame = c["frame"].as_f64().unwrap();
        let result = timectx::at(Some(frame), Some(24.0), || graph.cook(Some(c["target"].as_str().unwrap())));
        if let Some(err) = c.get("expected_error") {
            let e = result.expect_err(name);
            assert!(e.0.contains(err.as_str().unwrap()), "{name}: {e} vs {err}");
            continue;
        }
        let got = result.unwrap_or_else(|e| panic!("{name}: {e}"));
        // relaxation runs hundreds of float64 steps; the stored float32 positions agree to rounding
        let tol = if name.starts_with("solver") || name.starts_with("rbd") { 1e-6 } else { 2e-5 };
        compare(name, &got, &c["expected"], tol);
    }
}
