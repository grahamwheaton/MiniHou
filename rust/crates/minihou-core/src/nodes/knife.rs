//! Perspective-correct projected Knife from minihou/nodes/knife.py.
use super::{GEO, Inputs, NodeSpec, edge_tools as edges, model::*};
use crate::{
    error::{Error, Result},
    geometry::{Attr, AttrData, AttrTable, Geometry},
    params::Params,
    topo,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
const EPS: f64 = 1e-8;
type V2 = [f64; 2];
type Tri = [V2; 3];
type Weights = Vec<(usize, f64)>;
pub const KNIFE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "knife",
    inputs: GEO,
    params: crate::params!("cuts":"[]"; "cut_through":false; "snap_tolerance":10.0),
    cook: knife,
};
fn sub2(a: V2, b: V2) -> V2 {
    [a[0] - b[0], a[1] - b[1]]
}
fn add2(a: V2, b: V2) -> V2 {
    [a[0] + b[0], a[1] + b[1]]
}
fn mul2(a: V2, s: f64) -> V2 {
    a.map(|x| x * s)
}
fn dot2(a: V2, b: V2) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross2(a: V2, b: V2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn len2(a: V2) -> f64 {
    dot2(a, a).sqrt()
}
fn bary(p: V2, t: Tri) -> Option<[f64; 3]> {
    let d = cross2(sub2(t[1], t[0]), sub2(t[2], t[0]));
    if d.abs() < 1e-14 {
        return None;
    }
    let v = cross2(sub2(p, t[0]), sub2(t[2], t[0])) / d;
    let w = cross2(sub2(t[1], t[0]), sub2(p, t[0])) / d;
    Some([1. - v - w, v, w])
}
fn clip(a: V2, b: V2, t: Tri) -> Option<(V2, V2)> {
    let u = bary(a, t)?;
    let v = bary(b, t)?;
    let mut lo = 0.0f64;
    let mut hi = 1.0f64;
    for i in 0..3 {
        let d = v[i] - u[i];
        if d.abs() < EPS {
            if u[i] < -EPS {
                return None;
            }
        } else if d > 0. {
            lo = lo.max(-u[i] / d);
        } else {
            hi = hi.min(-u[i] / d);
        }
    }
    if hi - lo < 1e-8 {
        return None;
    }
    Some((add2(a, mul2(sub2(b, a), lo)), add2(a, mul2(sub2(b, a), hi))))
}
/// boolean.earclip's best-of-eight policy (different from model.py's first-ear policy).
fn earclip(xy: &[V2], mut ix: Vec<usize>) -> Vec<[usize; 3]> {
    let mut out = Vec::new();
    for _ in 0..4 * ix.len() + 16 {
        if ix.len() <= 3 {
            break;
        }
        let m = ix.len();
        let mut best: Option<(f64, usize)> = None;
        let mut found = 0;
        for k in 0..m {
            let (i0, i1, i2) = (ix[(k + m - 1) % m], ix[k], ix[(k + 1) % m]);
            let (a, b, c) = (xy[i0], xy[i1], xy[i2]);
            let area = cross2(sub2(b, a), sub2(c, a));
            if area <= 0. {
                continue;
            }
            let bad = ix.iter().any(|&j| {
                let p = xy[j];
                j != i0
                    && j != i1
                    && j != i2
                    && p != a
                    && p != b
                    && p != c
                    && cross2(sub2(b, a), sub2(p, a)) >= 0.
                    && cross2(sub2(c, b), sub2(p, b)) >= 0.
                    && cross2(sub2(a, c), sub2(p, c)) >= 0.
            });
            if bad {
                continue;
            }
            let e = dot2(sub2(b, a), sub2(b, a))
                .max(dot2(sub2(c, b), sub2(c, b)))
                .max(dot2(sub2(a, c), sub2(a, c)));
            let q = area / e.max(1e-300);
            if best.is_none_or(|(score, _)| q > score) {
                best = Some((q, k));
            }
            found += 1;
            if found >= 8 {
                break;
            }
        }
        let Some((_, k)) = best else {
            break;
        };
        out.push([
            ix[(k + ix.len() - 1) % ix.len()],
            ix[k],
            ix[(k + 1) % ix.len()],
        ]);
        ix.remove(k);
    }
    if ix.len() == 3 {
        out.push([ix[0], ix[1], ix[2]]);
    } else if ix.len() > 3 {
        for k in 1..ix.len() - 1 {
            out.push([ix[0], ix[k], ix[k + 1]]);
        }
    }
    out
}
fn cut_triangle(tri: Tri, a: V2, b: V2) -> Vec<Tri> {
    let Some((a, b)) = clip(a, b, tri) else {
        return vec![tri];
    };
    if len2(sub2(a, b)) < EPS {
        return vec![tri];
    }
    let mut boundary = tri.to_vec();
    let mut inside = Vec::new();
    for p in [a, b] {
        if boundary.iter().any(|&q| len2(sub2(p, q)) < EPS) {
            continue;
        }
        let mut inserted = false;
        for i in 0..boundary.len() {
            let q = boundary[i];
            let r = boundary[(i + 1) % boundary.len()];
            let d = sub2(r, q);
            let den = dot2(d, d);
            let t = dot2(sub2(p, q), d) / if den != 0. { den } else { 1. };
            if (-EPS..=1. + EPS).contains(&t) && len2(sub2(p, add2(q, mul2(d, t)))) < EPS {
                boundary.insert(i + 1, p);
                inserted = true;
                break;
            }
        }
        if !inserted {
            inside.push(p);
        }
    }
    if !inside.is_empty() {
        let p = inside[0];
        let made: Vec<_> = (0..boundary.len())
            .map(|i| [p, boundary[i], boundary[(i + 1) % boundary.len()]])
            .collect();
        if inside.len() > 1 {
            return made.iter().flat_map(|&t| cut_triangle(t, a, b)).collect();
        }
        return made
            .into_iter()
            .filter(|t| cross2(sub2(t[1], t[0]), sub2(t[2], t[0])).abs() > EPS)
            .collect();
    }
    let ia = (0..boundary.len())
        .min_by(|&i, &j| len2(sub2(boundary[i], a)).total_cmp(&len2(sub2(boundary[j], a))))
        .unwrap();
    let ib = (0..boundary.len())
        .min_by(|&i, &j| len2(sub2(boundary[i], b)).total_cmp(&len2(sub2(boundary[j], b))))
        .unwrap();
    let (ia, ib) = (ia.min(ib), ia.max(ib));
    let polys = if ib - ia == 1 || ib - ia == boundary.len() - 1 {
        vec![boundary]
    } else {
        vec![
            boundary[ia..=ib].to_vec(),
            boundary[ib..]
                .iter()
                .chain(&boundary[..=ia])
                .copied()
                .collect(),
        ]
    };
    let mut result = Vec::new();
    for poly in polys {
        if poly.len() < 3 {
            continue;
        }
        let sign = (0..poly.len())
            .map(|i| cross2(poly[i], poly[(i + 1) % poly.len()]))
            .sum::<f64>();
        let mut ix: Vec<_> = (0..poly.len()).collect();
        if sign < 0. {
            ix.reverse();
        }
        for t in earclip(&poly, ix) {
            let mut out = t.map(|i| poly[i]);
            if sign < 0. {
                out.reverse();
            }
            if cross2(sub2(out[1], out[0]), sub2(out[2], out[0])).abs()
                > EPS * len2(sub2(out[1], out[0])).max(len2(sub2(out[2], out[0])))
            {
                result.push(out);
            }
        }
    }
    result
}
struct Segment {
    matrix: [f64; 16],
    a: V2,
    b: V2,
    faces: Option<Vec<usize>>,
    through: bool,
}
impl Segment {
    fn project(&self, q: [f64; 3]) -> [f64; 4] {
        std::array::from_fn(|i| {
            self.matrix[i] * q[0]
                + self.matrix[4 + i] * q[1]
                + self.matrix[8 + i] * q[2]
                + self.matrix[12 + i]
        })
    }
}
fn attached_screen(point:&serde_json::Value, matrix:&serde_json::Value, positions:&[[f64;3]]) -> serde_json::Value {
    let Some(snap)=point.get("snap") else { return point["screen"].clone(); };
    if snap["point_count"].as_u64()!=Some(positions.len() as u64) { return point["screen"].clone(); }
    let Some(ids)=snap["element"].as_array() else { return point["screen"].clone(); };
    let Some(weights)=snap["params"].as_array() else { return point["screen"].clone(); };
    if ids.len()!=weights.len() || ids.is_empty() { return point["screen"].clone(); }
    let mut q=[0.;3];
    for (id,w) in ids.iter().zip(weights) { let (Some(id),Some(w))=(id.as_u64(),w.as_f64()) else {return point["screen"].clone()};
        let Some(p)=positions.get(id as usize) else {return point["screen"].clone()}; for k in 0..3 {q[k]+=p[k]*w;} }
    let Some(m)=matrix.as_array().filter(|m|m.len()==16) else {return point["screen"].clone()};
    let clip=|row:usize| (0..3).map(|k|m[k*4+row].as_f64().unwrap_or(0.)*q[k]).sum::<f64>()+m[12+row].as_f64().unwrap_or(0.);
    let w=clip(3); if w.abs()<EPS {return point["screen"].clone();}
    serde_json::json!([clip(0)/w,clip(1)/w])
}
fn parse(text: &str, positions: Option<&[[f64;3]]>) -> Result<Vec<Segment>> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| Error::new(format!("Invalid Knife cuts: {e}")))?;
    let array = v
        .as_array()
        .ok_or_else(|| Error::new("Knife cuts must be a list"))?;
    if array.len() > 512 {
        return Err(Error::new(
            "Knife supports up to 512 segments per operation.",
        ));
    }
    let mut out = Vec::new();
    for chain in array {
      let segments: Vec<serde_json::Value> = if let Some(points)=chain.get("points").and_then(|v|v.as_array()) {
          points.windows(2).map(|ab| serde_json::json!({"a":positions.map_or_else(||ab[0]["screen"].clone(),|p|attached_screen(&ab[0],&chain["matrix"],p)),"b":positions.map_or_else(||ab[1]["screen"].clone(),|p|attached_screen(&ab[1],&chain["matrix"],p)),"matrix":chain["matrix"],"through":chain.get("through").and_then(|v|v.as_bool()).unwrap_or(false)})).collect()
      } else { vec![chain.clone()] };
      for v in &segments {
        if out.len() >= 512 { return Err(Error::new("Knife supports up to 512 segments per operation.")); }
        let nums = |name: &str, count: usize| -> Result<Vec<f64>> {
            let a = v
                .get(name)
                .and_then(|x| x.as_array())
                .filter(|a| a.len() == count)
                .ok_or_else(|| Error::new("Invalid Knife projection."))?;
            a.iter()
                .map(|x| {
                    x.as_f64()
                        .filter(|x| x.is_finite())
                        .ok_or_else(|| Error::new("Invalid Knife projection."))
                })
                .collect()
        };
        let m = nums("matrix", 16)?;
        let a = nums("a", 2)?;
        let b = nums("b", 2)?;
        let faces = match v.get("faces") {
            None | Some(serde_json::Value::Null) => None,
            Some(x) => Some(
                x.as_array()
                    .ok_or_else(|| Error::new("invalid Knife face selection"))?
                    .iter()
                    .map(|x| {
                        x.as_u64()
                            .map(|i| i as usize)
                            .ok_or_else(|| Error::new("invalid Knife face selection"))
                    })
                    .collect::<Result<Vec<_>>>()?,
            ),
        };
        out.push(Segment {
            matrix: m.try_into().unwrap(),
            a: [a[0], a[1]],
            b: [b[0], b[1]],
            faces,
            through: v.get("through").and_then(|v| v.as_bool()).unwrap_or(false),
        });
      }
    }
    Ok(out)
}
fn knife(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = KNIFE.one(inputs, 0)?;
    polygons(&g, "Knife")?;
    let pos = positions(&g);
    let mut segments = parse(&p.s("cuts")?,Some(&pos))?;
    if p.b("cut_through")? { for segment in &mut segments { segment.through=true; } }
    if segments.is_empty() {
        return Ok((*g).clone());
    }
    let vp = &g.topology.vertex_point;
    let mut original = Vec::new();
    for f in 0..g.nprims() {
        let cs: Vec<_> = range(&g, f).collect();
        let q: Vec<_> = cs.iter().map(|&v| pos[vp[v] as usize]).collect();
        let mut normal = [0.; 3];
        for i in 0..q.len() {
            normal = add(normal, cross(q[i], q[(i + 1) % q.len()]));
        }
        let drop = (0..3)
            .max_by(|&i, &j| normal[i].abs().total_cmp(&normal[j].abs()).then(j.cmp(&i)))
            .unwrap();
        let axes: Vec<_> = (0..3).filter(|&k| k != drop).collect();
        let xy: Vec<_> = q.iter().map(|q| [q[axes[0]], q[axes[1]]]).collect();
        let reverse = (0..xy.len())
            .map(|i| cross2(xy[i], xy[(i + 1) % xy.len()]))
            .sum::<f64>()
            < 0.;
        let mut ids: Vec<_> = (0..q.len()).collect();
        if reverse {
            ids.reverse();
        }
        for mut t in earclip(&xy, ids) {
            if reverse {
                t.reverse();
            }
            original.push((f, t.map(|i| cs[i])));
        }
    }
    let mut rows = vec![vec![[[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]]; original.len()];
    let mut touched = BTreeSet::new();
    for seg in &segments {
        let projected: Vec<_> = pos.iter().map(|&q| seg.project(q)).collect();
        let w: Vec<_> = projected.iter().map(|q| q[3]).collect();
        let screen: Vec<_> = projected
            .iter()
            .map(|q| {
                let d = if q[3].abs() > EPS { q[3] } else { EPS };
                [q[0] / d, q[1] / d]
            })
            .collect();
        let depth: Vec<_> = projected
            .iter()
            .map(|q| q[2] / if q[3].abs() > EPS { q[3] } else { EPS })
            .collect();
        if len2(sub2(seg.a, seg.b)) < EPS {
            continue;
        }
        let mut candidates = Vec::new();
        for &(other, cs) in &original {
            let ids = cs.map(|c| vp[c] as usize);
            let pts = ids.map(|i| screen[i]);
            if ids.iter().any(|&i| w[i] <= EPS) {
                continue;
            }
            if (0..2).any(|k| {
                pts.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max)
                    < seg.a[k].min(seg.b[k]) - EPS
                    || pts.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min)
                        > seg.a[k].max(seg.b[k]) + EPS
            }) {
                continue;
            }
            if let Some(interval) = clip(seg.a, seg.b, pts) {
                candidates.push((other, cs, interval));
            }
        }
        for (ti, &(face, cs)) in original.iter().enumerate() {
            if seg.faces.as_ref().is_some_and(|ids| !ids.contains(&face)) {
                continue;
            }
            let ids = cs.map(|c| vp[c] as usize);
            if ids.iter().any(|&i| w[i] <= EPS) {
                continue;
            }
            let base = ids.map(|i| screen[i]);
            let Some(clipped) = clip(seg.a, seg.b, base) else {
                continue;
            };
            let pieces = if seg.through {
                vec![clipped]
            } else {
                let delta = sub2(seg.b, seg.a);
                let den = dot2(delta, delta);
                let mut cuts = vec![0., 1.];
                for &(other, _, (a, b)) in &candidates {
                    if other != face {
                        for p in [a, b] {
                            let t = (dot2(sub2(p, seg.a), delta) / den).clamp(0., 1.);
                            cuts.push((t * 1e10).round_ties_even() / 1e10);
                        }
                    }
                }
                cuts.sort_by(f64::total_cmp);
                cuts.dedup();
                let mut pieces = Vec::new();
                for c in cuts.windows(2) {
                    let Some(hit) = clip(
                        add2(seg.a, mul2(delta, c[0])),
                        add2(seg.a, mul2(delta, c[1])),
                        base,
                    ) else {
                        continue;
                    };
                    let mid = mul2(add2(hit.0, hit.1), 0.5);
                    let bw = bary(mid, base).unwrap();
                    let z = (0..3).map(|k| bw[k] * depth[ids[k]]).sum::<f64>();
                    let occluded = candidates.iter().any(|&(other, cs, _)| {
                        if other == face {
                            return false;
                        }
                        let ids = cs.map(|c| vp[c] as usize);
                        if let Some(bw) = bary(mid, ids.map(|i| screen[i])) {
                            bw.iter().all(|&v| v >= -EPS)
                                && (0..3).map(|k| bw[k] * depth[ids[k]]).sum::<f64>() < z - 1e-7
                        } else {
                            false
                        }
                    });
                    if !occluded {
                        pieces.push(hit);
                    }
                }
                pieces
            };
            for (start, end) in pieces {
                let mut made = Vec::new();
                for tri in &rows[ti] {
                    let q = tri.map(|bw| {
                        let mut q = [0.; 3];
                        for i in 0..3 {
                            q = add(q, mul(pos[ids[i]], bw[i]));
                        }
                        q
                    });
                    let h = q.map(|q| seg.project(q));
                    let xy = h.map(|q| [q[0] / q[3], q[1] / q[3]]);
                    if clip(start, end, xy).is_none() {
                        made.push(*tri);
                        continue;
                    }
                    let parts = cut_triangle(xy, start, end);
                    touched.insert(face);
                    for part in parts {
                        let mut weights = [[0.; 3]; 3];
                        for i in 0..3 {
                            let mut bc=bary(part[i],xy).ok_or_else(||Error::new("Knife encountered a degenerate projected face; change the view or remove zero-area faces"))?;
                            for j in 0..3 {
                                bc[j] /= h[j][3];
                            }
                            let total = bc.iter().sum::<f64>();
                            if !total.is_finite() || total.abs() < 1e-14 {
                                return Err(Error::new(
                                    "Knife intersection lies on the camera plane; change the view",
                                ));
                            }
                            for j in 0..3 {
                                bc[j] /= total;
                            }
                            for j in 0..3 {
                                weights[i] = add(weights[i], mul(tri[j], bc[j]));
                            }
                        }
                        made.push(weights);
                    }
                }
                rows[ti] = made;
            }
        }
    }
    if touched.is_empty() {
        return Ok((*g).clone());
    }
    let mut positions = pos.clone();
    let mut pointweights: Vec<Weights> = (0..g.npoints).map(|i| vec![(i, 1.)]).collect();
    let mut supports: BTreeMap<Vec<usize>, Vec<usize>> =
        (0..g.npoints).map(|i| (vec![i], vec![i])).collect();
    let mut faces = Vec::<Vec<usize>>::new();
    let mut sources = Vec::new();
    let mut vertexweights = Vec::<Vec<Weights>>::new();
    for (ti, &(face, cs)) in original.iter().enumerate() {
        if !touched.contains(&face) {
            continue;
        }
        for tri in &rows[ti] {
            let mut f = Vec::new();
            let mut vw = Vec::new();
            for &bw in tri {
                let mut mapping = Vec::<(usize, f64)>::new();
                for (&c, &v) in cs.iter().zip(&bw) {
                    if v.abs() > 1e-7 {
                        let q = vp[c] as usize;
                        if let Some((_, w)) = mapping.iter_mut().find(|(i, _)| *i == q) {
                            *w += v;
                        } else {
                            mapping.push((q, v));
                        }
                    }
                }
                let total = mapping.iter().map(|r| r.1).sum::<f64>();
                for r in &mut mapping {
                    r.1 /= total;
                }
                let mut support: Vec<_> = mapping.iter().map(|r| r.0).collect();
                support.sort();
                let existing = supports.get(&support).and_then(|ids| {
                    ids.iter().copied().find(|&i| {
                        mapping
                            .iter()
                            .all(|&(q, w)| (w - weight(&pointweights[i], q)).abs() < 2e-7)
                    })
                });
                let id = if let Some(i) = existing {
                    i
                } else {
                    let i = positions.len();
                    let q = mapping
                        .iter()
                        .map(|&(q, w)| mul(pos[q], w))
                        .fold([0.; 3], add);
                    positions.push(q);
                    pointweights.push(mapping);
                    supports.entry(support).or_default().push(i);
                    i
                };
                f.push(id);
                vw.push(
                    cs.iter()
                        .zip(&bw)
                        .filter(|(_, v)| v.abs() > 1e-7)
                        .map(|(&c, &v)| (c, v))
                        .collect(),
                );
            }
            faces.push(f);
            sources.push(face);
            vertexweights.push(vw);
        }
    }
    for face in 0..g.nprims() {
        if !touched.contains(&face) {
            let cs: Vec<_> = range(&g, face).collect();
            faces.push(cs.iter().map(|&c| vp[c] as usize).collect());
            sources.push(face);
            vertexweights.push(cs.iter().map(|&c| vec![(c, 1.)]).collect());
        }
    }
    let mut edgepoints = BTreeMap::<(usize, usize), Vec<usize>>::new();
    for (i, w) in pointweights.iter().enumerate() {
        if w.len() == 2 {
            edgepoints.entry(pair(w[0].0, w[1].0)).or_default().push(i);
        }
    }
    for (fi, face) in faces.iter_mut().enumerate() {
        let mut expanded = Vec::new();
        let mut attrs = Vec::new();
        for j in 0..face.len() {
            let a = face[j];
            let b = face[(j + 1) % face.len()];
            expanded.push(a);
            attrs.push(vertexweights[fi][j].clone());
            if a >= g.npoints || b >= g.npoints {
                continue;
            }
            let mut ids = edgepoints.get(&pair(a, b)).cloned().unwrap_or_default();
            ids.sort_by(|&i, &j| {
                weight(&pointweights[i], b).total_cmp(&weight(&pointweights[j], b))
            });
            for id in ids {
                let t = weight(&pointweights[id], b);
                expanded.push(id);
                attrs.push(mix(
                    &vertexweights[fi][j],
                    &vertexweights[fi][(j + 1) % face.len()],
                    t,
                ));
            }
        }
        *face = expanded;
        vertexweights[fi] = attrs;
    }
    let mut facepoints = BTreeMap::<usize, BTreeSet<usize>>::new();
    for (face, &source) in faces.iter().zip(&sources) {
        facepoints.entry(source).or_default().extend(face);
    }
    let mut extent = 0.0f64;
    for k in 0..3 {
        extent = extent.max(
            pos.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max)
                - pos.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min),
        );
    }
    let tolerance = (extent * 1e-8).max(1e-12);
    for (fi, face) in faces.iter_mut().enumerate() {
        let mut expanded = Vec::new();
        let mut attrs = Vec::new();
        for j in 0..face.len() {
            let a = face[j];
            let b = face[(j + 1) % face.len()];
            expanded.push(a);
            attrs.push(vertexweights[fi][j].clone());
            let delta = sub(positions[b], positions[a]);
            let den = dot(delta, delta);
            if den <= tolerance * tolerance {
                continue;
            }
            let mut on = Vec::new();
            for &id in &facepoints[&sources[fi]] {
                let relative = sub(positions[id], positions[a]);
                let t = dot(relative, delta) / den;
                if t > 1e-8 && t < 1. - 1e-8 && len(sub(relative, mul(delta, t))) < tolerance {
                    on.push((t, id));
                }
            }
            on.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (t, id) in on {
                expanded.push(id);
                attrs.push(mix(
                    &vertexweights[fi][j],
                    &vertexweights[fi][(j + 1) % face.len()],
                    t,
                ));
            }
        }
        *face = expanded;
        vertexweights[fi] = attrs;
    }
    let protected_in = |positions: &[[f64; 3]], a: usize, b: usize| {
        segments.iter().any(|seg| {
            let q = [seg.project(positions[a]), seg.project(positions[b])];
            if q.iter().any(|q| q[3] <= EPS) {
                return false;
            }
            let xy = q.map(|q| [q[0] / q[3], q[1] / q[3]]);
            let d = sub2(seg.b, seg.a);
            let den = dot2(d, d);
            den >= EPS
                && xy.iter().all(|&p| {
                    cross2(d, sub2(p, seg.a)).abs() < 1e-7
                        && (-EPS..=1. + EPS).contains(&(dot2(sub2(p, seg.a), d) / den))
                })
        })
    };
    let protected = |a: usize, b: usize| protected_in(&positions, a, b);
    loop {
        let mut owners = BTreeMap::<(usize, usize), Vec<usize>>::new();
        let mut order = Vec::new();
        for (i, f) in faces.iter().enumerate() {
            for j in 0..f.len() {
                let k = pair(f[j], f[(j + 1) % f.len()]);
                if !owners.contains_key(&k) {
                    order.push(k);
                }
                owners.entry(k).or_default().push(i);
            }
        }
        let mut changed = false;
        for (a, b) in order {
            let ii = &owners[&(a, b)];
            if ii.len() != 2 {
                continue;
            }
            let (i, j) = (ii[0], ii[1]);
            if sources[i] != sources[j] || protected(a, b) {
                continue;
            }
            let mut boundary = BTreeMap::new();
            let mut weights = BTreeMap::new();
            let mut start = None;
            let mut valid = true;
            for k in [i, j] {
                let f = &faces[k];
                for n in 0..f.len() {
                    let u = f[n];
                    let v = f[(n + 1) % f.len()];
                    weights.insert(u, vertexweights[k][n].clone());
                    if pair(u, v) == pair(a, b) {
                        continue;
                    }
                    if boundary.insert(u, v).is_some() {
                        valid = false;
                        break;
                    }
                    if start.is_none() {
                        start = Some(u);
                    }
                }
                if !valid {
                    break;
                }
            }
            if !valid || boundary.is_empty() {
                continue;
            }
            let start = start.unwrap();
            let mut at = start;
            let mut merged = Vec::new();
            while boundary.contains_key(&at) && !merged.contains(&at) {
                merged.push(at);
                at = boundary[&at];
            }
            if at != start || merged.len() != boundary.len() || merged.len() < 3 {
                continue;
            }
            vertexweights[i] = merged.iter().map(|i| weights[i].clone()).collect();
            faces[i] = merged;
            faces.remove(j);
            sources.remove(j);
            vertexweights.remove(j);
            changed = true;
            break;
        }
        if !changed {
            break;
        }
    }
    // Triangulation diagonals are an artefact: a crossing on a diagonal that is not an original
    // edge, sitting collinear between its face neighbours, is not a vertex the user asked for.
    {
        let original_edges: BTreeSet<(usize, usize)> = topo::edges(&g.topology, g.npoints)
            .points.chunks_exact(2).map(|p| pair(p[0] as usize, p[1] as usize)).collect();
        let on_diagonal = |i: usize| i >= g.npoints && pointweights[i].len() == 2
            && !original_edges.contains(&pair(pointweights[i][0].0, pointweights[i][1].0));
        for (fi, face) in faces.iter_mut().enumerate() {
            let mut keep = Vec::new();
            for j in 0..face.len() {
                let (prev, cur, next) = (face[(j + face.len() - 1) % face.len()], face[j], face[(j + 1) % face.len()]);
                let (u, v) = (sub(positions[cur], positions[prev]), sub(positions[next], positions[cur]));
                let collinear = len(cross(u, v)) <= 1e-9 * len(u).max(1e-30) * len(v).max(1e-30) && dot(u, v) > 0.;
                keep.push(!(on_diagonal(cur) && collinear));
            }
            if keep.iter().filter(|k| **k).count() < 3 { continue; }
            let mut j = 0;
            face.retain(|_| { j += 1; keep[j - 1] });
            let mut j = 0;
            vertexweights[fi].retain(|_| { j += 1; keep[j - 1] });
        }
        let mut used = vec![false; positions.len()];
        for &i in faces.iter().flatten() { used[i] = true; }
        let mut remap = vec![usize::MAX; positions.len()];
        let mut next = 0;
        for i in 0..positions.len() {
            if i < g.npoints || used[i] { remap[i] = next; next += 1; }
        }
        if next < positions.len() {
            let keep = |i: usize| remap[i] != usize::MAX;
            positions = positions.iter().enumerate().filter(|(i, _)| keep(*i)).map(|(_, p)| *p).collect();
            pointweights = pointweights.into_iter().enumerate().filter(|(i, _)| keep(*i)).map(|(_, w)| w).collect();
            for f in faces.iter_mut() { for i in f.iter_mut() { *i = remap[*i]; } }
        }
    }
    let protected = |a: usize, b: usize| protected_in(&positions, a, b);
    let mut point = interpolate(&g.point, &pointweights);
    point.insert("point_group_knife".into(), Arc::new(Attr::i32(1,
        (0..pointweights.len()).map(|i| i32::from(i>=g.npoints)).collect())));
    // _build.make takes P from its explicit positions argument, not the attribute table.
    point.insert(
        "P".into(),
        Arc::new(Attr::f32(
            3,
            positions.iter().flat_map(|q| q.map(|x| x as f32)).collect(),
        )),
    );
    let vw: Vec<_> = vertexweights.into_iter().flatten().collect();
    let mut out = make(
        point,
        faces.iter().flatten().map(|&i| i as i32).collect(),
        &faces.iter().map(Vec::len).collect::<Vec<_>>(),
        interpolate(&g.vertex, &vw),
        gather(&g.prim, &sources),
        g.detail.clone(),
        None,
    )?;
    let old = topo::edges(&g.topology, g.npoints);
    let new = topo::edges(&out.topology, out.npoints);
    let lookup: BTreeMap<_, _> = old
        .points
        .chunks_exact(2)
        .enumerate()
        .map(|(i, p)| ((p[0] as usize, p[1] as usize), i))
        .collect();
    let origin: Vec<_> = new
        .points
        .chunks_exact(2)
        .map(|q| {
            let support: BTreeSet<_> = pointweights[q[0] as usize]
                .iter()
                .chain(&pointweights[q[1] as usize])
                .map(|w| w.0)
                .collect();
            if support.len() == 2 {
                let ids: Vec<_> = support.into_iter().collect();
                lookup.get(&(ids[0], ids[1])).copied()
            } else {
                None
            }
        })
        .collect();
    for (name, a) in &g.vertex {
        if name.starts_with("edge_group_") || name == "creaseweight" {
            let old = edges::values(&g, name)?;
            edges::write_values(
                &mut out,
                name,
                &origin
                    .iter()
                    .map(|i| i.map_or(0., |i| old[i]))
                    .collect::<Vec<_>>(),
                matches!(a.data, AttrData::I32(_)),
            )?;
        }
    }
    let cut: Vec<_> = new
        .points
        .chunks_exact(2)
        .enumerate()
        .map(|(i, q)| (origin[i].is_none() && protected(q[0] as usize, q[1] as usize)) as u8 as f32)
        .collect();
    edges::write_values(&mut out, "edge_group_knife", &cut, true)?;
    Ok(out)
}
fn pair(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}
fn weight(w: &Weights, i: usize) -> f64 {
    w.iter().find(|w| w.0 == i).map_or(0., |w| w.1)
}
fn mix(a: &Weights, b: &Weights, t: f64) -> Weights {
    let keys: BTreeSet<_> = a.iter().chain(b).map(|w| w.0).collect();
    keys.into_iter()
        .map(|i| (i, weight(a, i) * (1. - t) + weight(b, i) * t))
        .collect()
}
fn interpolate(table: &AttrTable, weights: &[Weights]) -> AttrTable {
    table
        .iter()
        .map(|(name, a)| {
            let w = a.width;
            let attr = match &a.data {
                AttrData::F32(v) => Attr::f32(
                    w,
                    weights
                        .iter()
                        .flat_map(|row| {
                            (0..w).map(move |k| {
                                // ndarray scalar multiplication and additions retain float32.
                                let mut value = 0.0f32;
                                for &(i, t) in row {
                                    value += v[i * w + k] * t as f32;
                                }
                                value
                            })
                        })
                        .collect(),
                ),
                AttrData::I32(v) => Attr::i32(
                    w,
                    weights
                        .iter()
                        .flat_map(|row| {
                            let mut best = row[0];
                            for &r in &row[1..] {
                                if r.1 > best.1 {
                                    best = r;
                                }
                            }
                            (0..w).map(move |k| v[best.0 * w + k])
                        })
                        .collect(),
                ),
            };
            (name.clone(), Arc::new(attr))
        })
        .collect()
}
/// Cut only polygons whose plane contains the drawn segment. The Knife kernel handles
/// interpolated boundary points and attribute forwarding; unrelated planes stay intact.
pub(super) fn split_coplanar_segment(g: &Geometry, a: [f32;3], b: [f32;3]) -> Result<Geometry> {
    let pos=positions(g);
    let mut cuts=Vec::new();
    for face in 0..g.nprims() {
        if !g.topology.is_closed(face) { continue; }
        let ids:Vec<usize>=range(g,face).map(|v|g.topology.vertex_point[v] as usize).collect();
        if ids.len()<3 { continue; }
        let origin=pos[ids[0]];
        let normal=cross(sub(pos[ids[1]],origin),sub(pos[ids[2]],origin));
        let scale=normal.iter().map(|x|x*x).sum::<f64>().sqrt();
        if scale<1e-10 { continue; }
        let on_plane=|p:[f64;3]| (0..3).map(|k|(p[k]-origin[k])*normal[k]).sum::<f64>().abs()/scale < 1e-5;
        if !ids.iter().all(|&i|on_plane(pos[i])) || !on_plane(a.map(f64::from)) || !on_plane(b.map(f64::from)) { continue; }
        let drop=(0..3).max_by(|&i,&j|normal[i].abs().total_cmp(&normal[j].abs())).unwrap();
        let axes:Vec<usize>=(0..3).filter(|&k|k!=drop).collect();
        let mut matrix=[0.;16]; matrix[axes[0]*4]=1.; matrix[axes[1]*4+1]=1.; matrix[drop*4+2]=1.; matrix[15]=1.;
        cuts.push(serde_json::json!({"matrix":matrix,"a":[a[axes[0]],a[axes[1]]],
            "b":[b[axes[0]],b[axes[1]]],"faces":[face],"through":true}));
    }
    if cuts.is_empty() { return Ok(g.clone()); }
    let params=Params::resolve(KNIFE.params,serde_json::json!({"cuts":serde_json::Value::Array(cuts).to_string()}).as_object())?;
    knife(&[vec![Arc::new(g.clone())]],&params)
}

