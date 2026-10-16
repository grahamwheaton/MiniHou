//! Collision wireframe for the selected/displayed Ray SOP, kept out of cooked topology.
use crate::ui::{prim_handles::project, View};
use eframe::egui::{self, Rect, Stroke};

pub(super) fn show(ui: &egui::Ui, rect: Rect, v: &View<'_>) {
    if v.selected != v.display
        || v.doc["nodes"][v.display]["type"] != "ray"
        || !v.doc["nodes"][v.display]["params"]["show_guide_geometry"]
            .as_bool()
            .unwrap_or(false)
    {
        return;
    }
    let Some(geo) = &v.cooking.geometry else {
        return;
    };
    let Some(segments) = geo
        .detail
        .get("__ray_guide_segments")
        .and_then(|v| v.as_array())
    else {
        return;
    };
    let painter = ui.painter_at(rect);
    let stroke = Stroke::new(1., v.theme.tpl.gamma_multiply(0.65));
    for segment in segments {
        let point = |i: usize| -> Option<[f32; 3]> {
            let p = segment.get(i)?.as_array()?;
            Some([
                p.first()?.as_f64()? as f32,
                p.get(1)?.as_f64()? as f32,
                p.get(2)?.as_f64()? as f32,
            ])
        };
        if let (Some(a), Some(b)) = (
            point(0).and_then(|p| project(v.camera, rect, p)),
            point(1).and_then(|p| project(v.camera, rect, p)),
        ) {
            painter.line_segment([a, b], stroke);
        }
    }
}
