use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::{GLUE_CLUSTER, contacts};
use super::super::Inputs;
use super::super::model::{add, mul, positions, range, outgroup};
use super::super::np_random::Rng;
use super::super::sim_common::{float, int};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::Params;

fn cluster_pieces(ids: &[i32], a: &[i32], b: &[i32], area: &[f64], target: i64) -> Vec<i32> {
    let n = ids.len();
    if n == 0 {
        return Vec::new();
    }
    let mut parent: Vec<usize> = (0..n).collect();
    let mut size: Vec<usize> = vec![1; n];
    let mut clusters = n;

    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }

    if target > 0 && target < n as i64 {
        let target = target as usize;
        let cap = (1.5 * n as f64 / target as f64).ceil() as usize;
        let mut order: Vec<usize> = (0..area.len()).collect();
        order.sort_by(|&i, &j| (-area[i]).total_cmp(&-area[j]));

        for &limit in &[cap, n] {
            for &k in &order {
                if clusters <= target {
                    break;
                }
                let slot_a = ids.binary_search(&a[k]).unwrap();
                let slot_b = ids.binary_search(&b[k]).unwrap();
                let ra = find(&mut parent, slot_a);
                let rb = find(&mut parent, slot_b);
                if ra != rb && size[ra] + size[rb] <= limit {
                    let lo = ra.min(rb);
                    let hi = ra.max(rb);
                    parent[hi] = lo;
                    size[lo] += size[hi];
                    clusters -= 1;
                }
            }
        }
    }

    let root: Vec<usize> = (0..n).map(|i| find(&mut parent, i)).collect();
    let mut uniq_roots = root.clone();
    uniq_roots.sort_unstable();
    uniq_roots.dedup();
    root.iter().map(|r| uniq_roots.binary_search(r).unwrap() as i32).collect()
}

