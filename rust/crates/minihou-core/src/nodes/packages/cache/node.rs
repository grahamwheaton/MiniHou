//! Explicit geometry cache. Session uses the input's cook key to reuse this result.
use crate::{error::{Error, Result}, geometry::Geometry,
    nodes::{InputSpec, Inputs, NodeSpec}, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "cache",
    inputs: &[InputSpec { name: "geo", multi: false, optional: false }],
    params: crate::params!("mode": "auto"; "cache_any_frame": true; "max_frames": 2500; "clear_on_upstream_change": true; "clear_cache": false; "clear_current_frame_cache": false),
    cook,
};

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    p.e("mode", &["auto", "off"])?;
    p.b("cache_any_frame")?;
    if p.i("max_frames")? < 1 { return Err(Error::new("max_frames must be positive")); }
    p.b("clear_on_upstream_change")?;
    p.b("clear_cache")?;
    p.b("clear_current_frame_cache")?;
    Ok((**inputs[0].first().ok_or_else(|| Error::new("geo is not connected"))?).clone())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
