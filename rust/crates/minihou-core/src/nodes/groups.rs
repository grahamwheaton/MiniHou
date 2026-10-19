//! Point selections used by transform. Expression support is limited to a single P component comparison.
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};

fn flags(attr: &Attr) -> Vec<bool> {
    match &attr.data {
        AttrData::F32(v) => v.chunks(attr.width).map(|row| row[0] != 0.0).collect(),
        AttrData::I32(v) => v.chunks(attr.width).map(|row| row[0] != 0).collect(),
    }
}

fn group(geo: &Geometry, name: &str) -> Option<Vec<bool>> {
    let key = format!("group_{name}");
    if let Some(a) = geo.point.get(&key) { return Some(flags(a)); }
    let a = geo.prim.get(&key)?;
    let members = flags(a);
    let mut out = vec![false; geo.npoints];
    for (i, &in_group) in members.iter().enumerate() {
        if in_group {
            for &point in &geo.topology.vertex_point[geo.topology.prim_offsets[i] as usize..geo.topology.prim_offsets[i + 1] as usize] {
                out[point as usize] = true;
            }
        }
    }
    Some(out)
}

fn expression(geo: &Geometry, text: &str) -> Result<Vec<bool>> {
    let expr = text.trim().trim_start_matches('(').trim_end_matches(')').trim();
    let Some(axis) = expr.strip_prefix("@P.") else { return Err(Error::new("group expression is not supported by the Rust backend")); };
    let (k, rest) = match axis.chars().next() {
        Some('x') => (0, &axis[1..]), Some('y') => (1, &axis[1..]), Some('z') => (2, &axis[1..]),
        _ => return Err(Error::new("group expression is not supported by the Rust backend")),
    };
    let rest = rest.trim();
    let (op, rhs) = [">=", "<=", "==", "!=", ">", "<"].into_iter()
        .find_map(|op| rest.strip_prefix(op).map(|rhs| (op, rhs.trim())))
        .ok_or_else(|| Error::new("group expression is not supported by the Rust backend"))?;
    let value: f32 = rhs.parse().map_err(|_| Error::new("group expression is not supported by the Rust backend"))?;
    Ok(geo.p().as_chunks::<3>().0.iter().map(|q| match op {
        ">=" => q[k] >= value, "<=" => q[k] <= value, "==" => q[k] == value,
        "!=" => q[k] != value, ">" => q[k] > value, _ => q[k] < value,
    }).collect())
}

pub(super) fn point_mask(geo: &Geometry, pattern: &str) -> Result<Vec<bool>> {
    let text = pattern.trim();
    if text.is_empty() { return Ok(vec![true; geo.npoints]); }
    if text.contains('@') || ["<", ">", "==", "!="].iter().any(|s| text.contains(s)) {
        return expression(geo, text);
    }
    let tokens: Vec<_> = text.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).collect();
    if tokens.is_empty() { return Ok(vec![true; geo.npoints]); }
    let has_positive = tokens.iter().any(|token| !token.starts_with('^'));
    let mut selected = vec![!has_positive; geo.npoints];
    let mut excluded = vec![false; geo.npoints];
    for token in tokens {
        let exclude = token.starts_with('^');
        let token = token.strip_prefix('^').unwrap_or(token);
        if token.is_empty() { continue; }
        let mut hit = vec![false; geo.npoints];
        let wildcard = token.contains(['*', '?', '[']);
        let number = !wildcard && token.as_bytes().first().is_some_and(u8::is_ascii_digit);
        if number {
            let (span, stride) = token.split_once(':').unwrap_or((token, "1"));
            let step: usize = stride.parse().map_err(|_| Error::new(format!("invalid range stride in {token:?}")))?;
            if step == 0 { return Err(Error::new(format!("invalid range stride in {token:?}"))); }
            let (start, end) = span.split_once('-').unwrap_or((span, span));
            let lo: usize = start.parse().map_err(|_| Error::new(format!("invalid group range {token:?}")))?;
            let hi: usize = end.parse().map_err(|_| Error::new(format!("invalid group range {token:?}")))?;
            if lo < geo.npoints {
                for i in (lo..=hi.min(geo.npoints - 1)).step_by(step) { hit[i] = true; }
            }
        } else if token == "*" {
            hit.fill(true);
        } else if wildcard {
            for name in geo.point.keys().chain(geo.prim.keys()).filter_map(|name| name.strip_prefix("group_")) {
                if crate::group_pattern::matches(token, name) {
                    if let Some(m) = group(geo, name) { for (dst, src) in hit.iter_mut().zip(m) { *dst |= src; } }
                }
            }
        } else {
            hit = group(geo, token).ok_or_else(|| Error::new(format!("no group {token:?}")))?;
        }
        for ((dst, ex), src) in selected.iter_mut().zip(&mut excluded).zip(hit) {
            if exclude { *ex |= src; } else { *dst |= src; }
        }
    }
    for (dst, ex) in selected.iter_mut().zip(excluded) { *dst &= !ex; }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use crate::geometry::{Attr, Geometry};
    use super::point_mask;

    #[test]
    fn names_primitive_conversion_ranges_and_expression() {
        let mut geo = Geometry::from_arrays(vec![0.0,0.0,-1.0, 1.0,0.0,1.0, 2.0,0.0,2.0, 3.0,0.0,-2.0],
            vec![0,1,2, 2,3], vec![0,3,5], Some(vec![false, false])).unwrap();
        geo.point.insert("group_tip".into(), Arc::new(Attr::i32(1, vec![0,0,0,1])));
        geo.prim.insert("group_first".into(), Arc::new(Attr::i32(1, vec![1,0])));
        assert_eq!(point_mask(&geo, "first").unwrap(), [true,true,true,false]);
        assert_eq!(point_mask(&geo, "0-3:2 ^2").unwrap(), [true,false,false,false]);
        assert_eq!(point_mask(&geo, "^tip").unwrap(), [true,true,true,false]);
        assert_eq!(point_mask(&geo, "@P.z > 0").unwrap(), [false,true,true,false]);
        assert_eq!(point_mask(&geo, "f*").unwrap(), [true,true,true,false]);
    }

    #[test]
    fn wildcard_group_names_union_then_exclude() {
        let mut geo = Geometry::points_only(vec![0.0; 15]).unwrap();
        for (name, values) in [
            ("leg_0", vec![1, 0, 0, 0, 0]),
            ("leg_1", vec![0, 1, 0, 0, 0]),
            ("arm_a", vec![0, 0, 1, 0, 0]),
            ("debris_x", vec![0, 0, 0, 1, 0]),
        ] {
            geo.point.insert(format!("group_{name}"), Arc::new(Attr::i32(1, values)));
        }
        let check = |pattern, expected| assert_eq!(point_mask(&geo, pattern).unwrap(), expected, "{pattern}");
        check("leg_*", [true, true, false, false, false]);
        check("leg_?", [true, true, false, false, false]);
        check("leg_[01]", [true, true, false, false, false]);
        check("leg_[0-9]", [true, true, false, false, false]);
        check("leg_* arm_*", [true, true, true, false, false]);
        check("* ^debris_*", [true, true, true, false, true]);
        check("^debris_*", [true, true, true, false, true]);
        check("^debris_* debris_x", [false; 5]);
        check("arm_a", [false, false, true, false, false]);
    }
}
