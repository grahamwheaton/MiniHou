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

pub const CURVE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "curve",
    inputs: &[InputSpec { name: "geo", multi: false, optional: true }],
    params: crate::params!("points": "0,0,0  1,0,0  1,1,0  2,1,1"; "closed": false; "split": false; "as_mesh": false;),
    cook: curve,
};

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
    let mut new = Geometry::from_arrays(positions, (0..n as i32).collect(), vec![0, n as i32], Some(vec![p.b("closed")?]))?;
    if !p.b("as_mesh")? { new.prim.insert("__viewport_curve".into(), Arc::new(Attr::i32(1,vec![1]))); }
    if let Some(input) = inputs[0].first() {
        let mut input=(**input).clone();
        if p.b("split")? && !p.b("closed")? {
            for segment in new.p().chunks_exact(3).collect::<Vec<_>>().windows(2) {
                let a=[segment[0][0],segment[0][1],segment[0][2]];
                let b=[segment[1][0],segment[1][1],segment[1][2]];
                input=super::knife::split_coplanar_segment(&input,a,b)?;
            }
        }
        let params = Params::resolve(&[], None)?;
        super::merge::merge(&[vec![Arc::new(input), Arc::new(new)]], &params)
    } else {
        Ok(new)
    }
}

pub const CIRCLE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
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
    let mut g=Geometry::from_arrays(pos, (0..n as i32).collect(), vec![0, n as i32], None)?;
    g.prim.insert("__viewport_curve".into(),Arc::new(Attr::i32(1,vec![1])));
    Ok(g)
}

#[cfg(test)]
mod sketch_mesh_tests {
    #[test]
    fn as_mesh_marks_closed_sketch_as_surface() {
        let base=serde_json::json!({"version":1,"display":"c","nodes":{"c":{"type":"curve","params":{"points":"0,0,0 1,0,0 0,1,0","closed":true,"as_mesh":true}}}});
        let g=crate::Graph::from_value(&base).unwrap().cook(None).unwrap();
        assert_eq!(g.nprims(),1);
        assert!(g.topology.is_closed(0));
        assert!(!g.prim.contains_key("__viewport_curve"));
    }
}

pub const GRID: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "grid",
    inputs: &[],
    params: crate::params!(
        "size_x": 10.0; "size_y": 10.0; "columns": 3; "rows": 3; "center": [0.0, 0.0, 0.0];
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

pub const BOX: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
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

/// Drop repeated corners, then polygons with < 3 corners and curves with < 2 (points are kept).
pub fn drop_degenerate(geo: &Geometry) -> Geometry {
    if geo.nprims() == 0 {
        return geo.clone();
    }
    let topo = &geo.topology;
    let nprims = geo.nprims();
    let vp = &topo.vertex_point;
    let offs = &topo.prim_offsets;
    let closed_mask = topo.closed_mask();

    let mut keep_v = Vec::with_capacity(vp.len());
    let mut good_prims = Vec::with_capacity(nprims);
    let mut new_sizes = Vec::with_capacity(nprims);

    for p in 0..nprims {
        let start = offs[p] as usize;
        let end = offs[p + 1] as usize;
        let size = end - start;
        if size == 0 {
            continue;
        }
        let is_closed = closed_mask[p];

        let mut kept_for_p = Vec::with_capacity(size);
        for i in 0..size {
            let v = vp[start + i];
            let is_dup = if i + 1 < size {
                v == vp[start + i + 1]
            } else {
                is_closed && size > 1 && v == vp[start]
            };
            if !is_dup {
                kept_for_p.push(start + i);
            }
        }

        let new_size = kept_for_p.len();
        let is_good = if is_closed { new_size >= 3 } else { new_size >= 2 };
        if is_good {
            good_prims.push(p);
            new_sizes.push(new_size as i32);
            keep_v.extend(kept_for_p);
        }
    }

    let mut new_offs = Vec::with_capacity(good_prims.len() + 1);
    new_offs.push(0);
    let mut acc = 0;
    for &s in &new_sizes {
        acc += s;
        new_offs.push(acc);
    }

    let new_vp: Vec<i32> = keep_v.iter().map(|&idx| vp[idx]).collect();
    let new_closed = topo.closed.as_ref().map(|c| {
        good_prims.iter().map(|&p| c[p]).collect()
    });

    let mut out = geo.clone();
    out.topology = Arc::new(crate::geometry::Topology {
        vertex_point: new_vp,
        prim_offsets: new_offs,
        closed: new_closed,
    });

    for a in out.vertex.values_mut() {
        *a = Arc::new(take_rows(a, &keep_v));
    }
    for a in out.prim.values_mut() {
        *a = Arc::new(take_rows(a, &good_prims));
    }
    out
}

pub const SPHERE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "sphere",
    inputs: &[],
    params: crate::params!(
        "radius": 1.0; "center": [0.0, 0.0, 0.0]; "rotate": [0.0, 0.0, 0.0];
        "scale": [1.0, 1.0, 1.0]; "uniform_scale": 1.0; "rows": 16; "columns": 32;
    ),
    cook: sphere,
};

