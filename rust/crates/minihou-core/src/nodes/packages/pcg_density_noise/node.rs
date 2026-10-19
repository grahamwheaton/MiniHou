use crate::nodes::pcg::*;
use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry}, params::Params};
use std::collections::HashMap;
use std::sync::Arc;

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "pcg_density_noise",
    inputs: &[InputSpec {
        name: "points",
        multi: false,
        optional: false,
    }],
    params: crate::params!(
        "noise": "random";
        "mode": "set";
        "frequency": 0.2;
        "low": 0.0;
        "high": 1.0;
        "invert": false;
        "seed": 0
    ),
    cook: pcg_density_noise,
};

fn pcg_density_noise(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let points = NODE.one(inputs, 0)?;
    let noise = p.e("noise", &["random", "perlin"])?;
    let mode = p.e("mode", &["set", "multiply", "add", "minimum", "maximum"])?;
    let frequency = p.f("frequency")?;
    let low = p.f("low")?;
    let high = p.f("high")?;
    let invert = p.b("invert")?;
    let seed = p.i("seed")?;

    let a = read_pcg(&points);
    let n = points.npoints;
    let mut d_out = Vec::with_capacity(n);

    let perlin_vals = if noise == "perlin" {
        let scaled_p: Vec<f32> = points
            .p()
            .iter()
            .map(|&x| (x as f64 * frequency) as f32)
            .collect();
        crate::noise::fbm3(
            &scaled_p,
            (seed as u32) as i64,
            3,
            2.0,
            0.5,
        )
    } else {
        Vec::new()
    };

    for i in 0..n {
        let mut v = if noise == "random" {
            random01(a.seed[i], seed, 0)
        } else {
            (perlin_vals[i] as f64 * 0.5 + 0.5).clamp(0.0, 1.0)
        };
        if invert {
            v = 1.0 - v;
        }
        v = low + (high - low) * v;
        let d = a.density[i];
        let res = match mode.as_str() {
            "set" => v,
            "multiply" => d * v,
            "add" => d + v,
            "minimum" => d.min(v),
            "maximum" => d.max(v),
            _ => v,
        };
        d_out.push(res.clamp(0.0, 1.0) as f32);
    }

    let mut attrs = HashMap::new();
    attrs.insert("density".into(), NewAttrVal::F32(1, d_out));
    cloud(&points, None, Some(attrs))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
