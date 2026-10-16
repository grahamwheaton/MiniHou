//! Geometry Spreadsheet data model, column flattening, and paging logic.

use minihou_core::geometry::{AttrData, Geometry};
use serde_json::Value;

/// The element domains for spreadsheet inspection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ElementDomain {
    Points,
    Vertices,
    Primitives,
    Detail,
}

impl ElementDomain {
    pub const ALL: [ElementDomain; 4] = [
        ElementDomain::Points,
        ElementDomain::Vertices,
        ElementDomain::Primitives,
        ElementDomain::Detail,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            ElementDomain::Points => "point",
            ElementDomain::Vertices => "vertex",
            ElementDomain::Primitives => "prim",
            ElementDomain::Detail => "detail",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ElementDomain::Points => "Points",
            ElementDomain::Vertices => "Vertices",
            ElementDomain::Primitives => "Primitives",
            ElementDomain::Detail => "Detail",
        }
    }

    pub fn row_count(&self, g: &Geometry) -> usize {
        match self {
            ElementDomain::Points => g.npoints,
            ElementDomain::Vertices => g.nvertices(),
            ElementDomain::Primitives => g.nprims(),
            ElementDomain::Detail => if g.detail.is_empty() { 0 } else { 1 },
        }
    }
}

/// A flattened spreadsheet column (one per component of multi-component attributes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpreadsheetColumn {
    pub name: String,
    pub attr_name: String,
    pub comp_index: usize,
    pub total_comps: usize,
    pub dtype: &'static str,
    pub is_topology: bool,
}

/// Split an attribute into component column names according to Houdini conventions:
/// - Cd / Color / Colour: `.r`, `.g`, `.b`, `.a`
/// - uv: `.u`, `.v`, `.w`
/// - Vectors (P, N, v, etc.): `.x`, `.y`, `.z`, `.w`
/// - Single component: bare attribute name
pub fn split_component_names(attr_name: &str, width: usize) -> Vec<String> {
    if width <= 1 {
        return vec![attr_name.to_owned()];
    }
    let lower = attr_name.to_ascii_lowercase();
    if lower == "cd" || lower == "color" || lower == "colour" {
        let suffixes = ["r", "g", "b", "a"];
        (0..width)
            .map(|i| {
                if i < suffixes.len() {
                    format!("{attr_name}.{}", suffixes[i])
                } else {
                    format!("{attr_name}.{i}")
                }
            })
            .collect()
    } else if lower == "uv" {
        let suffixes = ["u", "v", "w"];
        (0..width)
            .map(|i| {
                if i < suffixes.len() {
                    format!("{attr_name}.{}", suffixes[i])
                } else {
                    format!("{attr_name}.{i}")
                }
            })
            .collect()
    } else {
        let suffixes = ["x", "y", "z", "w"];
        (0..width)
            .map(|i| {
                if i < suffixes.len() {
                    format!("{attr_name}.{}", suffixes[i])
                } else {
                    format!("{attr_name}.{i}")
                }
            })
            .collect()
    }
}