fn sphere(_inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let radius = p.f("radius")?;
    let center = p.v3("center")?;
    let rotate = p.v3("rotate")?;
    let scale = p.v3("scale")?;
    let uniform_scale = p.f("uniform_scale")?;
    let rows = p.i("rows")?;
    let columns = p.i("columns")?;

    if !(2..=4096).contains(&rows) || !(3..=4096).contains(&columns) {
        return Err(Error::new("sphere rows must be 2..4096 and columns 3..4096"));
    }

    let (rows, columns) = (rows as usize, columns as usize);
    let npoints = 1 + (rows - 1) * columns + 1;
    let mut pts = Vec::with_capacity(npoints * 3);

    // North pole (+Z): [0, 0, 1] * radius + center
    pts.push(center[0] as f32);
    pts.push(center[1] as f32);
    pts.push((radius + center[2]) as f32);

    // Rings from +Z pole down
    for r in 1..rows {
        let lat = r as f64 * (std::f64::consts::PI / rows as f64);
        let sl = lat.sin();
        let cl = lat.cos();
        for c in 0..columns {
            let lon = c as f64 * (2.0 * std::f64::consts::PI / columns as f64);
            let x = (sl * lon.cos()) * radius + center[0];
            let y = (sl * lon.sin()) * radius + center[1];
            let z = cl * radius + center[2];
            pts.push(x as f32);
            pts.push(y as f32);
            pts.push(z as f32);
        }
    }

    // South pole (-Z): [0, 0, -1] * radius + center
    pts.push(center[0] as f32);
    pts.push(center[1] as f32);
    pts.push((-radius + center[2]) as f32);

    let top = 0i32;
    let bottom = (1 + (rows - 1) * columns) as i32;

    let quads_count = (rows - 2) * columns;
    let total_v = columns * 3 + quads_count * 4 + columns * 3;
    let total_prims = columns + quads_count + columns;

    let mut vp = Vec::with_capacity(total_v);
    let mut offs = Vec::with_capacity(total_prims + 1);
    offs.push(0);

    // Top triangles: [top, 1 + c, 1 + ((c + 1) % columns)]
    for c in 0..columns as i32 {
        let cn = (c + 1) % columns as i32;
        vp.extend([top, 1 + c, 1 + cn]);
        offs.push(vp.len() as i32);
    }

    // Quads:
    for r in 0..(rows - 2) as i32 {
        let a = 1 + r * columns as i32;
        let b = 1 + (r + 1) * columns as i32;
        for c in 0..columns as i32 {
            let cn = (c + 1) % columns as i32;
            vp.extend([a + c, b + c, b + cn, a + cn]);
            offs.push(vp.len() as i32);
        }
    }

    // Bottom triangles: [bottom, r0 + cn, r0 + c]
    let r0 = 1 + ((rows - 2) * columns) as i32;
    for c in 0..columns as i32 {
        let cn = (c + 1) % columns as i32;
        vp.extend([bottom, r0 + cn, r0 + c]);
        offs.push(vp.len() as i32);
    }

    source_transform(&mut pts, center, rotate, scale, uniform_scale);
    Geometry::from_arrays(pts, vp, offs, None)
}

