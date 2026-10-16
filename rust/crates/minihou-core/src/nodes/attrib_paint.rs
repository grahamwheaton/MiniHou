//! Python `paint.py` mhp1 payload and Attribute Paint cook.
use super::{Inputs, NodeSpec, GEO};
use crate::{
    error::{Error, Result},
    geometry::{Attr, AttrData, Geometry},
    params::Params,
    topo,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use std::{
    collections::HashSet,
    io::{Read, Write},
    sync::Arc,
};

const MAX_EDITS: usize = 4_000_000;
const MAX_TEXT: usize = 48_000_000;

pub const ATTRIB_PAINT: NodeSpec = NodeSpec {
    name: "attrib_paint",
    inputs: GEO,
    params: crate::params!("attribute":"paint"; "domain":"point"; "kind":"float";
                           "default":0.0; "default_vec":[0.0,0.0,0.0]; "data":""),
    cook,
};

/// Decoded rows retain Python's signed element numbers, including stale negative ones.
pub fn decode(text: &str, width: usize) -> Result<Vec<(i32, Vec<f32>)>> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    if !text.starts_with("mhp1:") {
        return Err(Error::new("painted data is not in the mhp1 format"));
    }
    if text.len() > MAX_TEXT {
        return Err(Error::new("painted data is too large"));
    }
    if width == 0 {
        return Err(Error::new("painted data has zero components"));
    }
    let compressed = STANDARD
        .decode(&text[5..])
        .map_err(|_| Error::new("painted data is damaged (base64)"))?;
    // Limit decompression before allocation can grow beyond the maximum valid payload.
    let max_raw = 4 + MAX_EDITS * 4 * (1 + width);
    let mut raw = Vec::new();
    ZlibDecoder::new(compressed.as_slice())
        .take(max_raw as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|_| Error::new("painted data is damaged (zlib)"))?;
    if raw.len() < 4 {
        return Err(Error::new("painted data is damaged (too short)"));
    }
    let n = u32::from_le_bytes(raw[..4].try_into().unwrap()) as usize;
    if n > MAX_EDITS || raw.len() != 4 + n * 4 * (1 + width) {
        return Err(Error::new(format!(
            "painted data does not fit a {width}-component attribute"
        )));
    }
    let mut rows = Vec::with_capacity(n);
    for i in 0..n {
        let at = 4 + i * 4;
        let idx = i32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
        let mut values = Vec::with_capacity(width);
        for c in 0..width {
            let at = 4 + n * 4 + (i * width + c) * 4;
            let v = f32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
            if !v.is_finite() {
                return Err(Error::new(
                    "painted data holds a value that is not a number",
                ));
            }
            values.push(v);
        }
        rows.push((idx, values));
    }
    Ok(rows)
}

pub fn encode(rows: &[(i32, Vec<f32>)], width: usize) -> Result<String> {
    if rows.is_empty() {
        return Ok(String::new());
    }
    if rows.len() > MAX_EDITS || width == 0 {
        return Err(Error::new("too many painted elements or zero components"));
    }
    let mut raw = Vec::with_capacity(4 + rows.len() * 4 * (1 + width));
    raw.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    for (i, _) in rows {
        raw.extend_from_slice(&i.to_le_bytes());
    }
    for (_, v) in rows {
        if v.len() != width || v.iter().any(|x| !x.is_finite()) {
            return Err(Error::new("invalid painted value"));
        }
        for x in v {
            raw.extend_from_slice(&x.to_le_bytes());
        }
    }
    let mut z = ZlibEncoder::new(Vec::new(), Compression::new(3));
    z.write_all(&raw).map_err(|e| Error::new(e.to_string()))?;
    let compressed = z.finish().map_err(|e| Error::new(e.to_string()))?;
    let text = format!("mhp1:{}", STANDARD.encode(compressed));
    if text.len() > MAX_TEXT {
        return Err(Error::new("the painted data is too large for one node"));
    }
    Ok(text)
}

pub fn merge(
    text: &str,
    width: usize,
    indices: &[i32],
    values: Option<&[Vec<f32>]>,
) -> Result<String> {
    let mut rows = decode(text, width)?;
    if indices.len() > MAX_EDITS || indices.iter().any(|i| *i < 0) {
        return Err(Error::new("invalid painted element numbers"));
    }
    if let Some(values) = values {
        if values.len() != indices.len()
            || values
                .iter()
                .any(|v| v.len() != width || v.iter().any(|x| !x.is_finite()))
        {
            return Err(Error::new("invalid painted values"));
        }
        rows.extend(indices.iter().zip(values).map(|(&i, v)| (i, v.clone())));
        let mut seen = HashSet::new();
        rows = rows
            .into_iter()
            .rev()
            .filter(|r| seen.insert(r.0))
            .collect();
        rows.reverse();
    } else {
        let erase: HashSet<i32> = indices.iter().copied().collect();
        rows.retain(|r| !erase.contains(&r.0));
    }
    encode(&rows, width)
}

fn valid_name(name: &str, domain: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
        && name != "P"
        && !(domain == "edge" && name.starts_with("edge_group_"))
}

