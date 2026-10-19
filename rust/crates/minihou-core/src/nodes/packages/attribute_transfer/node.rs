use std::sync::Arc;
use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::{Params};
struct KdNode {
    axis: usize,
    split: f64,
    idx: usize,
    left: Option<usize>,
    right: Option<usize>,
}

struct KdTree {
    pts: Vec<[f64; 3]>,
    nodes: Vec<KdNode>,
}

impl KdTree {
    fn build(pts: Vec<[f64; 3]>) -> Self {
        let mut indices: Vec<usize> = (0..pts.len()).collect();
        let mut nodes = Vec::with_capacity(pts.len());
        let _ = Self::build_rec(&pts, &mut indices, 0, &mut nodes);
        KdTree { pts, nodes }
    }

    fn build_rec(
        pts: &[[f64; 3]],
        indices: &mut [usize],
        depth: usize,
        nodes: &mut Vec<KdNode>,
    ) -> Option<usize> {
        if indices.is_empty() {
            return None;
        }
        let axis = depth % 3;
        let mid = indices.len() / 2;
        indices.select_nth_unstable_by(mid, |&a, &b| {
            pts[a][axis].total_cmp(&pts[b][axis])
        });
        let idx = indices[mid];
        let split = pts[idx][axis];

        let node_idx = nodes.len();
        nodes.push(KdNode {
            axis,
            split,
            idx,
            left: None,
            right: None,
        });

        let (left_slice, rest) = indices.split_at_mut(mid);
        let right_slice = &mut rest[1..];

        let left = Self::build_rec(pts, left_slice, depth + 1, nodes);
        let right = Self::build_rec(pts, right_slice, depth + 1, nodes);

        nodes[node_idx].left = left;
        nodes[node_idx].right = right;

        Some(node_idx)
    }

    fn nearest(&self, q: [f64; 3]) -> (usize, f64) {
        if self.nodes.is_empty() {
            return (0, f64::INFINITY);
        }
        let mut best_idx = 0;
        let mut best_dist_sq = f64::INFINITY;
        self.search(0, q, &mut best_idx, &mut best_dist_sq);
        (best_idx, best_dist_sq.sqrt())
    }

    fn search(&self, node_idx: usize, q: [f64; 3], best_idx: &mut usize, best_dist_sq: &mut f64) {
        let node = &self.nodes[node_idx];
        let p = self.pts[node.idx];
        let d2 = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2);
        if d2 < *best_dist_sq {
            *best_dist_sq = d2;
            *best_idx = node.idx;
        }

        let diff = q[node.axis] - node.split;
        let (first, second) = if diff <= 0.0 {
            (node.left, node.right)
        } else {
            (node.right, node.left)
        };

        if let Some(child) = first {
            self.search(child, q, best_idx, best_dist_sq);
        }
        if diff * diff < *best_dist_sq {
            if let Some(child) = second {
                self.search(child, q, best_idx, best_dist_sq);
            }
        }
    }
}

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "attribute_transfer",
    inputs: &[
        InputSpec { name: "geo", multi: false, optional: false },
        InputSpec { name: "source", multi: false, optional: false },
    ],
    params: crate::params!(
        "names": "Cd";
        "max_distance": 1e9;
        "blend_width": 0.0;
        "group": "";
    ),
    cook: transfer,
};

fn transfer(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = NODE.one(inputs, 0)?;
    let source = NODE.one(inputs, 1)?;
    let names_str = p.s("names")?;
    let pats: Vec<&str> = names_str.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).collect();
    let mut want: Vec<String> = Vec::new();
    for (name, _) in &source.point {
        if name != "P" && pats.iter().any(|pat| crate::nodes::attribs::matches(pat, name)) {
            want.push(name.clone());
        }
    }
    if want.is_empty() {
        return Err(Error::new(format!("the source has no point attribute matching {names_str:?}")));
    }
    let max_dist = p.f("max_distance")?;
    let blend_width = p.f("blend_width")?;
    let group = p.s("group")?;
    let sel = crate::nodes::group::mask(&geo, "point", &group)?;

    let src_p = source.p();
    let src_pts: Vec<[f64; 3]> = (0..source.npoints)
        .map(|i| [src_p[i * 3] as f64, src_p[i * 3 + 1] as f64, src_p[i * 3 + 2] as f64])
        .collect();
    let tree = KdTree::build(src_pts);

    let geo_p = geo.p();
    let n = geo.npoints;
    let mut nearest_idx = Vec::with_capacity(n);
    let mut weights = Vec::with_capacity(n);

    for i in 0..n {
        let q = [geo_p[i * 3] as f64, geo_p[i * 3 + 1] as f64, geo_p[i * 3 + 2] as f64];
        let (idx, dist) = tree.nearest(q);
        nearest_idx.push(idx);

        let mut w = if dist <= max_dist { 1.0 } else { 0.0 };
        if blend_width > 0.0 {
            let fade = 1.0 - (dist - (max_dist - blend_width)) / blend_width;
            w = fade.clamp(0.0, 1.0);
        }
        if !sel[i] {
            w = 0.0;
        }
        weights.push(w);
    }

    let mut out = (*geo).clone();
    for nm in want {
        let src_attr = &source.point[&nm];
        let w = src_attr.width;
        let own_attr = geo.point.get(&nm);
        let own_matches = own_attr.map_or(false, |a| a.width == w && a.dtype() == src_attr.dtype());

        let res_attr = match &src_attr.data {
            AttrData::F32(src_v) => {
                let own_v = if own_matches {
                    own_attr.unwrap().as_f32().unwrap()
                } else {
                    &[]
                };
                let mut data = Vec::with_capacity(n * w);
                for i in 0..n {
                    let s_idx = nearest_idx[i];
                    let weight = weights[i] as f32;
                    for k in 0..w {
                        let s_val = src_v[s_idx * w + k];
                        let o_val = if own_matches { own_v[i * w + k] } else { 0.0 };
                        data.push(o_val * (1.0 - weight) + s_val * weight);
                    }
                }
                Attr::f32(w, data)
            }
            AttrData::I32(src_v) => {
                let own_v = if own_matches {
                    match &own_attr.unwrap().data {
                        AttrData::I32(v) => v.as_slice(),
                        _ => &[],
                    }
                } else {
                    &[]
                };
                let mut data = Vec::with_capacity(n * w);
                for i in 0..n {
                    let s_idx = nearest_idx[i];
                    let use_src = weights[i] >= 0.5;
                    for k in 0..w {
                        let s_val = src_v[s_idx * w + k];
                        let o_val = if own_matches { own_v[i * w + k] } else { 0 };
                        data.push(if use_src { s_val } else { o_val });
                    }
                }
                Attr::i32(w, data)
            }
        };

        out.prim.remove(&nm);
        out.vertex.remove(&nm);
        out.point.insert(nm, Arc::new(res_attr));
    }
    Ok(out)
}


#[cfg(test)]
#[path = "tests.rs"]
mod tests;
