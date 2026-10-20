use crate::nodes::{Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::{Params};
use crate::nodes::attrib_tools::{domain, size, set_with_group, piece_labels};
pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "color",
    inputs: GEO,
    params: crate::params!(
        "mode": "constant";
        "domain": "points";
        "color": [1.0, 0.5, 0.2];
        "color2": [0.1, 0.3, 1.0];
        "ramp_attrib": "P.z";
        "seed": 0;
        "group": ""
    ),
    cook: color_node,
};

fn read_domain_attr(g: &Geometry, dom: &str, name: &str) -> Option<(Vec<f64>, usize)> {
    if let Some(attr) = g.table(dom).get(name) {
        let width = attr.width;
        let vals: Vec<f64> = match &attr.data {
            AttrData::F32(v) => v.iter().map(|&x| x as f64).collect(),
            AttrData::I32(v) => v.iter().map(|&x| x as f64).collect(),
        };
        return Some((vals, width));
    }
    if dom == "vertex" {
        if let Some(attr) = g.point.get(name) {
            let width = attr.width;
            let vals: Vec<f64> = match &attr.data {
                AttrData::F32(v) => v.iter().map(|&x| x as f64).collect(),
                AttrData::I32(v) => v.iter().map(|&x| x as f64).collect(),
            };
            let mut out = Vec::with_capacity(g.nvertices() * width);
            for &pt in &g.topology.vertex_point {
                let off = (pt as usize) * width;
                out.extend_from_slice(&vals[off..off + width]);
            }
            return Some((out, width));
        }
        if let Some(attr) = g.prim.get(name) {
            let width = attr.width;
            let vals: Vec<f64> = match &attr.data {
                AttrData::F32(v) => v.iter().map(|&x| x as f64).collect(),
                AttrData::I32(v) => v.iter().map(|&x| x as f64).collect(),
            };
            let mut out = Vec::with_capacity(g.nvertices() * width);
            for prim_idx in 0..g.nprims() {
                let count = (g.topology.prim_offsets[prim_idx + 1] - g.topology.prim_offsets[prim_idx]) as usize;
                let off = prim_idx * width;
                let prim_slice = &vals[off..off + width];
                for _ in 0..count {
                    out.extend_from_slice(prim_slice);
                }
            }
            return Some((out, width));
        }
    } else if dom == "prim" {
        if let Some(attr) = g.point.get(name) {
            let width = attr.width;
            let vals: Vec<f64> = match &attr.data {
                AttrData::F32(v) => v.iter().map(|&x| x as f64).collect(),
                AttrData::I32(v) => v.iter().map(|&x| x as f64).collect(),
            };
            let mut out = Vec::with_capacity(g.nprims() * width);
            for prim_idx in 0..g.nprims() {
                let start = g.topology.prim_offsets[prim_idx] as usize;
                let end = g.topology.prim_offsets[prim_idx + 1] as usize;
                let count = end - start;
                if count == 0 {
                    out.resize(out.len() + width, 0.0);
                } else {
                    for c in 0..width {
                        let sum: f64 = (start..end)
                            .map(|v| {
                                let pt = g.topology.vertex_point[v] as usize;
                                vals[pt * width + c]
                            })
                            .sum();
                        out.push(sum / count as f64);
                    }
                }
            }
            return Some((out, width));
        }
        if let Some(attr) = g.vertex.get(name) {
            let width = attr.width;
            let vals: Vec<f64> = match &attr.data {
                AttrData::F32(v) => v.iter().map(|&x| x as f64).collect(),
                AttrData::I32(v) => v.iter().map(|&x| x as f64).collect(),
            };
            let mut out = Vec::with_capacity(g.nprims() * width);
            for prim_idx in 0..g.nprims() {
                let start = g.topology.prim_offsets[prim_idx] as usize;
                let end = g.topology.prim_offsets[prim_idx + 1] as usize;
                let count = end - start;
                if count == 0 {
                    out.resize(out.len() + width, 0.0);
                } else {
                    for c in 0..width {
                        let sum: f64 = (start..end)
                            .map(|v| vals[v * width + c])
                            .sum();
                        out.push(sum / count as f64);
                    }
                }
            }
            return Some((out, width));
        }
    } else if dom == "edge" {
        if let Some(attr) = g.point.get(name) {
            let width = attr.width;
            let vals: Vec<f64> = match &attr.data {
                AttrData::F32(v) => v.iter().map(|&x| x as f64).collect(),
                AttrData::I32(v) => v.iter().map(|&x| x as f64).collect(),
            };
            let e = crate::topo::edges(&g.topology, g.npoints);
            let mut out = Vec::with_capacity(e.len() * width);
            for i in 0..e.len() {
                let pa = e.points[2 * i] as usize;
                let pb = e.points[2 * i + 1] as usize;
                for c in 0..width {
                    out.push((vals[pa * width + c] + vals[pb * width + c]) * 0.5);
                }
            }
            return Some((out, width));
        }
        if let Some(attr) = g.vertex.get(name) {
            let width = attr.width;
            let vals: Vec<f64> = match &attr.data {
                AttrData::F32(v) => v.iter().map(|&x| x as f64).collect(),
                AttrData::I32(v) => v.iter().map(|&x| x as f64).collect(),
            };
            let e = crate::topo::edges(&g.topology, g.npoints);
            let mut out = vec![0.0f64; e.len() * width];
            let mut count = vec![0usize; e.len()];
            for v in 0..g.nvertices() {
                let ed = e.of_vertex[v];
                if ed >= 0 {
                    let ed = ed as usize;
                    for c in 0..width {
                        out[ed * width + c] += vals[v * width + c];
                    }
                    count[ed] += 1;
                }
            }
            for i in 0..e.len() {
                let cnt = count[i].max(1) as f64;
                for c in 0..width {
                    out[i * width + c] /= cnt;
                }
            }
            return Some((out, width));
        }
    }
    if let Some(val) = g.detail.get(name) {
        let n = size(g, dom);
        if let Some(num) = val.as_f64() {
            return Some((vec![num; n], 1));
        }
        if let Some(arr) = val.as_array() {
            if arr.len() <= 4 && arr.iter().all(|x| x.is_number()) {
                let width = arr.len();
                let nums: Vec<f64> = arr.iter().map(|x| x.as_f64().unwrap()).collect();
                let mut out = Vec::with_capacity(n * width);
                for _ in 0..n {
                    out.extend_from_slice(&nums);
                }
                return Some((out, width));
            }
        }
    }
    None
}