fn rounded(x: f64) -> i32 {
    x.round_ties_even() as i32
}

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = ATTRIB_PAINT.one(inputs, 0)?;
    let name = p.s("attribute")?;
    let domain = p.e("domain", &["point", "prim", "edge"])?;
    let kind = p.e("kind", &["float", "int", "vec3"])?;
    if !valid_name(&name, &domain) {
        return Err(Error::new(format!(
            "invalid painted attribute name {name:?}"
        )));
    }
    if domain == "edge" && kind == "vec3" {
        return Err(Error::new(
            "edge attribute length does not match derived edges",
        ));
    }
    let width = if kind == "vec3" { 3 } else { 1 };
    let rows = decode(&p.s("data")?, width)?;
    let n = match domain.as_str() {
        "point" => g.npoints,
        "prim" => g.nprims(),
        _ => topo::edges(&g.topology, g.npoints).len(),
    };
    let default = if width == 3 {
        p.v3("default_vec")?.to_vec()
    } else {
        vec![p.f("default")?]
    };
    let mut data: Vec<f64> = (0..n).flat_map(|_| default.iter().copied()).collect();
    let old = if domain == "edge" {
        g.vertex
            .get(&format!("edge_group_{name}"))
            .map(|_| super::edge_tools::values(&g, &format!("edge_group_{name}")))
            .transpose()?
            .map(|v| Attr::f32(1, v))
    } else {
        g.table(&domain).get(&name).map(|a| (**a).clone())
    };
    if let Some(old) = old.filter(|a| a.rows() == n) {
        for i in 0..n {
            for c in 0..width {
                let source_c = if old.width >= width { c } else { 0 };
                data[i * width + c] = match &old.data {
                    AttrData::F32(v) => v[i * old.width + source_c] as f64,
                    AttrData::I32(v) => v[i * old.width + source_c] as f64,
                };
            }
        }
    }
    let mut stale = 0;
    for (idx, values) in rows {
        if idx < 0 || idx as usize >= n {
            stale += 1;
            continue;
        }
        for c in 0..width {
            data[idx as usize * width + c] = values[c] as f64;
        }
    }
    let mut out = (*g).clone();
    if domain == "edge" {
        let edges = topo::edges(&g.topology, g.npoints);
        let attr = if kind == "int" {
            Attr::i32(
                1,
                edges
                    .of_vertex
                    .iter()
                    .map(|&e| if e < 0 { 0 } else { rounded(data[e as usize]) })
                    .collect(),
            )
        } else {
            Attr::f32(
                1,
                edges
                    .of_vertex
                    .iter()
                    .map(|&e| if e < 0 { 0. } else { data[e as usize] as f32 })
                    .collect(),
            )
        };
        out.vertex
            .insert(format!("edge_group_{name}"), Arc::new(attr));
    } else {
        let attr = if kind == "int" {
            Attr::i32(1, data.into_iter().map(rounded).collect())
        } else {
            Attr::f32(width, data.into_iter().map(|x| x as f32).collect())
        };
        if domain == "point" {
            out.point.insert(name.clone(), Arc::new(attr));
        } else {
            out.prim.insert(name.clone(), Arc::new(attr));
        }
    }
    if stale > 0 {
        out.detail
            .insert(format!("paint_stale_{name}"), serde_json::json!(stale));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Geometry {
        Geometry::from_arrays(vec![0.; 12], vec![0, 1, 2, 3], vec![0, 4], None).unwrap()
    }
    fn paint(g: Geometry, extra: serde_json::Value) -> Result<Geometry> {
        let p = Params::resolve(ATTRIB_PAINT.params, extra.as_object())?;
        cook(&[vec![Arc::new(g)]], &p)
    }

    #[test]
    fn attrib_paint_python_golden_codec_and_merge() {
        // Produced by Python minihou.paint.encode([1,4], [[0.25],[0.75]]).
        let golden = "mhp1:eF5jYmBgYARiFiBmYGiwY2BwsAcABXEBRQ==";
        assert_eq!(
            decode(golden, 1).unwrap(),
            vec![(1, vec![0.25]), (4, vec![0.75])]
        );
        let encoded = encode(&decode(golden, 1).unwrap(), 1).unwrap();
        assert_eq!(decode(&encoded, 1).unwrap(), decode(golden, 1).unwrap());
        assert_eq!(
            decode(&merge(golden, 1, &[1], Some(&[vec![0.5]])).unwrap(), 1).unwrap(),
            vec![(4, vec![0.75]), (1, vec![0.5])]
        );
        assert_eq!(merge(golden, 1, &[1, 4], None).unwrap(), "");
        for bad in ["garbage", "mhp1:@@@", "mhp1:", &golden[..golden.len() - 4]] {
            assert!(decode(bad, 1).is_err());
        }
        assert!(decode(golden, 3).is_err());
    }

    #[test]
    fn attrib_paint_domains_types_defaults_and_stale() {
        let golden = "mhp1:eF5jYmBgYARiFiBmYGiwY2BwsAcABXEBRQ==";
        let out = paint(
            sample(),
            json!({"attribute":"w","data":golden,"default":0.5}),
        )
        .unwrap();
        assert_eq!(out.point["w"].as_f32().unwrap(), &[0.5, 0.25, 0.5, 0.5]);
        assert_eq!(out.detail["paint_stale_w"], 1);
        let out2 = paint(
            out,
            json!({"attribute":"w","kind":"int","data":encode(&[(2,vec![2.5])],1).unwrap()}),
        )
        .unwrap();
        assert_eq!(out2.point["w"].data, AttrData::I32(vec![0, 0, 2, 0]));
        let v = encode(&[(0, vec![1., 0., 0.])], 3).unwrap();
        let out = paint(sample(), json!({"attribute":"Cd","kind":"vec3","domain":"prim","default_vec":[0.2,0.3,0.4],"data":v})).unwrap();
        assert_eq!(out.prim["Cd"].as_f32().unwrap(), &[1., 0., 0.]);
        let e = encode(&[(0, vec![1.])], 1).unwrap();
        let out = paint(
            sample(),
            json!({"attribute":"selected","kind":"int","domain":"edge","data":e}),
        )
        .unwrap();
        assert_eq!(
            super::super::edge_tools::values(&out, "edge_group_selected").unwrap()[0],
            1.
        );
    }
}
