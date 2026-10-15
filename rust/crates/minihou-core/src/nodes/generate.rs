//! Source nodes: `grid` (`minihou/nodes/grid.py`) and `box` (`minihou/nodes/primitives.py`).

use std::collections::HashMap;
use std::sync::Arc;

use rayon::prelude::*;
use serde_json::json;

use super::{linspace, InputSpec, Inputs, NodeSpec};
use crate::error::{Error, Result};
use crate::geometry::{Attr, Geometry};
use crate::math;
use crate::params::Params;

pub const CURVE: NodeSpec = NodeSpec {
    name: "curve",
    inputs: &[InputSpec { name: "geo", multi: false, optional: true }],
    params: crate::params!("points": "0,0,0  1,0,0  1,1,0  2,1,1"; "closed": false;),
    cook: curve,
};

pub const POINT: NodeSpec = NodeSpec {
    name: "point", inputs: &[], params: crate::params!("center": [0.0, 0.0, 0.0];), cook: point,
};

fn point(_inputs: &Inputs, p: &Params) -> Result<Geometry> {
    Geometry::points_only(p.v3("center")?.map(|v| v as f32).to_vec())
}

fn curve(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let mut positions = Vec::new();
    for token in p.s("points")?.split_whitespace() {
        let coords: Vec<&str> = token.split(',').collect();
        if coords.len() != 3 {
            return Err(Error::new("give at least two points, each as x,y,z"));
        }
        for c in coords {
            positions.push(c.parse::<f64>().map_err(|_| Error::new("points must look like 'x,y,z x,y,z ...'"))? as f32);
        }
    }
    let n = positions.len() / 3;
    if n < 2 {
        return Err(Error::new("give at least two points, each as x,y,z"));
    }
    let new = Geometry::from_arrays(positions, (0..n as i32).collect(), vec![0, n as i32], Some(vec![p.b("closed")?]))?;
    if let Some(input) = inputs[0].first() {
        let params = Params::resolve(&[], None)?;
        super::merge::merge(&[vec![input.clone(), Arc::new(new)]], &params)
    } else {
        Ok(new)
    }
}

pub const LINE: NodeSpec = NodeSpec {
    name: "line", inputs: &[],
    params: crate::params!("origin": [0.0, 0.0, 0.0]; "direction": [0.0, 0.0, 1.0]; "length": 1.0; "points": 10;),
    cook: line,
};

fn line(_inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let n = p.i("points")?;
    if !(2..=1_000_000).contains(&n) {
        return Err(Error::new("line points must be between 2 and 1000000"));
    }
    let n = n as usize;
    let origin = p.v3("origin")?;
    let mut d = p.v3("direction")?;
    let len = d.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-12);
    for x in &mut d { *x /= len; }
    let mut pos = Vec::with_capacity(n * 3);
    for t in linspace(0.0, p.f("length")?, n) {
        pos.extend((0..3).map(|k| (origin[k] + t * d[k]) as f32));
    }
    Geometry::from_arrays(pos, (0..n as i32).collect(), vec![0, n as i32], Some(vec![false]))
}

pub const CIRCLE: NodeSpec = NodeSpec {
    name: "circle", inputs: &[],
    params: crate::params!("radius": 1.0; "center": [0.0, 0.0, 0.0]; "rotate": [0.0, 0.0, 0.0]; "divisions": 32;),
    cook: circle,
};

fn circle(_inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let n = p.i("divisions")?;
    if !(3..=100_000).contains(&n) { return Err(Error::new("circle divisions must be between 3 and 100000")); }
    let n = n as usize;
    let r = p.f("radius")?;
    let center = p.v3("center")?;
    let m = math::euler_xyz(p.v3("rotate")?);
    let mut pos = Vec::with_capacity(n * 3);
    for i in 0..n {
        let angle = i as f64 * (2.0 * std::f64::consts::PI / n as f64);
        let q = math::apply(&m, [r * angle.cos(), r * angle.sin(), 0.0]);
        pos.extend((0..3).map(|k| (q[k] + center[k]) as f32));
    }
    Geometry::from_arrays(pos, (0..n as i32).collect(), vec![0, n as i32], None)
}

