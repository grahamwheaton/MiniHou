use std::sync::Arc;
use crate::{
    error::{Error, Result},
    geometry::{AttrData, Geometry},
    nodes::{curves::count, group::mask, Inputs, NodeSpec, GEO},
    params::Params,
};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "fillet",
    inputs: GEO,
    params: crate::params!("radius":0.2;"divisions":6;"min_angle":1.;"group":""),
    cook: fillet,
};

fn norm(v: [f64; 3]) -> f64 {
    v.iter().map(|x| x * x).sum::<f64>().sqrt()
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}

fn fillet(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    if g.nprims() == 0 {
        return Ok((*g).clone());
    }
    let radius = p.f("radius")?;
    let divisions = p.i("divisions")?;
    if !(1..=256).contains(&divisions) {
        return Err(Error::new("divisions must be 1..256"));
    }
    let min_angle = p.f("min_angle")?.max(1e-3).to_radians();
    let sel = mask(&g, "point", &p.s("group")?)?;
    let mut positions = Vec::new();
    let mut weights = Vec::<(usize, usize, usize, f64, f64, f64)>::new();
    let mut offs = vec![0];
    let mut rounded = false;
    for i in 0..g.nprims() {
        let start = g.topology.prim_offsets[i] as usize;
        let end = g.topology.prim_offsets[i + 1] as usize;
        let n = end - start;
        let cl = g.topology.is_closed(i);
        let ids: Vec<_> = g.topology.vertex_point[start..end]
            .iter()
            .map(|&v| v as usize)
            .collect();
        let q: Vec<[f64; 3]> = ids
            .iter()
            .map(|&pt| std::array::from_fn(|k| g.p()[pt * 3 + k] as f64))
            .collect();
        for j in 0..n {
            let prev = if j == 0 {
                if cl {
                    n - 1
                } else {
                    0
                }
            } else {
                j - 1
            };
            let next = if j + 1 == n {
                if cl {
                    0
                } else {
                    j
                }
            } else {
                j + 1
            };
            let a = sub(q[prev], q[j]);
            let b = sub(q[next], q[j]);
            let (la, lb) = (norm(a), norm(b));
            let interior = n >= 3 && (cl || (j > 0 && j + 1 < n));
            let ok = interior && la > 1e-12 && lb > 1e-12;
            let a = a.map(|v| v / if ok { la } else { 1. });
            let b = b.map(|v| v / if ok { lb } else { 1. });
            let theta = dot(a, b).clamp(-1., 1.).acos();
            let turn = std::f64::consts::PI - theta;
            let corner = ok && sel[ids[j]] && radius > 0. && turn >= min_angle && theta > 1e-3;
            if !corner {
                positions.extend(q[j].map(|v| v as f32));
                weights.push((ids[j], ids[prev], ids[next], 0., 0., 0.));
                continue;
            }
            rounded = true;
            let half = theta / 2.;
            let end_prev = !cl && (prev == 0 || prev + 1 == n);
            let end_next = !cl && (next == 0 || next + 1 == n);
            let room = (if end_prev { la } else { la / 2. }).min(if end_next { lb } else { lb / 2. });
            let tang = (radius / half.tan()).min(room);
            let r = tang * half.tan();
            let bis = std::array::from_fn(|k| a[k] + b[k]);
            let bn = norm(bis);
            let bis = bis.map(|v| v / if bn > 1e-12 { bn } else { 1. });
            let centre = std::array::from_fn::<_, 3, _>(|k| q[j][k] + bis[k] * (r / half.sin().max(1e-12)));
            let u1 = std::array::from_fn::<_, 3, _>(|k| (q[j][k] + a[k] * tang - centre[k]) / r.max(1e-12));
            let u2 = std::array::from_fn::<_, 3, _>(|k| (q[j][k] + b[k] * tang - centre[k]) / r.max(1e-12));
            let om = turn.max(1e-6);
            count(weights.len() + divisions as usize + 1)?;
            for k in 0..=divisions {
                let s = k as f64 / divisions as f64;
                let w1 = ((1. - s) * om).sin() / om.sin();
                let w2 = (s * om).sin() / om.sin();
                positions.extend((0..3).map(|c| (centre[c] + r * (u1[c] * w1 + u2[c] * w2)) as f32));
                weights.push((ids[j], ids[prev], ids[next], tang / la, tang / lb, s));
            }
        }
        offs.push(weights.len() as i32);
    }
    if !rounded {
        return Ok((*g).clone());
    }
    count(weights.len())?;
    let mut out = Geometry::from_arrays(
        positions,
        (0..weights.len() as i32).collect(),
        offs,
        Some(g.topology.closed_mask()),
    )?;
    for (name, attr) in &g.point {
        if name == "P" {
            continue;
        }
        let mut a = attr.zeros_like(weights.len());
        match (&attr.data, &mut a.data) {
            (AttrData::F32(src), AttrData::F32(dst)) => {
                for (i, &(ic, ip, inx, fa, fb, s)) in weights.iter().enumerate() {
                    let (fa, fb, s) = (fa as f32, fb as f32, s as f32);
                    for k in 0..a.width {
                        let v1 = src[ic * a.width + k] * (1. - fa) + src[ip * a.width + k] * fa;
                        let v2 = src[ic * a.width + k] * (1. - fb) + src[inx * a.width + k] * fb;
                        dst[i * a.width + k] = v1 * (1. - s) + v2 * s;
                    }
                }
            }
            (AttrData::I32(src), AttrData::I32(dst)) => {
                for (i, &(ic, _, _, _, _, _)) in weights.iter().enumerate() {
                    dst[i * a.width..(i + 1) * a.width]
                        .copy_from_slice(&src[ic * a.width..(ic + 1) * a.width]);
                }
            }
            _ => unreachable!(),
        }
        out.point.insert(name.clone(), Arc::new(a));
    }
    out.prim = g.prim.clone();
    out.detail = g.detail.clone();
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
