//! Split a primitive selection into two attribute-preserving pieces.
use crate::{
    error::Result,
    geometry::{AttrData, Geometry},
    nodes::{group, InputSpec, Inputs, NodeOutputs, NodeSpec},
    params::Params,
};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry", "remainder"], cook_multi: Some(cook_multi), cook_sidecar: None,
    name: "split",
    inputs: &[InputSpec { name: "geo", multi: false, optional: false }],
    params: crate::params!("group": ""; "group_type": "primitives"; "invert_selection": false; "delete_unused_groups": false),
    cook,
};

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
    let mut selected = if pattern.trim().is_empty() {
        vec![false; match kind.as_str() {
            "points" => g.npoints,
            "edges" => crate::topo::edges(&g.topology, g.npoints).len(),
            _ => g.nprims(),
        }]
    } else {
        match kind.as_str() {
            "points" => group::mask(g, "point", &pattern)?,
            "edges" => crate::nodes::edge_tools::edge_mask(g, &pattern)?,
            _ => group::mask(g, "prim", &pattern)?,
        }
    };
    // An empty selection is a no-op, including when Invert Selection is enabled.
    if !selected.iter().any(|&x| x) {
        return Ok((g.clone(), group::keep_prims(g, &vec![false; g.nprims()], true)?));
    }
    if p.b("invert_selection")? {
        for x in &mut selected { *x = !*x; }
    }
    let rest: Vec<_> = selected.iter().map(|&x| !x).collect();
    let (mut first, mut second) = match kind.as_str() {
        "points" => (crate::nodes::modify::keep_points(g, &selected)?,
            crate::nodes::modify::keep_points(g, &rest)?),
        "edges" => (keep_edges(g, &selected)?, keep_edges(g, &rest)?),
        _ => (group::keep_prims(g, &selected, true)?, group::keep_prims(g, &rest, true)?),
    };
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

fn cook_multi(inputs: &Inputs, p: &Params) -> Result<NodeOutputs> {
    let g = NODE.one(inputs, 0)?;
    let (selected, remainder) = pieces(&g, p)?;
    Ok(vec![selected, remainder].into())
}

fn keep_edges(g: &Geometry, selected: &[bool]) -> Result<Geometry> {
    let edges = crate::topo::edges(&g.topology, g.npoints);
    let chosen: Vec<_> = selected.iter().enumerate().filter_map(|(i, &yes)| yes.then_some(i)).collect();
    let mut points = vec![false; g.npoints];
    for &i in &chosen {
        points[edges.points[i * 2] as usize] = true;
        points[edges.points[i * 2 + 1] as usize] = true;
    }
    let retained: Vec<_> = points.iter().enumerate().filter_map(|(i, &yes)| yes.then_some(i)).collect();
    let mut remap = vec![-1i32; g.npoints];
    for (new, &old) in retained.iter().enumerate() { remap[old] = new as i32; }
    let vp = chosen.iter().flat_map(|&i| [remap[edges.points[i * 2] as usize],
        remap[edges.points[i * 2 + 1] as usize]]).collect();
    let offsets = (0..=chosen.len()).map(|i| (i * 2) as i32).collect();
    let positions = retained.iter().flat_map(|&i| g.p()[i * 3..i * 3 + 3].iter().copied()).collect();
    let mut out = Geometry::from_arrays(positions, vp, offsets, Some(vec![false; chosen.len()]))?;
    out.point = group::gather_table(&g.point, &retained);
    out.edge = group::gather_table(&g.edge, &chosen);
    out.detail = g.detail.clone();
    out.packed = std::sync::Arc::clone(&g.packed);
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