pub const TUBE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "tube",
    inputs: &[],
    params: crate::params!(
        "radius_bottom": 1.0; "radius_top": 1.0; "height": 2.0; "center": [0.0, 0.0, 0.0];
        "rotate": [0.0, 0.0, 0.0]; "scale": [1.0, 1.0, 1.0]; "uniform_scale": 1.0;
        "rows": 1; "columns": 24; "caps": true;
    ),
    cook: tube,
};

fn tube(_inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let radius_bottom = p.f("radius_bottom")?;
    let radius_top = p.f("radius_top")?;
    let height = p.f("height")?;
    let center = p.v3("center")?;
    let rotate = p.v3("rotate")?;
    let scale = p.v3("scale")?;
    let uniform_scale = p.f("uniform_scale")?;
    let rows = p.i("rows")?;
    let columns = p.i("columns")?;
    let caps = p.b("caps")?;

    if !(1..=4096).contains(&rows) || !(3..=4096).contains(&columns) {
        return Err(Error::new("tube rows must be 1..4096 and columns 3..4096"));
    }

    let (rows, columns) = (rows as usize, columns as usize);
    let npoints = (rows + 1) * columns;
    let mut pts = Vec::with_capacity(npoints * 3);

    let t = linspace(0.0, 1.0, rows + 1);
    for &tr in &t {
        let rad = radius_bottom + (radius_top - radius_bottom) * tr;
        let z = (tr - 0.5) * height + center[2];
        for c in 0..columns {
            let ang = c as f64 * (2.0 * std::f64::consts::PI / columns as f64);
            let x = rad * ang.cos() + center[0];
            let y = rad * ang.sin() + center[1];
            pts.push(x as f32);
            pts.push(y as f32);
            pts.push(z as f32);
        }
    }

    // Quads and UVs
    let quads_count = rows * columns;
    let cap_v = if caps { columns * 2 } else { 0 };
    let mut vp = Vec::with_capacity(quads_count * 4 + cap_v);
    let mut offs = Vec::with_capacity(quads_count + if caps { 2 } else { 0 } + 1);
    offs.push(0);
    let mut uv = Vec::with_capacity((quads_count * 4 + cap_v) * 2);

    for r in 0..rows {
        for c in 0..columns {
            let r1 = r + 1;
            let c1 = (c + 1) % columns;
            let q0 = (r * columns + c) as i32;
            let q1 = (r * columns + c1) as i32;
            let q2 = (r1 * columns + c1) as i32;
            let q3 = (r1 * columns + c) as i32;
            vp.extend([q0, q1, q2, q3]);
            offs.push(vp.len() as i32);

            let u0 = c as f64 / columns as f64;
            let u1 = (c + 1) as f64 / columns as f64;
            let v0 = r as f64 / rows as f64;
            let v1 = (r + 1) as f64 / rows as f64;
            uv.extend([u0 as f32, v0 as f32, u1 as f32, v0 as f32, u1 as f32, v1 as f32, u0 as f32, v1 as f32]);
        }
    }

    if caps {
        // Bottom cap: np.arange(columns)[::-1] (faces -Z)
        for i in (0..columns).rev() {
            vp.push(i as i32);
            let ang = i as f64 * (2.0 * std::f64::consts::PI / columns as f64);
            uv.push((ang.cos() * 0.5 + 0.5) as f32);
            uv.push((ang.sin() * 0.5 + 0.5) as f32);
        }
        offs.push(vp.len() as i32);

        // Top cap: rows * columns + np.arange(columns) (faces +Z)
        let top_base = (rows * columns) as i32;
        for i in 0..columns {
            vp.push(top_base + i as i32);
            let ang = i as f64 * (2.0 * std::f64::consts::PI / columns as f64);
            uv.push((ang.cos() * 0.5 + 0.5) as f32);
            uv.push((ang.sin() * 0.5 + 0.5) as f32);
        }
        offs.push(vp.len() as i32);
    }

    let mut g = Geometry::from_arrays(pts, vp, offs, None)?;
    g.vertex.insert("uv".into(), Arc::new(Attr::f32(2, uv)));

    if radius_bottom.min(radius_top) <= 0.0 {
        let weld_dist = 1e-7 * 1.0f64.max(radius_bottom).max(radius_top).max(height);
        g = drop_degenerate(&fuse_points(&g, weld_dist));
    }

    let mut transformed_p = g.p().to_vec();
    source_transform(&mut transformed_p, center, rotate, scale, uniform_scale);
    g.set_p(transformed_p);
    Ok(g)
}

