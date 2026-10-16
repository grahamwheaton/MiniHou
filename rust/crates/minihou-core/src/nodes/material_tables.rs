//! Reconcile slot tables before combining geometries with different sources.
use std::sync::Arc;

use serde_json::{json, Value};

use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry}};

fn names(g: &Geometry) -> Vec<String> {
    g.detail.get("materials").and_then(Value::as_array)
        .map(|a| a.iter().map(|v| v.as_str().unwrap_or("").to_owned()).collect())
        .unwrap_or_default()
}

fn bindings(g: &Geometry) -> Option<Vec<Option<String>>> {
    g.detail.get("material_bindings")?.get("slots")?.as_array().map(|a| a.iter()
        .map(|v| v.as_str().map(str::to_owned)).collect())
}

/// Identity is the immutable binding when present, otherwise a legacy name and
/// its USD source. Empty slots have no identity and are kept separate.
pub(super) fn reconcile(geos: &[Geometry]) -> Result<Vec<Geometry>> {
    let mut result = geos.to_vec();
    let mut combined: Vec<(String, Option<String>, Option<Value>)> = Vec::new();
    let has_bindings = geos.iter().any(|g| g.detail.contains_key("material_bindings"));
    let has_tables = geos.iter().any(|g| g.detail.contains_key("materials") || g.detail.contains_key("material_bindings"));
    if !has_tables { return Ok(result); }
    for (original, g) in geos.iter().zip(&mut result) {
        let list = names(original);
        let ids = bindings(original);
        let highest = [original.prim.get("material_index"), original.point.get("material_id")]
            .into_iter().flatten().filter_map(|a| a.as_i32())
            .flat_map(|v| v.iter().copied()).filter(|&i| i >= 0).max().map_or(0, |i| i as usize + 1);
        let count = list.len().max(ids.as_ref().map_or(0, Vec::len)).max(highest).max(1);
        let mut lut = Vec::with_capacity(count);
        for i in 0..count {
            let name = list.get(i).cloned().unwrap_or_default();
            let id = ids.as_ref().and_then(|a| a.get(i)).cloned().flatten();
            let usd = original.detail.get("usd_materials").and_then(Value::as_object).and_then(|m| m.get(&name)).cloned();
            let found = if let Some(ref id) = id {
                combined.iter().position(|(_, other, _)| other.as_ref() == Some(id))
            } else if !name.is_empty() {
                combined.iter().position(|(n, other, source)| other.is_none() && *n == name && *source == usd)
            } else { None };
            let slot = found.unwrap_or_else(|| { combined.push((name, id, usd)); combined.len() - 1 });
            lut.push(slot as i32);
        }
        let nprims = g.nprims();
        for (table, key, n) in [(&mut g.prim, "material_index", nprims), (&mut g.point, "material_id", g.npoints)] {
            if key == "material_id" && !table.contains_key(key) { continue; }
            let old = table.get(key).map(|a| match &a.data {
                AttrData::I32(v) if a.width == 1 => Ok(v.clone()),
                _ => Err(Error::new(format!("{key} must be scalar int32"))),
            }).transpose()?.unwrap_or_else(|| vec![0; n]);
            let mut mapped = Vec::with_capacity(n);
            for index in old {
                let i = if index < 0 { lut.len() as i64 + index as i64 } else { index as i64 };
                mapped.push(*lut.get(i as usize).ok_or_else(|| Error::new("material index out of range"))?);
            }
            table.insert(key.into(), Arc::new(Attr::i32(1, mapped)));
        }
    }
    let merged_names: Vec<_> = combined.iter().map(|(name, _, _)| name.clone()).collect();
    let merged_ids: Vec<_> = combined.iter().map(|(_, id, _)| id.clone()).collect();
    for g in &mut result {
        g.detail.insert("materials".into(), json!(merged_names));
        if has_bindings { g.detail.insert("material_bindings".into(), json!({"version":1,"slots":merged_ids})); }
    }
    Ok(result)
}

pub(super) fn merge_usd_sources(geos: &[Geometry], out: &mut Geometry) -> Result<()> {
    let mut sources = serde_json::Map::new();
    for g in geos {
        if let Some(map) = g.detail.get("usd_materials").and_then(Value::as_object) {
            for (name, value) in map {
                if sources.get(name).is_some_and(|old| old != value) {
                    return Err(Error::new(format!("conflicting USD material source for {name:?}")));
                }
                sources.insert(name.clone(), value.clone());
            }
        }
    }
    if !sources.is_empty() { out.detail.insert("usd_materials".into(), sources.into()); }
    Ok(())
}

/// Mark source-less faces after their rows have been appended to the geometry.
pub(super) fn neutral_new_faces(g: &mut Geometry, first_prim: usize, first_corner: usize) {
    if first_prim >= g.nprims() { return; }
    let mut names = names(g);
    let mut ids = bindings(g).unwrap_or_else(|| vec![None; names.len()]);
    let neutral = ids.iter().position(Option::is_none).unwrap_or_else(|| {
        names.push(String::new());
        ids.push(None);
        ids.len() - 1
    });
    g.detail.insert("materials".into(), json!(names));
    g.detail.insert("material_bindings".into(), json!({"version":1,"slots":ids}));
    let mut slots = g.prim.get("material_index").and_then(|a| a.as_i32()).map(Vec::from)
        .unwrap_or_else(|| vec![0; g.nprims()]);
    slots[first_prim..].fill(neutral as i32);
    g.prim.insert("material_index".into(), Arc::new(Attr::i32(1, slots)));
    for (name, attr) in &mut g.vertex {
        if matches!(name.as_str(), "N" | "Cd" | "P") || !matches!(attr.width, 2 | 3) { continue; }
        if let Some(values) = attr.as_f32() {
            let mut values = values.to_vec();
            values[first_corner * attr.width..].fill(f32::NAN);
            *attr = Arc::new(Attr::f32(attr.width, values));
        }
    }
}
