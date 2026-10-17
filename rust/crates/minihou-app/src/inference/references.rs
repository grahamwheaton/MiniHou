//! Reference tracking (hover-to-reference, from-point) and the persistent reference descriptors
//! that a future live-snap feature binds to. Live following is NOT implemented: see docs/RUST_INFERENCE.md.
use super::math::*;
use serde_json::{json, Value};

/// What the snapped point is attached to, in the evaluated geometry of the displayed node.
/// Element ids are point indices of that geometry; `params` are the interpolation weights.
/// Together with the node and its stable identity (see `measurement_identity`) this is enough to
/// re-evaluate the position after an upstream change while the topology (point count) is unchanged.
#[derive(Clone, Debug, PartialEq)]
pub enum RefDescriptor {
    /// A point of the geometry.
    Vertex { point: u32 },
    /// A point on the edge `a -> b` at parameter `t` (0 = a, 0.5 = midpoint).
    Edge { a: u32, b: u32, t: f32 },
    /// A point on triangle `(a,b,c)` with barycentric weights.
    Face { points: [u32; 3], weights: [f32; 3] },
    /// The intersection of edges `(a0,b0)` and `(a1,b1)` at parameters `(t0,t1)`.
    Intersection { edges: [[u32; 2]; 2], t: [f32; 2] },
    /// World origin (always valid).
    Origin,
}
impl RefDescriptor {
    pub fn kind(&self) -> &'static str { match self { Self::Vertex { .. } => "vertex", Self::Edge { .. } => "edge", Self::Face { .. } => "face", Self::Intersection { .. } => "intersection", Self::Origin => "origin" } }
    /// Compact document form. `node` and `point_count` let a reader detect topology changes.
    pub fn to_json(&self, node: &str, point_count: usize) -> Value {
        let (element, params): (Vec<u32>, Vec<f32>) = match self {
            Self::Vertex { point } => (vec![*point], vec![1.]),
            Self::Edge { a, b, t } => (vec![*a, *b], vec![1. - t, *t]),
            Self::Face { points, weights } => (points.to_vec(), weights.to_vec()),
            Self::Intersection { edges, t } => (edges.concat(), t.to_vec()),
            Self::Origin => (vec![], vec![]),
        };
        json!({"kind": self.kind(), "node": node, "element": element, "params": params, "point_count": point_count})
    }
    /// Re-evaluate against (possibly re-cooked) positions; None when the topology no longer fits.
    pub fn evaluate(&self, positions: &[f32]) -> Option<V3> {
        let n = positions.len() / 3;
        let p = |i: u32| ((i as usize) < n).then(|| [positions[i as usize * 3], positions[i as usize * 3 + 1], positions[i as usize * 3 + 2]]);
        Some(match self {
            Self::Vertex { point } => p(*point)?,
            Self::Edge { a, b, t } => lerp(p(*a)?, p(*b)?, *t),
            Self::Face { points, weights } => { let q = [p(points[0])?, p(points[1])?, p(points[2])?]; [0, 1, 2].map(|k| (0..3).map(|i| q[i][k] * weights[i]).sum()) }
            Self::Intersection { edges, t } => { let a = lerp(p(edges[0][0])?, p(edges[0][1])?, t[0]); let b = lerp(p(edges[1][0])?, p(edges[1][1])?, t[1]); lerp(a, b, 0.5) }
            Self::Origin => [0.; 3],
        })
    }
}

/// An acquired tracking reference.
#[derive(Clone, Debug, PartialEq)]
pub enum Reference {
    Point { point: V3, label: &'static str, descriptor: Option<RefDescriptor> },
    Edge { a: V3, b: V3, descriptor: Option<RefDescriptor> },
}
impl Reference {
    pub fn key(&self) -> u64 { match self { Self::Point { point, .. } => super::scoring::point_key(0xA1, *point), Self::Edge { a, b, .. } => super::scoring::point_key(0xA2, lerp(*a, *b, 0.5)) } }
    pub fn anchor(&self) -> V3 { match self { Self::Point { point, .. } => *point, Self::Edge { a, b, .. } => lerp(*a, *b, 0.5) } }
}

/// Hover dwell tracking + FIFO-capped acquired references.
#[derive(Clone, Debug, Default)]
pub struct Tracker {
    pub acquired: Vec<Reference>,
    hover: Option<(u64, f64)>,
    /// Last hovered edge (transient reference for parallel/perpendicular/extension; no dwell).
    pub transient_edge: Option<(V3, V3)>,
}
impl Tracker {
    /// Feed the hovered acquirable reference; returns true when a new reference was acquired.
    pub fn hover(&mut self, candidate: Option<Reference>, now: f64, dwell: f64, cap: usize) -> bool {
        let Some(r) = candidate else { self.hover = None; return false; };
        let key = r.key();
        match self.hover {
            Some((k, since)) if k == key => {
                if now - since >= dwell && !self.acquired.iter().any(|a| a.key() == key) {
                    self.acquired.push(r);
                    while self.acquired.len() > cap.max(1) { self.acquired.remove(0); }
                    return true;
                }
            }
            _ => self.hover = Some((key, now)),
        }
        false
    }
    pub fn clear(&mut self) -> bool { let had = !self.acquired.is_empty(); self.acquired.clear(); self.hover = None; had }
}
