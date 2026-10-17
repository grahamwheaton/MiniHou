//! Modal region gestures. Input runs before navigation; overlays paint after the viewport.
use super::{Actions, UiAction, View};
use crate::picking::{RegionShape, SelectOp, Selection};
use eframe::egui::{self, pos2, Pos2, PointerButton, Rect, Stroke, StrokeKind};

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub selection: Selection,
    pub selected: String,
    pub selected_set: std::collections::BTreeSet<String>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Kind { #[default] Box, Circle, Lasso }
#[derive(Clone, Default)]
struct State {
    armed: bool, implicit: bool, kind: Kind, start: Option<Pos2>, last: Option<Pos2>, points: Vec<Pos2>,
    button: Option<PointerButton>, op: SelectOp, snapshot: Option<Snapshot>,
}
fn id() -> egui::Id { egui::Id::new("viewport_region_select") }
fn radius_id() -> egui::Id { egui::Id::new("viewport_circle_radius") }
#[derive(Clone, Copy, Default)]
pub struct Options { pub kind: Kind, pub op: SelectOp }
fn options_id() -> egui::Id { egui::Id::new("viewport_select_options") }
pub fn options(ctx: &egui::Context) -> Options { ctx.data(|d| d.get_temp(options_id())).unwrap_or_default() }
pub fn set_options(ctx: &egui::Context, options: Options) {
    cancel(ctx);
    ctx.data_mut(|d| d.insert_temp(options_id(), options));
}
pub fn cancel(ctx: &egui::Context) { ctx.data_mut(|d| d.remove::<State>(id())); }
pub fn active(ctx: &egui::Context) -> bool { ctx.data(|d| d.get_temp::<State>(id()).is_some_and(|s| s.armed)) }
pub fn arm(ctx: &egui::Context) { arm_kind(ctx, Kind::Box, None); }
pub fn arm_lasso(ctx: &egui::Context) { arm_kind(ctx, Kind::Lasso, None); }
pub fn arm_circle(ctx: &egui::Context, snapshot: Snapshot) { arm_kind(ctx, Kind::Circle, Some(snapshot)); }
fn arm_kind(ctx: &egui::Context, kind: Kind, snapshot: Option<Snapshot>) {
    ctx.data_mut(|d| d.insert_temp(id(), State { armed: true, kind, snapshot, ..Default::default() }));
    ctx.request_repaint();
}
fn paint_stroke(out: &mut Actions, from: Pos2, to: Pos2, radius: f32, op: SelectOp) {
    if let Some(UiAction::RegionSelect {shape, op: previous}) = out.0.last_mut() {
        if *previous == op {
            match shape {
                RegionShape::Circle {center, radius: r} if *r == radius && *center == from => {
                    *shape=RegionShape::CircleStroke {points:vec![from,to],radius}; return;
                }
                RegionShape::CircleStroke {points, radius: r} if *r == radius && points.last() == Some(&from) => {
                    if from != to { points.push(to); } return;
                }
                _ => {},
            }
        }
    }
    let shape=if from==to {RegionShape::Circle {center:to,radius}} else {RegionShape::CircleStroke {points:vec![from,to],radius}};
    out.push(UiAction::RegionSelect {shape,op});
}
/// Tool-mode drags may start over geometry; handle hits still take precedence.
/// Return true for the entire consumed frame, including release/cancel, to suppress other viewport gestures.
pub fn handle(ctx: &egui::Context, viewport: Rect, v: &View, out: &mut Actions) -> bool {
    let mut s: State = ctx.data(|d| d.get_temp(id())).unwrap_or_default();
    let mut radius: f32 = ctx.data(|d| d.get_temp(radius_id())).unwrap_or(30.);
    let options = options(ctx);
    let mut consumed = s.armed;
    let mut used = Vec::new();
    for (index, event) in ctx.input(|i| i.events.clone()).into_iter().enumerate() {
        let canvas = |p| viewport.contains(p) && super::viewport_ui::selection_canvas_contains(viewport, p);
        if !s.armed {
            if let egui::Event::PointerButton { pos, button, pressed: true, modifiers } = &event {
                if canvas(*pos) && !modifiers.alt {
                    if *button == PointerButton::Secondary && modifiers.ctrl {
                        s = State { armed: true, kind: Kind::Lasso, ..Default::default() };
                    } else if *button == PointerButton::Primary && matches!(v.editor.tool, super::Tool::Select | super::Tool::Move | super::Tool::Rotate | super::Tool::Scale | super::Tool::Transform)
                        && !super::transform_gizmo::hit(viewport, v, *pos) && !super::prim_handles::hit(viewport, v, *pos)
                        && !super::curve_tool::hit(viewport, v, *pos, ctx) && !super::curve_tool::active(ctx) {
                        s = State { armed: true, implicit: true, kind: options.kind, ..Default::default() };
                    }
                }
            }
        }
        if !s.armed { continue; }
        consumed = true;
        match event {
            egui::Event::PointerButton { button: PointerButton::Secondary, pressed: true, .. } if s.kind == Kind::Circle => {
                s=State::default(); used.push(index);
            }
            egui::Event::Key { key: egui::Key::Escape, pressed: true, .. } |
            egui::Event::PointerButton { button: PointerButton::Secondary, pressed: true, .. }
                if !(s.kind == Kind::Lasso && s.start.is_none() && matches!(&event, egui::Event::PointerButton { modifiers, .. } if modifiers.ctrl)) => {
                    if let Some(snapshot) = s.snapshot.take() { out.push(UiAction::RestoreRegionSelection(snapshot)); }
                    s = State::default(); used.push(index);
                }
            egui::Event::Key { key: egui::Key::Enter, pressed: true, .. } => { s = State::default(); used.push(index); }
            egui::Event::MouseWheel { delta, .. } if s.kind == Kind::Circle => {
                radius = (radius + delta.y.signum() * 5.).clamp(4., 300.);
                ctx.input_mut(|i| { i.smooth_scroll_delta = egui::Vec2::ZERO; i.raw_scroll_delta = egui::Vec2::ZERO; });
                used.push(index);
            }
            egui::Event::PointerButton { pos, button, pressed: true, modifiers }
                if canvas(pos) && s.button.is_none() && (button != PointerButton::Primary || !super::transform_gizmo::hit(viewport,v,pos))
                    && (button == PointerButton::Primary || button == PointerButton::Middle || (s.kind == Kind::Lasso && button == PointerButton::Secondary && modifiers.ctrl)) => {
                    s.start = Some(pos); s.last = Some(pos); s.points = vec![pos]; s.button = Some(button);
                    // Ctrl initiates the Blender RMB lasso; Shift modifies that gesture to add.
                    s.op = if button == PointerButton::Middle || (modifiers.ctrl && button != PointerButton::Secondary) { SelectOp::Subtract }
                        else if modifiers.shift || (s.kind == Kind::Circle && !s.implicit) { SelectOp::Add }
                        else if s.implicit { options.op } else { SelectOp::Replace };
                    if s.kind == Kind::Circle {
                        paint_stroke(out, pos, pos, radius, s.op);
                        // Set clears once at the first dab; subsequent samples extend this stroke.
                        if s.op == SelectOp::Replace { s.op = SelectOp::Add; }
                    }
                    used.push(index);
                }
            egui::Event::PointerMoved(pos) if s.start.is_some() => {
                if s.kind == Kind::Circle && s.last != Some(pos) { paint_stroke(out, s.last.unwrap_or(pos), pos, radius, s.op); }
                if s.kind == Kind::Lasso { s.points.push(pos); }
                s.last = Some(pos); used.push(index);
            }
            egui::Event::PointerButton { pos, button, pressed: false, .. } if s.button == Some(button) => {
                if let Some(start) = s.start {
                    match s.kind {
                        Kind::Box => {
                            let rect = Rect::from_two_pos(start, pos);
                            if rect.width().max(rect.height()) > 3. { let shape=RegionShape::Rect(rect);
                                if !super::curve_tool::region_select(ctx,viewport,v,&shape,s.op){out.push(UiAction::RegionSelect { shape, op: s.op });}
                            }
                            else if s.implicit { consumed = false; } // Let ordinary clicks reach click picking.
                        }
                        Kind::Lasso => { s.points.push(pos); if s.points.len() >= 3 { let shape=RegionShape::Lasso(s.points.clone());
                            if !super::curve_tool::region_select(ctx,viewport,v,&shape,s.op){out.push(UiAction::RegionSelect { shape, op: s.op });}
                        } }
                        Kind::Circle => { if s.last != Some(pos) { paint_stroke(out, s.last.unwrap_or(pos), pos, radius, s.op); } },
                    }
                }
                if s.kind == Kind::Circle && !s.implicit { s.start = None; s.last = None; s.button = None; } else { s = State::default(); }
                used.push(index);
            }
            _ => {}
        }
    }
    if consumed && !used.is_empty() {
        ctx.input_mut(|i| { let mut n = 0; i.events.retain(|_| { let keep = !used.contains(&n); n += 1; keep }); });
        ctx.request_repaint();
    }
    ctx.data_mut(|d| { d.insert_temp(id(), s); d.insert_temp(radius_id(), radius); });
    consumed
}
pub fn paint(ui: &egui::Ui, viewport: Rect, v: &View) {
    let ctx = ui.ctx();
    let s: State = ctx.data(|d| d.get_temp(id())).unwrap_or_default();
    if !s.armed { return; }
    let p = ui.painter_at(viewport);
    let stroke = Stroke::new(1., v.theme.accent);
    if s.kind == Kind::Circle {
        let radius = ctx.data(|d| d.get_temp::<f32>(radius_id())).unwrap_or(30.);
        if let Some(now) = ctx.input(|i| i.pointer.hover_pos()) { p.circle_stroke(now, radius, stroke); }
    } else if s.kind == Kind::Lasso && !s.points.is_empty() {
        p.add(egui::Shape::line(s.points.clone(), stroke));
        p.line_segment([*s.points.last().unwrap(), s.points[0]], stroke);
    } else if let (Some(start), Some(now)) = (s.start, ctx.input(|i| i.pointer.hover_pos())) {
        p.rect_stroke(Rect::from_two_pos(start, now), 0., stroke, StrokeKind::Inside);
    }
    let text = match s.kind {
        Kind::Box => "Box: LMB select / MMB subtract / RMB or Esc cancel",
        Kind::Circle => "Circle: LMB add / MMB or Ctrl+LMB subtract / wheel size / RMB or Enter finish / Esc restore",
        Kind::Lasso => "Lasso: drag / Shift add / MMB subtract / RMB or Esc cancel",
    };
    p.text(pos2(viewport.left() + 56., viewport.top() + 80.), egui::Align2::LEFT_TOP, text, egui::FontId::monospace(11.), v.theme.text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, PointerButton, Pos2, RawInput};

    #[test]
    fn selection_works_with_every_transform_tool() {
        for tool in [super::super::Tool::Move, super::super::Tool::Rotate,
            super::super::Tool::Scale, super::super::Tool::Transform] {
            for kind in [Kind::Box, Kind::Circle, Kind::Lasso] {
              for (ctrl,shift,expected) in [(false,false,SelectOp::Replace),(false,true,SelectOp::Add),(true,false,SelectOp::Subtract)] {
                let mut fixture = crate::ui::testing::Fixture::new();
                fixture.editor.tool = tool;
                let ctx = egui::Context::default();
                set_options(&ctx, Options { kind, op: SelectOp::Replace });
                let viewport = Rect::from_min_max(pos2(0., 0.), pos2(800., 600.));
                let a = pos2(330., 230.); let b = pos2(420., 270.); let c = pos2(440., 340.);
                let mut out = Actions::default();
                let _ = ctx.run(RawInput { screen_rect: Some(viewport), events: vec![
                    button(a, PointerButton::Primary, true, ctrl, shift),
                    Event::PointerMoved(b), Event::PointerMoved(c),
                    button(c, PointerButton::Primary, false, ctrl, shift)], ..Default::default() }, |ctx| {
                    handle(ctx, viewport, &fixture.view(), &mut out);
                });
                assert!(out.0.iter().any(|action| matches!(action, UiAction::RegionSelect {op,..} if *op==expected)), "{tool:?} {kind:?} {expected:?}");
              }
            }
        }
    }
    #[test]
    fn gizmo_press_keeps_priority_over_armed_selection() {
        for tool in [super::super::Tool::Move,super::super::Tool::Rotate,super::super::Tool::Scale,super::super::Tool::Transform] {
            let mut f=crate::ui::testing::Fixture::new(); f.editor.tool=tool;
            f.camera=super::super::CameraInfo {projection:[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]],
                ortho:true,ray_right:[1.,0.,0.],ray_up:[0.,1.,0.],ray_forward:[0.,0.,-1.],..Default::default()};
            let mut v=f.view(); v.selection_center=Some([0.;3]);
            let ctx=egui::Context::default(); arm(&ctx);
            let rect=Rect::from_min_max(pos2(0.,0.),pos2(500.,500.)); let center=rect.center();
            let mut out=Actions::default();
            let _=ctx.run(RawInput {screen_rect:Some(rect),events:vec![button(center,PointerButton::Primary,true,false,false)],..Default::default()},|ctx|{
                egui::CentralPanel::default().show(ctx,|ui|{
                    handle(ctx,rect,&v,&mut out);
                    super::super::transform_gizmo::show(ui,rect,&v,&mut out);
                });
            });
            assert!(super::super::transform_gizmo::active(&ctx),"{tool:?}");
            assert!(!out.0.iter().any(|a|matches!(a,UiAction::RegionSelect {..})),"{tool:?}");
        }
    }

    #[test]
    fn box_gesture_requests_region_selection() {
        let f = crate::ui::testing::Fixture::new();
        let ctx = egui::Context::default();
        let viewport = Rect::from_min_max(pos2(0., 0.), pos2(800., 600.));
        let frame = |events: Vec<Event>| {
            let input = RawInput { screen_rect: Some(viewport), events, ..Default::default() };
            let mut out = Actions::default();
            let _ = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| handle(ui.ctx(), viewport, &f.view(), &mut out)); });
            out.0
        };
        arm(&ctx);
        let a = Pos2::new(100., 100.); let b = Pos2::new(210., 160.);
        assert!(frame(vec![Event::PointerMoved(a), Event::PointerButton { pos: a, button: PointerButton::Primary, pressed: true, modifiers: Default::default() }]).is_empty());
        let actions = frame(vec![Event::PointerMoved(b), Event::PointerButton { pos: b, button: PointerButton::Primary, pressed: false, modifiers: Default::default() }]);
        assert!(matches!(&actions[..], [UiAction::RegionSelect { shape: RegionShape::Rect(rect), op: SelectOp::Replace }] if *rect == Rect::from_two_pos(a, b)));
    }
    fn frame(ctx: &egui::Context, events: Vec<Event>) -> Vec<UiAction> {
        let f = crate::ui::testing::Fixture::new();
        let mut out = Actions::default();
        let viewport = Rect::from_min_max(pos2(0., 0.), pos2(800., 600.));
        let _ = ctx.run(RawInput { screen_rect: Some(viewport), events, ..Default::default() }, |ctx| {
            handle(ctx, viewport, &f.view(), &mut out);
        });
        out.0
    }
    fn button(pos: Pos2, button: PointerButton, pressed: bool, ctrl: bool, shift: bool) -> Event {
        Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers { ctrl, shift, ..Default::default() } }
    }
    #[test]
    fn box_overlay_has_no_fill() {
        let ctx = egui::Context::default();
        let f = crate::ui::testing::Fixture::new();
        let viewport = Rect::from_min_max(pos2(0., 0.), pos2(800., 600.));
        let a = pos2(150., 150.); let b = pos2(250., 250.);
        ctx.data_mut(|d| d.insert_temp(id(), State { armed: true, start: Some(a), ..Default::default() }));
        let output = ctx.run(RawInput { screen_rect: Some(viewport), events: vec![Event::PointerMoved(b)], ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| paint(ui, viewport, &f.view()));
        });
        assert!(output.shapes.iter().any(|s| matches!(&s.shape,
            egui::Shape::Rect(r) if r.rect == Rect::from_two_pos(a, b) && r.fill == egui::Color32::TRANSPARENT && r.stroke.width == 1.)));
    }
    #[test]
    fn tool_modes_and_operations_apply_to_plain_drags_and_end_on_release() {
        for kind in [Kind::Box, Kind::Circle, Kind::Lasso] {
            for op in [SelectOp::Replace, SelectOp::Add, SelectOp::Subtract] {
                let ctx = egui::Context::default();
                set_options(&ctx, Options { kind, op });
                let a = pos2(150., 150.); let b = pos2(250., 150.); let c = pos2(250., 250.);
                let actions = frame(&ctx, vec![button(a, PointerButton::Primary, true, false, false),
                    Event::PointerMoved(b), Event::PointerMoved(c), button(c, PointerButton::Primary, false, false, false)]);
                assert!(!active(&ctx));
                assert!(matches!(actions.first(), Some(UiAction::RegionSelect { op: actual, .. }) if *actual == op));
                assert!(matches!((&kind, &actions[0]),
                    (Kind::Box, UiAction::RegionSelect { shape: RegionShape::Rect(_), .. }) |
                    (Kind::Circle, UiAction::RegionSelect { shape: RegionShape::Circle { .. } | RegionShape::CircleStroke { .. }, .. }) |
                    (Kind::Lasso, UiAction::RegionSelect { shape: RegionShape::Lasso(_), .. })));
                if kind == Kind::Circle && op == SelectOp::Replace {
                    assert!(actions[1..].iter().all(|a| matches!(a, UiAction::RegionSelect { op: SelectOp::Add, .. })));
                }
            }
        }
    }
    #[test]
    fn hotkey_modes_do_not_change_the_select_tool_mode() {
        let ctx = egui::Context::default();
        set_options(&ctx, Options { kind: Kind::Lasso, op: SelectOp::Subtract });
        arm(&ctx);
        assert_eq!(options(&ctx).kind, Kind::Lasso);
        assert_eq!(options(&ctx).op, SelectOp::Subtract);
        cancel(&ctx);
        assert!(!active(&ctx));
    }
    #[test]
    fn circle_sweeps_and_escape_restores_snapshot() {
        let ctx = egui::Context::default();
        let snapshot = Snapshot { selection: Selection { points: vec![7], ..Default::default() }, selected: "box1".into(), selected_set: ["box1".into()].into() };
        arm_circle(&ctx, snapshot.clone());
        let a = pos2(100., 100.); let b = pos2(190., 100.);
        let actions = frame(&ctx, vec![button(a, PointerButton::Primary, true, false, false), Event::PointerMoved(b)]);
        assert_eq!(actions.len(), 1); // One swept region rather than repeated mesh scans.
        assert!(matches!(&actions[0], UiAction::RegionSelect {shape:RegionShape::CircleStroke {points,radius:30.},op:SelectOp::Add} if points == &vec![a,b]));
        frame(&ctx, vec![button(b, PointerButton::Primary, false, false, false)]);
        assert!(active(&ctx));
        let actions = frame(&ctx, vec![Event::Key {key:Key::Escape,physical_key:None,pressed:true,repeat:false,modifiers:Default::default()}]);
        assert!(matches!(&actions[..], [UiAction::RestoreRegionSelection(s)] if s.selection == snapshot.selection && s.selected == snapshot.selected && s.selected_set == snapshot.selected_set));
        assert!(!active(&ctx));
    }
    #[test]
    fn right_click_confirms_circle_including_during_a_stroke() {
        for held in [false,true] {
            let ctx=egui::Context::default();
            arm_circle(&ctx,Snapshot {selection:Selection {points:vec![7],..Default::default()},selected:"box".into(),selected_set:Default::default()});
            let p=pos2(200.,200.);
            frame(&ctx,vec![button(p,PointerButton::Primary,true,false,false)]);
            if !held {frame(&ctx,vec![button(p,PointerButton::Primary,false,false,false)]);}
            let actions=frame(&ctx,vec![button(p,PointerButton::Secondary,true,false,false)]);
            assert!(actions.is_empty(),"confirm must not restore the old selection");
            assert!(!active(&ctx));
            assert!(frame(&ctx,vec![button(p,PointerButton::Secondary,false,false,false)]).is_empty());
        }
    }
    #[test]
    fn circle_subtracts_wheel_clamps_and_enter_keeps_selection() {
        let ctx = egui::Context::default();
        arm_circle(&ctx, Snapshot { selection: Selection::default(), selected: String::new(), selected_set: Default::default() });
        let wheel = |y| Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0., y), modifiers: Default::default() };
        frame(&ctx, (0..100).map(|_| wheel(-1.)).collect());
        assert_eq!(ctx.data(|d| d.get_temp::<f32>(radius_id())), Some(4.));
        frame(&ctx, (0..100).map(|_| wheel(1.)).collect());
        assert_eq!(ctx.data(|d| d.get_temp::<f32>(radius_id())), Some(300.));
        let a = pos2(200., 200.);
        for (button_kind, ctrl) in [(PointerButton::Middle, false), (PointerButton::Primary, true)] {
            let actions = frame(&ctx, vec![button(a, button_kind, true, ctrl, false)]);
            assert!(matches!(&actions[..], [UiAction::RegionSelect { op: SelectOp::Subtract, .. }]));
            frame(&ctx, vec![button(a, button_kind, false, ctrl, false)]);
        }
        let actions = frame(&ctx, vec![Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
        assert!(actions.is_empty()); assert!(!active(&ctx));
    }
    #[test]
    fn blender_rmb_lasso_replace_and_shift_add() {
        for shift in [false, true] {
            let ctx = egui::Context::default();
            let a = pos2(100., 100.); let b = pos2(200., 100.); let c = pos2(200., 200.);
            let actions = frame(&ctx, vec![button(a, PointerButton::Secondary, true, true, shift), Event::PointerMoved(b),
                Event::PointerMoved(c), button(c, PointerButton::Secondary, false, true, shift)]);
            assert!(matches!(&actions[..], [UiAction::RegionSelect { shape: RegionShape::Lasso(points), op }]
                if points[0] == a && *op == if shift { SelectOp::Add } else { SelectOp::Replace }));
            assert!(!active(&ctx));
        }
    }
    #[test]
    fn select_drag_can_start_over_geometry_and_box_middle_subtracts() {
        let a = pos2(100., 100.); let b = pos2(220., 200.);
        let ctx = egui::Context::default();
        let actions = frame(&ctx, vec![button(a, PointerButton::Primary, true, false, false), Event::PointerMoved(b), button(b, PointerButton::Primary, false, false, false)]);
        assert_eq!(actions.len(), 1);
        let ctx = egui::Context::default(); arm(&ctx);
        let actions = frame(&ctx, vec![button(a, PointerButton::Middle, true, false, false), button(b, PointerButton::Middle, false, false, false)]);
        assert!(matches!(&actions[..], [UiAction::RegionSelect { op: SelectOp::Subtract, .. }]));
        arm(&ctx);
        assert!(frame(&ctx, vec![button(a, PointerButton::Secondary, true, false, false)]).is_empty());
        assert!(!active(&ctx));
    }

}
