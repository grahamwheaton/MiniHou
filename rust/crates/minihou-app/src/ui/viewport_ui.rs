//! Viewport chrome drawn over the 3-D view (design: Viewport.dc.html): header bar (selection modes, orientation, pivot,
//! snapping, proportional editing, overlays, X-ray, shading, Frame), left tool strip, bottom timeline, stats chip, view gizmo,
//! error and dense-mesh notes. `rect` is the viewport's area; the 3-D image is already painted into it.
use super::{theme::Theme, Actions, EditorChange, Falloff, Orientation, Pivot, SelectionMode, SnapTo, Tool, UiAction, View};
use crate::viewport::{Mode, ObjectVisibility};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
#[path = "view_gizmo.rs"] mod view_gizmo;
#[path = "ray_guide.rs"] mod ray_guide;
mod chrome;
mod popovers;
use popovers::{caret_button, falloff_samples, note, show_popover};
use chrome::{button, button_text, display_row, frame_at, header_layout, icon, section, show_shading_pie, split_error, toggle_popover, HeaderLayout, Icon, Popover, VpState, DENSE_PRIMS, FRAME_END, FRAME_START, HEADER_H, OVERLAYS, TIMELINE_H, TOOLS};

// The viewport image uses the CentralPanel's Middle layer. Draw the scene tools
// on another Middle layer, then all chrome and menus above it in Foreground.
pub fn scene_layer_id() -> egui::LayerId {
    egui::LayerId::new(egui::Order::Middle, egui::Id::new("viewport_scene_tools"))
}
fn chrome_layer_id() -> egui::LayerId {
    egui::LayerId::new(egui::Order::Foreground, egui::Id::new("viewport_chrome"))
}
fn popover_layer_id() -> egui::LayerId {
    egui::LayerId::new(egui::Order::Foreground, egui::Id::new("viewport_popovers"))
}

/// Render primitive handles over the viewport when a primitive generator is selected.
pub fn handles(ui: &mut egui::Ui, rect: Rect, v: &View, out: &mut Actions) -> bool {
    ray_guide::show(ui, rect, v);
    super::prim_handles::show(ui, rect, v, out)
}

/// Space for operator panels above the stats chip and timeline, beside the tools.
pub fn operator_panel_rect(rect: Rect) -> Rect {
    Rect::from_min_max(
        Pos2::new(rect.left() + 56., rect.top() + HEADER_H + 46.),
        Pos2::new(rect.right() - 12., rect.bottom() - TIMELINE_H - 50.),
    )
}

/// Keep region presses off viewport chrome (header, timeline, tool strip and view gizmo).
pub fn selection_canvas_contains(rect: Rect, p: Pos2) -> bool {
    p.y > rect.top() + HEADER_H
        && !(p.x >= rect.left() + 56. && p.x <= rect.left() + 252. && p.y < rect.top() + HEADER_H + 34.) && p.y < rect.bottom() - TIMELINE_H
        && !(p.x < rect.left() + 52. && p.y < rect.top() + HEADER_H + 12. + TOOLS.len() as f32 * 28. + 28.)
        && !(p.x > rect.right() - 110. && p.y < rect.top() + HEADER_H + 115.)
}
/// The part of the viewport not covered by the header and the timeline.
pub fn cursor_clip(rect: Rect) -> Rect {
    Rect::from_min_max(egui::pos2(rect.left(), rect.top() + HEADER_H), egui::pos2(rect.right(), (rect.bottom() - TIMELINE_H).max(rect.top() + HEADER_H)))
}
pub fn timeline_command(ctx: &egui::Context, command: &str) -> Option<minihou_app::Edit> {
    let id = egui::Id::new("viewport_ui_state");
    let changed = ctx.data_mut(|d| {
        let mut st: VpState = d.get_temp(id).unwrap_or_default();
        let before = st.frame;
        match command {
            "timeline_toggle" => st.playing = !st.playing,
            "timeline_previous" => st.frame = st.frame.saturating_sub(1).max(FRAME_START),
            "timeline_next" => st.frame = (st.frame + 1).min(FRAME_END),
            "timeline_start" => { st.playing = false; st.frame = FRAME_START; }
            "timeline_end" => { st.playing = false; st.frame = FRAME_END; }
            _ => return None,
        }
        let edit = minihou_app::timeline_time_edit(before, st.frame, 24.);
        d.insert_temp(id, st);
        edit
    });
    ctx.request_repaint();
    changed
}