#[cfg(test)]
mod sketch_split_tests {
    use super::*;
    #[test]
    fn coplanar_stroke_divides_a_quad() {
        let g=Geometry::from_arrays(vec![-1.,-1.,0.,1.,-1.,0.,1.,1.,0.,-1.,1.,0.],vec![0,1,2,3],vec![0,4],None).unwrap();
        let cut=split_coplanar_segment(&g,[0.,-1.,0.],[0.,1.,0.]).unwrap();
        assert!(cut.nprims()>g.nprims());
        let untouched=split_coplanar_segment(&g,[0.,-1.,1.],[0.,1.,1.]).unwrap();
        assert_eq!(untouched.p(),g.p());
        assert_eq!(untouched.topology.vertex_point,g.topology.vertex_point);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test] fn attached_vertex_and_edge_follow_positions_with_topology_fallback() {
        let matrix=json!([1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.]);
        let pos=[[2.,3.,0.],[4.,3.,0.]];
        let vertex=json!({"screen":[0.,0.],"snap":{"point_count":2,"element":[0],"params":[1.]}});
        let edge=json!({"screen":[0.,0.],"snap":{"point_count":2,"element":[0,1],"params":[0.5,0.5]}});
        assert_eq!(attached_screen(&vertex,&matrix,&pos),json!([2.,3.]));
        assert_eq!(attached_screen(&edge,&matrix,&pos),json!([3.,3.]));
        assert_eq!(attached_screen(&edge,&matrix,&pos[..1]),json!([0.,0.]));
    }
    #[test]
    fn cut_quad_through_and_stitch_shared_edge() {
        let g = Geometry::from_arrays(
            vec![-1., -1., 0., 1., -1., 0., 1., 1., 0., -1., 1., 0.],
            vec![0, 1, 2, 3],
            vec![0, 4],
            None,
        )
        .unwrap();
        let cuts=json!([{"matrix":[1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.],"a":[0.,-2.],"b":[0.,2.],"through":true}]).to_string();
        let p = Params::resolve(KNIFE.params, json!({"cuts":cuts}).as_object()).unwrap();
        let out = knife(&[vec![Arc::new(g)]], &p).unwrap();
        out.validate().unwrap();
        assert_eq!(out.nprims(), 2);
        // Two edge crossings only; the old 7th point was the triangulation-diagonal artefact.
        assert_eq!(out.npoints, 6);
        assert!(
            edges::values(&out, "edge_group_knife")
                .unwrap()
                .iter()
                .any(|&v| v > 0.)
        );
    }
    #[test] fn two_chains_create_point_and_edge_groups() {
        let g=Geometry::from_arrays(vec![-1.,-1.,0.,1.,-1.,0.,1.,1.,0.,-1.,1.,0.],vec![0,1,2,3],vec![0,4],None).unwrap();
        let matrix=[1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.];
        let cuts=json!([
            {"matrix":matrix,"points":[{"screen":[-2.,0.],"position":[-2.,0.,0.]},{"screen":[2.,0.],"position":[2.,0.,0.]}]},
            {"matrix":matrix,"points":[{"screen":[0.,-2.],"position":[0.,-2.,0.]},{"screen":[0.,2.],"position":[0.,2.,0.]}]}
        ]).to_string();
        let p=Params::resolve(KNIFE.params,json!({"cuts":cuts}).as_object()).unwrap();
        let out=knife(&[vec![Arc::new(g)]],&p).unwrap();
        out.validate().unwrap();
        assert!(out.nprims()>=4);
        assert!(out.point.get("point_group_knife").is_some());
        assert!(edges::values(&out,"edge_group_knife").unwrap().iter().any(|v|*v>0.));
    }
    #[test] fn projected_path_splits_non_coplanar_neighbours() {
        let g=Geometry::from_arrays(vec![-1.,-1.,0.,0.,-1.,0.,0.,1.,0.,-1.,1.,0.,1.,-1.,1.,1.,1.,1.],
            vec![0,1,2,3,1,4,5,2],vec![0,4,8],None).unwrap();
        let matrix=[1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.];
        let cuts=json!([{"matrix":matrix,"a":[-2.,0.],"b":[2.,0.]}]).to_string();
        let p=Params::resolve(KNIFE.params,json!({"cuts":cuts}).as_object()).unwrap();
        let out=knife(&[vec![Arc::new(g)]],&p).unwrap(); out.validate().unwrap();
        assert_eq!(out.nprims(),4);
        let edges=topo::edges(&out.topology,out.npoints);
        assert_eq!(out.npoints as i64-(edges.points.len()/2) as i64+out.nprims() as i64,1);
    }
    #[test] fn cut_through_reaches_occluded_face() {
        let g=Geometry::from_arrays(vec![-1.,-1.,0.,1.,-1.,0.,1.,1.,0.,-1.,1.,0.,-1.,-1.,-1.,1.,-1.,-1.,1.,1.,-1.,-1.,1.,-1.],
            vec![0,1,2,3,4,5,6,7],vec![0,4,8],None).unwrap();
        let matrix=[1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.];
        let cuts=json!([{"matrix":matrix,"a":[-2.,0.],"b":[2.,0.]}]).to_string();
        let cook=|through| {let p=Params::resolve(KNIFE.params,json!({"cuts":cuts,"cut_through":through}).as_object()).unwrap();
            knife(&[vec![Arc::new(g.clone())]],&p).unwrap()};
        let visible=cook(false);let through=cook(true);
        assert!(through.nprims()>visible.nprims());
    }
    #[test]
    fn diagonal_cut_adds_only_edge_crossings() {
        let g=Geometry::from_arrays(vec![-1.,-1.,0.,1.,-1.,0.,1.,1.,0.,-1.,1.,0.],vec![0,1,2,3],vec![0,4],None).unwrap();
        let matrix=[1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.];
        // Crosses the quad's internal triangulation diagonal, which must not leave a point.
        let cuts=json!([{"matrix":matrix,"a":[-2.,-1.],"b":[2.,1.],"through":true}]).to_string();
        let p=Params::resolve(KNIFE.params,json!({"cuts":cuts}).as_object()).unwrap();
        let out=knife(&[vec![Arc::new(g)]],&p).unwrap(); out.validate().unwrap();
        assert_eq!(out.nprims(),2);
        assert_eq!(out.npoints,6);
        let extra=out.p().chunks_exact(3).skip(4).map(|q|(q[0],q[1])).collect::<Vec<_>>();
        assert!(extra.iter().any(|&(x,y)|x==-1.&&y==-0.5)&&extra.iter().any(|&(x,y)|x==1.&&y==0.5),"{extra:?}");
    }
    #[test]
    fn cut_endpoint_on_internal_diagonal_is_kept() {
        let g=Geometry::from_arrays(vec![-1.,-1.,0.,1.,-1.,0.,1.,1.,0.,-1.,1.,0.],vec![0,1,2,3],vec![0,4],None).unwrap();
        let matrix=[1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.];
        // Two strokes meet on the quad's internal triangulation diagonal.
        let cuts=json!([{"matrix":matrix,"a":[-2.,0.],"b":[0.,0.],"through":true},
                        {"matrix":matrix,"a":[0.,0.],"b":[2.,0.],"through":true}]).to_string();
        let p=Params::resolve(KNIFE.params,json!({"cuts":cuts}).as_object()).unwrap();
        let out=knife(&[vec![Arc::new(g)]],&p).unwrap(); out.validate().unwrap();
        let endpoints:Vec<_>=out.p().chunks_exact(3).enumerate()
            .filter(|(_,q)|q[0].abs()<1e-6&&q[1].abs()<1e-6&&q[2].abs()<1e-6)
            .map(|(i,_)|i).collect();
        assert_eq!(endpoints.len(),1,"cut endpoint is missing from {:?}",out.p());
        assert!(out.topology.vertex_point.contains(&(endpoints[0] as i32)));
    }
    #[test]
    fn empty_and_invalid_projection() {
        let g = Geometry::points_only(vec![0., 0., 0.]).unwrap();
        let p = Params::resolve(KNIFE.params, None).unwrap();
        assert_eq!(knife(&[vec![Arc::new(g)]], &p).unwrap().npoints, 1);
        assert!(parse("[{\"matrix\":[],\"a\":[0,0],\"b\":[1,1]}]",None).is_err());
    }
}
