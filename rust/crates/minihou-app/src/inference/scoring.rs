//! Ranking, hysteresis and candidate cycling.
use super::candidates::{Candidate, Kind};
use super::math::V3;

/// Identity of a candidate across frames: kind + quantised position (coincident vertices of
/// unwelded meshes share an identity, so they cannot flicker between each other).
pub fn point_key(tag: u64, p: V3) -> u64 {
    let q = |x: f32| ((x * 1e4).round() as i64) as u64;
    let mut h = 0xcbf29ce484222325u64 ^ tag;
    for x in p { h = (h ^ q(x)).wrapping_mul(0x100000001b3); }
    h
}
pub fn key(c: &Candidate) -> u64 {
    let tag = match c.kind { Kind::Endpoint | Kind::Midpoint | Kind::Centre | Kind::Origin | Kind::Reference | Kind::Intersection => 1, Kind::Axis(a) | Kind::FromPoint(a) | Kind::Locked(a) => 0x100 + a as u64 + ((c.kind.tier() as u64) << 4), k => k.tier() as u64 * 31 + k.label().len() as u64 };
    // Linear candidates move with the cursor: identify them by their guide line instead.
    match (c.kind.is_linear() || matches!(c.kind, Kind::OnEdge | Kind::OnFace), c.guides.first(), c.edge) {
        (true, Some(g), _) => point_key(tag, [g.origin[0] + g.dir[0] * 1e3, g.origin[1] + g.dir[1] * 1e3, g.origin[2] + g.dir[2] * 1e3]),
        (true, None, Some((a, b))) => point_key(tag, [a[0] + b[0], a[1] + b[1], a[2] + b[2]]),
        _ => point_key(tag, if matches!(c.kind, Kind::OnFace) { [0.; 3] } else { c.point }),
    }
}

/// Sort by tier, then screen distance, then depth; drop duplicates of the same identity.
pub fn rank(mut cands: Vec<Candidate>) -> Vec<Candidate> {
    // Within a tier, points where inferences meet beat the lines that produce them.
    let sub = |k: Kind| u8::from(matches!(k, Kind::FromPoint(_)));
    cands.sort_by(|a, b| a.kind.tier().cmp(&b.kind.tier()).then(sub(a.kind).cmp(&sub(b.kind))).then(a.distance.total_cmp(&b.distance)).then(a.depth.total_cmp(&b.depth)));
    let mut seen = std::collections::HashSet::new();
    cands.retain(|c| seen.insert(key(c)));
    cands
}

/// Keep the previous winner unless the best is in a better tier or at least
/// `max(px, ratio * previous distance)` closer.
pub fn hysteresis(ranked: &[Candidate], previous: Option<u64>, px: f32, ratio: f32) -> usize {
    let Some(prev) = previous else { return 0; };
    let Some(best) = ranked.first() else { return 0; };
    match ranked.iter().position(|c| key(c) == prev) {
        Some(i) if i > 0 => {
            let old = &ranked[i];
            let margin = px.max(ratio * old.distance);
            if old.kind.tier() == best.kind.tier() && best.distance > old.distance - margin { i } else { 0 }
        }
        _ => 0,
    }
}