pub fn overlay(ui: &mut egui::Ui, rect: Rect, v: &View, out: &mut Actions) {
    ui.scope_builder(egui::UiBuilder::new().layer_id(chrome_layer_id()).max_rect(rect), |ui| {
        overlay_contents(ui, rect, v, out);
    });
    show_shading_pie(ui.ctx(), rect, v, out);
}

fn overlay_contents(ui: &mut egui::Ui, rect: Rect, v: &View, out: &mut Actions) {
    let ctx = ui.ctx().clone();
    let id = egui::Id::new("viewport_ui_state");
    let mut st: VpState = ctx.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    let previous_frame = st.frame;
    let th = v.theme;
    let painter = ui.painter_at(rect);
    let stroke = Stroke::new(1.0_f32, th.stroke);

    // ---- header bar
    painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(rect.width(), HEADER_H)), 0., th.panel);
    painter.line_segment([Pos2::new(rect.left(), rect.top() + HEADER_H), Pos2::new(rect.right(), rect.top() + HEADER_H)], stroke);
    let ed = v.editor;
    let show = [ed.overlays.grid, ed.overlays.axes, ed.overlays.wireframe, ed.overlays.stats, ed.overlays.normals, ed.overlays.bounds].map(|b| b && ed.overlays.enabled);
    let h = header_layout(rect.left(), rect.top(), rect.width(), ed.snap.enabled);
    for g in &h.groups { painter.rect(*g, 5., th.panel2, stroke, StrokeKind::Inside); }
    for x in &h.seps { painter.line_segment([Pos2::new(*x, rect.top() + 9.), Pos2::new(*x, rect.top() + HEADER_H - 9.)], stroke); }
    for (i, r) in h.modes.iter().enumerate() {
        let mode = SelectionMode::ALL[i];
        if button(ui, th, *r, ("vp_mode", i), ed.selection == mode, ["Points (1)", "Edges (2)", "Faces (3)", "Objects (4)"][i], [Icon::Points, Icon::Edges, Icon::Faces, Icon::Object][i]) { out.push(UiAction::Editor(EditorChange::Selection(mode))); }
    }
    if button_text(ui, th, h.orient, "vp_orient", st.popover == Some(Popover::Orientation), "Transform orientation", Icon::Orient, (Some(ed.orientation.label()), true)) { st.popover = toggle_popover(st.popover, Popover::Orientation); }
    if button_text(ui, th, h.pivot, "vp_pivot", st.popover == Some(Popover::Pivot), "Pivot point", Icon::Pivot, (None, true)) { st.popover = toggle_popover(st.popover, Popover::Pivot); }
    let snap_label = if ed.snap.to == SnapTo::Increment.into() { crate::units::format(ed.snap.increment as f64, v.units) }
        else if ed.snap.to.iter().count()>1 { "Multi".into() }
        else { match ed.snap.to.iter().next().unwrap_or_default() {
            SnapTo::Vertex=>"Vtx",SnapTo::Edge=>"Edge",SnapTo::Face=>"Face",SnapTo::Volume=>"Vol",SnapTo::EdgeCenter=>"Mid",SnapTo::FaceCenter=>"Ctr",SnapTo::Increment=>"Grid"
        }.into() };
    let snap_main = Rect::from_min_max(h.snap.min, Pos2::new(h.snap.right() - 16., h.snap.bottom()));
    if button_text(ui, th, snap_main, "vp_snap_toggle", ed.snap.enabled, "Toggle snapping (Shift+Tab)", Icon::Magnet, (if ed.snap.enabled && h.snap.width() > 44. { Some(snap_label.as_str()) } else { None }, false)) { out.push(UiAction::Editor(EditorChange::SnapEnabled(!ed.snap.enabled))); }
    if caret_button(ui, th, h.snap, "vp_snap_menu", st.popover == Some(Popover::Snap)) { st.popover = toggle_popover(st.popover, Popover::Snap); }
    let prop_main = Rect::from_min_max(h.prop.min, Pos2::new(h.prop.right() - 16., h.prop.bottom()));
    if button_text(ui, th, prop_main, "vp_prop_toggle", ed.proportional.enabled, "Toggle proportional editing (O)", Icon::Prop, (None, false)) { out.push(UiAction::Editor(EditorChange::ProportionalEnabled(!ed.proportional.enabled))); }
    if caret_button(ui, th, h.prop, "vp_prop_menu", st.popover == Some(Popover::Proportional)) { st.popover = toggle_popover(st.popover, Popover::Proportional); }
    ui.scope_builder(egui::UiBuilder::new().max_rect(h.proxy), |ui| {
        ui.set_clip_rect(h.proxy);
        super::proxy_ui::menu(ui, v, out, h.proxy.width() < 80.);
    });
    let overlay_main=Rect::from_min_max(h.overlays.min,Pos2::new(h.overlays.right()-16.,h.overlays.bottom()));
    if button_text(ui,th,overlay_main,"vp_overlays_toggle",ed.overlays.enabled,"Toggle all overlays",Icon::Overlays,(None,false)) {
        let mut o=ed.overlays; o.enabled=!o.enabled; out.push(UiAction::Editor(EditorChange::Overlays(o)));
    }
    if caret_button(ui,th,h.overlays,"vp_overlays_menu",st.popover==Some(Popover::Overlays)) { st.popover=toggle_popover(st.popover,Popover::Overlays); }
    if button_text(ui, th, h.shading, "vp_shading", st.popover == Some(Popover::Shading), v.mode.label(), Icon::Display(v.mode), (None,true)) { st.popover=toggle_popover(st.popover,Popover::Shading); }
    if button_text(ui, th, h.visibility, "vp_visibility", st.popover == Some(Popover::Visibility), ed.object_visibility.label(), Icon::Visibility(ed.object_visibility), (None,true)) { st.popover=toggle_popover(st.popover,Popover::Visibility); }
    if button(ui, th, h.frame, ("vp_frame", 0), false, "Frame the mesh (Home)", Icon::Frame) { out.push(UiAction::Frame); }

    // ---- tool strip
    let strip_h = TOOLS.len() as f32 * 28. + 4. * 5. + 8.;
    let strip = Rect::from_min_size(Pos2::new(rect.left() + 12., rect.top() + HEADER_H + 12.), Vec2::new(36., strip_h));
    painter.rect(strip, 6., th.panel, stroke, StrokeKind::Inside);
    let mut y = strip.top() + 4.;
    for (i, (name, icon, tool)) in TOOLS.iter().enumerate() {
        if i == 2 || i == 7 || i == 8 || i == 13 {
            painter.line_segment([Pos2::new(strip.left() + 6., y + 2.), Pos2::new(strip.right() - 6., y + 2.)], stroke);
            y += 5.;
        }
        let r = Rect::from_min_size(Pos2::new(strip.left() + 4., y), Vec2::splat(28.));
        if button(ui, th, r, ("vp_tool", i), tool.is_some_and(|t| ed.tool == t), name, *icon) {
            out.push(tool.map_or(UiAction::StartScaleCage, |t| UiAction::Editor(EditorChange::Tool(t))));
        }
        y += 28.;
    }

    // Active Select tool options, separate from the tool strip and one-shot hotkeys.
    if matches!(ed.tool, Tool::Select | Tool::Move | Tool::Rotate | Tool::Scale | Tool::Transform) && !super::sketch::active(&ctx) {
        use super::selection_region::{self, Kind};
        use crate::picking::SelectOp;
        let mut options = selection_region::options(&ctx);
        let bar = Rect::from_min_size(Pos2::new(rect.left() + 56., rect.top() + HEADER_H + 4.), Vec2::new(196., 28.));
        painter.rect(bar, 4., th.panel, stroke, StrokeKind::Inside);
        for (i, (kind, ic, tip)) in [
            (Kind::Box, Icon::BoxSelect, "Select Box (B for one-shot box)"),
            (Kind::Circle, Icon::CircleSelect, "Select Circle (C for modal circle)"),
            (Kind::Lasso, Icon::LassoSelect, "Select Lasso (Ctrl+RMB drag)"),
        ].into_iter().enumerate() {
            let r = Rect::from_min_size(bar.min + Vec2::new(2. + i as f32 * 28., 2.), Vec2::splat(24.));
            if button(ui, th, r, ("vp_select_kind", i), options.kind == kind, tip, ic) {
                options.kind = kind;
                selection_region::set_options(&ctx, options);
            }
        }
        for (i, (op, label, tip)) in [
            (SelectOp::Replace, "=", "Set: replace selection"),
            (SelectOp::Add, "+", "Extend selection (Shift)"),
            (SelectOp::Subtract, "-", "Subtract selection (Ctrl)"),
        ].into_iter().enumerate() {
            let r = Rect::from_min_size(bar.min + Vec2::new(102. + i as f32 * 30., 2.), Vec2::new(28., 24.));
            if ui.put(r, egui::Button::new(label).selected(options.op == op)).on_hover_text(tip).clicked() {
                options.op = op;
                selection_region::set_options(&ctx, options);
            }
        }
    }

    // ---- timeline
    let tl = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - TIMELINE_H), rect.max);
    painter.rect_filled(tl, 0., th.panel);
    painter.line_segment([tl.left_top(), tl.right_top()], stroke);
    let cy = tl.center().y;
    let mut x = tl.left() + 10.;
    let group = Rect::from_min_size(Pos2::new(x, cy - 13.), Vec2::new(5. * 25. + 3., 26.));
    painter.rect(group, 5., th.panel2, stroke, StrokeKind::Inside);
    x += 2.;
    for (i, (icon, name)) in [(Icon::ToStart, "Jump to start"), (Icon::Prev, "Previous frame"), (if st.playing { Icon::Pause } else { Icon::Play }, "Play (Space)"), (Icon::Next, "Next frame"), (Icon::ToEnd, "Jump to end")].into_iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(x + i as f32 * 25., cy - 11.), Vec2::new(24., 22.));
        if button(ui, th, r, ("vp_tl", i), i == 2 && st.playing, name, icon) {
            match i {
                0 => st.frame = FRAME_START,
                1 => st.frame = st.frame.saturating_sub(1).max(FRAME_START),
                2 => st.playing = !st.playing,
                3 => st.frame = (st.frame + 1).min(FRAME_END),
                _ => st.frame = FRAME_END,
            }
        }
    }
    x = group.right() + 12.;
    let mono = egui::FontId::monospace(11.);
    let label = painter.text(Pos2::new(x, cy), egui::Align2::LEFT_CENTER, "Frame", mono.clone(), th.dim);
    x = label.right() + 8.;
    let val = Rect::from_min_size(Pos2::new(x, cy - 11.), Vec2::new(44., 22.));
    painter.rect(val, 4., th.panel2, stroke, StrokeKind::Inside);
    painter.text(Pos2::new(val.right() - 6., cy), egui::Align2::RIGHT_CENTER, st.frame.to_string(), egui::FontId::monospace(12.), th.text);
    let readout = format!("Start {FRAME_START}  End {FRAME_END}  24 fps");
    let rg = painter.layout_no_wrap(readout, mono.clone(), th.dim);
    let rx = tl.right() - 10. - rg.size().x;
    painter.galley(Pos2::new(rx, cy - rg.size().y / 2.), rg, th.dim);
    let bar = Rect::from_min_max(Pos2::new(val.right() + 10., cy - 12.), Pos2::new(rx - 12., cy + 12.));
    if bar.width() > 40. {
        painter.rect(bar, 4., th.panel2, stroke, StrokeKind::Inside);
        let mut tx = bar.left() + 24.;
        while tx < bar.right() { painter.line_segment([Pos2::new(tx, bar.bottom() - 8.), Pos2::new(tx, bar.bottom() - 1.)], stroke); tx += 24.; }
        let at = |f: f32| bar.left() + f * bar.width();
        for f in [0., 0.35, 0.7] {
            let c = Pos2::new(at(f).max(bar.left() + 5.), bar.top() + 11.);
            painter.add(egui::Shape::convex_polygon(vec![c + Vec2::new(0., -4.), c + Vec2::new(4., 0.), c + Vec2::new(0., 4.), c + Vec2::new(-4., 0.)], th.warn, Stroke::NONE));
        }
        let resp = ui.interact(bar, egui::Id::new("vp_scrub"), Sense::click_and_drag());
        if let Some(p) = resp.interact_pointer_pos() { if resp.dragged() || resp.clicked() { st.frame = frame_at(p.x, bar.left(), bar.right()); } }
        let px = at((st.frame - FRAME_START) as f32 / (FRAME_END - FRAME_START) as f32);
        painter.rect_filled(Rect::from_min_max(Pos2::new(px - 1., bar.top() - 1.), Pos2::new(px + 1., bar.bottom() + 1.)), 0., th.accent);
    }
    if st.playing {
        st.clock += ctx.input(|i| i.stable_dt);
        let step = (st.clock * 24.) as u32;
        if step > 0 { st.clock -= step as f32 / 24.; st.frame = FRAME_START + (st.frame - FRAME_START + step) % (FRAME_END - FRAME_START + 1); }
        ctx.request_repaint();
    }

    // ---- stats chip, axis gizmo
    let (points, prims) = v.cooking.geometry.as_ref().map_or((0, 0), |g| (g.npoints, g.nprims()));
    if show[3] {
        let text = format!("Points {points}   Prims {prims}   Cook {:.1} ms", v.cooking.milliseconds);
        let g = painter.layout_no_wrap(text, mono.clone(), th.text);
        let chip = Rect::from_min_size(Pos2::new(rect.left() + 12., tl.top() - 12. - 26.), Vec2::new(g.size().x + 20., 26.));
        painter.rect(chip, 6., th.panel, stroke, StrokeKind::Inside);
        painter.galley(Pos2::new(chip.left() + 10., chip.center().y - g.size().y / 2.), g, th.text);
    }
    if show[1] { view_gizmo::show(ui, rect, v, out); }

    // ---- notes
    if let Some(error) = &v.cooking.error {
        let (node, msg) = split_error(error);
        let title = match node { Some(n) => format!("Cook failed at {n}."), None => "Cook failed.".into() };
        note(&painter, th, Pos2::new(rect.left() + 56., rect.top() + HEADER_H + 12.), (th.err, th.err_bg), (&title, &format!("{msg}  Showing the last good mesh.")), rect.width() - 68.);
    } else if prims > DENSE_PRIMS {
        note(&painter, th, Pos2::new(rect.left() + 56., tl.top() - 12. - 40.), (th.warn, th.warn_bg), ("Mesh is dense.", "Edges are fading out. Zoom in or lower rows and columns to see them."), rect.width() - 190.);
    }

    if let Some(which) = st.popover {
        let anchor = match which { Popover::Orientation => h.orient, Popover::Pivot => h.pivot, Popover::Snap => h.snap, Popover::Proportional => h.prop, Popover::Overlays => h.overlays, Popover::Shading => h.shading, Popover::Visibility => h.visibility };
        let before=out.0.len();
        let pop = ui.scope_builder(egui::UiBuilder::new().layer_id(popover_layer_id()).max_rect(rect), |ui| {
            show_popover(ui, rect, anchor, which, v, out)
        }).inner;
        ctx.move_to_top(popover_layer_id());
        if out.0.len()>before && matches!(which,Popover::Shading | Popover::Visibility) { st.popover=None; }
        let dismiss = ctx.input(|i| i.key_pressed(egui::Key::Escape) || (i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !pop.contains(p) && !anchor.contains(p))));
        if dismiss { st.popover = None; }
    }
    if let Some(edit) = minihou_app::timeline_time_edit(previous_frame, st.frame, 24.) { out.push(UiAction::Time(edit)); }
    ctx.data_mut(|d| d.insert_temp(id, st));
}

