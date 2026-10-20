use super::*;

pub fn apply_through(
    g: &Geometry,
    group: &str,
    distance: f64,
    through: Through,
) -> Result<Geometry> {
    if g.nprims() > 20000 || g.npoints > 50000 {
        return Err(Error::new(
            "Push/Pull supports up to 20,000 faces / 50,000 points; simplify the solid first",
        ));
    }
    if !distance.is_finite() && through != Through::Off {
        return Err(Error::new("Push/Pull distance must be finite"));
    }
    if through != Through::Off && g.nprims() > 0 && twins(g).iter().any(Option::is_none) {
        // Open sheet: there is no opposite surface to punch, so this is a plain push/pull.
        polygons(g, "Push/Pull")?;
        if !prim_mask(g, group)?.iter().any(|&x| x) {
            return Err(Error::new("Push/Pull: select at least one face"));
        }
        return model::extrude(g, group, distance, 0., "individual", false, None);
    }
    solid(g, "Push/Pull")?;
    let selected = prim_mask(g, group)?;
    let ids: Vec<_> = (0..g.nprims()).filter(|&i| selected[i]).collect();
    if through == Through::Off && ids.len() != 1 {
        return Err(Error::new(
            "Push/Pull: select exactly one planar convex face (use Inset for a smaller opening)",
        ));
    }
    if ids.is_empty() || ids.len() == g.nprims() {
        return Err(Error::new(
            "Push/Pull: select one or more faces, not the whole solid",
        ));
    }
    if !distance.is_finite() {
        return Err(Error::new("Push/Pull distance must be finite"));
    }
    let pos = positions(g);
    let extent = extent_of(&pos);
    let tolerance = extent * 1e-6;
    let regions = regions(g, &pos, &ids, tolerance, through == Through::Off)?;
    if distance.abs() <= tolerance && through != Through::Force {
        return Ok(g.clone());
    }
    let (tris, _, tri_prim) = boolean::triangles(g);
    let snap = extent * 1e-5;
    let overlap = tolerance * 10.;
    let mut merge = false;
    let mut plans = vec![];
    for r in &regions {
        let mut d = distance;
        if through != Through::Off {
            if d < 0. || through == Through::Force {
                let samples: Vec<_> = r
                    .prims
                    .iter()
                    .flat_map(|&f| face_samples(&pos, &tris, &tri_prim, f))
                    .collect();
                let dir = mul(r.n, -1.);
                let hit = exit_depth(&pos, &tris, &tri_prim, &r.prims, &samples, dir, tolerance);
                if let Some((_, deepest)) =
                    hit.filter(|&(_, deep)| through == Through::Force || -d >= deep - snap)
                {
                    // Stop just past the far surface: excess push is ignored, and the
                    // cutter never ends coplanar with the opposite face.
                    d = -(deepest + extent * 1e-4);
                }
            } else if let Some(t) = coplanar_target(g, &pos, &selected, r, d, snap) {
                d = t;
                merge = true;
            }
        }
        plans.push(d);
    }
    let mut out = g.clone();
    for (&d, r) in plans.iter().zip(&regions) {
        if d.abs() <= tolerance {
            continue;
        }
        let (low, high) = if d > 0. { (-overlap, d) } else { (d, overlap) };
        let op = if d > 0. { "union" } else { "subtract" };
        let expand = if through != Through::Off && d < 0. { extent * 1e-4 } else { 0. };
        out = cut(&out, g, &pos, r, low, high, op, merge, expand)?;
        if through != Through::Off {
            out = drop_collinear(&out, extent * 1e-6)?;
        }
    }
    if merge {
        out = merge_coplanar(&out, extent * 1e-5)?;
    }
    if out.prim.remove(TAG).is_some() {
        out.validate()?;
    }
    Ok(out)
}
