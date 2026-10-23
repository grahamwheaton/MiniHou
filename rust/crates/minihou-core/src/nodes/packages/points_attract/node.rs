use crate::error::Result;
use crate::geometry::Geometry;
use crate::nodes::{Inputs, NodeSpec, GEO};
use crate::params::Params;

pub(super) const NODE: NodeSpec = NodeSpec {
    outputs: &["geometry"],
    cook_multi: None,
    cook_sidecar: None,
    name: "points_attract",
    inputs: GEO,
    params: crate::params!(
        "group": "";
        "domain": "points";
        "strength": 0.5;
        "iterations": 1;
    ),
    cook: points_attract,
};

fn points_attract(inputs: &Inputs, params: &Params) -> Result<Geometry> {
    let geometry = NODE.one(inputs, 0)?;
    let group = params.s("group")?;
    let domain = params.e("domain", &["points", "primitives"])?;
    let strength = params.f("strength")?.clamp(0.0, 1.0);
    let iterations = params.i("iterations")?.max(0) as usize;

    if geometry.npoints < 2 || strength == 0.0 || iterations == 0 {
        return Ok((*geometry).clone());
    }

    let has_group = !group.trim().is_empty();
    let targets = if !has_group {
        vec![true; geometry.npoints]
    } else if domain == "points" {
        crate::nodes::group::mask(&geometry, "point", &group)?
    } else {
        let primitives = crate::nodes::group::mask(&geometry, "prim", &group)?;
        let mut points = vec![false; geometry.npoints];
        for (prim, selected) in primitives.iter().enumerate() {
            if *selected {
                let start = geometry.topology.prim_offsets[prim] as usize;
                let end = geometry.topology.prim_offsets[prim + 1] as usize;
                for &point in &geometry.topology.vertex_point[start..end] {
                    points[point as usize] = true;
                }
            }
        }
        points
    };

    if !targets.iter().any(|&selected| selected) {
        return Ok((*geometry).clone());
    }

    let target_indices: Vec<usize> = targets
        .iter()
        .enumerate()
        .filter_map(|(index, &selected)| selected.then_some(index))
        .collect();
    let mut positions = geometry.p().to_vec();
    for _ in 0..iterations {
        let previous = positions.clone();
        for point in 0..geometry.npoints {
            if has_group && targets[point] {
                continue;
            }
            let base = point * 3;
            let nearest = target_indices
                .iter()
                .copied()
                .filter(|&target| target != point)
                .min_by(|&a, &b| {
                    let distance = |target: usize| -> f64 {
                        (0..3)
                            .map(|axis| {
                                let difference = previous[base + axis] as f64
                                    - previous[target * 3 + axis] as f64;
                                difference * difference
                            })
                            .sum()
                    };
                    distance(a).total_cmp(&distance(b)).then(a.cmp(&b))
                });
            if let Some(target) = nearest {
                for axis in 0..3 {
                    positions[base + axis] = previous[base + axis]
                        + strength as f32 * (previous[target * 3 + axis] - previous[base + axis]);
                }
            }
        }
    }

    let mut output = (*geometry).clone();
    output.set_p(positions);
    Ok(output)
}
