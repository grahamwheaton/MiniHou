//! The nodes this backend can cook. Each one mirrors the Python node of the same name: same parameters and defaults, same
//! output (exact topology and attribute names; floats within rounding). Parameters it does not implement yet are declared
//! `unsupported`, so a graph that sets one fails loudly instead of cooking something different.

use std::sync::Arc;

use crate::error::{Error, Result};
use crate::geometry::Geometry;
use crate::params::{ParamSpec, Params};

mod generate;
mod groups;
mod merge;
mod modify;

/// One input port.
pub struct InputSpec {
    pub name: &'static str,
    /// Takes any number of connections (Python's `Input(multi=True)`).
    pub multi: bool,
    pub optional: bool,
}

/// What a node's function receives for each input port, in declaration order: the connected geometries (none, one, or for
/// a multi input any number).
pub type Inputs = [Vec<Arc<Geometry>>];

pub struct NodeSpec {
    pub name: &'static str,
    pub inputs: &'static [InputSpec],
    pub params: &'static [ParamSpec],
    pub cook: fn(&Inputs, &Params) -> Result<Geometry>,
}

impl NodeSpec {
    /// The single geometry on input port `i` (an error naming the port when nothing is connected).
    pub fn one(&self, inputs: &Inputs, i: usize) -> Result<Arc<Geometry>> {
        inputs[i].first().cloned().ok_or_else(|| Error::new(format!("input '{}' is not connected", self.inputs[i].name)))
    }
}

const GEO: &[InputSpec] = &[InputSpec { name: "geo", multi: false, optional: false }];

/// Every node type, by name.
pub fn registry() -> &'static [NodeSpec] {
    static NODES: &[NodeSpec] = &[
        generate::GRID,
        generate::BOX,
        generate::POINT,
        generate::CURVE,
        generate::LINE,
        generate::CIRCLE,
        generate::SPHERE,
        generate::TUBE,
        generate::TORUS,
        generate::BOUND,
        modify::TRANSFORM,
        modify::NORMAL,
        modify::NOISE,
        merge::MERGE,
        NodeSpec { name: "lod_bake", inputs: GEO, params: crate::params!("cluster_size": 128; "error_px": 1.0; "enabled": true; "min_triangles": 50000), cook: lod_bake },
        NodeSpec { name: "null", inputs: GEO, params: crate::params!("label": ""), cook: pass_through },
        NodeSpec { name: "output", inputs: GEO, params: crate::params!("object": ""), cook: pass_through },
    ];
    NODES
}

pub fn get(name: &str) -> Option<&'static NodeSpec> {
    registry().iter().find(|n| n.name == name)
}

fn pass_through(inputs: &Inputs, _p: &Params) -> Result<Geometry> {
    inputs[0].first().map(|g| (**g).clone()).ok_or_else(|| Error::new("input 'geo' is not connected"))
}

fn lod_bake(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let cluster_size=p.i("cluster_size")?;
    let min_triangles=p.i("min_triangles")?;
    let error_px=p.f("error_px")?;
    p.b("enabled")?;
    if !(16..=128).contains(&cluster_size) || min_triangles < 0 || !(error_px as f32).is_finite() || error_px <= 0.0 {
        return Err(Error::new("cluster_size must be 16..128, min_triangles must be nonnegative, and error_px must be positive and finite"));
    }
    pass_through(inputs,p)
}

/// `np.linspace(start, stop, num)` in float64: `i * step + start`, with the last value exactly `stop`.
pub(crate) fn linspace(start: f64, stop: f64, num: usize) -> Vec<f64> {
    if num == 0 {
        return Vec::new();
    }
    if num == 1 {
        return vec![start];
    }
    let step = (stop - start) / (num - 1) as f64;
    let mut v: Vec<f64> = (0..num).map(|i| i as f64 * step + start).collect();
    v[num - 1] = stop;
    v
}