pub const GRID: NodeSpec = NodeSpec {
    name: "grid",
    inputs: &[],
    params: crate::params!(
        "size_x": 10.0; "size_y": 10.0; "columns": 10; "rows": 10; "center": [0.0, 0.0, 0.0];
        "rotate": [0.0, 0.0, 0.0]; "scale": [1.0, 1.0, 1.0]; "uniform_scale": 1.0;
    ),
    cook: grid,
};

/// `source_transform`: scale and rotate about `center`, in float64, back to float32.
fn source_transform(p: &mut [f32], center: [f64; 3], rotate: [f64; 3], scale: [f64; 3], uniform_scale: f64) {
    let r = math::euler_xyz(rotate);
    p.par_chunks_exact_mut(3).for_each(|q| {
        let v = [0, 1, 2].map(|k| (q[k] as f64 - center[k]) * scale[k] * uniform_scale);
        let w = math::apply(&r, v);
        for k in 0..3 {
            q[k] = (w[k] + center[k]) as f32;
        }
    });
}

fn grid(_inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let (size_x, size_y) = (p.f("size_x")?, p.f("size_y")?);
    let (columns, rows) = (p.i("columns")?, p.i("rows")?);
    if !(2..=20000).contains(&columns) || !(2..=20000).contains(&rows) {
        return Err(Error::new("columns and rows must be between 2 and 20000"));
    }
    if size_x < 0.0 || size_y < 0.0 {
        return Err(Error::new("size_x and size_y must not be negative"));
    }
    let (cols, rows) = (columns as usize, rows as usize);
    let center = p.v3("center")?;
    let u: Vec<f32> = linspace(0.0, 1.0, cols).into_iter().map(|x| x as f32).collect();
    let v: Vec<f32> = linspace(0.0, 1.0, rows).into_iter().map(|x| x as f32).collect();
    let (sx, sy) = (size_x as f32, size_y as f32);
    let c32 = center.map(|c| c as f32);
    let n = rows * cols;
    let mut pts = vec![0.0f32; n * 3];
    let mut uv = vec![0.0f32; n * 2];
    pts.par_chunks_mut(cols * 3).zip(uv.par_chunks_mut(cols * 2)).zip(v.par_iter()).for_each(|((prow, uvrow), &vj)| {
        for (i, &ui) in u.iter().enumerate() {
            prow[i * 3..i * 3 + 3].copy_from_slice(&[(ui - 0.5) * sx + c32[0], (vj - 0.5) * sy + c32[1], c32[2]]);
            uvrow[i * 2..i * 2 + 2].copy_from_slice(&[ui, vj]);
        }
    });
    let quads = (cols - 1) * (rows - 1);
    let mut vp = vec![0i32; quads * 4];
    vp.par_chunks_mut((cols - 1) * 4).enumerate().for_each(|(j, row)| {
        let c = cols as i32;
        for i in 0..cols - 1 {
            let b = (j * cols + i) as i32;
            row[i * 4..i * 4 + 4].copy_from_slice(&[b, b + 1, b + c + 1, b + c]);
        }
    });
    let offs: Vec<i32> = (0..=quads as i32).map(|k| k * 4).collect();
    source_transform(&mut pts, center, p.v3("rotate")?, p.v3("scale")?, p.f("uniform_scale")?);
    let mut g = Geometry::from_arrays(pts, vp, offs, None)?;
    g.point.insert("uv".into(), Arc::new(Attr::f32(2, uv)));
    g.detail.insert("grid_columns".into(), json!(cols));
    g.detail.insert("grid_rows".into(), json!(rows));
    Ok(g)
}

pub const BOX: NodeSpec = NodeSpec {
    name: "box",
    inputs: &[InputSpec { name: "geo", multi: false, optional: true }],
    params: crate::params!(
        "primitive_type": "polygons"; "connectivity": "quadrilaterals"; "consolidate": true; "size": [1.0, 1.0, 1.0];
        "center": [0.0, 0.0, 0.0]; "rotate": [0.0, 0.0, 0.0]; "uniform_scale": 1.0; "divisions": 1;
        "axis_divisions": [0.0, 0.0, 0.0]; "add_vertex_normals": false, unsupported;
    ),
    cook: box_node,
};

