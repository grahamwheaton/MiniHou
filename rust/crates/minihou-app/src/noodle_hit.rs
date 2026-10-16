//! Pick the connection under a drop pointer in screen coordinates.
use eframe::egui::Pos2;

pub fn closest_at_point<T: Clone + Ord>(wires: &[(T, Vec<Pos2>)], at: Pos2, radius: f32,
    valid: impl Fn(&T) -> bool) -> Option<T> {
    wires.iter().filter_map(|(wire, points)| {
        if !valid(wire) { return None; }
        let distance = points.windows(2).map(|segment| {
            let [a, b] = [segment[0], segment[1]];
            let delta = b - a;
            let t = if delta.length_sq() > 0.0 { ((at - a).dot(delta) / delta.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
            at.distance(a + delta * t)
        }).fold(f32::INFINITY, f32::min);
        (distance <= radius).then_some((wire, distance))
    }).min_by(|(a, da), (b, db)| da.total_cmp(db).then_with(|| a.cmp(b)))
        .map(|(wire, _)| wire.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{pos2, vec2, Rect};

    #[test]
    fn noodle_shelf_drop_uses_pointer_when_occupied_card_slot_moves() {
        let wire = vec![pos2(210., 156.), pos2(210., 400.)];
        let pointer = pos2(210., 360.);
        let displaced_card = Rect::from_min_size(pos2(224., 448.), vec2(220., 56.));
        assert!(!wire.iter().any(|at| displaced_card.contains(*at)));
        assert_eq!(closest_at_point(&[("a->out", wire)], pointer, 8., |_| true), Some("a->out"));
    }
}
