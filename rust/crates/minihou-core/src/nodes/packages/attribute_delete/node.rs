use crate::{error::Result, geometry::Geometry, nodes::{attribs::matches, Inputs, NodeSpec, GEO}, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "attribute_delete",
    inputs: GEO,
    params: crate::params!("names": ""),
    cook: delete,
};

fn delete(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let text = p.s("names")?;
    let pats: Vec<_> = text.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty()).collect();
    let mut out = (*g).clone();
    for table in [&mut out.point, &mut out.vertex, &mut out.prim] {
        table.retain(|n, _| n == "P" || !pats.iter().any(|pat| matches(pat, n)));
    }
    out.detail.retain(|n, _| !pats.iter().any(|pat| matches(pat, n)));
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