fn color_node(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let mode = p.e("mode", &["constant", "random", "random_by_piece", "ramp"])?;
    let dom = p.e("domain", &["points", "edges", "faces", "primitives", "vertices"])?;
    let dom = domain(&dom);
    let color = p.v3("color")?;
    let color2 = p.v3("color2")?;
    let c1: [f32; 3] = [color[0] as f32, color[1] as f32, color[2] as f32];
    let ramp_attrib = p.s("ramp_attrib")?;
    let seed = p.i("seed")?;
    let group = p.s("group")?;
    let n = size(&g, dom);

    let cd = match mode.as_str() {
        "constant" => {
            let mut cd = Vec::with_capacity(n * 3);
            for _ in 0..n {
                cd.extend_from_slice(&c1);
            }
            cd
        }
        "random" => {
            let mut rng = crate::nodes::np_random::Rng::new(&[(seed as u64) & 0xFFFFFFFF]);
            let mut cd = Vec::with_capacity(n * 3);
            for _ in 0..n {
                for _ in 0..3 {
                    cd.push(rng.random() as f32);
                }
            }
            cd
        }
        "random_by_piece" => {
            let mut rng = crate::nodes::np_random::Rng::new(&[(seed as u64) & 0xFFFFFFFF]);
            let (pts, prim) = piece_labels(&g);
            let lab: Vec<i32> = match dom {
                "point" => pts,
                "prim" => prim,
                "edge" => {
                    let e = crate::topo::edges(&g.topology, g.npoints);
                    (0..e.len())
                        .map(|i| {
                            let pa = e.points[2 * i] as usize;
                            pts.get(pa).copied().unwrap_or(0)
                        })
                        .collect()
                }
                "vertex" => {
                    let mut pv = Vec::with_capacity(g.nvertices());
                    for i in 0..g.nprims() {
                        let count = (g.topology.prim_offsets[i + 1] - g.topology.prim_offsets[i]) as usize;
                        pv.resize(pv.len() + count, prim[i]);
                    }
                    pv
                }
                _ => unreachable!(),
            };
            let max_lab = lab.iter().copied().max();
            let pal_count = match max_lab {
                Some(m) if m >= 0 => (m + 1) as usize,
                _ => 0,
            };
            let mut pal = Vec::with_capacity(pal_count * 3);
            for _ in 0..pal_count {
                for _ in 0..3 {
                    pal.push(rng.random() as f32);
                }
            }
            let mut cd = Vec::with_capacity(lab.len() * 3);
            for &l in &lab {
                if l >= 0 && (l as usize) < pal_count {
                    let off = (l as usize) * 3;
                    cd.extend_from_slice(&pal[off..off + 3]);
                } else if l < 0 && pal_count > 0 {
                    let wrapped = ((l % (pal_count as i32)) + (pal_count as i32)) % (pal_count as i32);
                    let off = (wrapped as usize) * 3;
                    cd.extend_from_slice(&pal[off..off + 3]);
                } else {
                    cd.extend_from_slice(&[0.0, 0.0, 0.0]);
                }
            }
            cd
        }
        "ramp" => {
            let (nm, comp) = match ramp_attrib.split_once('.') {
                Some((nm, comp)) => (nm, comp),
                None => (ramp_attrib.as_str(), ""),
            };
            let (a, width) = read_domain_attr(&g, dom, nm)
                .ok_or_else(|| Error::new(format!("no attribute {nm:?} to ramp by")))?;
            let v: Vec<f64> = if !comp.is_empty() {
                let comp_idx = match comp {
                    "x" => 0,
                    "y" => 1,
                    "z" => 2,
                    "w" => 3,
                    _ => return Err(Error::new(format!("unknown component {comp:?} in {ramp_attrib:?}"))),
                };
                if comp_idx >= width {
                    return Err(Error::new(format!("component index {comp_idx} out of range for attribute {nm:?} with width {width}")));
                }
                (0..n).map(|i| a[i * width + comp_idx]).collect()
            } else if width == 1 {
                a
            } else {
                (0..n).map(|i| {
                    let row = &a[i * width..(i + 1) * width];
                    row.iter().map(|&x| x * x).sum::<f64>().sqrt()
                }).collect()
            };
            if v.is_empty() {
                Vec::new()
            } else {
                let mut v_min = f64::INFINITY;
                let mut v_max = f64::NEG_INFINITY;
                for &x in &v {
                    if x < v_min { v_min = x; }
                    if x > v_max { v_max = x; }
                }
                let span = (v_max - v_min).max(1e-12);
                let mut cd = Vec::with_capacity(n * 3);
                for x in v {
                    let t = (x - v_min) / span;
                    for c in 0..3 {
                        let val = (color[c] as f64) * (1.0 - t) + (color2[c] as f64) * t;
                        cd.push(val as f32);
                    }
                }
                cd
            }
        }
        _ => unreachable!(),
    };

    set_with_group(&g, dom, "Cd", Attr::f32(3, cd), &group)
}


#[cfg(test)]
#[path = "tests.rs"]
mod tests;