pub const TORUS: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "torus",
    inputs: &[],
    params: crate::params!(
        "radius": 1.0; "thickness": 0.25; "center": [0.0, 0.0, 0.0];
        "rotate": [0.0, 0.0, 0.0]; "scale": [1.0, 1.0, 1.0]; "uniform_scale": 1.0;
        "rows": 12; "columns": 32;
    ),
    cook: torus,
};

fn torus(_inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let radius = p.f("radius")?;
    let thickness = p.f("thickness")?;
    let center = p.v3("center")?;
    let rotate = p.v3("rotate")?;
    let scale = p.v3("scale")?;
    let uniform_scale = p.f("uniform_scale")?;
    let rows = p.i("rows")?;
    let columns = p.i("columns")?;

    if !(3..=4096).contains(&rows) || !(3..=4096).contains(&columns) {
        return Err(Error::new("torus rows and columns must be between 3 and 4096"));
    }

    let (rows, columns) = (rows as usize, columns as usize);
    let npoints = rows * columns;
    let mut pts = Vec::with_capacity(npoints * 3);

    for r in 0..rows {
        let phi = r as f64 * (2.0 * std::f64::consts::PI / rows as f64);
        let w = radius + thickness * phi.cos();
        let z = thickness * phi.sin() + center[2];
        for c in 0..columns {
            let th = c as f64 * (2.0 * std::f64::consts::PI / columns as f64);
            let x = w * th.cos() + center[0];
            let y = w * th.sin() + center[1];
            pts.push(x as f32);
            pts.push(y as f32);
            pts.push(z as f32);
        }
    }

    let quads_count = rows * columns;
    let mut vp = Vec::with_capacity(quads_count * 4);
    let mut offs = Vec::with_capacity(quads_count + 1);
    offs.push(0);
    let mut uv = Vec::with_capacity(quads_count * 4 * 2);

    for r in 0..rows {
        for c in 0..columns {
            let r1 = (r + 1) % rows;
            let c1 = (c + 1) % columns;
            let q0 = (r * columns + c) as i32;
            let q1 = (r * columns + c1) as i32;
            let q2 = (r1 * columns + c1) as i32;
            let q3 = (r1 * columns + c) as i32;
            vp.extend([q0, q1, q2, q3]);
            offs.push(vp.len() as i32);

            let u0 = c as f64 / columns as f64;
            let u1 = (c + 1) as f64 / columns as f64;
            let v0 = r as f64 / rows as f64;
            let v1 = (r + 1) as f64 / rows as f64;
            uv.extend([u0 as f32, v0 as f32, u1 as f32, v0 as f32, u1 as f32, v1 as f32, u0 as f32, v1 as f32]);
        }
    }

    source_transform(&mut pts, center, rotate, scale, uniform_scale);
    let mut g = Geometry::from_arrays(pts, vp, offs, None)?;
    g.vertex.insert("uv".into(), Arc::new(Attr::f32(2, uv)));
    Ok(g)
}
