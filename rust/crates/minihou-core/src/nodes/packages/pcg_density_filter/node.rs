use crate::nodes::pcg::{cloud, random01, read_pcg};
use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::{error::Result, geometry::Geometry, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec {
    name: "pcg_density_filter",
    inputs: &[InputSpec {
        name: "points",
        multi: false,
        optional: false,
    }],
    params: crate::params!(
        "lower": 0.5;
        "upper": 1.0;
        "invert": false;
        "probabilistic": false;
        "seed": 0
    ),
    cook: pcg_density_filter,
};

fn pcg_density_filter(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let points = NODE.one(inputs, 0)?;
    let lower = p.f("lower")?;
    let upper = p.f("upper")?;
    let invert = p.b("invert")?;
    let probabilistic = p.b("probabilistic")?;
    let seed = p.i("seed")?;

    let a = read_pcg(&points);
    let mut keep = Vec::with_capacity(points.npoints);
    for i in 0..points.npoints {
        let d = a.density[i];
        let k = if probabilistic {
            random01(a.seed[i], seed, 0) < d
        } else {
            d >= lower && d <= upper
        };
        keep.push(if invert { !k } else { k });
    }
    cloud(&points, Some(&keep), None)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
