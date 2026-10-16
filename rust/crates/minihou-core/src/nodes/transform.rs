//! `peak` and `copy_transform` from minihou/nodes/transform.py.
use std::sync::Arc;

use super::{GEO, InputSpec, Inputs, NodeSpec};
use crate::error::{Error, Result};
use crate::geometry::{Attr, Geometry};
use crate::math;
use crate::params::Params;

pub const PEAK: NodeSpec = NodeSpec {
    name: "peak",
    inputs: GEO,
    params: crate::params!(
        "distance": 0.1;
        "group": "";
    ),
    cook: peak,
};

fn peak(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = PEAK.one(inputs, 0)?;
    let distance = p.f("distance")? as f32;
    let group = p.s("group")?;
    let sel = super::group::mask(&geo, "point", &group)?;

    let has_n = geo.point.get("N").filter(|a| a.width == 3).and_then(|a| a.as_f32()).is_some();
    let normals = if has_n {
        geo.point["N"].as_f32().unwrap().to_vec()
    } else {
        crate::normals::point_normals(geo.p(), &geo.topology, geo.npoints)
    };

    let p_slice = geo.p();
    let mut new_p = p_slice.to_vec();
    for i in 0..geo.npoints {
        if sel[i] {
            new_p[i * 3] += normals[i * 3] * distance;
            new_p[i * 3 + 1] += normals[i * 3 + 1] * distance;
            new_p[i * 3 + 2] += normals[i * 3 + 2] * distance;
        }
    }

    let mut out = (*geo).clone();
    out.set_p(new_p);
    Ok(out)
}

pub const COPY_TRANSFORM: NodeSpec = NodeSpec {
    name: "copy_transform",
    inputs: GEO,
    params: crate::params!(
        "copies": 2;
        "translate": [1.0, 0.0, 0.0];
        "rotate": [0.0, 0.0, 0.0];
        "scale": 1.0;
        "pivot": [0.0, 0.0, 0.0];
    ),
    cook: copy_transform,
};

fn copy_transform(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = COPY_TRANSFORM.one(inputs, 0)?;
    let copies = p.i("copies")?;
    if copies < 1 {
        return Err(Error::new("copies must be at least 1"));
    }
    let translate = p.v3("translate")?;
    let rotate = p.v3("rotate")?;
    let scale = p.f("scale")?;
    let pivot = p.v3("pivot")?;

    let r_base = math::euler_xyz(rotate);
    let mut r_mat = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r_mat[i][j] = r_base[i][j] * scale;
        }
    }

    let p0 = geo.p();
    let n0 = geo.point.get("N").filter(|a| a.width == 3).and_then(|a| a.as_f32());

    let mut m = math::IDENTITY;
    let mut off = [0.0; 3];
    let mut parts: Vec<Arc<Geometry>> = Vec::with_capacity(copies as usize);

    for i in 0..copies {
        let mut new_p = Vec::with_capacity(geo.npoints * 3);
        for pt_idx in 0..geo.npoints {
            let px = p0[pt_idx * 3] as f64 - pivot[0];
            let py = p0[pt_idx * 3 + 1] as f64 - pivot[1];
            let pz = p0[pt_idx * 3 + 2] as f64 - pivot[2];
            let applied = math::apply(&m, [px, py, pz]);
            new_p.push((applied[0] + pivot[0] + off[0]) as f32);
            new_p.push((applied[1] + pivot[1] + off[1]) as f32);
            new_p.push((applied[2] + pivot[2] + off[2]) as f32);
        }

        let mut g = (*geo).clone();
        g.set_p(new_p);

        if let Some(src_n) = n0 {
            let mut new_n = Vec::with_capacity(geo.npoints * 3);
            for pt_idx in 0..geo.npoints {
                let nx = src_n[pt_idx * 3] as f64;
                let ny = src_n[pt_idx * 3 + 1] as f64;
                let nz = src_n[pt_idx * 3 + 2] as f64;
                let applied = math::apply(&m, [nx, ny, nz]);
                let len = (applied[0] * applied[0] + applied[1] * applied[1] + applied[2] * applied[2]).sqrt().max(1e-30);
                new_n.push((applied[0] / len) as f32);
                new_n.push((applied[1] / len) as f32);
                new_n.push((applied[2] / len) as f32);
            }
            g.point.insert("N".to_string(), Arc::new(Attr::f32(3, new_n)));
        }

        g.point.insert("copynum".to_string(), Arc::new(Attr::i32(1, vec![i as i32; geo.npoints])));
        parts.push(Arc::new(g));

        // next = T(R(previous))
        m = math::mul(&r_mat, &m);
        let roff = math::apply(&r_mat, off);
        off = [roff[0] + translate[0], roff[1] + translate[1], roff[2] + translate[2]];
    }

    if parts.len() == 1 {
        return Ok((*parts[0]).clone());
    }

    let merge_params = Params::resolve(super::merge::MERGE.params, None)?;
    super::merge::merge(&[parts], &merge_params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peak_displaces_along_normals() {
        let pts = vec![
            0.0, 0.0, 0.0,
            1.0, 0.0, 0.0,
            1.0, 1.0, 0.0,
            0.0, 1.0, 0.0,
        ];
        let vp = vec![0, 1, 2, 3];
        let offs = vec![0, 4];
        let mut geo = Geometry::from_arrays(pts, vp, offs, None).unwrap();
        geo.point.insert("N".to_string(), Arc::new(Attr::f32(3, [0.0, 0.0, 1.0].repeat(4))));

        let p = Params::resolve(PEAK.params, serde_json::json!({"distance": 2.0}).as_object()).unwrap();
        let out = peak(&[vec![Arc::new(geo)]], &p).unwrap();
        let p_out = out.p();
        assert_eq!(p_out[2], 2.0);
        assert_eq!(p_out[5], 2.0);
        assert_eq!(p_out[8], 2.0);
        assert_eq!(p_out[11], 2.0);
    }

    #[test]
    fn test_copy_transform_accumulates() {
        let geo = Geometry::points_only(vec![0.5, 0.0, 0.0]).unwrap();
        let p = Params::resolve(COPY_TRANSFORM.params, serde_json::json!({
            "copies": 4,
            "translate": [0.0, 0.0, 0.2],
            "rotate": [0.0, 0.0, 90.0]
        }).as_object()).unwrap();

        let out = copy_transform(&[vec![Arc::new(geo)]], &p).unwrap();
        assert_eq!(out.npoints, 4);
        let copynum = match &out.point["copynum"].data {
            crate::geometry::AttrData::I32(v) => v.clone(),
            _ => panic!("expected i32"),
        };
        assert_eq!(copynum, vec![0, 1, 2, 3]);

        let p_out = out.p();
        // copy 0: [0.5, 0.0, 0.0]
        assert!((p_out[0] - 0.5).abs() < 1e-5);
        assert!(p_out[1].abs() < 1e-5);
        assert!(p_out[2].abs() < 1e-5);

        // copy 1: [0.0, 0.5, 0.2]
        assert!(p_out[3].abs() < 1e-5);
        assert!((p_out[4] - 0.5).abs() < 1e-5);
        assert!((p_out[5] - 0.2).abs() < 1e-5);
    }
}
