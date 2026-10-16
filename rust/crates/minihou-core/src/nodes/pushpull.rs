//! Bounded solid Push/Pull and named inset/outset, from pushpull.py.
use super::{GEO, Inputs, NodeSpec, model};
use super::{boolean, model::*, modeling_tools::twins};
use crate::{
    error::{Error, Result},
    geometry::Geometry,
    params::Params,
};
pub const PUSHPULL: NodeSpec = NodeSpec {
    name: "pushpull",
    inputs: GEO,
    params: crate::params!("group":"";"distance":0.25),
    cook: pushpull,
};
fn solid(g: &Geometry, label: &str) -> Result<()> {
    polygons(g, label)?;
    if g.nprims() > 0
        && (twins(g).iter().any(Option::is_none) || g.p().iter().any(|x| !x.is_finite()))
    {
        return Err(Error::new(format!(
            "{label} requires a closed, consistently wound manifold solid"
        )));
    }
    let pos = positions(g);
    for face in 0..g.nprims() {
        let q: Vec<_> = range(g, face)
            .map(|v| pos[g.topology.vertex_point[v] as usize])
            .collect();
        if q.len() > 256 {
            return Err(Error::new(format!(
                "{label} supports polygons with at most 256 corners; triangulate first"
            )));
        }
        let mut n = [0.; 3];
        for k in 0..q.len() {
            n = add(n, cross(q[k], q[(k + 1) % q.len()]));
        }
        let axis = (0..3)
            .max_by(|&a, &b| n[a].abs().total_cmp(&n[b].abs()).then(b.cmp(&a)))
            .unwrap();
        let xy: Vec<_> = q
            .iter()
            .map(|p| {
                let v: Vec<_> = (0..3).filter(|&k| k != axis).map(|k| p[k]).collect();
                [v[0], v[1]]
            })
            .collect();
        let cr = crate::boolean_kernel::cross2;
        for i in 0..q.len() {
            for j in i + 2..q.len() - usize::from(i == 0) {
                let (a, b, c, d) = (xy[i], xy[(i + 1) % q.len()], xy[j], xy[(j + 1) % q.len()]);
                if cr(a, b, c) * cr(a, b, d) < 0. && cr(c, d, a) * cr(c, d, b) < 0. {
                    let ei = [b[0] - a[0], b[1] - a[1]];
                    let ej = [d[0] - c[0], d[1] - c[1]];
                    let angle = (ei[0] * ej[1] - ei[1] * ej[0]).abs()
                        / (ei[0].hypot(ei[1]) * ej[0].hypot(ej[1])).max(1e-30);
                    if angle > 1e-3 {
                        return Err(Error::new(format!(
                            "{label} contains a self-crossing polygon"
                        )));
                    }
                }
            }
        }
    }
    if g.nprims() > 0 {
        let (ts, _, _) = boolean::triangles(g);
        let volume = ts
            .iter()
            .map(|t| dot(pos[t[0]], cross(pos[t[1]], pos[t[2]])))
            .sum::<f64>()
            / 6.;
        if volume <= 0. {
            return Err(Error::new(format!(
                "{label} requires outward-facing solid geometry; reverse inward faces first"
            )));
        }
    }
    Ok(())
}
fn pushpull(i: &Inputs, p: &Params) -> Result<Geometry> {
    let g = PUSHPULL.one(i, 0)?;
    apply(&g, &p.s("group")?, p.f("distance")?)
}
pub(super) fn apply(g: &Geometry, group: &str, distance: f64) -> Result<Geometry> {
    if g.nprims() > 20000 || g.npoints > 50000 {
        return Err(Error::new(
            "Push/Pull supports up to 20,000 faces / 50,000 points; simplify the solid first",
        ));
    }
    solid(g, "Push/Pull")?;
    let selected = prim_mask(g, group)?;
    let ids: Vec<_> = (0..g.nprims()).filter(|&i| selected[i]).collect();
    if ids.len() != 1 {
        return Err(Error::new(
            "Push/Pull: select exactly one planar convex face (use Inset for a smaller opening)",
        ));
    }
    if !distance.is_finite() {
        return Err(Error::new("Push/Pull distance must be finite"));
    }
    let face = ids[0];
    let pos = positions(g);
    let points: Vec<_> = range(g, face)
        .map(|v| pos[g.topology.vertex_point[v] as usize])
        .collect();
    let normal = crate::normals::prim_normals(g.p(), &g.topology)[face];
    let n = crate::boolean_kernel::unit(normal);
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in &pos {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let extent = (0..3).map(|k| hi[k] - lo[k]).fold(1e-8, f64::max);
    let tolerance = extent * 1e-6;
    if points.len() < 3
        || len(n) < 0.5
        || points
            .iter()
            .any(|&p| dot(sub(p, points[0]), n).abs() > tolerance)
    {
        return Err(Error::new(
            "Push/Pull: selected face must be non-degenerate and planar",
        ));
    }
    let count = points.len();
    let edges: Vec<_> = (0..count)
        .map(|i| sub(points[(i + 1) % count], points[i]))
        .collect();
    if (0..count).any(|i| dot(cross(edges[i], edges[(i + 1) % count]), n) < -tolerance * tolerance)
    {
        return Err(Error::new(
            "Push/Pull: concave faces are not supported; split into convex faces first",
        ));
    }
    if distance.abs() <= tolerance {
        return Ok(g.clone());
    }
    let overlap = tolerance * 10.;
    let (low, high) = if distance > 0. {
        (-overlap, distance)
    } else {
        (distance, overlap)
    };
    let cutterpos: Vec<_> = [low, high]
        .iter()
        .flat_map(|&d| points.iter().map(move |&p| add(p, mul(n, d))))
        .collect();
    let mut faces = vec![
        (0..count).rev().collect::<Vec<_>>(),
        (count..2 * count).collect(),
    ];
    for i in 0..count {
        faces.push(vec![i, (i + 1) % count, (i + 1) % count + count, i + count]);
    }
    let mut cutter = Geometry::from_arrays(
        cutterpos.iter().flat_map(|p| p.map(|v| v as f32)).collect(),
        faces.iter().flatten().map(|&v| v as i32).collect(),
        {
            let mut o = vec![0];
            for f in &faces {
                o.push(o.last().unwrap() + f.len() as i32);
            }
            o
        },
        None,
    )?;
    cutter.prim = gather(&g.prim, &vec![face; faces.len()]);
    cutter.detail = g.detail.clone();
    let origin = mul(points.iter().copied().fold([0.; 3], add), 1. / count as f64);
    let mut last = String::new();
    for angle in [0f64, 0.37, 0.91, 1.57] {
        let u = crate::boolean_kernel::unit(edges[0]);
        let v = cross(n, u);
        let (sin, cos) = angle.sin_cos();
        let ur = add(mul(u, cos), mul(v, sin));
        let vr = sub(mul(v, cos), mul(u, sin));
        let basis = [ur, vr, n];
        let mut local = g.clone();
        let mut tool = cutter.clone();
        let transform = |points: &[[f64; 3]]| {
            points
                .iter()
                .flat_map(|&p| basis.map(|a| dot(sub(p, origin), a) as f32))
                .collect()
        };
        local.set_p(transform(&pos));
        tool.set_p(transform(&positions(&cutter)));
        let attempt = boolean::boolean(
            &local,
            &tool,
            if distance > 0. { "union" } else { "subtract" },
            None,
        )
        .and_then(|mut out| {
            let points = positions(&out);
            out.set_p(
                points
                    .iter()
                    .flat_map(|p| {
                        add(origin, add(add(mul(ur, p[0]), mul(vr, p[1])), mul(n, p[2])))
                            .map(|v| v as f32)
                    })
                    .collect(),
            );
            solid(&out, "Push/Pull result")?;
            out.validate()?;
            Ok(out)
        });
        match attempt {
            Ok(out) => return Ok(out),
            Err(e) => last = e.to_string(),
        }
    }
    Err(Error::new(format!(
        "Push/Pull could not produce a watertight solid; adjust the distance or split the face. {last}"
    )))
}
pub const INSET: NodeSpec = NodeSpec {
    name: "inset",
    inputs: GEO,
    params: crate::params!("group":"";"amount":0.1;"mode":"connected"),
    cook: inset,
};
pub const OUTSET: NodeSpec = NodeSpec {
    name: "outset",
    inputs: GEO,
    params: crate::params!("group":"";"amount":0.1;"mode":"connected"),
    cook: outset,
};
fn inset(i: &Inputs, p: &Params) -> Result<Geometry> {
    let g = INSET.one(i, 0)?;
    model::extrude(
        &g,
        &p.s("group")?,
        0.,
        p.f("amount")?,
        &p.e("mode", &["connected", "individual"])?,
        false,
        None,
    )
}
fn outset(i: &Inputs, p: &Params) -> Result<Geometry> {
    let g = OUTSET.one(i, 0)?;
    model::extrude(
        &g,
        &p.s("group")?,
        0.,
        -p.f("amount")?,
        &p.e("mode", &["connected", "individual"])?,
        false,
        None,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    #[test]
    fn inset_has_no_height_and_python_groups() {
        let g = Geometry::from_arrays(
            vec![0., 0., 0., 2., 0., 0., 2., 2., 0., 0., 2., 0.],
            vec![0, 1, 2, 3],
            vec![0, 4],
            None,
        )
        .unwrap();
        let p = Params::resolve(INSET.params, None).unwrap();
        let o = inset(&[vec![Arc::new(g)]], &p).unwrap();
        assert_eq!((o.npoints, o.nprims()), (8, 5));
        assert!(o.p().chunks_exact(3).all(|p| p[2] == 0.));
        assert!(o.prim.contains_key("group_extrudeFront"));
        assert!(o.prim.contains_key("group_extrudeSide"));
    }
}
