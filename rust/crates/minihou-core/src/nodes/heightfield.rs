//! Heightfield terrain erosion node from minihou/nodes/heightfield.py.
use std::sync::Arc;
use super::{GEO, InputSpec, Inputs, NodeSpec, np_random::Rng};
use crate::{
    error::{Error, Result},
    geometry::{Attr, Geometry},
    heightfield::HydraulicState,
    params::Params,
};

const HF_INPUTS: &[InputSpec] = &[
    InputSpec { name: "geo", multi: false, optional: false },
    InputSpec { name: "mask", multi: false, optional: true },
];

pub const HEIGHTFIELD_ERODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "heightfield_erode",
    inputs: HF_INPUTS,
    params: crate::params!(
        "iterations": 30;
        "droplets": 4096;
        "rain": 0.01;
        "capacity": 4.0;
        "erode_rate": 0.3;
        "deposit_rate": 0.3;
        "evaporation": 0.02;
        "inertia": 0.1;
        "seed": 0;
        "thermal_iterations": 20;
        "talus_angle": 35.0;
        "thermal_rate": 0.5
    ),
    cook: heightfield_erode,
};

fn lattice(geo: &Geometry) -> Result<(usize, usize)> {
    let cols = geo.detail.get("grid_columns").and_then(|a| a.as_u64()).and_then(|v| usize::try_from(v).ok());
    let rows = geo.detail.get("grid_rows").and_then(|a| a.as_u64()).and_then(|v| usize::try_from(v).ok());

    if let (Some(c), Some(r)) = (cols, rows) {
        if c > 0 && r > 0 && c.checked_mul(r) == Some(geo.npoints) {
            return Ok((r, c));
        }
    }

    Err(Error::new(format!(
        "heightfield nodes need a grid from the grid node (its points in rows); this geometry has {} points and no matching grid_columns/grid_rows detail attributes",
        geo.npoints
    )))
}

fn np_percentile(sorted_a: &[f64], q: f64) -> f64 {
    if sorted_a.is_empty() {
        return 0.0;
    }
    if sorted_a.len() == 1 {
        return sorted_a[0];
    }
    let idx = (q / 100.0) * (sorted_a.len() - 1) as f64;
    let lo = idx.floor() as usize;
    let hi = (lo + 1).min(sorted_a.len() - 1);
    let frac = idx - lo as f64;
    sorted_a[lo] * (1.0 - frac) + sorted_a[hi] * frac
}

