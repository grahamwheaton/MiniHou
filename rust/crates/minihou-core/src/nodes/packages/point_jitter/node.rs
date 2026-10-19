use crate::nodes::{Inputs, NodeSpec, GEO};
use crate::{error::Result, geometry::Geometry, params::Params};
pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "point_jitter",
    inputs: GEO,
    params: crate::params!(
        "scale": [0.1, 0.1, 0.1];
        "seed": 0;
        "group": "";
    ),
    cook: point_jitter,
};

fn point_jitter(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let scale = p.v3("scale")?;
    let seed = p.i("seed")?;
    let group = p.s("group")?;
    let sel = crate::nodes::group::mask(&g, "point", &group)?;
    let mut rng = crate::nodes::np_random::Rng::new(&[(seed as u64) & 0xFFFFFFFF]);
    let mut positions = g.p().to_vec();
    for i in 0..g.npoints {
        let rx = rng.random();
        let ry = rng.random();
        let rz = rng.random();
        if sel[i] {
            positions[i * 3] += ((rx * 2.0 - 1.0) * scale[0]) as f32;
            positions[i * 3 + 1] += ((ry * 2.0 - 1.0) * scale[1]) as f32;
            positions[i * 3 + 2] += ((rz * 2.0 - 1.0) * scale[2]) as f32;
        }
    }
    let mut out = (*g).clone();
    out.set_p(positions);
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
