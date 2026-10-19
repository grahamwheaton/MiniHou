use crate::{error::Result, geometry::Geometry, nodes::{InputSpec, Inputs, NodeSpec}, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "match_size",
    inputs: &[
        InputSpec { name: "geo", multi: false, optional: false },
        InputSpec { name: "target", multi: false, optional: true },
    ],
    params: crate::params!(
        "group": "";
        "target_position": [0.0, 0.0, 0.0];
        "target_size": [1.0, 1.0, 1.0];
        "translate": true;
        "justify_x": "center";
        "justify_y": "center";
        "justify_z": "center";
        "justify_to": "target";
        "offset_by": [0.0, 0.0, 0.0];
        "scale_to_fit": false;
        "uniform_scale": false;
        "restore_transform": false, unsupported;
        "stash_transform": false, unsupported
    ),
    cook: match_size,
};

fn bounds(g: &Geometry, selected: &[bool]) -> Option<([f64; 3], [f64; 3])> {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    let mut found = false;
    for (point, &include) in g.p().chunks_exact(3).zip(selected) {
        if !include { continue; }
        found = true;
        for axis in 0..3 {
            lo[axis] = lo[axis].min(point[axis] as f64);
            hi[axis] = hi[axis].max(point[axis] as f64);
        }
    }
    found.then_some((lo, hi))
}

fn anchor(lo: f64, hi: f64, mode: &str) -> f64 {
    match mode { "min" => lo, "max" => hi, _ => (lo + hi) / 2.0 }
}

fn match_size(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = NODE.one(inputs, 0)?;
    let selected = crate::nodes::group::mask(&geo, "point", &p.s("group")?)?;
    let Some((source_lo, source_hi)) = bounds(&geo, &selected) else { return Ok((*geo).clone()); };
    let target_position = p.v3("target_position")?;
    let target_size = p.v3("target_size")?;
    let (target_lo, target_hi) = if let Some(target) = inputs[1].first() {
        bounds(target, &vec![true; target.npoints]).unwrap_or_else(|| {
            (std::array::from_fn(|i| target_position[i] - target_size[i] / 2.0),
             std::array::from_fn(|i| target_position[i] + target_size[i] / 2.0))
        })
    } else {
        (std::array::from_fn(|i| target_position[i] - target_size[i] / 2.0),
         std::array::from_fn(|i| target_position[i] + target_size[i] / 2.0))
    };
    let fit = p.b("scale_to_fit")?;
    let uniform = p.b("uniform_scale")?;
    let mut scale = [1.0; 3];
    if fit {
        for i in 0..3 {
            let extent = source_hi[i] - source_lo[i];
            if extent.abs() > f64::EPSILON { scale[i] = (target_hi[i] - target_lo[i]) / extent; }
        }
        if uniform {
            let factor = (0..3).filter(|&i| source_hi[i] > source_lo[i])
                .map(|i| scale[i]).fold(f64::INFINITY, f64::min);
            scale = [if factor.is_finite() { factor } else { 1.0 }; 3];
        }
    }
    let modes = [p.e("justify_x", &["center", "min", "max"])? ,
                 p.e("justify_y", &["center", "min", "max"])? ,
                 p.e("justify_z", &["center", "min", "max"])?];
    let destination = p.e("justify_to", &["target", "same"])?;
    let offset = p.v3("offset_by")?;
    let translate = p.b("translate")?;
    let center: [f64; 3] = std::array::from_fn(|i| (source_lo[i] + source_hi[i]) / 2.0);
    let shift: [f64; 3] = std::array::from_fn(|i| {
        if !translate { return 0.0; }
        let source_anchor = anchor(source_lo[i], source_hi[i], &modes[i]);
        let target_anchor = if destination == "target" {
            anchor(target_lo[i], target_hi[i], &modes[i])
        } else { source_anchor };
        target_anchor - (center[i] + (source_anchor - center[i]) * scale[i]) + offset[i]
    });
    let mut positions = geo.p().to_vec();
    for (point, &include) in positions.chunks_exact_mut(3).zip(&selected) {
        if !include { continue; }
        for i in 0..3 {
            point[i] = (center[i] + (point[i] as f64 - center[i]) * scale[i] + shift[i]) as f32;
        }
    }
    let mut out = (*geo).clone();
    out.set_p(positions);
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