pub(super) fn glue_cluster(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = GLUE_CLUSTER.one(inputs, 0)?;
    if geo.topology.has_open() {
        let n = geo.topology.closed_mask().iter().filter(|&&c| !c).count();
        return Err(Error::new(format!(
            "glue_cluster works on polygons, but the input has {n} open curve(s); remove them first, e.g. blast with group '@closed == 0'"
        )));
    }

    let piece_attrib = p.s("piece_attrib")?;
    let output = p.e("output", &["pieces", "constraints"])?;
    let strength = float(p, "strength", Some(0.0), None)?;
    let strength_variation = float(p, "strength_variation", Some(0.0), Some(1.0))?;
    let seed = p.i("seed")?;
    let max_gap = float(p, "max_gap", Some(0.0), None)?;
    let min_area = float(p, "min_area", Some(0.0), None)?;
    let min_share = float(p, "min_share", Some(0.0), Some(1.0))?;
    let clusters = int(p, "clusters", Some(0), None)?;
    let weak = float(p, "weak", Some(0.0), Some(1.0))?;

    let pieces: Vec<i32> = if piece_attrib.is_empty() {
        super::super::attrib_tools::piece_labels(&geo).1
    } else {
        let attr = geo.prim.get(&piece_attrib).ok_or_else(|| {
            Error::new(format!(
                "no primitive attribute {piece_attrib:?}: fracture first, or leave piece attribute empty to use connectivity"
            ))
        })?;
        if attr.width != 1 {
            return Err(Error::new(format!("{piece_attrib:?} must be a scalar integer primitive attribute")));
        }
        match &attr.data {
            AttrData::I32(v) => v.clone(),
            _ => return Err(Error::new(format!("{piece_attrib:?} must be a scalar integer primitive attribute"))),
        }
    };

    let ids: Vec<i32> = pieces.iter().copied().collect::<BTreeSet<_>>().into_iter().collect();

    let contact_list = contacts(&geo, &pieces, max_gap);
    let mut pair_map = BTreeMap::<(i32, i32), f64>::new();
    for (fa, fb, ar) in &contact_list {
        *pair_map.entry((pieces[*fa], pieces[*fb])).or_default() += *ar;
    }

    let max_area = pair_map.values().copied().fold(0.0f64, f64::max);
    let cutoff = min_area.max(min_share * max_area).max(1e-300);

    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut area = Vec::new();
    for ((pa, pb), ar) in pair_map {
        if ar >= cutoff {
            a.push(pa);
            b.push(pb);
            area.push(ar);
        }
    }

    let mut rng = Rng::new(&[(seed & 0xFFFF_FFFF) as u64, 41]);
    let force: Vec<f64> = (0..area.len()).map(|i| {
        let noise = rng.random() * 2.0 - 1.0;
        strength * area[i] * (1.0 + strength_variation * noise).max(0.01)
    }).collect();

    let label = cluster_pieces(&ids, &a, &b, &area, clusters);
    let body: &[i32] = if clusters > 0 { &label } else { &ids };

    if output == "pieces" {
        let mut out = (*geo).clone();
        let prim_clusters: Vec<i32> = if ids.is_empty() {
            Vec::new()
        } else {
            pieces.iter().map(|&p| {
                let slot = ids.binary_search(&p).unwrap();
                body[slot]
            }).collect()
        };
        out.prim.insert("cluster".into(), Arc::new(Attr::i32(1, prim_clusters)));

        let mut linked = vec![false; ids.len()];
        if !a.is_empty() {
            for &x in &a {
                if let Ok(slot) = ids.binary_search(&x) {
                    linked[slot] = true;
                }
            }
            for &x in &b {
                if let Ok(slot) = ids.binary_search(&x) {
                    linked[slot] = true;
                }
            }
        }

        let mut face = vec![false; geo.nprims()];
        for (fa, fb, _) in contact_list {
            let pa = pieces[fa];
            if let Ok(slot) = ids.binary_search(&pa) {
                if linked[slot] {
                    face[fa] = true;
                    face[fb] = true;
                }
            }
        }

        let per_prim: Vec<bool> = if ids.is_empty() {
            Vec::new()
        } else {
            pieces.iter().map(|&p| {
                let slot = ids.binary_search(&p).unwrap();
                linked[slot]
            }).collect()
        };
        let loose: Vec<bool> = per_prim.iter().map(|&x| !x).collect();

        outgroup(&mut out, p, "glued", "prim", &per_prim)?;
        outgroup(&mut out, p, "loose", "prim", &loose)?;
        outgroup(&mut out, p, "interface", "prim", &face)?;
        return Ok(out);
    }

    // output == "constraints"
    let pos = positions(&geo);
    let mut centres = vec![[0.0f64; 3]; ids.len()];
    let mut counts = vec![0usize; ids.len()];
    for (i, &piece) in pieces.iter().enumerate() {
        if let Ok(k) = ids.binary_search(&piece) {
            for v in range(&geo, i) {
                let pt = geo.topology.vertex_point[v] as usize;
                centres[k] = add(centres[k], pos[pt]);
                counts[k] += 1;
            }
        }
    }
    for (c, n) in centres.iter_mut().zip(counts) {
        *c = mul(*c, 1.0 / n.max(1) as f64);
    }

    let ba: Vec<i32> = a.iter().map(|&x| body[ids.binary_search(&x).unwrap()]).collect();
    let bb: Vec<i32> = b.iter().map(|&x| body[ids.binary_search(&x).unwrap()]).collect();

    let n_links = a.len();
    let mut vp = Vec::with_capacity(n_links * 2);
    for k in 0..n_links {
        let sa = ids.binary_search(&a[k]).unwrap() as i32;
        let sb = ids.binary_search(&b[k]).unwrap() as i32;
        vp.push(sa);
        vp.push(sb);
    }
    let prim_offsets: Vec<i32> = (0..=n_links).map(|i| (i * 2) as i32).collect();
    let closed = vec![false; n_links];
    let p_flat: Vec<f32> = centres.into_iter().flatten().map(|v| v as f32).collect();

    let mut out = Geometry::from_arrays(p_flat, vp, prim_offsets, Some(closed))?;
    out.point.insert("piece".into(), Arc::new(Attr::i32(1, ids)));
    out.point.insert("cluster".into(), Arc::new(Attr::i32(1, label)));

    out.prim.insert("piece_a".into(), Arc::new(Attr::i32(1, a)));
    out.prim.insert("piece_b".into(), Arc::new(Attr::i32(1, b)));
    out.prim.insert("rbd_a".into(), Arc::new(Attr::i32(1, ba.clone())));
    out.prim.insert("rbd_b".into(), Arc::new(Attr::i32(1, bb.clone())));
    out.prim.insert("area".into(), Arc::new(Attr::f32(1, area.iter().map(|&x| x as f32).collect())));
    let strength_attr: Vec<f32> = if strength > 0.0 {
        force.iter().map(|&x| x as f32).collect()
    } else {
        vec![0.0f32; n_links]
    };
    out.prim.insert("strength".into(), Arc::new(Attr::f32(1, strength_attr)));
    let length_attr: Vec<f32> = area.iter().map(|&x| x.sqrt() as f32).collect();
    out.prim.insert("length".into(), Arc::new(Attr::f32(1, length_attr)));

    let (internal, external): (Vec<bool>, Vec<bool>) = if clusters > 0 {
        let int_mask: Vec<bool> = (0..n_links).map(|i| ba[i] == bb[i]).collect();
        let ext_mask: Vec<bool> = int_mask.iter().map(|&x| !x).collect();
        (int_mask, ext_mask)
    } else {
        (vec![false; n_links], vec![true; n_links])
    };
    outgroup(&mut out, p, "internal", "prim", &internal)?;
    outgroup(&mut out, p, "external", "prim", &external)?;

    let ext_count = external.iter().filter(|&&x| x).count();
    let mean = if ext_count > 0 {
        let sum: f64 = (0..n_links).filter(|&i| external[i]).map(|i| force[i]).sum();
        sum / ext_count as f64
    } else {
        0.0
    };
    let is_weak: Vec<bool> = (0..n_links).map(|i| force[i] < weak * mean).collect();
    outgroup(&mut out, p, "weak", "prim", &is_weak)?;

    Ok(out)
}
