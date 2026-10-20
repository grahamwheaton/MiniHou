use super::*;
use super::loops::{key, slide_selected_edges};

pub(super) fn polysplit(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = POLYSPLIT.one(inputs, 0)?;
    polygons(&g, "polysplit")?;
    if p.e("mode", &["edge_loop", "points"])? == "points" {
        return split_points(&g, p);
    }
    let text = p.s("split_edges")?;
    if text.trim().is_empty() {
        return Ok((*g).clone());
    }
    let e = topo::edges(&g.topology, g.npoints);
    let sel = edges::edge_mask(&g, &text)?;
    let (_, nx, _) = corners(&g);
    let vp = &g.topology.vertex_point;
    let lt = LoopTopology::with_edges(&g, &e);
    let quads = &lt.quads;
    let seeds = (0..e.len())
        .filter(|&i| sel[i])
        .map(|i| (e.points[2 * i] as usize, e.points[2 * i + 1] as usize));
    let loop_ring = lt.ring(seeds);
    let (ring, split) = (&loop_ring.dirs, &loop_ring.quads);
    if ring.is_empty() {
        return Ok((*g).clone());
    }
    let loops = p.i("loops")?;
    if loops < 1 || loops as i64 > LOOP_CUT_MAX as i64 {
        return Err(Error::new("loops must be 1..64"));
    }
    let n = loops as usize;
    let position = p.f("position")?;
    if !(0.0..=1.0).contains(&position) {
        return Err(Error::new("position must be 0..1"));
    }
    let factor=p.f("factor")?;
    if !(-1. ..=1.).contains(&factor) { return Err(Error::new("factor must be -1..1")); }
    let smoothness=p.f("smoothness")?;
    if !(-1. ..=1.).contains(&smoothness) { return Err(Error::new("smoothness must be -1..1")); }
    let falloff=p.e("falloff", &["inverse_square","linear","smooth"])?;
    let fractions = loop_cut_fractions(n,if factor==0. { position } else { 0.5 });
    let keys: Vec<_> = ring.keys().copied().collect();
    let mut ia = Vec::new();
    let mut ib = Vec::new();
    let mut fr = Vec::new();
    for k in &keys {
        let (a, b) = ring[k];
        for &f in &fractions {
            ia.push(a);
            ib.push(b);
            fr.push(f);
        }
    }
    let point = concat(&g.point, &lerp(&g.point, &ia, &ib, &fr));
    let along = |a: usize, b: usize| {
        let ki = keys.binary_search(&key(a, b)).unwrap();
        let (js, fs) = loop_ring.along(&fractions, a, b);
        let ids: Vec<_> = js.into_iter().map(|j| g.npoints + ki * n + j).collect();
        (ids, fs)
    };
    type Row = (usize, usize, f64, usize);
    let mut faces = Vec::<Vec<Row>>::new();
    let mut sources = Vec::new();
    let mut cut_pairs = Vec::new();
    for i in 0..g.nprims() {
        let r = range(&g, i);
        if let Some(&(entry, _)) = split.get(&i) {
            let q = &quads[&i];
            let k = q.iter().position(|&v| v == entry.0).unwrap();
            let forward = q[(k + 1) % 4] == entry.1;
            let order: Vec<_> = (0..4)
                .map(|s| {
                    if forward {
                        (k + s) % 4
                    } else {
                        (k + 4 - s) % 4
                    }
                })
                .collect();
            let ra: Vec<_> = order.iter().map(|&o| r.start + o).collect();
            let pa: Vec<_> = order.iter().map(|&o| q[o]).collect();
            let (ab, fa) = along(pa[0], pa[1]);
            let (dc, fd) = along(pa[3], pa[2]);
            let mut top = vec![(ra[0], ra[0], 0., pa[0])];
            let mut bot = vec![(ra[3], ra[3], 0., pa[3])];
            top.extend(ab.iter().zip(&fa).map(|(&id, &f)| (ra[0], ra[1], f, id)));
            bot.extend(dc.iter().zip(&fd).map(|(&id, &f)| (ra[3], ra[2], f, id)));
            top.push((ra[1], ra[1], 0., pa[1]));
            bot.push((ra[2], ra[2], 0., pa[2]));
            for j in 0..n + 1 {
                let mut f = vec![top[j], top[j + 1], bot[j + 1], bot[j]];
                if !forward {
                    f.reverse();
                }
                faces.push(f);
                sources.push(i);
            }
            for j in 1..n + 1 {
                cut_pairs.push((top[j].3, bot[j].3));
            }
            continue;
        }
        let mut face = Vec::new();
        for c in r {
            let a = vp[c] as usize;
            let b = vp[nx[c]] as usize;
            face.push((c, c, 0., a));
            if ring.contains_key(&key(a, b)) {
                let (ids, fs) = along(a, b);
                face.extend(ids.iter().zip(&fs).map(|(&id, &f)| (c, nx[c], f, id)));
            }
        }
        faces.push(face);
        sources.push(i);
    }
    let rows: Vec<_> = faces.iter().flatten().copied().collect();
    let ia: Vec<_> = rows.iter().map(|r| r.0).collect();
    let ib: Vec<_> = rows.iter().map(|r| r.1).collect();
    let ft: Vec<_> = rows.iter().map(|r| r.2).collect();
    let closed = g
        .topology
        .closed
        .as_ref()
        .map(|c| sources.iter().map(|&i| c[i]).collect());
    let mut out = make(
        point,
        rows.iter().map(|r| r.3 as i32).collect(),
        &faces.iter().map(Vec::len).collect::<Vec<_>>(),
        lerp(&g.vertex, &ia, &ib, &ft),
        gather(&g.prim, &sources),
        g.detail.clone(),
        closed,
    )?;
    edges::remap(&g, &mut out, None)?;
    let np = out.npoints;
    outgroup(
        &mut out,
        p,
        "points",
        "point",
        &(0..np).map(|i| i >= g.npoints).collect::<Vec<_>>(),
    )?;
    let mask = pair_mask(&out, &cut_pairs);
    outgroup(&mut out, p, "edges", "edge", &mask)?;
    if smoothness != 0. {
        let mut neighbours=BTreeMap::<usize,Vec<usize>>::new();
        for &(a,b) in &cut_pairs { neighbours.entry(a).or_default().push(b); neighbours.entry(b).or_default().push(a); }
        let old=out.p().to_vec(); let mut new=old.clone();
        let weight=match falloff.as_str() { "linear"=>smoothness, "smooth"=>smoothness.abs().powi(2)*(3.-2.*smoothness.abs())*smoothness.signum(), _=>smoothness.abs().powi(2)*smoothness.signum() };
        for (&v,n) in &neighbours { if n.len()!=2 { continue; }
            for k in 0..3 { let mid=(old[3*n[0]+k] as f64+old[3*n[1]+k] as f64)*0.5;
                new[3*v+k]=(old[3*v+k] as f64+(mid-old[3*v+k] as f64)*weight) as f32; }
        }
        out.set_p(new);
    }
    if factor != 0. {
        out=slide_selected_edges(&out,&cut_pairs.iter().map(|&(a,b)|key(a,b)).collect(),factor,
            p.b("even")?,p.b("flipped")?,p.b("clamp")?,p.b("correct_uvs")?);
    }
    Ok(out)
}
fn split_points(g: &Geometry, p: &Params) -> Result<Geometry> {
    let pairs = parse_pairs(&p.s("split_points")?, g.npoints)?;
    if pairs.is_empty() {
        return Ok(g.clone());
    }
    let mut faces: Vec<Vec<_>> = (0..g.nprims()).map(|i| range(g, i).collect()).collect();
    let mut sources: Vec<_> = (0..g.nprims()).collect();
    let mut cuts = Vec::new();
    for (a, b) in pairs {
        let mut found = false;
        for fi in 0..faces.len() {
            let f = &faces[fi];
            let ia = f
                .iter()
                .position(|&v| g.topology.vertex_point[v] as usize == a);
            let ib = f
                .iter()
                .position(|&v| g.topology.vertex_point[v] as usize == b);
            if let (Some(i), Some(j)) = (ia, ib) {
                if f.len() <= 3 {
                    continue;
                }
                let (i, j) = (i.min(j), i.max(j));
                if j - i < 2 || f.len() - (j - i) < 2 {
                    continue;
                }
                let f1 = f[i..=j].to_vec();
                let f2 = f[j..].iter().chain(&f[..=i]).copied().collect();
                faces[fi] = f1;
                faces.push(f2);
                sources.push(sources[fi]);
                cuts.push((a, b));
                found = true;
                break;
            }
        }
        if !found {
            return Err(Error::new(format!(
                "polysplit: no face has both points {a} and {b} (not neighbours)"
            )));
        }
    }
    let closed = g
        .topology
        .closed
        .as_ref()
        .map(|c| sources.iter().map(|&i| c[i]).collect());
    let mut out = edges::rebuild_edges(g, &faces, &sources, closed)?;
    let mask = pair_mask(&out, &cuts);
    outgroup(&mut out, p, "edges", "edge", &mask)?;
    Ok(out)
}