/// Flatten all columns of the selected domain with split components.
pub fn flatten_columns(g: &Geometry, domain: ElementDomain) -> Vec<SpreadsheetColumn> {
    let mut cols = Vec::new();
    match domain {
        ElementDomain::Points => {
            // Position P always comes first if present
            if let Some(p_attr) = g.point.get("P") {
                let names = split_component_names("P", p_attr.width);
                for (comp_idx, name) in names.into_iter().enumerate() {
                    cols.push(SpreadsheetColumn {
                        name,
                        attr_name: "P".into(),
                        comp_index: comp_idx,
                        total_comps: p_attr.width,
                        dtype: p_attr.dtype(),
                        is_topology: false,
                    });
                }
            }
            // Other point attributes in alphabetical order
            for (attr_name, attr) in &g.point {
                if attr_name == "P" {
                    continue;
                }
                let names = split_component_names(attr_name, attr.width);
                for (comp_idx, name) in names.into_iter().enumerate() {
                    cols.push(SpreadsheetColumn {
                        name,
                        attr_name: attr_name.clone(),
                        comp_index: comp_idx,
                        total_comps: attr.width,
                        dtype: attr.dtype(),
                        is_topology: false,
                    });
                }
            }
        }
        ElementDomain::Vertices => {
            // Built-in topology columns: point index and prim index
            cols.push(SpreadsheetColumn {
                name: "point".into(),
                attr_name: "point".into(),
                comp_index: 0,
                total_comps: 1,
                dtype: "int32",
                is_topology: true,
            });
            cols.push(SpreadsheetColumn {
                name: "prim".into(),
                attr_name: "prim".into(),
                comp_index: 0,
                total_comps: 1,
                dtype: "int32",
                is_topology: true,
            });
            // Vertex attributes in alphabetical order
            for (attr_name, attr) in &g.vertex {
                let names = split_component_names(attr_name, attr.width);
                for (comp_idx, name) in names.into_iter().enumerate() {
                    cols.push(SpreadsheetColumn {
                        name,
                        attr_name: attr_name.clone(),
                        comp_index: comp_idx,
                        total_comps: attr.width,
                        dtype: attr.dtype(),
                        is_topology: false,
                    });
                }
            }
        }
        ElementDomain::Primitives => {
            // Built-in topology columns: vertex count and closed flag
            cols.push(SpreadsheetColumn {
                name: "size".into(),
                attr_name: "size".into(),
                comp_index: 0,
                total_comps: 1,
                dtype: "int32",
                is_topology: true,
            });
            cols.push(SpreadsheetColumn {
                name: "closed".into(),
                attr_name: "closed".into(),
                comp_index: 0,
                total_comps: 1,
                dtype: "int32",
                is_topology: true,
            });
            // Primitive attributes in alphabetical order
            for (attr_name, attr) in &g.prim {
                let names = split_component_names(attr_name, attr.width);
                for (comp_idx, name) in names.into_iter().enumerate() {
                    cols.push(SpreadsheetColumn {
                        name,
                        attr_name: attr_name.clone(),
                        comp_index: comp_idx,
                        total_comps: attr.width,
                        dtype: attr.dtype(),
                        is_topology: false,
                    });
                }
            }
        }
        ElementDomain::Detail => {
            // Detail attributes in sorted key order
            let mut keys: Vec<&String> = g.detail.keys().collect();
            keys.sort();
            for attr_name in keys {
                let val = &g.detail[attr_name];
                match val {
                    Value::Array(items) => {
                        let names = split_component_names(attr_name, items.len());
                        for (comp_idx, name) in names.into_iter().enumerate() {
                            let item_dtype = match items.get(comp_idx) {
                                Some(Value::Number(n)) if n.is_i64() || n.is_u64() => "int64",
                                Some(Value::Number(_)) => "float64",
                                Some(Value::Bool(_)) => "bool",
                                Some(Value::String(_)) => "string",
                                _ => "json",
                            };
                            cols.push(SpreadsheetColumn {
                                name,
                                attr_name: attr_name.clone(),
                                comp_index: comp_idx,
                                total_comps: items.len(),
                                dtype: item_dtype,
                                is_topology: false,
                            });
                        }
                    }
                    Value::Number(n) => {
                        cols.push(SpreadsheetColumn {
                            name: attr_name.clone(),
                            attr_name: attr_name.clone(),
                            comp_index: 0,
                            total_comps: 1,
                            dtype: if n.is_i64() || n.is_u64() { "int64" } else { "float64" },
                            is_topology: false,
                        });
                    }
                    Value::Bool(_) => {
                        cols.push(SpreadsheetColumn {
                            name: attr_name.clone(),
                            attr_name: attr_name.clone(),
                            comp_index: 0,
                            total_comps: 1,
                            dtype: "bool",
                            is_topology: false,
                        });
                    }
                    Value::String(_) => {
                        cols.push(SpreadsheetColumn {
                            name: attr_name.clone(),
                            attr_name: attr_name.clone(),
                            comp_index: 0,
                            total_comps: 1,
                            dtype: "string",
                            is_topology: false,
                        });
                    }
                    _ => {
                        cols.push(SpreadsheetColumn {
                            name: attr_name.clone(),
                            attr_name: attr_name.clone(),
                            comp_index: 0,
                            total_comps: 1,
                            dtype: "json",
                            is_topology: false,
                        });
                    }
                }
            }
        }
    }
    cols
}