/// Compact Blender-style status strip for the active viewport transform.
pub fn modal_status(ui: &egui::Ui, rect: egui::Rect, status: &str) {
    let painter = egui::Painter::new(ui.ctx().clone(), chrome_layer_id(), rect);
    let bar = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom()-29.), rect.right_bottom());
    painter.rect_filled(bar, 0., egui::Color32::from_black_alpha(205));
    painter.text(bar.left_center()+egui::vec2(10.,0.), egui::Align2::LEFT_CENTER, status,
        egui::FontId::proportional(13.), egui::Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn viewport_chrome_is_above_scene_tools() {
        assert!(scene_layer_id().order < chrome_layer_id().order);
        assert_eq!(popover_layer_id().order, egui::Order::Foreground);
        assert_ne!(popover_layer_id(), chrome_layer_id());
    }
    #[test] fn scale_cage_is_directly_below_scale_in_tool_strip() {
        let scale = TOOLS.iter().position(|(_, _, tool)| *tool == Some(Tool::Scale)).unwrap();
        assert_eq!(TOOLS[scale].1, Icon::Scale);
        assert_eq!(TOOLS[scale + 1], ("Scale Cage (Shift+S)", Icon::ScaleCage, None));
    }
    #[test] fn timeline_shortcuts_step_and_jump() {
        let ctx = egui::Context::default();
        timeline_command(&ctx, "timeline_start");
        timeline_command(&ctx, "timeline_previous");
        assert_eq!(ctx.data(|d| d.get_temp::<VpState>(egui::Id::new("viewport_ui_state")).unwrap().frame), FRAME_START);
        timeline_command(&ctx, "timeline_next");
        assert_eq!(ctx.data(|d| d.get_temp::<VpState>(egui::Id::new("viewport_ui_state")).unwrap().frame), FRAME_START + 1);
        timeline_command(&ctx, "timeline_end");
        assert_eq!(ctx.data(|d| d.get_temp::<VpState>(egui::Id::new("viewport_ui_state")).unwrap().frame), FRAME_END);
    }

    fn overlap(a: &Rect, b: &Rect) -> bool { a.shrink(0.5).intersects(b.shrink(0.5)) }

    #[test]
    fn header_fits_and_does_not_overlap_down_to_640() {
        for width in [640., 720., 920., 1280.] {
            for snap in [false, true] {
                let h = header_layout(0., 0., width, snap);
                let mut rects: Vec<Rect> = h.modes.to_vec();
                rects.extend([h.orient, h.pivot, h.snap, h.prop, h.proxy, h.overlays, h.frame]);
                rects.extend([h.shading,h.visibility]);
                for (i, a) in rects.iter().enumerate() {
                    assert!(a.left() >= 0. && a.right() <= width, "width {width}: {a:?} outside");
                    for b in &rects[i + 1..] { assert!(!overlap(a, b), "width {width} snap {snap}: {a:?} overlaps {b:?}"); }
                }
                // the left cluster ends before the right cluster starts
                assert!(h.prop.right() < h.overlays.left(), "width {width} snap {snap}: clusters collide");
                assert!(h.prop.right() + 8. <= h.proxy.left() && h.proxy.right() + 8. <= h.overlays.left());
                assert!(h.proxy.width() >= 50., "width {width} snap {snap}: proxy chip too narrow");
            }
        }
    }

    #[test]
    fn shading_and_visibility_dropdowns_are_adjacent() {
        let h = header_layout(0., 0., 800., false);
        assert!(h.shading.right() < h.visibility.left());
        assert_eq!(Mode::ALL.map(Mode::label), [
            "Wireframe Bounding Box", "Wireframe", "Wireframe Ghost", "Hidden Line Invisible",
            "Hidden Line Ghost", "MatCap Shaded", "Flat Shaded", "Smooth Shaded",
        ]);
    }

    #[test]
    fn error_text_is_split_into_node_and_message() {
        assert_eq!(split_error("node 'noise1' (noise): ValueError: frequency must be greater than 0"), (Some("noise1"), "ValueError: frequency must be greater than 0"));
        assert_eq!(split_error("graph has a cycle"), (None, "graph has a cycle"));
    }

    #[test]
    fn scrubbing_maps_the_bar_to_frames() {
        assert_eq!(frame_at(100., 100., 300.), FRAME_START);
        assert_eq!(frame_at(300., 100., 300.), FRAME_END);
        assert_eq!(frame_at(-50., 100., 300.), FRAME_START);
        let mid = frame_at(200., 100., 300.);
        assert!((mid as i32 - 120).abs() <= 1);
    }

    #[test]
    fn the_overlay_draws_a_frame_without_asking_for_anything() {
        let f = super::super::testing::Fixture::new();
        let acts = f.run(|ui, v, out| overlay(ui, Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 600.)), v, out));
        assert!(acts.is_empty());
    }

    #[test]
    fn popover_switches_and_closes() {
        let mut open = None;
        open = toggle_popover(open, Popover::Orientation);
        assert_eq!(open, Some(Popover::Orientation));
        open = toggle_popover(open, Popover::Pivot);
        assert_eq!(open, Some(Popover::Pivot));
        assert_eq!(toggle_popover(open, Popover::Pivot), None);
    }

    #[test]
    fn overlay_header_toggle_menu_and_geometry_checkbox_are_independent() {
        use egui::{Event,PointerButton,RawInput};
        let f=super::super::testing::Fixture::new(); let ctx=egui::Context::default(); f.theme.apply(&ctx);
        let viewport=Rect::from_min_size(Pos2::ZERO,Vec2::new(900.,600.));
        let h=header_layout(0.,0.,900.,false);
        let frame=|events:Vec<Event>| {
            let mut out=Actions::default();
            let _=ctx.run(RawInput {screen_rect:Some(viewport),events,..Default::default()},|ctx| {
                egui::CentralPanel::default().show(ctx,|ui|overlay(ui,viewport,&f.view(),&mut out));
            }); out
        };
        frame(vec![]);
        let click=|p:Pos2| {
            frame(vec![Event::PointerMoved(p),Event::PointerButton{pos:p,button:PointerButton::Primary,pressed:true,modifiers:Default::default()}]);
            frame(vec![Event::PointerButton{pos:p,button:PointerButton::Primary,pressed:false,modifiers:Default::default()}])
        };
        let toggle=click(h.overlays.left_center()+Vec2::new(12.,0.));
        assert!(toggle.0.iter().any(|a|matches!(a,UiAction::Editor(EditorChange::Overlays(o)) if !o.enabled && o.points && o.lines && o.faces)));
        let menu=click(h.overlays.right_center()-Vec2::new(8.,0.));
        assert!(!menu.0.iter().any(|a|matches!(a,UiAction::Editor(EditorChange::Overlays(_)))));
        assert_eq!(ctx.data(|d|d.get_temp::<VpState>(egui::Id::new("viewport_ui_state"))).unwrap().popover,Some(Popover::Overlays));
        let x=(h.overlays.right()-232.).clamp(6.,662.);
        let points=click(Pos2::new(x+60.,h.overlays.bottom()+4.+28.+9.*24.+12.));
        assert!(points.0.iter().any(|a|matches!(a,UiAction::Editor(EditorChange::Overlays(o)) if !o.points && o.enabled && o.lines && o.faces)));
    }

    #[test]
    fn popover_closes_on_escape_and_outside_click() {
        use egui::{Event, PointerButton, RawInput};
        let f = super::super::testing::Fixture::new();
        let ctx = egui::Context::default();
        f.theme.apply(&ctx);
        let id = egui::Id::new("viewport_ui_state");
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 600.));
        let mut frame = |events: Vec<Event>| {
            let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 800.))), events, ..Default::default() };
            let mut out = Actions::default();
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| overlay(ui, viewport, &f.view(), &mut out));
            });
        };
        let click = |p: Pos2, frame: &mut dyn FnMut(Vec<Event>)| {
            frame(vec![Event::PointerMoved(p), Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Default::default() }]);
            frame(vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Default::default() }]);
        };
        let h = header_layout(0., 0., 900., false);
        frame(vec![]);
        click(h.orient.center(), &mut frame);
        assert_eq!(ctx.data(|d| d.get_temp::<VpState>(id)).unwrap().popover, Some(Popover::Orientation));
        frame(vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
        assert_eq!(ctx.data(|d| d.get_temp::<VpState>(id)).unwrap().popover, None);
        click(h.pivot.center(), &mut frame);
        assert_eq!(ctx.data(|d| d.get_temp::<VpState>(id)).unwrap().popover, Some(Popover::Pivot));
        click(Pos2::new(450., 400.), &mut frame);
        assert_eq!(ctx.data(|d| d.get_temp::<VpState>(id)).unwrap().popover, None);
    }

    #[test]
    fn falloff_curves_have_finite_samples_and_end_at_zero() {
        for falloff in Falloff::ALL {
            let samples = falloff_samples(falloff);
            assert!(samples.iter().all(|x| x.is_finite() && (0. ..=1.).contains(x)));
            assert_eq!(samples[12], 0.);
        }
        assert!((falloff_samples(Falloff::Linear)[6] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn snap_increment_shows_units_in_header() {
        let mut f = super::super::testing::Fixture::new();
        let mut editor = crate::ui::EditorState::default();
        editor.snap.enabled = true;
        editor.snap.to = SnapTo::Increment.into();
        editor.snap.increment = 0.25;
        f.editor = editor;
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 600.));
        let mut out = Actions::default();
        let ctx = egui::Context::default();
        f.theme.apply(&ctx);
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 800.))), ..Default::default() };
        let full = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| overlay(ui, viewport, &f.view(), &mut out));
        });
        fn extract_texts(shapes: &[egui::epaint::ClippedShape], out: &mut Vec<String>) {
            for s in shapes {
                match &s.shape {
                    egui::epaint::Shape::Text(t) => out.push(t.galley.text().to_owned()),
                    egui::epaint::Shape::Vec(subs) => {
                        let clipped: Vec<egui::epaint::ClippedShape> = subs.iter().map(|sub| egui::epaint::ClippedShape { clip_rect: s.clip_rect, shape: sub.clone() }).collect();
                        extract_texts(&clipped, out);
                    }
                    _ => {}
                }
            }
        }
        let mut texts = Vec::new();
        extract_texts(&full.shapes, &mut texts);
        assert!(texts.iter().any(|t| t.contains("250 mm")), "expected 250 mm in {texts:?}");
    }
}