fn box_node(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    if !inputs[0].is_empty() {
        return Err(Error::new("box with an input to bound is not supported by the Rust backend yet"));
    }
    let primitive_type = p.e("primitive_type", &["polygons", "points"])?;
    let connectivity = p.e("connectivity", &["quadrilaterals", "triangles"])?;
    let size = p.v3("size")?;
    let center = p.v3("center")?;
    let us = p.f("uniform_scale")?;
    let divisions = p.i("divisions")?;
    if !(1..=1000).contains(&divisions) || us < 0.0 || size.iter().any(|&s| s < 0.0) {
        return Err(Error::new("divisions must be 1..1000; size and uniform_scale must not be negative"));
    }
    let ad = p.v3("axis_divisions")?;
    if ad.iter().any(|v| v.fract() != 0.0) {
        return Err(Error::new("Axis divisions must be integers"));
    }
    let dims: [usize; 3] = ad.map(|v| if v as i64 != 0 { v as i64 } else { divisions }.max(0) as usize);
    if [(0, 1), (1, 2), (0, 2)].iter().map(|&(i, j)| dims[i] * dims[j]).sum::<usize>() > 1_000_000 {
        return Err(Error::new("Box exceeds one million face cells"));
    }
    let mut pts: Vec<[f64; 3]> = Vec::new();
    let mut quads: Vec<[i32; 4]> = Vec::new();
    for (u, v, w, sign) in [(0, 1, 2, 1.0), (1, 0, 2, -1.0), (1, 2, 0, 1.0), (2, 1, 0, -1.0), (2, 0, 1, 1.0), (0, 2, 1, -1.0)] {
        let (nu, nv) = (dims[u], dims[v]);
        let lu = linspace(-0.5, 0.5, nu + 1);
        let lv = linspace(-0.5, 0.5, nv + 1);
        let offset = pts.len() as i32;
        for &y in &lv {
            for &x in &lu {
                let mut q = [0.0; 3];
                q[u] = x;
                q[v] = y;
                q[w] = sign * 0.5;
                pts.push(q);
            }
        }
        for j in 0..nv {
            for i in 0..nu {
                let q = (j * (nu + 1) + i) as i32 + offset;
                let s = (nu + 1) as i32;
                quads.push([q, q + 1, q + s + 1, q + s]);
            }
        }
    }
    let r = math::euler_xyz(p.v3("rotate")?);
    let mut flat = Vec::with_capacity(pts.len() * 3);
    for q in &pts {
        let scaled = [0, 1, 2].map(|k| q[k] * size[k] * us);
        let w = math::apply(&r, scaled);
        flat.extend((0..3).map(|k| (w[k] + center[k]) as f32));
    }
    let (vp, offs) = if connectivity == "triangles" {
        let mut vp: Vec<i32> = quads.iter().flat_map(|q| [q[0], q[1], q[2]]).collect();
        vp.extend(quads.iter().flat_map(|q| [q[0], q[2], q[3]]));
        let n = vp.len() as i32 / 3;
        (vp, (0..=n).map(|k| k * 3).collect::<Vec<i32>>())
    } else {
        let n = quads.len() as i32;
        (quads.iter().flatten().copied().collect(), (0..=n).map(|k| k * 4).collect())
    };
    let mut g = Geometry::from_arrays(flat, vp, offs, None)?;
    if p.b("consolidate")? {
        let biggest = size.iter().cloned().fold(f64::NEG_INFINITY, f64::max) * us;
        g = fuse_points(&g, 1e-6 * biggest.max(1.0));
    }
    if primitive_type == "points" {
        return Geometry::points_only(g.p().to_vec());
    }
    Ok(g)
}

