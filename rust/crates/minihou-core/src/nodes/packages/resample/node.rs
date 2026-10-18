use crate::{
    error::{Error, Result},
    geometry::Geometry,
    nodes::{
        curves::{count, lengths, sample},
        group::mask,
        linspace, Inputs, NodeSpec, GEO,
    },
    params::Params,
};

pub(super) const NODE: NodeSpec = NodeSpec {
    name: "resample",
    inputs: GEO,
    params: crate::params!("by":"length";"length":0.1;"segments":20;"group":""),
    cook: resample,
};

fn resample(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let sel = mask(&g, "prim", &p.s("group")?)?;
    let by = p.e("by", &["length", "count"])?;
    let length = p.f("length")?;
    let segments = p.i("segments")?;
    if length <= 0. || segments < 1 {
        return Err(Error::new("length must be positive and segments at least 1"));
    }
    let mut prims = Vec::new();
    let mut lists = Vec::new();
    let mut count_out = 0usize;
    for (i, &selected) in sel.iter().enumerate() {
        if !selected || g.topology.prim_offsets[i + 1] - g.topology.prim_offsets[i] < 2 {
            continue;
        }
        let (_, _, total) = lengths(&g, i);
        let n = if by == "count" {
            segments as usize
        } else {
            (total / length - 1e-9).ceil().max(1.) as usize
        };
        count_out = count_out
            .checked_add(n)
            .and_then(|v| v.checked_add(1))
            .ok_or_else(|| Error::new("resample count overflow"))?;
        count(count_out)?;
        let mut s = linspace(0., total, n + 1);
        if g.topology.is_closed(i) {
            s.pop();
        }
        prims.push(i);
        lists.push(s);
    }
    sample(&g, &prims, &lists)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