/// Find which primitive a given vertex belongs to from CSR `prim_offsets`.
pub fn prim_of_vertex(offsets: &[i32], vertex_idx: usize) -> Option<usize> {
    if offsets.len() < 2 {
        return None;
    }
    let v = vertex_idx as i32;
    if v < 0 || v >= *offsets.last().unwrap_or(&0) {
        return None;
    }
    match offsets.binary_search(&v) {
        Ok(idx) => {
            let mut p = idx;
            while p + 1 < offsets.len() && offsets[p + 1] <= v {
                p += 1;
            }
            if p + 1 < offsets.len() && offsets[p] <= v && v < offsets[p + 1] {
                Some(p)
            } else if idx > 0 && offsets[idx - 1] <= v && v < offsets[idx] {
                Some(idx - 1)
            } else {
                Some(p)
            }
        }
        Err(idx) => {
            if idx > 0 && idx < offsets.len() {
                Some(idx - 1)
            } else {
                None
            }
        }
    }
}

/// Format float values cleanly with consistent precision.
pub fn format_float(x: f32) -> String {
    if x.is_nan() {
        "NaN".into()
    } else if x.is_infinite() {
        if x > 0.0 { "inf".into() } else { "-inf".into() }
    } else if x.abs() < 1e-7 {
        "0.000".into()
    } else {
        format!("{x:.3}")
    }
}

/// Format float64 values cleanly.
pub fn format_float64(x: f64) -> String {
    if x.is_nan() {
        "NaN".into()
    } else if x.is_infinite() {
        if x > 0.0 { "inf".into() } else { "-inf".into() }
    } else if x.abs() < 1e-7 {
        "0.000".into()
    } else {
        format!("{x:.3}")
    }
}

/// Get the string representation of a specific cell without allocating for other rows.
pub fn format_cell(g: &Geometry, domain: ElementDomain, row: usize, col: &SpreadsheetColumn) -> String {
    if col.is_topology {
        match domain {
            ElementDomain::Vertices => {
                if col.attr_name == "point" {
                    return g.topology.vertex_point.get(row).map_or_else(|| "—".into(), |p| p.to_string());
                } else if col.attr_name == "prim" {
                    return prim_of_vertex(&g.topology.prim_offsets, row).map_or_else(|| "—".into(), |p| p.to_string());
                }
            }
            ElementDomain::Primitives => {
                if col.attr_name == "size" {
                    let p0 = g.topology.prim_offsets.get(row).copied().unwrap_or(0);
                    let p1 = g.topology.prim_offsets.get(row + 1).copied().unwrap_or(0);
                    return (p1 - p0).to_string();
                } else if col.attr_name == "closed" {
                    return if g.topology.is_closed(row) { "1".into() } else { "0".into() };
                }
            }
            _ => {}
        }
    }

    match domain {
        ElementDomain::Points => {
            if let Some(attr) = g.point.get(&col.attr_name) {
                let idx = row * attr.width + col.comp_index;
                match &attr.data {
                    AttrData::F32(v) => v.get(idx).map_or_else(|| "—".into(), |&x| format_float(x)),
                    AttrData::I32(v) => v.get(idx).map_or_else(|| "—".into(), |&x| x.to_string()),
                }
            } else {
                "—".into()
            }
        }
        ElementDomain::Vertices => {
            if let Some(attr) = g.vertex.get(&col.attr_name) {
                let idx = row * attr.width + col.comp_index;
                match &attr.data {
                    AttrData::F32(v) => v.get(idx).map_or_else(|| "—".into(), |&x| format_float(x)),
                    AttrData::I32(v) => v.get(idx).map_or_else(|| "—".into(), |&x| x.to_string()),
                }
            } else {
                "—".into()
            }
        }
        ElementDomain::Primitives => {
            if let Some(attr) = g.prim.get(&col.attr_name) {
                let idx = row * attr.width + col.comp_index;
                match &attr.data {
                    AttrData::F32(v) => v.get(idx).map_or_else(|| "—".into(), |&x| format_float(x)),
                    AttrData::I32(v) => v.get(idx).map_or_else(|| "—".into(), |&x| x.to_string()),
                }
            } else {
                "—".into()
            }
        }
        ElementDomain::Detail => {
            if let Some(val) = g.detail.get(&col.attr_name) {
                match val {
                    Value::Array(items) => items.get(col.comp_index).map_or_else(|| "—".into(), format_json_scalar),
                    Value::Number(n) => n.as_f64().map_or_else(|| n.to_string(), format_float64),
                    Value::Bool(b) => b.to_string(),
                    Value::String(s) => s.clone(),
                    Value::Null => "null".into(),
                    _ => val.to_string(),
                }
            } else {
                "—".into()
            }
        }
    }
}