fn heightfield_erode(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = HEIGHTFIELD_ERODE.one(inputs, 0)?;
    let mask_input = inputs.get(1).and_then(|v| v.first());

    let (rows, cols) = lattice(&geo)?;

    let mut keep = None;
    if let Some(mask) = mask_input {
        if mask.npoints != geo.npoints || !mask.point.contains_key("mask") {
            return Err(Error::new(format!(
                "heightfield_erode: the mask input needs the same {} points and a 'mask' point attribute",
                geo.npoints
            )));
        }
        if let Some(mask_attr) = mask.point.get("mask").and_then(|a| a.as_f32()) {
            let w = mask.point["mask"].width;
            let vals: Vec<f64> = (0..geo.npoints)
                .map(|i| (mask_attr[i * w] as f64).clamp(0.0, 1.0))
                .collect();
            keep = Some(vals);
        }
    }

    let iterations = p.i("iterations")?;
    let droplets = p.i("droplets")?;
    let rain = p.f("rain")?;
    let capacity = p.f("capacity")?;
    let erode_rate = p.f("erode_rate")?;
    let deposit_rate = p.f("deposit_rate")?;
    let evaporation = p.f("evaporation")?;
    let inertia = p.f("inertia")?;
    let seed = p.i("seed")?;
    let thermal_iterations = p.i("thermal_iterations")?;
    let talus_angle = p.f("talus_angle")?;
    let thermal_rate = p.f("thermal_rate")?;

    let p_slice = geo.p();
    let mut h: Vec<f64> = (0..rows * cols).map(|i| p_slice[i * 3 + 2] as f64).collect();

    let cell_y = if rows > 1 {
        (p_slice[cols * 3 + 1] - p_slice[1]).abs().max(1e-9) as f64
    } else {
        1.0
    };
    let cell_x = if cols > 1 {
        (p_slice[1 * 3] - p_slice[0]).abs().max(1e-9) as f64
    } else {
        1.0
    };
    let cell = (cell_y, cell_x);

    let mut flow = vec![0.0f64; rows * cols];
    let mut eroded = vec![0.0f64; rows * cols];
    let mut deposited = vec![0.0f64; rows * cols];

    if iterations > 0 && droplets > 0 {
        let mut state = HydraulicState::new(h, rows, cols);
        let mut rng = Rng::new(&[(seed as u32) as u64]);
        let w_bound = (cols as f64) - 1.001;
        let h_bound = (rows as f64) - 1.001;

        for _ in 0..iterations {
            let mut x = Vec::with_capacity(droplets as usize);
            for _ in 0..droplets {
                x.push(rng.random() * w_bound);
            }
            let mut y = Vec::with_capacity(droplets as usize);
            for _ in 0..droplets {
                y.push(rng.random() * h_bound);
            }

            state.step(
                &x,
                &y,
                cell,
                rain,
                capacity,
                erode_rate,
                deposit_rate,
                evaporation,
                48,
                inertia,
                4.0,
                0.01,
            );
        }

        let (h_res, f_res, e_res, d_res) = state.finish();
        h = h_res;
        flow = f_res;
        eroded = e_res;
        deposited = d_res;
    }

    if thermal_iterations > 0 {
        let tan = talus_angle.to_radians().tan();
        let neighbours = [
            (-1isize, -1isize), (-1, 0), (-1, 1),
            (0, -1),                     (0, 1),
            (1, -1),           (1, 0),  (1, 1),
        ];
        let dist: Vec<f64> = neighbours
            .iter()
            .map(|&(dy, dx)| ((dy as f64 * cell.0).powi(2) + (dx as f64 * cell.1).powi(2)).sqrt())
            .collect();
        h = crate::heightfield::thermal(&h, rows, cols, tan, &dist, thermal_rate, thermal_iterations as usize);
    }

    if let Some(mask) = keep {
        for i in 0..rows * cols {
            let orig_z = p_slice[i * 3 + 2] as f64;
            h[i] = orig_z + (h[i] - orig_z) * mask[i];
            flow[i] *= mask[i];
            eroded[i] *= mask[i];
            deposited[i] *= mask[i];
        }
    }

    let mut new_p = p_slice.to_vec();
    for i in 0..rows * cols {
        new_p[i * 3 + 2] = h[i] as f32;
    }

    // flow log-scale and normalization
    let mut pos_flow: Vec<f64> = flow.iter().copied().filter(|&v| v > 0.0).collect();
    let med = if pos_flow.is_empty() {
        1.0
    } else {
        pos_flow.sort_by(|a, b| a.total_cmp(b));
        np_percentile(&pos_flow, 50.0).max(1e-12)
    };

    let f_log: Vec<f64> = flow.iter().map(|&v| (v / med).ln_1p()).collect();
    let mut sorted_f = f_log.clone();
    sorted_f.sort_by(|a, b| a.total_cmp(b));
    let lo = if sorted_f.is_empty() { 0.0 } else { np_percentile(&sorted_f, 50.0) };
    let hi = if sorted_f.is_empty() { 1.0 } else { np_percentile(&sorted_f, 99.5) };
    let denom = (hi - lo).max(1e-9);

    let flow_attr: Vec<f32> = f_log
        .iter()
        .map(|&v| ((v - lo) / denom).clamp(0.0, 1.0) as f32)
        .collect();
    let erosion_attr: Vec<f32> = eroded.iter().map(|&v| v as f32).collect();
    let deposit_attr: Vec<f32> = deposited.iter().map(|&v| v as f32).collect();

    let mut out = (*geo).clone();
    out.set_p(new_p);
    out.point.insert("flow".into(), Arc::new(Attr::f32(1, flow_attr)));
    out.point.insert("erosion".into(), Arc::new(Attr::f32(1, erosion_attr)));
    out.point.insert("deposit".into(), Arc::new(Attr::f32(1, deposit_attr)));

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_detail_dimensions_feed_erosion() {
        let grid = &super::super::generate::GRID;
        let params = Params::resolve(grid.params, serde_json::json!({"columns": 3, "rows": 2}).as_object()).unwrap();
        let geo = (grid.cook)(&[], &params).unwrap();
        assert_eq!(lattice(&geo).unwrap(), (2, 3));
        let params = Params::resolve(HEIGHTFIELD_ERODE.params,
            serde_json::json!({"iterations": 0, "thermal_iterations": 0}).as_object()).unwrap();
        let out = heightfield_erode(&[vec![Arc::new(geo.clone())], vec![]], &params).unwrap();
        out.validate().unwrap();
        assert_eq!(out.p(), geo.p());
        for name in ["flow", "erosion", "deposit"] {
            assert_eq!(out.point[name].as_f32().unwrap(), &[0.0; 6]);
        }
    }

    #[test]
    fn test_not_a_grid_err() {
        let geo = Geometry::points_only(vec![0.0, 0.0, 0.0]).unwrap();
        let p = Params::resolve(HEIGHTFIELD_ERODE.params, None).unwrap();
        assert!(heightfield_erode(&[vec![Arc::new(geo)]], &p).is_err());
    }
}
