use super::*;
use super::drag::selected_node;

pub fn show(ui: &mut egui::Ui, rect: Rect, v: &View, out: &mut Actions) -> bool {
    if v.editor.tool != Tool::Select { return false; }
    let Some((node_id, ty, params)) = selected_node(v) else { return false; };

    let (grips, ring) = handles_for_node(node_id, ty, params);
    if grips.is_empty() { return false; }

    let p = ui.painter_at(rect);
    let drag_id = Id::new("prim_handle_drag");
    let mut current_drag: Option<PrimDrag> = ui.ctx().data(|d| d.get_temp(drag_id));

    let pointer = ui.input(|i| i.pointer.hover_pos());
    let primary_down = ui.input(|i| i.pointer.primary_down());
    let primary_clicked = ui.input(|i| i.pointer.primary_pressed());
    let shift = ui.input(|i| i.modifiers.shift);
    if ty=="curve" && shift && ui.input(|i|i.pointer.button_double_clicked(PointerButton::Primary)) && current_drag.is_none() {
        if let Some(pointer)=pointer.filter(|p|rect.contains(*p)) {
            let default="0,0,0  1,0,0  1,1,0  2,1,1";
            if let Some(mut points)=parse_curve_points(params["points"].as_str().unwrap_or(default)) {
                let base=*points.last().unwrap();let (origin,dir)=ray(v.camera,rect,pointer);let n=v.camera.ray_forward;
                let denom=(0..3).map(|k|dir[k]*n[k]).sum::<f32>() as f64;
                if denom.abs()>1e-8 {let t=(0..3).map(|k|(base[k]-origin[k] as f64)*n[k] as f64).sum::<f64>()/denom;
                    points.push([0,1,2].map(|k|origin[k] as f64+dir[k] as f64*t));
                    out.push(UiAction::Edit{edit:Edit::Param{node:node_id.into(),name:"points".into(),value:json!(write_curve_points(&points))},continuous:false});
                }
            }
        }
    }

    // Render spinner ring if present
    if let Some(r) = ring {
        let center = r.center;
        let ring_radius = r.radius;
        let mut screen_pts = Vec::new();
        for i in 0..=32 {
            let theta = i as f32 * std::f32::consts::TAU / 32.0;
            let offset = [ring_radius * theta.cos(), ring_radius * theta.sin(), 0.0];
            let pt = [center[0] + offset[0], center[1] + offset[1], center[2] + offset[2]];
            if let Some(sp) = project(v.camera, rect, pt) {
                screen_pts.push(sp);
            }
        }
        if screen_pts.len() >= 2 {
            for pair in screen_pts.windows(2) {
                p.line_segment([pair[0], pair[1]], Stroke::new(1.2_f32, Color32::from_rgba_unmultiplied(79, 156, 240, 160)));
            }
        }
        if let Some(center_proj) = project(v.camera, rect, center) {
            p.text(
                center_proj + Vec2::new(0.0, -12.0),
                Align2::CENTER_BOTTOM,
                r.label,
                FontId::monospace(11.0),
                v.theme.text,
            );
        }
    }

    let accent = v.theme.accent;
    let accent_bright = Color32::from_rgb(140, 195, 255);

    let mut hit_any = false;

    // Handle active drag
    if let Some(drag) = &current_drag {
        if primary_down {
            if let Some(pos) = pointer {
                let edits = compute_edits_for_drag(drag, pos, shift);
                for (name, val) in edits {
                    out.push(UiAction::Edit {
                        edit: Edit::Param {
                            node: drag.node.clone(),
                            name: name.to_string(),
                            value: val,
                        },
                        continuous: true,
                    });
                }
            }
        } else {
            // Drag finished
            ui.ctx().data_mut(|d| d.remove::<PrimDrag>(drag_id));
            current_drag = None;
        }
    }

    // Process grips
    for grip in &grips {
        let Some(pos) = project(v.camera, rect, grip.world_pos) else { continue; };
        let end_pt = [
            grip.world_pos[0] + grip.world_dir[0] * 0.5,
            grip.world_pos[1] + grip.world_dir[1] * 0.5,
            grip.world_pos[2] + grip.world_dir[2] * 0.5,
        ];
        let end_pos = project(v.camera, rect, end_pt).unwrap_or(pos + Vec2::X * 30.0);
        let screen_dir = end_pos - pos;
        let span = screen_dir.length();

        if grip.action == GripAction::TubeCapsToggle {
            // Caps toggle chip
            let text = grip.label.as_deref().unwrap_or("Caps");
            let galley = p.layout_no_wrap(text.to_owned(), FontId::monospace(11.0), Color32::WHITE);
            let btn_rect = Rect::from_center_size(pos, galley.size() + Vec2::new(12.0, 6.0));
            let hovered = pointer.is_some_and(|pt| btn_rect.contains(pt));
            if hovered { hit_any = true; }

            let bg = if hovered { accent } else { Color32::from_rgba_unmultiplied(30, 36, 48, 220) };
            p.rect(btn_rect, 4.0, bg, Stroke::new(1.0_f32, accent), StrokeKind::Inside);
            p.galley(btn_rect.min + Vec2::new(6.0, 3.0), galley, Color32::WHITE);

            if hovered && primary_clicked && current_drag.is_none() {
                let caps = get_bool(params, "caps", true);
                out.push(UiAction::Edit {
                    edit: Edit::Param {
                        node: node_id.to_string(),
                        name: "caps".into(),
                        value: json!(!caps),
                    },
                    continuous: false,
                });
            }
            continue;
        }

        let hovered = pointer.is_some_and(|pt| (pt - pos).length() <= 9.0);
        if hovered { hit_any = true; }

        let color = if hovered { accent_bright } else { accent };
        let stroke = Stroke::new(1.2_f32, Color32::WHITE);

        match grip.shape {
            GripShape::Square => {
                let r = Rect::from_center_size(pos, Vec2::splat(9.0));
                p.rect(r, 1.5, color, stroke, StrokeKind::Inside);
            }
            GripShape::Circle => {
                p.circle_filled(pos, 5.0, color);
                p.circle_stroke(pos, 5.0, stroke);
            }
        }

        // Draw label on hover or if dragging this grip
        if let Some(text) = &grip.label {
            if hovered || current_drag.as_ref().is_some_and(|d| d.action == grip.action) {
                let galley = p.layout_no_wrap(text.clone(), FontId::monospace(10.5), Color32::WHITE);
                let chip_rect = Rect::from_min_size(pos + Vec2::new(8.0, -18.0), galley.size() + Vec2::new(8.0, 4.0));
                p.rect_filled(chip_rect, 3.0, Color32::from_rgba_unmultiplied(20, 24, 32, 230));
                p.galley(chip_rect.min + Vec2::new(4.0, 2.0), galley, Color32::WHITE);
            }
        }

        // Initiate drag
        if hovered && primary_clicked && current_drag.is_none() {
            let drag = PrimDrag {
                node: node_id.to_string(),
                start_pointer: pointer.unwrap_or(pos),
                screen_dir,
                span,
                start_params: params.clone(),
                action: grip.action.clone(),
                world_dir: grip.world_dir,
                camera: v.camera,
                rect,
            };
            ui.ctx().data_mut(|d| d.insert_temp(drag_id, drag));
            current_drag = ui.ctx().data(|d| d.get_temp(drag_id));
        }
    }

    hit_any || current_drag.is_some()
}
