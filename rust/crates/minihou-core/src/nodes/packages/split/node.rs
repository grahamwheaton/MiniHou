//! Split a primitive selection into two attribute-preserving pieces.
use crate::{
    error::Result,
    geometry::{AttrData, Geometry},
    nodes::{group, InputSpec, Inputs, NodeSpec},
    params::Params,
};

pub(super) const NODE: NodeSpec = NodeSpec {
    name: "split",
    inputs: &[InputSpec { name: "geo", multi: false, optional: false }],
    params: crate::params!("group": ""; "group_type": "primitives"; "invert_selection": false; "delete_unused_groups": false),
    cook,
};

fn selection(g: &Geometry, pattern: &str, group_type: &str) -> Result<Vec<bool>> {
    if pattern.trim().is_empty() {
        return Ok(vec![false; g.nprims()]);
    }
    match group_type {
        "primitives" | "faces" => group::mask(g, "prim", pattern),
        "points" => {
            let points = group::mask(g, "point", pattern)?;
            Ok(g.topology.prim_offsets.windows(2).map(|r| {
                let ids = &g.topology.vertex_point[r[0] as usize..r[1] as usize];
                !ids.is_empty() && ids.iter().all(|&id| points[id as usize])
            }).collect())
        }
        "edges" => {
            let selected = crate::nodes::edge_tools::edge_mask(g, pattern)?;
            let edges = crate::topo::edges(&g.topology, g.npoints);
            Ok(g.topology.prim_offsets.windows(2).map(|r| {
                let range = r[0] as usize..r[1] as usize;
                !range.is_empty() && range.into_iter().all(|v| {
                    let id = edges.of_vertex[v];
                    id >= 0 && selected[id as usize]
                })
            }).collect())
        }
        _ => unreachable!(),
    }
}

fn remove_empty_groups(g: &mut Geometry) {
    for table in [&mut g.point, &mut g.vertex, &mut g.edge, &mut g.prim] {
        table.retain(|name, attr| {
            if !name.starts_with("group_") && !name.starts_with("edge_group_") {
                return true;
            }
            match &attr.data {
                AttrData::I32(v) => v.iter().any(|&x| x != 0),
                AttrData::F32(v) => v.iter().any(|&x| x != 0.0),
            }
        });
    }
}

fn pieces(g: &Geometry, p: &Params) -> Result<(Geometry, Geometry)> {
    let pattern = p.s("group")?;
    let kind = p.e("group_type", &["primitives", "faces", "points", "edges"])?;
    let mut selected = selection(g, &pattern, &kind)?;
    // An empty selection is a no-op, including when Invert Selection is enabled.
    if !selected.iter().any(|&x| x) {
        return Ok((g.clone(), group::keep_prims(g, &vec![false; g.nprims()], true)?));
    }
    if p.b("invert_selection")? {
        for x in &mut selected { *x = !*x; }
    }
    let rest: Vec<_> = selected.iter().map(|&x| !x).collect();
    let mut first = group::keep_prims(g, &selected, true)?;
    let mut second = group::keep_prims(g, &rest, true)?;
    if p.b("delete_unused_groups")? {
        remove_empty_groups(&mut first);
        remove_empty_groups(&mut second);
    }
    Ok((first, second))
}

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    Ok(pieces(&g, p)?.0)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
