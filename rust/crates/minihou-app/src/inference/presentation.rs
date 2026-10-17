//! What to draw for a result. World-space primitives; the viewport projects and paints them.
//! Colour is never the only cue: each kind has its own marker shape, line style and tooltip text.
use super::candidates::{Candidate, Kind};
use super::math::*;
use super::references::Reference;
use super::settings::{InferenceSettings, Rgba};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker { Circle, Square, Diamond, Cross, Dot, Ring }

#[derive(Clone, Debug, PartialEq)]
pub enum Primitive {
    Marker { at: V3, shape: Marker, color: Rgba, size: f32 },
    Line { a: V3, b: V3, color: Rgba, width: f32, dashed: bool },
    Tooltip { at: V3, text: String, color: Rgba },
}

pub fn marker_for(kind: Kind) -> Marker { match kind {
    Kind::Endpoint => Marker::Circle, Kind::Midpoint | Kind::Centre => Marker::Circle, Kind::Intersection | Kind::AlignIntersection => Marker::Cross,
    Kind::Origin => Marker::Square, Kind::Reference => Marker::Ring, Kind::OnEdge | Kind::OnFace | Kind::Extension => Marker::Diamond,
    _ => Marker::Dot,
} }
pub fn color_for(kind: Kind, s: &InferenceSettings) -> Rgba { let c = &s.colors; match kind {
    Kind::Endpoint => c.endpoint, Kind::Midpoint | Kind::Centre => c.midpoint, Kind::Intersection | Kind::AlignIntersection => c.intersection,
    Kind::Origin => c.endpoint, Kind::Reference => c.reference, Kind::OnEdge | Kind::OnFace => c.on, Kind::Extension => c.parallel,
    Kind::Axis(0) | Kind::FromPoint(0) | Kind::Locked(0) => c.x, Kind::Axis(1) | Kind::FromPoint(1) | Kind::Locked(1) => c.y,
    Kind::Axis(_) | Kind::FromPoint(_) | Kind::Locked(2) => c.z, Kind::Parallel | Kind::Perpendicular | Kind::Locked(3) => c.parallel,
    _ => c.free,
} }

/// Primitives for the winning candidate (only the most relevant inference is drawn), the acquired
/// references and the active tracking lines.
pub fn build(winner: &Candidate, start: Option<V3>, refs: &[Reference], locked: bool, tooltip: bool, s: &InferenceSettings) -> Vec<Primitive> {
    let mut out = Vec::new();
    for r in refs {
        match r {
            Reference::Point { point, .. } => out.push(Primitive::Marker { at: *point, shape: Marker::Dot, color: s.colors.reference, size: s.marker_size * 0.6 }),
            Reference::Edge { a, b, .. } => out.push(Primitive::Line { a: *a, b: *b, color: s.colors.reference, width: 2., dashed: false }),
        }
    }
    let color = color_for(winner.kind, s);
    for g in &winner.guides {
        let gc = color_for(g.kind, s);
        let temporary = matches!(g.kind, Kind::FromPoint(_) | Kind::Extension);
        // From-point tracking lines extend a little beyond the point so the alignment reads clearly.
        let b = if temporary { add(winner.point, mul(sub(winner.point, g.origin), 0.15)) } else { winner.point };
        out.push(Primitive::Line { a: g.origin, b, color: gc, width: if locked { 2.5 } else { 1.5 }, dashed: temporary && !locked });
    }
    if let (Kind::Locked(_), Some(a), true) = (winner.kind, start, winner.guides.is_empty()) {
        out.push(Primitive::Line { a, b: winner.point, color, width: 2.5, dashed: false });
    }
    if let Some((a, b)) = winner.cause { out.push(Primitive::Line { a, b, color: s.colors.parallel, width: 3., dashed: false }); }
    if winner.kind != Kind::Free {
        out.push(Primitive::Marker { at: winner.point, shape: marker_for(winner.kind), color, size: s.marker_size });
        if tooltip && s.show_tooltips { out.push(Primitive::Tooltip { at: winner.point, text: winner.kind.label().into(), color }); }
    }
    out
}
