use crate::geometry::Geometry;
use serde_json::{json, Value};

pub(super) fn compare(name: &str, rust: &Geometry, python: &Geometry) {
    // The Python oracle predates neutral provenance slots and binding IDs.
    for (key, value) in &python.detail {
        if key == "materials" {
            let expected = value.as_array().expect("Python materials array");
            let actual = rust
                .detail
                .get(key)
                .and_then(Value::as_array)
                .expect("Rust materials array");
            assert!(
                actual.iter().all(Value::is_string),
                "{name}: material names"
            );
            let mut next = 0;
            for slot in actual {
                if expected.get(next) == Some(slot) {
                    next += 1;
                } else {
                    assert_eq!(slot, "", "{name}: unexpected named material slot");
                }
            }
            assert_eq!(next, expected.len(), "{name}: missing Python material slot");
            continue;
        }
        assert_eq!(rust.detail.get(key), Some(value), "{name}: detail {key}");
    }
    let mut extra = rust.detail.clone();
    for key in python.detail.keys() {
        extra.remove(key);
    }
    let bindings = extra.remove("material_bindings");
    let materials = extra.remove("materials");
    assert!(extra.is_empty(), "{name}: unexpected detail keys {extra:?}");
    if let Some(names) = materials {
        assert!(
            python.detail.get("materials").is_none(),
            "{name}: unexpected material override"
        );
        let slots = names.as_array().expect("materials array");
        assert!(
            slots.iter().all(|v| v == ""),
            "{name}: non-neutral material {names}"
        );
        assert_eq!(
            bindings,
            Some(json!({"version":1,"slots":vec![Value::Null; slots.len()]})),
            "{name}: neutral bindings"
        );
        if let Some(indices) = rust.prim.get("material_index").and_then(|a| a.as_i32()) {
            assert!(
                indices
                    .iter()
                    .all(|&i| i >= 0 && (i as usize) < slots.len()),
                "{name}: neutral slot index"
            );
        }
    } else if let Some(value) = bindings {
        let names = rust
            .detail
            .get("materials")
            .and_then(Value::as_array)
            .expect("binding names");
        assert_eq!(value["version"], 1, "{name}: binding version");
        let slots = value["slots"].as_array().expect("binding slots");
        assert_eq!(slots.len(), names.len(), "{name}: binding count");
        assert!(
            slots.iter().all(|v| v.is_null() || v.is_string()),
            "{name}: binding IDs"
        );
        for (n, id) in names.iter().zip(slots) {
            if n == "" {
                assert!(id.is_null(), "{name}: neutral slot bound to material");
            }
        }
    }
}

pub(super) fn invalid_uv_rows(
    rust: &Geometry,
    python: &Geometry,
    key: &str,
    map: &[usize],
) -> Vec<bool> {
    let mut allowed = vec![false; rust.nvertices()];
    let Some(a) = rust.vertex.get(key) else {
        return allowed;
    };
    let Some(b) = python.vertex.get(key) else {
        return allowed;
    };
    if !matches!(a.width, 2 | 3)
        || a.width != b.width
        || b.as_f32().is_none()
        || map.len() != rust.nvertices()
        || matches!(key, "N" | "Cd" | "P")
    {
        return allowed;
    }
    let Some(av) = a.as_f32() else {
        return allowed;
    };
    let indices = rust.prim.get("material_index").and_then(|a| a.as_i32());
    let names = rust.detail.get("materials").and_then(Value::as_array);
    let bindings = rust
        .detail
        .get("material_bindings")
        .and_then(|v| v["slots"].as_array());
    for p in 0..rust.nprims() {
        let first = rust.topology.prim_offsets[p] as usize;
        let end = rust.topology.prim_offsets[p + 1] as usize;
        let neutral =
            if let (Some(indices), Some(names), Some(bindings)) = (indices, names, bindings) {
                let slot = indices[p] as usize;
                names.get(slot).is_some_and(|v| v == "")
                    && bindings.get(slot).is_some_and(Value::is_null)
            } else {
                // Legacy booleans without material tables still mark untraceable faces with NaN UVs.
                indices.is_none()
                    && names.is_none()
                    && bindings.is_none()
                    && !python.detail.contains_key("materials")
            };
        // Only whole invalid Rust UV faces qualify; the caller still checks each oracle zero.
        if neutral && (first..end).all(|i| (0..a.width).all(|c| av[i * a.width + c].is_nan())) {
            allowed[first..end].fill(true);
        }
    }
    allowed
}

pub(super) fn neutral_index_added(rust: &Geometry, python: &Geometry) -> bool {
    if python.prim.contains_key("material_index") {
        return false;
    }
    let (Some(names), Some(bindings), Some(indices)) = (
        rust.detail.get("materials").and_then(Value::as_array),
        rust.detail
            .get("material_bindings")
            .and_then(|v| v["slots"].as_array()),
        rust.prim.get("material_index").and_then(|a| a.as_i32()),
    ) else {
        return false;
    };
    names.len() == bindings.len()
        && names.iter().all(|v| v == "")
        && bindings.iter().all(Value::is_null)
        && indices
            .iter()
            .all(|&i| i >= 0 && (i as usize) < names.len())
}

pub(super) fn compare_material_indices(
    name: &str,
    rust: &Geometry,
    python: &Geometry,
    map: &[usize],
) {
    let a = rust.prim["material_index"]
        .as_i32()
        .expect("Rust material indices");
    let b = python.prim["material_index"]
        .as_i32()
        .expect("Python material indices");
    let an = rust.detail["materials"]
        .as_array()
        .expect("Rust material names");
    let bn = python.detail["materials"]
        .as_array()
        .expect("Python material names");
    for (p, &q) in map.iter().enumerate() {
        let actual = usize::try_from(a[p])
            .ok()
            .and_then(|i| an.get(i))
            .expect("Rust slot in range");
        let expected = usize::try_from(b[q])
            .ok()
            .and_then(|i| bn.get(i))
            .expect("Python slot in range");
        assert_eq!(actual, expected, "{name}: face {p} material slot");
    }
}
