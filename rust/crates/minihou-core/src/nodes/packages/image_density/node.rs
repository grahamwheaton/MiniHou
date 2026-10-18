use std::sync::Arc;
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry}, nodes::{InputSpec, Inputs, NodeSpec}, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec {
    name: "image_density",
    inputs: &[InputSpec { name: "geometry", multi: false, optional: false }],
    params: crate::params!("channel": "luminance"; "invert": true; "contrast": 1.0; "brightness": 0.0; "threshold": 0.0; "output_attribute": "density"),
    cook,
};

fn cook(inputs: &Inputs, params: &Params) -> Result<Geometry> {
    let source = NODE.one(inputs, 0)?;
    let cd = source.point.get("Cd").ok_or_else(|| Error::new("image_density needs point colour attribute 'Cd'"))?;
    if cd.width == 0 || cd.rows() != source.npoints {
        return Err(Error::new("image_density: Cd must have one row per point and at least one channel"));
    }
    let channel = params.e("channel", &["luminance", "red", "green", "blue", "alpha"])?;
    let contrast = params.f("contrast")?;
    let brightness = params.f("brightness")?;
    let threshold = params.f("threshold")?;
    let invert = params.b("invert")?;
    if !contrast.is_finite() || !brightness.is_finite() || !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
        return Err(Error::new("image_density: contrast and brightness must be finite; threshold must be in [0, 1]"));
    }
    let output = params.s("output_attribute")?;
    if output.is_empty() || output == "P" { return Err(Error::new("image_density: output attribute must be a nonempty name other than P")); }
    let alpha = source.point.get("Alpha");
    if channel == "alpha" {
        if let Some(a) = alpha { if a.width != 1 || a.rows() != source.npoints { return Err(Error::new("image_density: Alpha must be one float per point")); } }
    }
    let values: Vec<f32> = (0..source.npoints).map(|i| {
        let sample = |a: &Attr, c: usize| -> f64 {
            let index = i * a.width + c;
            match &a.data { AttrData::F32(v) => v[index] as f64, AttrData::I32(v) => v[index] as f64 }
        };
        let rgb = |c: usize| -> f64 { if cd.width == 1 { sample(cd, 0) } else if c < cd.width { sample(cd, c) } else { 0.0 } };
        let raw = match channel.as_str() {
            "red" => rgb(0), "green" => rgb(1), "blue" => rgb(2),
            "alpha" => alpha.map(|a| sample(a, 0)).unwrap_or_else(|| if cd.width >= 4 { sample(cd, 3) } else { 1.0 }),
            _ => 0.2126 * rgb(0) + 0.7152 * rgb(1) + 0.0722 * rgb(2),
        };
        if !raw.is_finite() { return 0.0; }
        let adjusted = ((raw - 0.5) * contrast + 0.5 + brightness).clamp(0.0, 1.0);
        let value = if invert { 1.0 - adjusted } else { adjusted };
        (if threshold > 0.0 { if value >= threshold { 1.0 } else { 0.0 } } else { value }) as f32
    }).collect();
    let mut result = (*source).clone();
    result.point.insert(output, Arc::new(Attr::f32(1, values)));
    Ok(result)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