fn format_json_scalar(v: &Value) -> String {
    match v {
        Value::Number(n) => n.as_f64().map_or_else(|| n.to_string(), format_float64),
        Value::Bool(b) => b.to_string(),
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        _ => v.to_string(),
    }
}

/// Format an entire row as tab-separated values (TSV) for clipboard copy.
pub fn format_row_tsv(g: &Geometry, domain: ElementDomain, row: usize, cols: &[SpreadsheetColumn]) -> String {
    let mut parts = Vec::with_capacity(cols.len() + 1);
    parts.push(row.to_string());
    for col in cols {
        parts.push(format_cell(g, domain, row, col));
    }
    parts.join("\t")
}

/// Pure paging calculations.
pub fn page_row_range(total_rows: usize, page: usize, page_size: usize) -> std::ops::Range<usize> {
    if page_size == 0 || total_rows == 0 {
        return 0..total_rows;
    }
    let start = (page * page_size).min(total_rows);
    let end = (start + page_size).min(total_rows);
    start..end
}

pub fn total_pages(total_rows: usize, page_size: usize) -> usize {
    if page_size == 0 || total_rows == 0 {
        1
    } else {
        total_rows.div_ceil(page_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minihou_core::geometry::Attr;
    use serde_json::json;
    use std::sync::Arc;

    #[test]
    fn geo_info_point_columns_split_and_p_first() {
        let mut g = Geometry::from_arrays(
            vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            vec![0, 1],
            vec![0, 2],
            None,
        ).unwrap();
        g.point.insert("Cd".into(), Arc::new(Attr::f32(3, vec![1.0, 0.5, 0.25, 0.0, 0.1, 0.2])));
        g.point.insert("weight".into(), Arc::new(Attr::f32(1, vec![0.75, 0.25])));

        let cols = flatten_columns(&g, ElementDomain::Points);
        let names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
        // P components come first, followed by Cd.r, Cd.g, Cd.b, then weight
        assert_eq!(names, vec!["P.x", "P.y", "P.z", "Cd.r", "Cd.g", "Cd.b", "weight"]);

        assert_eq!(format_cell(&g, ElementDomain::Points, 0, &cols[0]), "1.000");
        assert_eq!(format_cell(&g, ElementDomain::Points, 0, &cols[1]), "2.000");
        assert_eq!(format_cell(&g, ElementDomain::Points, 0, &cols[2]), "3.000");
        assert_eq!(format_cell(&g, ElementDomain::Points, 0, &cols[3]), "1.000");
        assert_eq!(format_cell(&g, ElementDomain::Points, 0, &cols[4]), "0.500");
        assert_eq!(format_cell(&g, ElementDomain::Points, 0, &cols[5]), "0.250");
        assert_eq!(format_cell(&g, ElementDomain::Points, 0, &cols[6]), "0.750");

        assert_eq!(format_cell(&g, ElementDomain::Points, 1, &cols[0]), "4.000");
        assert_eq!(format_cell(&g, ElementDomain::Points, 1, &cols[6]), "0.250");
    }

    #[test]
    fn geo_info_vertex_built_in_and_custom_columns() {
        let mut g = Geometry::from_arrays(
            vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            vec![0, 1, 2],
            vec![0, 3],
            None,
        ).unwrap();
        g.vertex.insert("uv".into(), Arc::new(Attr::f32(2, vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6])));

        let cols = flatten_columns(&g, ElementDomain::Vertices);
        let names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["point", "prim", "uv.u", "uv.v"]);

        assert_eq!(format_cell(&g, ElementDomain::Vertices, 0, &cols[0]), "0");
        assert_eq!(format_cell(&g, ElementDomain::Vertices, 0, &cols[1]), "0");
        assert_eq!(format_cell(&g, ElementDomain::Vertices, 0, &cols[2]), "0.100");
        assert_eq!(format_cell(&g, ElementDomain::Vertices, 0, &cols[3]), "0.200");

        assert_eq!(format_cell(&g, ElementDomain::Vertices, 1, &cols[0]), "1");
        assert_eq!(format_cell(&g, ElementDomain::Vertices, 2, &cols[0]), "2");
    }

    #[test]
    fn geo_info_primitive_topology_and_attributes() {
        let mut g = Geometry::from_arrays(
            vec![
                0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0,
                2.0, 0.0, 0.0, 3.0, 0.0, 0.0, 3.0, 1.0, 0.0, 2.0, 1.0, 0.0,
            ],
            vec![0, 1, 2, 3, 4, 5, 6],
            vec![0, 3, 7],
            Some(vec![true, false]),
        ).unwrap();
        g.prim.insert("mat_id".into(), Arc::new(Attr::i32(1, vec![10, 20])));

        let cols = flatten_columns(&g, ElementDomain::Primitives);
        let names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["size", "closed", "mat_id"]);

        // Prim 0: triangle, closed
        assert_eq!(format_cell(&g, ElementDomain::Primitives, 0, &cols[0]), "3");
        assert_eq!(format_cell(&g, ElementDomain::Primitives, 0, &cols[1]), "1");
        assert_eq!(format_cell(&g, ElementDomain::Primitives, 0, &cols[2]), "10");

        // Prim 1: quad, open curve
        assert_eq!(format_cell(&g, ElementDomain::Primitives, 1, &cols[0]), "4");
        assert_eq!(format_cell(&g, ElementDomain::Primitives, 1, &cols[1]), "0");
        assert_eq!(format_cell(&g, ElementDomain::Primitives, 1, &cols[2]), "20");
    }

    #[test]
    fn geo_info_detail_columns_and_formatting() {
        let mut g = Geometry::from_arrays(
            vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            vec![0, 1, 2],
            vec![0, 3],
            None,
        ).unwrap();
        g.detail.insert("scale".into(), json!([2.5, 3.5, 4.5]));
        g.detail.insert("author".into(), json!("minihou"));

        assert_eq!(ElementDomain::Detail.row_count(&g), 1);
        let cols = flatten_columns(&g, ElementDomain::Detail);
        let names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["author", "scale.x", "scale.y", "scale.z"]);

        assert_eq!(format_cell(&g, ElementDomain::Detail, 0, &cols[0]), "minihou");
        assert_eq!(format_cell(&g, ElementDomain::Detail, 0, &cols[1]), "2.500");
        assert_eq!(format_cell(&g, ElementDomain::Detail, 0, &cols[2]), "3.500");
        assert_eq!(format_cell(&g, ElementDomain::Detail, 0, &cols[3]), "4.500");
    }

    #[test]
    fn geo_info_paging_row_ranges() {
        assert_eq!(page_row_range(1250, 0, 500), 0..500);
        assert_eq!(page_row_range(1250, 1, 500), 500..1000);
        assert_eq!(page_row_range(1250, 2, 500), 1000..1250);
        assert_eq!(page_row_range(1250, 3, 500), 1250..1250);
        assert_eq!(total_pages(1250, 500), 3);
        assert_eq!(total_pages(0, 500), 1);
        assert_eq!(page_row_range(0, 0, 500), 0..0);
        assert_eq!(page_row_range(100, 0, 0), 0..100);
    }

    #[test]
    fn geo_info_copy_row_tsv_formatting() {
        let g = Geometry::from_arrays(
            vec![1.5, -2.25, 3.0],
            vec![0],
            vec![0, 1],
            None,
        ).unwrap();
        let cols = flatten_columns(&g, ElementDomain::Points);
        let tsv = format_row_tsv(&g, ElementDomain::Points, 0, &cols);
        assert_eq!(tsv, "0\t1.500\t-2.250\t3.000");
    }

    #[test]
    fn geo_info_virtual_row_slice_performance() {
        // Construct geometry with 100,000 points
        let count = 100_000;
        let p_data = vec![1.234f32; count * 3];
        let g = Geometry::points_only(p_data).unwrap();
        let cols = flatten_columns(&g, ElementDomain::Points);

        // Format only 20 visible rows around index 50,000
        let visible_range = 50_000..50_020;
        let mut formatted = Vec::new();
        let t0 = std::time::Instant::now();
        for r in visible_range {
            formatted.push(format_cell(&g, ElementDomain::Points, r, &cols[0]));
        }
        let elapsed = t0.elapsed();
        assert_eq!(formatted.len(), 20);
        assert_eq!(formatted[0], "1.234");
        assert!(elapsed.as_millis() < 50, "Virtual row formatting took too long: {elapsed:?}");
    }
}
