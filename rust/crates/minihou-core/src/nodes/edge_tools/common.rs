use super::*;
pub(crate) fn values(g: &Geometry, name: &str) -> Result<Vec<f32>> {
    let e = topo::edges(&g.topology, g.npoints);
    let mut out = vec![0.0f32; e.len()];
    if let Some(a) = g.vertex.get(name) {
        if a.width != 1 {
            return Err(Error::new(format!(
                "{name} must be a finite scalar vertex attribute"
            )));
        }
        for v in 0..g.nvertices() {
            let x = match &a.data {
                AttrData::F32(a) => a[v],
                AttrData::I32(a) => a[v] as f32,
            };
            if !x.is_finite() || name == "creaseweight" && x < 0. {
                return Err(Error::new(format!("invalid {name}")));
            }
            let i = e.of_vertex[v];
            if i >= 0 {
                out[i as usize] = out[i as usize].max(x);
            }
        }
    }
    Ok(out)
}
pub(crate) fn write_values(
    g: &mut Geometry,
    name: &str,
    data: &[f32],
    integer: bool,
) -> Result<()> {
    let e = topo::edges(&g.topology, g.npoints);
    if data.len() != e.len() {
        return Err(Error::new(
            "edge attribute length does not match derived edges",
        ));
    }
    let v: Vec<_> = e
        .of_vertex
        .iter()
        .map(|&i| if i < 0 { 0. } else { data[i as usize] })
        .collect();
    g.vertex.insert(
        name.into(),
        Arc::new(if integer {
            Attr::i32(1, v.iter().map(|&x| x as i32).collect())
        } else {
            Attr::f32(1, v)
        }),
    );
    Ok(())
}
pub(crate) fn wildcard(p: &[u8], n: &[u8]) -> bool {
    match p {
        [] => n.is_empty(),
        [b'*', rest @ ..] => (0..=n.len()).any(|i| wildcard(rest, &n[i..])),
        [b'?', rest @ ..] => !n.is_empty() && wildcard(rest, &n[1..]),
        [b'[', rest @ ..] => {
            let skip = usize::from(rest.first() == Some(&b'!'));
            let first = skip + usize::from(rest.get(skip) == Some(&b']'));
            if let Some(end) = rest[first..]
                .iter()
                .position(|&c| c == b']')
                .map(|i| i + first)
            {
                let Some(&c) = n.first() else {
                    return false;
                };
                let mut hit = false;
                let mut i = skip;
                while i < end {
                    if i + 2 < end && rest[i + 1] == b'-' {
                        hit |= rest[i] <= c && c <= rest[i + 2];
                        i += 3;
                    } else {
                        hit |= rest[i] == c;
                        i += 1;
                    }
                }
                (hit != (skip == 1)) && wildcard(&rest[end + 1..], &n[1..])
            } else {
                n.first() == Some(&b'[') && wildcard(rest, &n[1..])
            }
        }
        [c, rest @ ..] => n.first() == Some(c) && wildcard(rest, &n[1..]),
    }
}
pub(crate) fn edge_mask(g: &Geometry, pattern: &str) -> Result<Vec<bool>> {
    let e = topo::edges(&g.topology, g.npoints);
    let tokens: Vec<_> = pattern
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .collect();
    if tokens.is_empty() {
        return Ok(vec![true; e.len()]);
    }
    let mut out = vec![tokens[0].starts_with('^'); e.len()];
    for token in tokens {
        let exclude = token.starts_with('^');
        let token = token.strip_prefix('^').unwrap_or(token);
        let mut hit = vec![false; e.len()];
        if let Some(pair) = token
            .strip_prefix('p')
            .and_then(|s| s.split_once('-'))
            .filter(|(a, b)| {
                a.chars().all(|c| c.is_ascii_digit()) && b.chars().all(|c| c.is_ascii_digit())
            })
        {
            let a: i64 = pair
                .0
                .parse()
                .map_err(|_| Error::new("invalid edge pair"))?;
            let b: i64 = pair
                .1
                .parse()
                .map_err(|_| Error::new("invalid edge pair"))?;
            for i in 0..e.len() {
                hit[i] = e.points[2 * i] == a.min(b) && e.points[2 * i + 1] == a.max(b);
            }
            if !hit.iter().any(|&x| x) {
                return Err(Error::new(format!("edge {token:?} does not exist")));
            }
        } else if token.as_bytes().first().is_some_and(u8::is_ascii_digit) {
            let (span, step) = token.split_once(':').unwrap_or((token, "1"));
            let step: usize = step
                .parse()
                .map_err(|_| Error::new("edge range stride must be a positive integer"))?;
            if step == 0 {
                return Err(Error::new("edge range stride must be a positive integer"));
            }
            let (a, b) = span.split_once('-').unwrap_or((span, span));
            let a: usize = a.parse().map_err(|_| Error::new("invalid edge range"))?;
            let b: usize = b.parse().map_err(|_| Error::new("invalid edge range"))?;
            if a < e.len() {
                for i in (a..=b.min(e.len() - 1)).step_by(step) {
                    hit[i] = true;
                }
            }
        } else if token == "*" {
            hit.fill(true);
        } else {
            let mut found = false;
            // Group Create stores edge groups on the derived-edge domain.
            for (key, attr) in g.edge.iter().filter(|(k, _)| k.starts_with("group_")) {
                if wildcard(token.as_bytes(), key[6..].as_bytes()) {
                    found = true;
                    let members = super::super::group::flags(attr);
                    if attr.width != 1 || members.len() != e.len() {
                        return Err(Error::new(format!("edge group {key:?} has invalid dimensions")));
                    }
                    for (b, selected) in hit.iter_mut().zip(members) {
                        *b |= selected;
                    }
                }
            }
            // Older Edge Group nodes store membership on outgoing corners.
            for key in g.vertex.keys().filter(|k| k.starts_with("edge_group_")) {
                if wildcard(token.as_bytes(), key[11..].as_bytes()) {
                    found = true;
                    for (b, x) in hit.iter_mut().zip(values(g, key)?) {
                        *b |= x != 0.;
                    }
                }
            }
            if !found && !token.contains(['*', '?', '[']) {
                return Err(Error::new(format!("no edge group {token:?}")));
            }
        }
        for (o, b) in out.iter_mut().zip(hit) {
            if exclude {
                *o &= !b;
            } else {
                *o |= b;
            }
        }
    }
    Ok(out)
}
pub(crate) fn remap(
    before: &Geometry,
    after: &mut Geometry,
    sources: Option<&[usize]>,
) -> Result<()> {
    let a = topo::edges(&before.topology, before.npoints);
    let b = topo::edges(&after.topology, after.npoints);
    let lookup: BTreeMap<_, _> = a
        .points
        .chunks_exact(2)
        .enumerate()
        .map(|(i, p)| ((p[0], p[1]), i))
        .collect();
    remap_edge_attributes(before, after, sources)?;
    for (name, attr) in &before.vertex {
        if !name.starts_with("edge_group_") && name != "creaseweight" {
            continue;
        }
        let old = values(before, name)?;
        let data = b
            .points
            .chunks_exact(2)
            .map(|p| {
                let (mut a, mut b) = (p[0], p[1]);
                if let Some(src) = sources {
                    a = src[a as usize] as i64;
                    b = src[b as usize] as i64;
                }
                lookup.get(&(a.min(b), a.max(b))).map_or(0., |&i| old[i])
            })
            .collect::<Vec<_>>();
        write_values(after, name, &data, matches!(attr.data, AttrData::I32(_)))?;
    }
    Ok(())
}

