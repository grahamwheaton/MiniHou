use crate::{error::{Error, Result}, geometry::Geometry, nodes::{Inputs, NodeSpec, GEO}, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec {
    name: "attribute_rename",
    inputs: GEO,
    params: crate::params!("from_name": ""; "to_name": ""),
    cook: rename,
};

fn rename(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let from = p.s("from_name")?;
    let to = p.s("to_name")?;
    if from.is_empty() || to.is_empty() || from == to { return Ok((*g).clone()); }
    if from == "P" || to == "P" { return Err(Error::new("P cannot be renamed")); }
    let src = [&g.point, &g.vertex, &g.prim].iter()
        .position(|t| t.contains_key(&from))
        .ok_or_else(|| Error::new(format!("no attribute {from:?}")))?;
    let mut out = (*g).clone();
    let mut tables = [&mut out.point, &mut out.vertex, &mut out.prim];
    let a = tables[src].remove(&from).unwrap();
    for t in &mut tables { t.remove(&to); }
    tables[src].insert(to, a);
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