pub fn popup_open(ctx:&egui::Context)->bool {
    ctx.data(|d| d.get_temp::<VpState>(egui::Id::new("viewport_ui_state")).is_some_and(|s| s.popover.is_some())
        || d.get_temp::<Pos2>(egui::Id::new("shading_pie")).is_some())
}

pub fn open_shading_pie(ctx:&egui::Context) {
    let at=ctx.input(|i| i.pointer.hover_pos()).unwrap_or(Pos2::new(300.,200.));
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("shading_pie"),at));
}

#[cfg(test)] mod display_menu_tests {
    use super::*;
    use egui::{Event,PointerButton,RawInput};
    fn frame(ctx:&egui::Context,f:&super::super::testing::Fixture,events:Vec<Event>)->Vec<UiAction> {
        let mut out=Actions::default();
        let _=ctx.run(RawInput { screen_rect:Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(900.,600.))),events,..Default::default() },|ctx| {
            egui::CentralPanel::default().show(ctx,|ui| overlay(ui,Rect::from_min_size(Pos2::ZERO,Vec2::new(900.,600.)),&f.view(),&mut out));
        }); out.0
    }
    fn click(ctx:&egui::Context,f:&super::super::testing::Fixture,p:Pos2)->Vec<UiAction> {
        frame(ctx,f,vec![Event::PointerMoved(p),Event::PointerButton { pos:p,button:PointerButton::Primary,pressed:true,modifiers:Default::default() }]);
        frame(ctx,f,vec![Event::PointerButton { pos:p,button:PointerButton::Primary,pressed:false,modifiers:Default::default() }])
    }
    #[test] fn dropdowns_emit_every_display_and_visibility_choice() {
        let f=super::super::testing::Fixture::new(); let ctx=egui::Context::default(); f.theme.apply(&ctx);
        let h=header_layout(0.,0.,900.,false);
        frame(&ctx,&f,vec![]);
        for (i,mode) in Mode::ALL.into_iter().enumerate() {
            click(&ctx,&f,h.shading.center());
            assert!(popup_open(&ctx));
            let at=Pos2::new(h.shading.right()-100.,h.shading.bottom()+4.+28.+i as f32*24.+12.);
            let actions=click(&ctx,&f,at);
            assert!(actions.iter().any(|a| matches!(a,UiAction::SetMode(m) if *m==mode)),"{} row failed",mode.label());
            assert!(!popup_open(&ctx));
        }
        for (i,visibility) in ObjectVisibility::ALL.into_iter().enumerate() {
            click(&ctx,&f,h.visibility.center());
            let at=Pos2::new(h.visibility.right()-100.,h.visibility.bottom()+4.+28.+i as f32*24.+12.);
            let actions=click(&ctx,&f,at);
            assert!(actions.iter().any(|a| matches!(a,UiAction::Editor(EditorChange::ObjectVisibility(v)) if *v==visibility)));
            assert!(!popup_open(&ctx));
        }
    }
    #[test] fn z_pie_number_selects_mapped_mode_and_closes() {
        let f=super::super::testing::Fixture::new(); let ctx=egui::Context::default();
        frame(&ctx,&f,vec![]); open_shading_pie(&ctx);
        let actions=frame(&ctx,&f,vec![Event::Key { key:egui::Key::Num8,physical_key:None,pressed:true,repeat:false,modifiers:Default::default() }]);
        assert!(actions.iter().any(|a| matches!(a,UiAction::SetMode(Mode::SmoothShaded))));
        assert!(!popup_open(&ctx));
    }
}
