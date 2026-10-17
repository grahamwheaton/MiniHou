//! Generated Python oracles; run with `cargo test -p minihou-core boolean_parity`.
use super::{model::*, modeling_tools::twins};
use crate::{
    boolean_kernel as k,
    geometry::{Attr, AttrData, AttrTable, Geometry, Topology},
    params::Params,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
fn table(v: &Value) -> AttrTable {
    v.as_object()
        .unwrap()
        .iter()
        .map(|(name, a)| {
            let w = a["width"].as_u64().unwrap() as usize;
            let d = a["data"].as_array().unwrap();
            let a = if a["dtype"] == "float32" {
                Attr::f32(w, d.iter().map(|v| v.as_f64().unwrap() as f32).collect())
            } else {
                Attr::i32(w, d.iter().map(|v| v.as_i64().unwrap() as i32).collect())
            };
            (name.clone(), Arc::new(a))
        })
        .collect()
}
fn geometry(v: &Value) -> Geometry {
    let point = table(&v["point"]);
    let g = Geometry {
        npoints: point["P"].rows(),
        point,
        vertex: table(&v["vertex"]),
        prim: table(&v["prim"]),
        detail: v["detail"].as_object().unwrap().clone(),
        topology: Arc::new(Topology {
            vertex_point: v["vp"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap() as i32)
                .collect(),
            prim_offsets: v["offsets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap() as i32)
                .collect(),
            closed: Some(
                v["closed"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_bool().unwrap())
                    .collect(),
            ),
        }),
    };
    g.validate().unwrap();
    g
}
fn volume(g: &Geometry) -> f64 {
    let pos = positions(g);
    (0..g.nprims())
        .map(|p| {
            let r = range(g, p);
            if r.len() < 3 {
                return 0.;
            }
            let vp = &g.topology.vertex_point;
            (1..r.len() - 1)
                .map(|i| {
                    dot(
                        pos[vp[r.start] as usize],
                        cross(
                            pos[vp[r.start + i] as usize],
                            pos[vp[r.start + i + 1] as usize],
                        ),
                    ) / 6.
                })
                .sum::<f64>()
        })
        .sum()
}
fn crossings(g: &Geometry) -> usize {
    let pos = positions(g);
    let mut bad = 0;
    for face in 0..g.nprims() {
        let q: Vec<_> = range(g, face)
            .map(|v| pos[g.topology.vertex_point[v] as usize])
            .collect();
        if q.len() < 4 {
            continue;
        }
        let n = k::unit(
            (0..q.len())
                .map(|i| cross(q[i], q[(i + 1) % q.len()]))
                .fold([0.; 3], add),
        );
        let (u, w) = k::basis(n);
        let xy: Vec<_> = q.iter().map(|&p| [dot(p, u), dot(p, w)]).collect();
        for i in 0..xy.len() {
            for j in i + 2..xy.len() - usize::from(i == 0) {
                let (a, b, c, d) = (xy[i], xy[(i + 1) % xy.len()], xy[j], xy[(j + 1) % xy.len()]);
                let ei = [b[0] - a[0], b[1] - a[1]];
                let ej = [d[0] - c[0], d[1] - c[1]];
                let sin = (ei[0] * ej[1] - ei[1] * ej[0]).abs()
                    / (ei[0].hypot(ei[1]) * ej[0].hypot(ej[1])).max(1e-300);
                if sin > 1e-3
                    && k::cross2(a, b, c) * k::cross2(a, b, d) < 0.
                    && k::cross2(c, d, a) * k::cross2(c, d, b) < 0.
                {
                    bad += 1;
                }
            }
        }
    }
    bad
}
fn attr_rows(a: &Attr, b: &Attr, indices: &[usize], tol: f64, label: &str, invalid_rows: Option<&[bool]>) {
    assert_eq!(a.width, b.width, "{label} width");
    assert_eq!(a.rows(), b.rows(), "{label} rows");
    for (i, &j) in indices.iter().enumerate() {
        for c in 0..a.width {
            match (&a.data, &b.data) {
                (AttrData::F32(av), AttrData::F32(bv)) => {
                    let (a, b) = (av[i * a.width + c], bv[j * b.width + c]);
                    // Source-less neutral faces carry invalid UVs; Python wrote zero there.
                    if invalid_rows.is_some_and(|rows| rows[i]) && a.is_nan() && b == 0. { continue; }
                    assert!(
                        (a as f64 - b as f64).abs() <= tol * (1. + (b as f64).abs()),
                        "{label} row {i} -> {j} component {c}: {a} != {b}"
                    );
                }
                (AttrData::I32(av), AttrData::I32(bv)) => {
                    assert_eq!(av[i * a.width + c], bv[j * b.width + c], "{label} row {i}")
                }
                _ => panic!("{label} dtype"),
            }
        }
    }
}
fn vertex_manifold(g: &Geometry) -> bool {
    let mut links = vec![Vec::new(); g.npoints];
    for p in 0..g.nprims() {
        let vs = &g.topology.vertex_point[range(g, p)];
        for (k, &v) in vs.iter().enumerate() {
            links[v as usize].push((
                vs[(k + vs.len() - 1) % vs.len()] as usize,
                vs[(k + 1) % vs.len()] as usize,
            ));
        }
    }
    links.into_iter().all(|edges| {
        let mut adj: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (a, b) in edges {
            adj.entry(a).or_default().push(b);
            adj.entry(b).or_default().push(a);
        }
        let Some(&start) = adj.keys().next() else {
            return true;
        };
        let mut seen = BTreeSet::new();
        let mut todo = vec![start];
        while let Some(v) = todo.pop() {
            if seen.insert(v) {
                todo.extend(&adj[&v]);
            }
        }
        let ends = adj.values().filter(|v| v.len() == 1).count();
        seen.len() == adj.len() && adj.values().all(|v| v.len() <= 2) && (ends == 0 || ends == 2)
    })
}
/// Compare up to point numbering, polygon ordering and cyclic start corners.
/// Winding, every polygon outline and every attribute row still have to agree.
fn canonical_compare(name: &str, a: &Geometry, b: &Geometry, tol: f64) {
    let ap = positions(a);
    let bp = positions(b);
    let mut taken = BTreeSet::new();
    let mut pmap = vec![];
    for p in &ap {
        let j = (0..bp.len())
            .filter(|j| !taken.contains(j))
            .min_by(|&i, &j| {
                len(sub(*p, bp[i]))
                    .total_cmp(&len(sub(*p, bp[j])))
                    .then(i.cmp(&j))
            })
            .unwrap();
        assert!(
            len(sub(*p, bp[j])) <= tol,
            "{name}: unmatched point {p:?}, nearest {:?}",
            bp[j]
        );
        taken.insert(j);
        pmap.push(j);
    }
    let canonical = |mut v: Vec<usize>| {
        let k = v
            .iter()
            .enumerate()
            .min_by_key(|&(_, v)| v)
            .map(|(k, _)| k)
            .unwrap_or(0);
        v.rotate_left(k);
        v
    };
    let mut faces: BTreeMap<Vec<usize>, Vec<usize>> = BTreeMap::new();
    for p in 0..b.nprims() {
        let sig = canonical(
            range(b, p)
                .map(|v| b.topology.vertex_point[v] as usize)
                .collect(),
        );
        faces.entry(sig).or_default().push(p);
    }
    let mut primmap = vec![];
    let mut vmap = vec![0; a.nvertices()];
    for p in 0..a.nprims() {
        let r = range(a, p);
        let pts: Vec<_> = r
            .clone()
            .map(|v| pmap[a.topology.vertex_point[v] as usize])
            .collect();
        let key = canonical(pts.clone());
        let candidates = faces
            .get_mut(&key)
            .unwrap_or_else(|| panic!("{name}: polygon {p} outline {key:?} absent in Python"));
        let q = candidates.pop().expect("duplicate Rust polygon");
        primmap.push(q);
        let br = range(b, q);
        let k = br
            .clone()
            .position(|v| b.topology.vertex_point[v] as usize == pts[0])
            .unwrap();
        for (i, v) in r.enumerate() {
            vmap[v] = br.start + (i + k) % br.len();
        }
    }
    // Python lacks neutral provenance metadata; compare its material names and other keys exactly.
    super::oracle_material_detail::compare(name, a, b);
    for (domain, idx) in [("point", pmap), ("vertex", vmap), ("prim", primmap)] {
        let (at, bt) = (a.table(domain), b.table(domain));
        let neutral_index = domain == "prim" && super::oracle_material_detail::neutral_index_added(a, b);
        assert_eq!(
            at.keys().filter(|key| !neutral_index || key.as_str() != "material_index").collect::<Vec<_>>(),
            bt.keys().collect::<Vec<_>>(),
            "{name}: {domain} names"
        );
        for (key, aa) in at {
            if neutral_index && key == "material_index" { continue; }
            if domain == "prim" && key == "material_index" && a.detail.get("materials") != b.detail.get("materials") {
                // Reconciled neutral slots can shift numeric IDs; the resolved face name must match.
                super::oracle_material_detail::compare_material_indices(name, a, b, &idx);
                continue;
            }
            let invalid_rows = if domain == "vertex" { super::oracle_material_detail::invalid_uv_rows(a, b, key, &idx) } else { vec![false; aa.rows()] };
            attr_rows(aa, &bt[key], &idx, tol, &format!("{name}: {domain} {key}"), Some(&invalid_rows));
        }
    }
}
const RUST_ONLY: &[(&str, &str)] = &[("pushpull", "through")];
/// The Python oracle is compared against the legacy value of each Rust-only parameter.
const RUST_ONLY_LEGACY: &[(&str, &str, &str)] = &[("pushpull", "through", "off")];
fn check(c: &Value) {
    let name = c["name"].as_str().unwrap();
    let spec = super::get(c["type"].as_str().unwrap()).unwrap();
    let defaults: serde_json::Map<_, _> = spec
        .params
        .iter()
        .map(|p| (p.name.to_string(), (p.default)()))
        // Rust-only parameters (e.g. pushpull `through`) have no Python oracle.
        .filter(|(k, _)| !RUST_ONLY.contains(&(spec.name, k.as_str())))
        .collect();
    assert_eq!(Value::Object(defaults), c["defaults"], "{name} defaults");
    let mut params = c["params"].as_object().cloned().unwrap_or_default();
    for (ty, key, legacy) in RUST_ONLY_LEGACY {
        if spec.name == *ty && !params.contains_key(*key) {
            params.insert(key.to_string(), Value::from(*legacy));
        }
    }
    let p = Params::resolve(spec.params, Some(&params)).unwrap();
    let inputs: Vec<_> = c["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| vec![Arc::new(geometry(v))])
        .collect();
    let result = (spec.cook)(&inputs, &p);
    if c.get("error").is_some() {
        assert!(result.is_err(), "{name}: expected error");
        return;
    }
    let g = result.unwrap_or_else(|e| panic!("{name}: {e}"));
    g.validate().unwrap();
    let expected = geometry(&c["expected"]);
    assert_eq!(
        (g.npoints, g.nprims(), g.nvertices()),
        (expected.npoints, expected.nprims(), expected.nvertices()),
        "{name}: counts Rust vs Python"
    );
    assert_eq!(
        twins(&g).iter().all(Option::is_some),
        c["stats"]["manifold"].as_bool().unwrap(),
        "{name}: closed/manifold/winding"
    );
    let edges = crate::topo::edges(&g.topology, g.npoints);
    assert_eq!(
        edges.count.iter().filter(|&&n| n == 1).count(),
        c["stats"]["boundary_edges"].as_u64().unwrap() as usize,
        "{name}: boundary edges"
    );
    assert_eq!(
        edges.count.iter().filter(|&&n| n > 2).count(),
        c["stats"]["nonmanifold_edges"].as_u64().unwrap() as usize,
        "{name}: nonmanifold edges"
    );
    assert_eq!(
        g.npoints as i64 - edges.len() as i64 + g.nprims() as i64,
        c["stats"]["euler"].as_i64().unwrap(),
        "{name}: Euler characteristic"
    );
    assert_eq!(
        vertex_manifold(&g),
        c["stats"]["vertex_manifold"].as_bool().unwrap(),
        "{name}: manifold vertex links"
    );
    assert_eq!(
        crossings(&g),
        crossings(&expected),
        "{name}: self-crossing polygons"
    );
    assert!(
        (volume(&g) - c["stats"]["volume"].as_f64().unwrap()).abs()
            <= 2e-5 * (1. + volume(&expected).abs()),
        "{name}: volume {} != {}",
        volume(&g),
        volume(&expected)
    );
    let tol = if spec.name == "polyreduce" {
        2e-5
    } else {
        1e-5
    };
    canonical_compare(name, &g, &expected, tol);
}
fn cases(ty: &str) {
    let cases: Value = serde_json::from_str(include_str!("test_data/boolean_parity.json")).unwrap();
    let filter = std::env::var("MINIHOU_PARITY_CASE").unwrap_or_default();
    let mut failures = vec![];
    let mut count = 0;
    for c in cases.as_array().unwrap() {
        if c["type"] != ty || !c["name"].as_str().unwrap().contains(&filter) {
            continue;
        }
        count += 1;
        let result = std::panic::catch_unwind(|| check(c));
        if result.is_err() {
            failures.push(c["name"].as_str().unwrap());
        }
    }
    assert!(
        failures.is_empty(),
        "{count} {ty} cases; failed: {failures:?}"
    );
    println!("{count} {ty} Python parity cases passed");
}
#[test]
fn boolean_python_parity() {
    // Neutral slots and invalid UVs on source-less faces differ from the older Python oracle.
    cases("boolean");
}
#[test]
fn polyreduce_python_parity() {
    cases("polyreduce");
}
#[test]
fn pushpull_python_parity() {
    cases("pushpull");
}
#[test]
fn pushpull_auto_through_keeps_python_shapes() {
    // The default `through: auto` must give the same solid as Python whenever Python
    // succeeds (it may additionally drop collinear split points).
    let cases: Value = serde_json::from_str(include_str!("test_data/boolean_parity.json")).unwrap();
    let mut count = 0;
    for c in cases.as_array().unwrap() {
        if c["type"] != "pushpull" || c.get("error").is_some() {
            continue;
        }
        let name = c["name"].as_str().unwrap();
        let spec = super::get("pushpull").unwrap();
        let p = Params::resolve(spec.params, c["params"].as_object()).unwrap();
        let inputs: Vec<_> = c["inputs"].as_array().unwrap().iter().map(|v| vec![Arc::new(geometry(v))]).collect();
        let g = (spec.cook)(&inputs, &p).unwrap_or_else(|e| panic!("{name}: {e}"));
        let expected = geometry(&c["expected"]);
        assert!(twins(&g).iter().all(Option::is_some), "{name}: closed");
        let edges = crate::topo::edges(&g.topology, g.npoints);
        // Same genus; Python's rotated_5 oracle keeps a degenerate sliver (Euler 4) that
        // auto's collinear clean-up removes, so the count may only go down.
        let chi = g.npoints as i64 - edges.len() as i64 + g.nprims() as i64;
        assert!(chi <= c["stats"]["euler"].as_i64().unwrap() && chi % 2 == 0, "{name}: Euler {chi}");
        assert!((volume(&g) - volume(&expected)).abs() <= 2e-5 * (1. + volume(&expected).abs()), "{name}: volume");
        count += 1;
    }
    assert!(count >= 10);
}
#[test]
fn boolean_fracture_python_parity() {
    // Fracture may add empty neutral slots; named slots and each face's material still match Python.
    cases("boolean_fracture");
}
#[test]
fn adaptive_orient3d_exact_sign() {
    let a = [0., 0., 0.];
    let b = [1., 0., 0.];
    let c = [0., 1., 0.];
    assert!(k::orient3d(a, b, c, [0.2, 0.3, 1e-30]) < 0.);
    assert_eq!(k::orient3d(a, b, c, [0.2, 0.3, 0.]), 0.);
    // Integers below 2^53: exact determinant = 1, ordinary products cancel.
    let n = 134217728.;
    assert_eq!(
        k::orient3d([n, n - 1., 0.], [n + 1., n, 0.], [0., 0., 1.], [0., 0., 0.]),
        1.
    );
    // Cancellation under permutations and translations: compare signs to an
    // independent i128 determinant, which is exact for these integer inputs.
    for n in [134217728i128, 268435456, 536870912] {
        let p = [[n, n - 1, 0], [n + 1, n, 0], [0, 0, 1], [0, 0, 0]];
        for shift in [0i128, 1073741824] {
            for order in [[0, 1, 2, 3], [1, 0, 2, 3], [2, 1, 0, 3], [3, 1, 2, 0]] {
                let q = order.map(|i| p[i].map(|v| v + shift));
                let d: Vec<_> = q[..3]
                    .iter()
                    .map(|v| std::array::from_fn::<_, 3, _>(|i| v[i] - q[3][i]))
                    .collect();
                let det = d[0][0] * (d[1][1] * d[2][2] - d[1][2] * d[2][1])
                    - d[0][1] * (d[1][0] * d[2][2] - d[1][2] * d[2][0])
                    + d[0][2] * (d[1][0] * d[2][1] - d[1][1] * d[2][0]);
                let f = q.map(|p| p.map(|v| v as f64));
                assert_eq!(
                    k::orient3d(f[0], f[1], f[2], f[3]).signum(),
                    (det as f64).signum()
                );
            }
        }
    }
}
