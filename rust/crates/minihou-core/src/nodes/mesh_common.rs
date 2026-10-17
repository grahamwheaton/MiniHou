//! Helpers shared by the Blender-audit mesh nodes (`grid_fill`, `recalc_normals`, `tris_to_quads`, extended `inset`).
//! See docs/RUST_MESH_NODES.md.
use std::sync::Arc;

use super::{material_tables, model};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry, Topology};

/// A point to append: its position and the old points (with weights) its other attributes blend.
pub(super) struct NewPoint {
    pub pos: [f64; 3],
    pub src: Vec<(usize, f64)>,
}

/// Appends points and closed polygons to `g`. `faces` index points (old ones first, then `pts` in order). `corner_src[f][c]`
/// lists old corners whose vertex attributes are averaged for corner `c` of face `f` (empty = zero). New faces get a zero
/// primitive row, the neutral material slot and NaN corner UVs (docs/RUST_MATERIALS_SCHEMA.md).
pub(super) fn append(g: &Geometry, pts: &[NewPoint], faces: &[Vec<usize>], corner_src: &[Vec<Vec<usize>>]) -> Result<Geometry> {
    let mut out = g.clone();
    let np = g.npoints;
    let pos = g.p();
    if pos.iter().any(|x| !x.is_finite()) { return Err(Error::new("input has non-finite point positions")); }
    for p in pts {
        if p.pos.iter().any(|x| !x.is_finite()) { return Err(Error::new("generated a non-finite point position")); }
    }
    out.npoints = np + pts.len();
    out.point = g.point.iter().map(|(name, a)| {
        let w = a.width;
        let mut data = (**a).clone();
        let data = &mut data;
        for p in pts {
            if name == "P" {
                if let AttrData::F32(v) = &mut data.data { v.extend(p.pos.map(|x| x as f32)); }
                continue;
            }
            let total: f64 = p.src.iter().map(|s| s.1).sum();
            match &mut data.data {
                AttrData::F32(v) => {
                    for c in 0..w {
                        let x: f64 = if total.abs() > 1e-12 { p.src.iter().map(|&(i, wt)| v[i * w + c] as f64 * wt).sum::<f64>() / total } else { 0. };
                        v.push(x as f32);
                    }
                }
                AttrData::I32(v) => {
                    for c in 0..w {
                        let x = if name.starts_with("group_") { p.src.iter().all(|&(i, _)| v[i * w + c] != 0) && !p.src.is_empty() } else { false };
                        let first = p.src.first().map_or(0, |&(i, _)| v[i * w + c]);
                        v.push(if name.starts_with("group_") { x as i32 } else { first });
                    }
                }
            }
        }
        (name.clone(), Arc::new(data.clone()))
    }).collect();
    let first_prim = g.nprims();
    let first_corner = g.nvertices();
    let mut t: Topology = (*g.topology).clone();
    for f in faces {
        t.vertex_point.extend(f.iter().map(|&x| x as i32));
        t.prim_offsets.push(t.vertex_point.len() as i32);
    }
    if let Some(c) = &mut t.closed { c.extend(std::iter::repeat_n(true, faces.len())); }
    out.topology = Arc::new(t);
    for (name, a) in &g.vertex {
        let w = a.width;
        let mut na = (**a).clone();
        for srcs in corner_src.iter().flatten() {
            match &mut na.data {
                AttrData::F32(v) => for c in 0..w {
                    let x = if srcs.is_empty() { 0. } else { srcs.iter().map(|&i| a.as_f32().unwrap()[i * w + c] as f64).sum::<f64>() / srcs.len() as f64 };
                    v.push(x as f32);
                },
                AttrData::I32(v) => for c in 0..w { v.push(srcs.first().map_or(0, |&i| a.as_i32().unwrap()[i * w + c])); },
            }
        }
        out.vertex.insert(name.clone(), Arc::new(na));
    }
    for (name, a) in &g.prim {
        let mut na = (**a).clone();
        match &mut na.data {
            AttrData::F32(v) => v.extend(std::iter::repeat_n(0., faces.len() * a.width)),
            AttrData::I32(v) => v.extend(std::iter::repeat_n(0, faces.len() * a.width)),
        }
        out.prim.insert(name.clone(), Arc::new(na));
    }
    material_tables::neutral_new_faces(&mut out, first_prim, first_corner);
    out.validate()?;
    Ok(out)
}

/// Writes a primitive group (replacing a point group of the same name).
pub(super) fn write_prim_group(g: &mut Geometry, name: &str, mask: &[bool]) -> Result<()> {
    model::write_group(g, "prim", name, mask)
}

/// Optional output group named by `output_group` / `output_group_name`.
pub(super) fn out_group(g: &mut Geometry, p: &crate::params::Params, mask: &[bool]) -> Result<()> {
    if p.b("output_group")? {
        let name = p.s("output_group_name")?;
        let name = name.trim();
        model::group_name(name).map_err(|_| Error::new(format!("invalid output_group_name {name:?}")))?;
        write_prim_group(g, name, mask)?;
    }
    Ok(())
}

/// Recomputes corner normals `N` (if present) as the flat normal of every face from `first_prim` on.
pub(super) fn refresh_corner_normals(g: &mut Geometry, first_prim: usize) {
    let mask: Vec<bool> = (0..g.nprims()).map(|p| p >= first_prim).collect();
    refresh_corner_normals_for(g, &mask);
}

/// Recomputes corner normals `N` (if present) as the flat normal of the faces in `mask`.
pub(super) fn refresh_corner_normals_for(g: &mut Geometry, mask: &[bool]) {
    let Some(old) = g.vertex.get("N").filter(|a| a.width == 3).and_then(|a| a.as_f32()).map(<[f32]>::to_vec) else { return };
    let fnorm = crate::normals::prim_normals(g.p(), &g.topology);
    let mut data = old;
    for p in (0..g.nprims()).filter(|&p| mask[p]) {
        let n = model::unit32(fnorm[p]).map(|x| x as f32);
        for v in model::range(g, p) { data[v * 3..v * 3 + 3].copy_from_slice(&n); }
    }
    g.vertex.insert("N".into(), Arc::new(Attr::f32(3, data)));
}