pub const BOUND: NodeSpec = NodeSpec {
    name: "bound",
    inputs: &[InputSpec { name: "geo", multi: false, optional: false }],
    params: crate::params!("padding": [0.0, 0.0, 0.0]; "oriented": false, unsupported;),
    cook: bound,
};

/// The world-axis box around the input (`minihou/nodes/transform.py`), in float64 like the numpy body, back to float32.
/// `oriented` (a principal-axis fit through `np.linalg.svd`) is refused: the Rust core has no SVD and the box's detail
/// attributes depend on the sign convention LAPACK happens to pick, which the box points do not.
fn bound(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = BOUND.one(inputs, 0)?;
    let padding = p.v3("padding")?;
    if geo.npoints == 0 {
        return box_sized([0.0, 0.0, 0.0]);
    }
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for q in geo.p().as_chunks::<3>().0 {
        for k in 0..3 {
            let v = q[k] as f64;
            if v.is_nan() {
                lo[k] = v;
                hi[k] = v;
            } else {
                if v < lo[k] { lo[k] = v; }
                if v > hi[k] { hi[k] = v; }
            }
        }
    }
    for k in 0..3 {
        lo[k] -= padding[k];
        hi[k] += padding[k];
    }
    let size = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    let center = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let mut out = box_sized(size)?;
    let positions = out.p().as_chunks::<3>().0.iter().flat_map(|q| (0..3).map(move |k| (q[k] as f64 + center[k]) as f32)).collect();
    out.set_p(positions);
    out.detail.insert("bound_min".into(), json!(lo));
    out.detail.insert("bound_max".into(), json!(hi));
    out.detail.insert("bound_size".into(), json!(size));
    Ok(out)
}

/// A default `box` (one division, welded) of the given size at the origin, as `bound` builds it.
fn box_sized(size: [f64; 3]) -> Result<Geometry> {
    let mut given = serde_json::Map::new();
    given.insert("size".into(), json!(size));
    let params = Params::resolve(BOX.params, Some(&given))?;
    let empty: [Vec<Arc<Geometry>>; 1] = [Vec::new()];
    box_node(&empty, &params)
}

/// Weld points that fall in the same cell of size `distance` (`floor(P / d + 0.5)`), keeping the first point of each cell
/// with its attributes, survivors in order of first appearance (`primitives.fuse_points`). Vertex and primitive attributes
/// are untouched.
pub fn fuse_points(geo: &Geometry, distance: f64) -> Geometry {
    if geo.npoints == 0 {
        return geo.clone();
    }
    let d = distance.max(1e-12);
    let p = geo.p();
    let mut cell_of: HashMap<[i64; 3], i32> = HashMap::with_capacity(geo.npoints);
    let mut remap = Vec::with_capacity(geo.npoints);
    let mut keep = Vec::new();
    for (i, q) in p.as_chunks::<3>().0.iter().enumerate() {
        let key = [0, 1, 2].map(|k| (q[k] as f64 / d + 0.5).floor() as i64);
        let next = keep.len() as i32;
        let id = *cell_of.entry(key).or_insert_with(|| {
            keep.push(i);
            next
        });
        remap.push(id);
    }
    let topo = &geo.topology;
    let mut out = geo.clone();
    out.npoints = keep.len();
    out.topology = Arc::new(crate::geometry::Topology {
        vertex_point: topo.vertex_point.iter().map(|&v| remap[v as usize]).collect(),
        prim_offsets: topo.prim_offsets.clone(),
        closed: topo.closed.clone(),
    });
    for a in out.point.values_mut() {
        *a = Arc::new(take_rows(a, &keep));
    }
    out
}

/// The rows `idx` of an attribute.
pub fn take_rows(a: &Attr, idx: &[usize]) -> Attr {
    use crate::geometry::AttrData::*;
    let w = a.width;
    match &a.data {
        F32(v) => Attr::f32(w, idx.iter().flat_map(|&i| v[i * w..(i + 1) * w].iter().copied()).collect()),
        I32(v) => Attr::i32(w, idx.iter().flat_map(|&i| v[i * w..(i + 1) * w].iter().copied()).collect()),
    }
}
