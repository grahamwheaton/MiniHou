//! Compact, owned geometry data for the editor's Geometry panel.
use crate::ui::{AttrInfo, GeoSummary};
use minihou_core::geometry::{AttrData, AttrTable, Geometry};
use serde_json::Value;

fn finite_range(values: impl IntoIterator<Item = f64>) -> (Option<f64>, Option<f64>) {
    let mut range: Option<(f64, f64)> = None;
    for value in values.into_iter().filter(|v| v.is_finite()) {
        range = Some(range.map_or((value, value), |(lo, hi)| (lo.min(value), hi.max(value))));
    }
    range.map_or((None, None), |(lo, hi)| (Some(lo), Some(hi)))
}

fn attributes(table: &AttrTable) -> Vec<AttrInfo> {
    table.iter().map(|(name, attr)| {
        let (min, max) = match &attr.data {
            AttrData::F32(data) => finite_range(data.iter().map(|&v| f64::from(v))),
            AttrData::I32(data) => finite_range(data.iter().map(|&v| f64::from(v))),
        };
        AttrInfo { name: name.clone(), kind: attr.dtype().into(), components: attr.width, min, max }
    }).collect()
}

fn json_numbers(value: &Value, visit: &mut impl FnMut(f64)) {
    match value {
        Value::Number(n) => { if let Some(v) = n.as_f64() { visit(v); } }
        Value::Array(items) => { for item in items { json_numbers(item, visit); } }
        _ => {}
    }
}

fn detail_attributes(g: &Geometry) -> Vec<AttrInfo> {
    let mut attrs: Vec<_> = g.detail.iter().map(|(name, value)| {
        let kind = match value {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(n) if n.is_i64() || n.is_u64() => "int64",
            Value::Number(_) => "float64",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        };
        let components = match value { Value::Array(items) => items.len(), _ => 1 };
        let mut range = None::<(f64, f64)>;
        json_numbers(value, &mut |v| {
            if v.is_finite() { range = Some(range.map_or((v, v), |(lo, hi)| (lo.min(v), hi.max(v)))); }
        });
        AttrInfo { name: name.clone(), kind: kind.into(), components,
            min: range.map(|r| r.0), max: range.map(|r| r.1) }
    }).collect();
    attrs.sort_by(|a, b| a.name.cmp(&b.name));
    attrs
}

fn bounds(g: &Geometry) -> Option<([f32; 3], [f32; 3])> {
    let p = g.point.get("P")?.as_f32()?;
    if g.npoints == 0 || p.len() != g.npoints * 3 { return None; }
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for xyz in p.as_chunks::<3>().0 {
        for axis in 0..3 {
            let v = xyz[axis];
            if !v.is_finite() { return None; }
            lo[axis] = lo[axis].min(v);
            hi[axis] = hi[axis].max(v);
        }
    }
    Some((lo, hi))
}

pub fn summarize(node: &str, g: &Geometry) -> GeoSummary {
    GeoSummary { node: node.into(), points: g.npoints, vertices: g.nvertices(), prims: g.nprims(),
        bounds: bounds(g), point_attrs: attributes(&g.point), vertex_attrs: attributes(&g.vertex),
        prim_attrs: attributes(&g.prim), detail_attrs: detail_attributes(g) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minihou_core::geometry::Attr;
    use serde_json::json;
    use std::sync::Arc;

    #[test]
    fn triangle_counts_bounds_and_all_domains() {
        let mut g = Geometry::from_arrays(vec![-1., 2., 3., 4., -5., 6., 2., 1., -2.], vec![0, 1, 2], vec![0, 3], None).unwrap();
        g.point.insert("weight".into(), Arc::new(Attr::f32(1, vec![0.25, 1.5, -2.])));
        g.vertex.insert("uv".into(), Arc::new(Attr::f32(2, vec![0., 1., 2., 3., 4., 5.])));
        g.prim.insert("id".into(), Arc::new(Attr::i32(1, vec![7])));
        g.detail.insert("scale".into(), json!([3, -2, 9]));
        g.detail.insert("label".into(), json!("mesh"));
        let s = summarize("box1", &g);
        assert_eq!((s.node.as_str(), s.points, s.vertices, s.prims), ("box1", 3, 3, 1));
        assert_eq!(s.bounds, Some(([-1., -5., -2.], [4., 2., 6.])));
        assert_eq!(s.point_attrs.iter().find(|a| a.name == "weight").map(|a| (a.kind.as_str(), a.components, a.min, a.max)),
            Some(("float32", 1, Some(-2.), Some(1.5))));
        assert_eq!((s.vertex_attrs[0].components, s.vertex_attrs[0].max), (2, Some(5.)));
        assert_eq!((s.prim_attrs[0].kind.as_str(), s.prim_attrs[0].min), ("int32", Some(7.)));
        assert_eq!(s.detail_attrs[0].min, None);
        assert_eq!((s.detail_attrs[1].components, s.detail_attrs[1].min, s.detail_attrs[1].max), (3, Some(-2.), Some(9.)));
    }

    #[test]
    fn empty_geometry_has_no_bounds_or_ranges() {
        let g = Geometry::points_only(vec![]).unwrap();
        let s = summarize("empty", &g);
        assert_eq!((s.points, s.vertices, s.prims), (0, 0, 0));
        assert_eq!(s.bounds, None);
        assert_eq!((s.point_attrs[0].min, s.point_attrs[0].max), (None, None));
    }

    #[test]
    fn non_finite_values_do_not_poison_ranges() {
        let mut g = Geometry::points_only(vec![0., 0., 0.]).unwrap();
        g.point.insert("x".into(), Arc::new(Attr::f32(2, vec![f32::NAN, 4.])));
        let s = summarize("p", &g);
        assert_eq!((s.point_attrs[1].min, s.point_attrs[1].max), (Some(4.), Some(4.)));
    }
}
