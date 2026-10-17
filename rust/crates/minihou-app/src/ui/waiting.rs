//! Neutral "waiting for external data" state (a Blender Object node with nothing picked, Blender not
//! connected, an unknown image name): not an error. The cook reports these in `CookStats::waiting`; nodes draw
//! with an amber outline and a quiet note, the status line says what to do, and no red banner appears.
use eframe::egui::{self, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use minihou_core::session::CookStats;

/// The waiting message of node `id` in the last cook, if any.
pub fn waiting_of<'a>(stats: &'a CookStats, id: &str) -> Option<&'a str> {
    stats.waiting.iter().find(|(n, _)| n == id).map(|(_, m)| m.as_str())
}

/// Node status chip text (short): "waiting".
pub const NODE_STATUS: &str = "waiting";

/// What the network card shows under a waiting node: a one-line hint.
pub fn card_note(message: &str) -> String {
    match message.split_once(". ") {
        Some((head, _)) if message.contains("Pick an object") => format!("{head}: pick an object in Blender"),
        Some((head, _)) => head.to_owned(),
        None => message.to_owned(),
    }
}

/// Status-line text when nothing failed but nodes wait, e.g. "Waiting: blender_object1 (no object picked. Pick ...)".
pub fn status_line(stats: &CookStats) -> Option<String> {
    let (first, message) = stats.waiting.first()?;
    let others = stats.waiting.len() - 1;
    Some(if others == 0 { format!("Waiting: {first} ({message})") }
         else { format!("Waiting: {first} and {others} more ({message})") })
}

/// Outline colour of a waiting node card: amber, softened so it never reads as an error.
pub fn outline(warn: Color32) -> Color32 { warn.gamma_multiply(0.75) }

/// The Parameters panel note: neutral panel box, dot, dim text (never the red error style).
pub fn panel_note(ui: &mut egui::Ui, theme: &super::theme::Theme, message: &str) {
    let width = ui.available_width() - 24.;
    let galley = ui.painter().layout(message.to_owned(), FontId::proportional(11.), theme.dim, width - 30.);
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), galley.size().y + 12.), Sense::hover());
    let r = Rect::from_min_max(r.min + Vec2::new(12., 0.), r.max - Vec2::new(12., 0.));
    ui.painter().rect(r, 6., theme.panel2, Stroke::new(1.0_f32, outline(theme.warn)), StrokeKind::Inside);
    ui.painter().circle_filled(Pos2::new(r.left() + 11., r.top() + 12.), 3.5, theme.warn);
    ui.painter().galley(Pos2::new(r.left() + 22., r.top() + 6.), galley, theme.dim);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stats(w: &[(&str, &str)]) -> CookStats {
        CookStats { waiting: w.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(), ..Default::default() }
    }
    #[test]
    fn lookup_notes_and_status_line() {
        let s = stats(&[("blender_object1", "no object picked. Pick an object in the MiniHou group in Blender, or connect Blender")]);
        assert_eq!(waiting_of(&s, "blender_object1").map(card_note).as_deref(), Some("no object picked: pick an object in Blender"));
        assert_eq!(waiting_of(&s, "box1"), None);
        assert!(status_line(&s).unwrap().starts_with("Waiting: blender_object1 (no object picked."));
        assert_eq!(status_line(&stats(&[])), None);
        let two = stats(&[("a", "x"), ("b", "y")]);
        assert_eq!(status_line(&two).as_deref(), Some("Waiting: a and 1 more (x)"));
        assert_eq!(card_note("waiting for Blender image \"H\": connect Blender"), "waiting for Blender image \"H\": connect Blender");
    }
}