pub(crate) fn remap_edge_attributes(
    before: &Geometry,
    after: &mut Geometry,
    sources: Option<&[usize]>,
) -> Result<()> {
    let old_pairs = before.topology.edge_pairs();
    let lookup: BTreeMap<_, _> = old_pairs.iter().copied().enumerate().map(|(i, p)| (p, i)).collect();
    let indices: Vec<_> = after.topology.edge_pairs().into_iter().map(|(a, b)| {
        let (a, b) = if let Some(src) = sources {
            (src[a as usize] as i32, src[b as usize] as i32)
        } else { (a, b) };
        lookup.get(&(a.min(b), a.max(b))).copied()
    }).collect();
    for (name, attr) in &before.edge {
        // These edge tags and groups have separate semantics; leave their existing behavior alone.
        if matches!(name.as_str(), "crease_edge" | "sharp_edge") || name.starts_with("group_") {
            continue;
        }
        if attr.rows() != old_pairs.len() {
            return Err(Error::new(format!("edge attribute {name:?} has invalid dimensions")));
        }
        let mut mapped = attr.zeros_like(indices.len());
        match (&attr.data, &mut mapped.data) {
            (AttrData::F32(src), AttrData::F32(dst)) => for (row, old) in indices.iter().enumerate() {
                if let Some(old) = old { dst[row * attr.width..(row + 1) * attr.width]
                    .copy_from_slice(&src[old * attr.width..(old + 1) * attr.width]); }
            },
            (AttrData::I32(src), AttrData::I32(dst)) => for (row, old) in indices.iter().enumerate() {
                if let Some(old) = old { dst[row * attr.width..(row + 1) * attr.width]
                    .copy_from_slice(&src[old * attr.width..(old + 1) * attr.width]); }
            },
            _ => unreachable!(),
        }
        after.edge.insert(name.clone(), Arc::new(mapped));
    }
    debug_assert!(after.edge.values().all(|a| a.rows() == after.topology.nedges()));
    Ok(())
}
pub(crate) fn rebuild_edges(
    g: &Geometry,
    faces: &[Vec<usize>],
    sources: &[usize],
    closed: Option<Vec<bool>>,
) -> Result<Geometry> {
    let mut out = rebuild(g, faces, sources, closed)?;
    remap(g, &mut out, None)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuild_edges_keeps_bevel_weight_on_surviving_pairs() {
        let mut g = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.],
            vec![0, 1, 2, 3], vec![0, 4], None,
        ).unwrap();
        assert_eq!(g.topology.edge_pairs(), vec![(0, 1), (0, 3), (1, 2), (2, 3)]);
        g.edge.insert("bevel_weight_edge".into(), Arc::new(Attr::f32(1, vec![0.2, 0.3, 0.4, 0.5])));
        let out = rebuild_edges(&g, &[vec![0, 1, 2]], &[0], None).unwrap();
        assert_eq!(out.topology.edge_pairs(), vec![(0, 1), (0, 2), (1, 2)]);
        assert_eq!(out.edge["bevel_weight_edge"].as_f32().unwrap(), &[0.2, 0., 0.4]);
        assert_eq!(out.edge["bevel_weight_edge"].rows(), out.topology.nedges());
        out.validate().unwrap();
    }
}
