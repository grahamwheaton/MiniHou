//! Node `measure_distance`: distance between two points or two element groups (detail attribute distance; points from a seed index or the first/last point).

use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::error::Result;
use crate::geometry::Geometry;
use crate::params::Params;

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "measure_distance",
    inputs: &[InputSpec { name: "geo", multi: false, optional: false }],
    params: crate::params!(
        "group": "";
        "group2": "";
        "point1": 0;
        "point2": -1;
    ),
    cook: measure_distance,
};

fn measure_distance(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let mut out = (*g).clone();
    let group1 = p.s("group")?;
    let group2 = p.s("group2")?;
    let p1 = p.i("point1")?;
    let p2 = p.i("point2")?;

    if g.npoints == 0 {
        out.detail.insert("distance".into(), serde_json::json!(0.0));
        return Ok(out);
    }

    let dist = if !group1.trim().is_empty() && !group2.trim().is_empty() {
        let sel1 = crate::nodes::group::mask(&g, "point", &group1).unwrap_or_else(|_| vec![false; g.npoints]);
        let sel2 = crate::nodes::group::mask(&g, "point", &group2).unwrap_or_else(|_| vec![false; g.npoints]);
        let pts1: Vec<usize> = sel1.iter().enumerate().filter_map(|(i, &b)| if b { Some(i) } else { None }).collect();
        let pts2: Vec<usize> = sel2.iter().enumerate().filter_map(|(i, &b)| if b { Some(i) } else { None }).collect();
        if pts1.is_empty() || pts2.is_empty() {
            0.0
        } else {
            let p_arr = g.p();
            let mut min_dist_sq = f64::INFINITY;
            for &a in &pts1 {
                for &b in &pts2 {
                    let dx = p_arr[a * 3] as f64 - p_arr[b * 3] as f64;
                    let dy = p_arr[a * 3 + 1] as f64 - p_arr[b * 3 + 1] as f64;
                    let dz = p_arr[a * 3 + 2] as f64 - p_arr[b * 3 + 2] as f64;
                    let d2 = dx * dx + dy * dy + dz * dz;
                    if d2 < min_dist_sq {
                        min_dist_sq = d2;
                    }
                }
            }
            if min_dist_sq.is_finite() { min_dist_sq.sqrt() } else { 0.0 }
        }
    } else if !group1.trim().is_empty() {
        let sel = crate::nodes::group::mask(&g, "point", &group1).unwrap_or_else(|_| vec![false; g.npoints]);
        let pts: Vec<usize> = sel.iter().enumerate().filter_map(|(i, &b)| if b { Some(i) } else { None }).collect();
        if pts.len() < 2 {
            0.0
        } else {
            let n = pts.len() as i64;
            let idx1 = if p1 < 0 { (n + p1).clamp(0, n - 1) } else { p1.clamp(0, n - 1) } as usize;
            let idx2 = if p2 < 0 { (n + p2).clamp(0, n - 1) } else { p2.clamp(0, n - 1) } as usize;
            let a = pts[idx1];
            let b = pts[idx2];
            let p_arr = g.p();
            let dx = p_arr[a * 3] as f64 - p_arr[b * 3] as f64;
            let dy = p_arr[a * 3 + 1] as f64 - p_arr[b * 3 + 1] as f64;
            let dz = p_arr[a * 3 + 2] as f64 - p_arr[b * 3 + 2] as f64;
            (dx * dx + dy * dy + dz * dz).sqrt()
        }
    } else if g.npoints < 2 {
        0.0
    } else {
        let n = g.npoints as i64;
        let idx1 = if p1 < 0 { (n + p1).clamp(0, n - 1) } else { p1.clamp(0, n - 1) } as usize;
        let idx2 = if p2 < 0 { (n + p2).clamp(0, n - 1) } else { p2.clamp(0, n - 1) } as usize;
        let p_arr = g.p();
        let dx = p_arr[idx1 * 3] as f64 - p_arr[idx2 * 3] as f64;
        let dy = p_arr[idx1 * 3 + 1] as f64 - p_arr[idx2 * 3 + 1] as f64;
        let dz = p_arr[idx1 * 3 + 2] as f64 - p_arr[idx2 * 3 + 2] as f64;
        (dx * dx + dy * dy + dz * dz).sqrt()
    };

    out.detail.insert("distance".into(), serde_json::json!(dist));
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
