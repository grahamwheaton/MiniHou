    use super::*;
    // ---- inference routing through the real sketch input path (shared engine in ctx memory)
    const VP: Rect = Rect { min: Pos2::ZERO, max: Pos2 { x: 1280., y: 800. } };
    fn rig(mode: SketchMode) -> (egui::Context, crate::ui::testing::Fixture) {
        let mut f = crate::ui::testing::Fixture::new();
        f.editor.tool = mode.tool();
        f.camera.projection = inf::math::view_projection([4., -9., 7.], [1.5, 1., 0.], [0., 0., 1.], 0.8, None);
        (egui::Context::default(), f)
    }
    fn key(k: egui::Key) -> egui::Event { egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() } }
    fn frame(ctx: &egui::Context, f: &crate::ui::testing::Fixture, events: Vec<egui::Event>, mesh: Option<(&[f32], &[u32])>, modifiers: egui::Modifiers) -> Actions {
        let mut out = Actions::default();
        let v = f.view();
        let _ = ctx.run(egui::RawInput { screen_rect: Some(VP), events, modifiers, ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| show(ui, VP, &v, &mut out, mesh));
        });
        out
    }
    fn scr(f: &crate::ui::testing::Fixture, p: [f32; 3]) -> Pos2 { transform_gizmo::project(f.camera, VP, p).unwrap() }
    fn click(ctx: &egui::Context, f: &crate::ui::testing::Fixture, at: Pos2, mesh: Option<(&[f32], &[u32])>) -> Actions {
        let mut out = frame(ctx, f, vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }], mesh, Default::default());
        out.0.extend(frame(ctx, f, vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }], mesh, Default::default()).0);
        out
    }
    fn state(ctx: &egui::Context) -> State { ctx.data(|d| d.get_temp(id())).unwrap() }

    #[test] fn alt_lock_primary_click_commits_locked_point_in_line_and_polyline() {
        for mode in [SketchMode::Line, SketchMode::Pen] {
            let (ctx, f) = rig(mode);
            click(&ctx, &f, scr(&f, [1., 1., 0.]), None);
            let along = scr(&f, [2., 1., 0.]);
            frame(&ctx, &f, vec![egui::Event::PointerMoved(along)], None, Default::default());
            let off = scr(&f, [2., 1.3, 0.]);
            frame(&ctx, &f, vec![egui::Event::PointerMoved(off)], None, egui::Modifiers::ALT);
            let locked = state(&ctx).hover.expect("locked hover");
            assert!(engine(&ctx).lock().unwrap().active_lock().is_some());
            frame(&ctx, &f, vec![egui::Event::PointerButton {pos: off, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::ALT}], None, egui::Modifiers::ALT);
            assert_eq!(state(&ctx).points.len(), 2, "{mode:?}");
            assert_eq!(state(&ctx).points[1], locked);
        }
    }

    #[test] fn line_right_click_stages_chains_and_enter_releases_mmb_orbit() {
        let (ctx, f) = rig(SketchMode::Line);
        for (a,b) in [([0.,0.,0.],[1.,0.,0.]), ([0.,1.,0.],[1.,1.,0.])] {
            click(&ctx,&f,scr(&f,a),None); click(&ctx,&f,scr(&f,b),None);
            let out=frame(&ctx,&f,vec![egui::Event::PointerButton {pos:scr(&f,b),button:egui::PointerButton::Secondary,pressed:true,modifiers:Default::default()}],None,Default::default());
            assert!(out.0.is_empty());
            assert!(state(&ctx).points.is_empty() && active(&ctx));
            assert!(!blocks_navigation(&ctx));
        }
        assert_eq!(state(&ctx).pending.len(),2);
        let out=frame(&ctx,&f,vec![key(egui::Key::Enter)],None,Default::default());
        assert!(matches!(&out.0[..],[UiAction::SketchBatch { strokes: rows, .. }] if rows.len()==2));
        assert!(state(&ctx).pending.is_empty());
        let mid=scr(&f,[0.5,0.5,0.]);
        let orbit=frame(&ctx,&f,vec![egui::Event::PointerMoved(mid),egui::Event::PointerButton {pos:mid,button:egui::PointerButton::Middle,pressed:true,modifiers:Default::default()}],None,Default::default());
        assert!(orbit.0.is_empty(), "MMB must remain a viewport navigation input");
        assert!(!blocks_navigation(&ctx) && active(&ctx));
    }

    #[test] fn arrow_lock_and_typed_length_commit_exact_line() {
        let (ctx, f) = rig(SketchMode::Line);
        click(&ctx, &f, scr(&f, [1., 1., 0.]), None);
        let a = state(&ctx).points[0];
        frame(&ctx, &f, vec![key(egui::Key::ArrowRight)], None, Default::default());
        let m = frame(&ctx, &f, vec![egui::Event::PointerMoved(scr(&f, [3., 2.2, 0.])), egui::Event::Text("2.5m".into())], None, Default::default());
        assert!(m.0.is_empty(), "hovering/typing must not emit document actions");
        frame(&ctx, &f, vec![], None, Default::default());
        let s = state(&ctx);
        assert_eq!(s.axis, Some(0));
        let h = s.hover.unwrap();
        assert!(length(sub(h, add(a, [2.5, 0., 0.]))) < 1e-5, "{h:?} vs {a:?}");
        let out = frame(&ctx, &f, vec![key(egui::Key::Enter)], None, Default::default());
        assert!(matches!(&out.0[..], [UiAction::SketchBatch { strokes: rows, .. }] if rows.len() == 1 && rows[0].points.len() == 2 && rows[0].points[1][1] == a[1] && rows[0].points[1][2] == a[2]), "{:?}", out.0.len());
        // Up arrow locks Z (3D, not limited to the sketch plane).
        let (ctx, f) = rig(SketchMode::Line);
        click(&ctx, &f, scr(&f, [1., 1., 0.]), None);
        let a = state(&ctx).points[0];
        frame(&ctx, &f, vec![key(egui::Key::ArrowUp), egui::Event::PointerMoved(scr(&f, [1.3, 1.2, 1.5]))], None, Default::default());
        frame(&ctx, &f, vec![], None, Default::default());
        let h = state(&ctx).hover.unwrap();
        assert!(h[0] == a[0] && h[1] == a[1] && h[2] > 0.5, "{h:?}");
    }

    #[test] fn escape_cancels_all_pending_chains() {
        let (ctx, f) = rig(SketchMode::Pen);
        click(&ctx, &f, scr(&f, [1., 1., 0.]), None);
        frame(&ctx, &f, vec![key(egui::Key::ArrowRight)], None, Default::default());
        let select = |o: &Actions| o.0.iter().any(|a| matches!(a, UiAction::Editor(super::super::EditorChange::Tool(super::super::Tool::Select))));
        let o = frame(&ctx, &f, vec![key(egui::Key::Escape)], None, Default::default());
        assert!(select(&o) && !active(&ctx));
    }

    #[test] fn f_cycles_linear_mode_and_alt_click_does_not() {
        let (ctx, f) = rig(SketchMode::Line);
        frame(&ctx, &f, vec![key(egui::Key::F)], None, Default::default());
        let mode = engine(&ctx).lock().unwrap().linear_mode(&InferenceSettings::default());
        assert_eq!(mode, inf::LinearMode::PointsOnly);
        // Alt used with a click does not cycle the linear mode.
        frame(&ctx, &f, vec![], None, egui::Modifiers::ALT);
        frame(&ctx, &f, vec![egui::Event::PointerButton { pos: pos2(600., 400.), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::ALT }], None, egui::Modifiers::ALT);
        frame(&ctx, &f, vec![], None, Default::default());
        assert_eq!(engine(&ctx).lock().unwrap().linear_mode(&InferenceSettings::default()), inf::LinearMode::PointsOnly);
    }

    #[test] fn mesh_snaps_record_reference_descriptors() {
        // Migrated from sketch_inference: endpoint priority within the radius, nothing far away.
        let positions = [0., 0., 0., 2., 0., 0., 0., 2., 0.5];
        let tris = [0u32, 1, 2];
        let mesh = Some((&positions[..], &tris[..]));
        let (ctx, f) = rig(SketchMode::Line);
        let at = scr(&f, [2., 0., 0.]) + vec2(3., -2.);
        frame(&ctx, &f, vec![egui::Event::PointerMoved(at)], mesh, Default::default());
        let r = state(&ctx).result.unwrap();
        assert_eq!(r.winner.kind, Kind::Endpoint);
        assert_eq!(state(&ctx).hover, Some([2., 0., 0.]));
        frame(&ctx, &f, vec![egui::Event::PointerMoved(scr(&f, [6., 5., 0.]))], mesh, Default::default());
        assert!(!matches!(state(&ctx).result.unwrap().winner.kind, Kind::Endpoint | Kind::Midpoint | Kind::OnEdge));
        click(&ctx, &f, at, mesh);
        let mid = scr(&f, [1., 1., 0.25]) + vec2(2., 2.);
        click(&ctx, &f, mid, mesh);
        let out = frame(&ctx, &f, vec![key(egui::Key::Enter)], mesh, Default::default());
        let Some(UiAction::SketchBatch { strokes: rows, .. }) = out.0.first() else { panic!("no commit") };
        let (refs, points) = (&rows[0].refs, &rows[0].points);
        assert_eq!(points[1], [1., 1., 0.25]);
        assert_eq!(refs[0].as_ref().unwrap()["kind"], "vertex");
        assert_eq!(refs[0].as_ref().unwrap()["element"], json!([1]));
        assert_eq!(refs[1].as_ref().unwrap()["kind"], "edge");
        // Measure keeps its anchor format (node / point / point_count).
        let (ctx, mut f) = rig(SketchMode::Tape);
        f.editor.tool = super::super::Tool::Measure;
        click(&ctx, &f, at, mesh);
        assert_eq!(state(&ctx).refs[0], Some(json!({"node": f.display, "point": 1, "point_count": 3})));
    }
    #[test] fn guide_inference_table_matches_default_tool_keys() {
        // Line endings differ between checkouts (CRLF on Windows): normalise before comparing.
        let guide = include_str!("../../../docs/NATIVE_EDITOR.md").replace("\r\n", "\n");
        let table = guide.split_once("<!-- inference:start -->\n").unwrap().1.split_once("<!-- inference:end -->").unwrap().0;
        let mut rows = String::from("| Input | While a sketch tool (Line, Polyline, Polygon, Rectangle, Circle, Measure) is active |\n| --- | --- |\n");
        for (k, a) in inf::settings::InferenceKeys::default().table_rows() { rows.push_str(&format!("| {k} | {a} |\n")); }
        assert_eq!(table, rows);
        for name in [&InferenceSettings::default().keys.lock_x, &InferenceSettings::default().keys.cycle_candidate] { assert!(key_named(name).is_some()); }
    }
    #[test] fn ruler_finish_cancel_and_tool_switch_release_navigation() {
        let ctx = egui::Context::default();
        let mut f = crate::ui::testing::Fixture::new();
        f.editor.tool = super::super::Tool::Measure;
        let mut tape = State { mode: Some(SketchMode::Tape), points: vec![[0.; 3], [1., 0., 0.]], ..Default::default() };
        finish(&mut tape, &mut Actions::default());
        assert!(tape.mode.is_none() && tape.points.is_empty());
        ctx.data_mut(|d| d.insert_temp(id(), State { mode: Some(SketchMode::Tape), points: vec![[0.; 3]], ..Default::default() }));
        assert!(active(&ctx));
        assert!(!blocks_navigation(&ctx));
        ctx.data_mut(|d| d.insert_temp(id(), tape));
        assert!(!active(&ctx) && !blocks_navigation(&ctx));
        for event in [
            egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() },
            egui::Event::PointerButton { pos: pos2(200., 200.), button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() },
        ] {
            start(&ctx, SketchMode::Tape);
            let _ = ctx.run(egui::RawInput { events: vec![event], ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| show(ui, ui.max_rect(), &f.view(), &mut Actions::default(), None));
            });
            assert!(!active(&ctx) && !blocks_navigation(&ctx));
        }
        start(&ctx, SketchMode::Tape);
        f.editor.tool = super::super::Tool::Select;
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| show(ui, ui.max_rect(), &f.view(), &mut Actions::default(), None));
        });
        assert!(!active(&ctx) && !blocks_navigation(&ctx));
    }
    #[test] fn geometry_and_constraints() {
        assert_eq!(grid_snap([0.26,0.74,3.], Plane::XY, 0.25), [0.25,0.75,3.]);
        assert_eq!(axis_lock([2.,3.,0.], [1.,1.,0.], 0, Plane::XY), [2.,1.,0.]);
        assert_eq!(numeric_length([2.,0.,0.], [0.;3], 3.), [3.,0.,0.]);
        let r = shape(SketchMode::Rectangle, &[[0.;3], [2.,3.,0.]], Plane::XY, 32);
        assert_eq!(r, vec![[0.,0.,0.],[2.,0.,0.],[2.,3.,0.],[0.,3.,0.]]);
        assert_eq!(shape(SketchMode::Circle, &[[0.;3], [1.,0.,0.]], Plane::XY, 32).len(), 32);
        assert!(specification(SketchMode::Polygon, &[[0.;3],[1.,0.,0.],[2.,0.,0.]], Plane::XY).is_none());
        assert!(specification(SketchMode::Polygon, &[[0.;3],[1.,0.,0.],[0.,1.,0.]], Plane::XY).is_some());
        assert!(specification(SketchMode::Polygon, &[[0.,0.,0.],[3.,2.,0.],[0.,2.,0.],[2.,0.,0.]], Plane::XY).is_none());
    }
    #[test] fn sampled_circle_and_arc_offer_centres() {
        for mode in [SketchMode::Circle, SketchMode::Arc] {
            let points=shape(mode,&[[2.,3.,0.],[4.,3.,0.]],Plane::XY,32);
            let g=minihou_core::Geometry::from_arrays(points.iter().flatten().copied().collect(),
                (0..points.len() as i32).collect(),vec![0,points.len() as i32],Some(vec![mode==SketchMode::Circle])).unwrap();
            let mut g=g;
            g.prim.insert("__viewport_curve".into(),std::sync::Arc::new(minihou_core::geometry::Attr::i32(1,vec![1])));
            let (lines,centers)=curve_snap_geometry(&g);
            assert!(!lines.is_empty());
            assert_eq!(centers.len(),1);
            let expected=if mode==SketchMode::Circle {[2.,3.,0.]} else {[3.,3.,0.]};
            assert!(length(sub(centers[0],expected))<1e-3);
        }
    }
    #[test] fn headless_frame() { let f = crate::ui::testing::Fixture::new(); assert!(f.run(|ui,v,out| show(ui, ui.max_rect(), v, out, None)).is_empty()); }
    #[test] fn finishing_a_line_and_tape_emits_document_actions() {
        let mut line = State { mode: Some(SketchMode::Line), points: vec![[0.; 3], [2., 0., 0.]], ..Default::default() };
        let mut out = Actions::default();
        finish(&mut line, &mut out);
        assert!(matches!(&out.0[..], [UiAction::SketchBatch { strokes: rows, .. }] if rows.len()==1 && rows[0].points.len() == 2 && !rows[0].closed));
        let mut tape = State { mode: Some(SketchMode::Tape), points: vec![[0.; 3], [0., 3., 0.]], ..Default::default() };
        let mut out = Actions::default();
        finish(&mut tape, &mut out);
        assert!(matches!(&out.0[..], [UiAction::AddMeasurement { a, b, .. }] if *a == [0.; 3] && *b == [0., 3., 0.]));
    }
    #[test] fn closing_a_line_emits_a_face() {
        let mut line=State {mode:Some(SketchMode::Line),closed:true,
            points:vec![[0.,0.,0.],[2.,0.,0.],[0.,2.,0.]],..Default::default()};
        let mut out=Actions::default(); finish(&mut line,&mut out);
        assert!(matches!(&out.0[..],[UiAction::SketchBatch { strokes: rows, .. }] if rows.len()==1 && rows[0].closed && rows[0].points.len()==3));
    }
    #[test] fn every_sketch_mode_emits_the_curve_it_draws() {
        for (mode, source, expected_len, closed) in [
            (SketchMode::Line, vec![[0.;3], [2.,0.,0.]], 2, false),
            (SketchMode::Pen, vec![[0.;3], [2.,0.,0.], [2.,2.,0.]], 3, false),
            (SketchMode::Polygon, vec![[0.;3], [2.,0.,0.], [2.,2.,0.]], 3, true),
            (SketchMode::Rectangle, vec![[0.;3], [2.,2.,0.]], 4, true),
            (SketchMode::Circle, vec![[0.;3], [2.,0.,0.]], 32, true),
            (SketchMode::Arc, vec![[0.;3], [2.,0.,0.]], 33, false),
            (SketchMode::Bezier, vec![[0.;3], [1.,1.,0.], [2.,0.,0.]], 3, false),
        ] {
            let mut state = State { mode: Some(mode), points: source, ..Default::default() };
            let mut out = Actions::default();
            finish(&mut state, &mut out);
            assert!(matches!(&out.0[..], [UiAction::SketchBatch { strokes: rows, .. }]
                if rows.len()==1 && rows[0].points.len() == expected_len && rows[0].closed == closed), "{mode:?}");
        }
    }
    #[test] fn quick_shapes_remain_active_across_commits() {
        for mode in SketchMode::DRAW {
            assert_eq!(SketchMode::from_tool(mode.tool()),Some(mode));
            let mut state = State { mode: Some(mode), plane: Plane::XZ, ..Default::default() };
            let mut out = Actions::default();
            for _ in 0..2 {
                state.points = vec![[0.;3],[2.,0.,0.],[2.,0.,2.]];
                state.numeric = "2m".into(); state.axis = Some(0);
                finish(&mut state,&mut out);
                assert_eq!(state.mode,Some(mode)); assert!(state.points.is_empty());
                assert!(state.numeric.is_empty()); assert_eq!(state.axis,None); assert_eq!(state.plane,Plane::XZ);
            }
            assert_eq!(out.0.len(),2);
        }
    }
    #[test] fn drawing_escape_requests_select() {
        for mode in SketchMode::DRAW {
            for event in [egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }] {
                let mut f = crate::ui::testing::Fixture::new(); f.editor.tool = mode.tool();
                let ctx = egui::Context::default(); let mut out = Actions::default();
                let _ = ctx.run(egui::RawInput { events:vec![event], ..Default::default() }, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| show(ui,ui.max_rect(),&f.view(),&mut out,None));
                });
                assert!(out.0.iter().any(|a| matches!(a,UiAction::Editor(super::super::EditorChange::Tool(super::super::Tool::Select)))));
                assert!(!active(&ctx));
            }
        }
    }
    #[test] fn every_draw_mode_has_a_distinct_command() {
        let ids: std::collections::BTreeSet<_> = SketchMode::DRAW.iter().map(|m| m.command_id()).collect();
        assert_eq!(ids.len(), SketchMode::DRAW.len());
    }
    #[test] fn starting_a_mode_replaces_the_previous_one() {
        let ctx = egui::Context::default();
        start(&ctx, SketchMode::Rectangle); assert!(active(&ctx));
        start(&ctx, SketchMode::Polygon);
        let s: State = ctx.data(|d| d.get_temp(id())).unwrap();
        assert_eq!(s.mode, Some(SketchMode::Polygon)); assert!(s.points.is_empty());
    }
    #[test] fn arc_and_bezier_keep_their_mode_while_sharing_polyline_tool() {
        for mode in [SketchMode::Arc,SketchMode::Bezier] {
            let (ctx,f)=rig(mode);
            start(&ctx,mode);
            frame(&ctx,&f,vec![],None,Default::default());
            assert_eq!(state(&ctx).mode,Some(mode));
        }
    }
    #[test] fn measurements_render_with_units() {
        let mut f = crate::ui::testing::Fixture::new();
        f.camera.projection = [[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]];
        let mut doc = f.doc.value().clone();
        doc["measurements"] = serde_json::json!([
            { "a": [0.0, 0.0, 0.0], "b": [1.0, 0.0, 0.0] }
        ]);
        f.doc = minihou_app::Document::parse(doc.to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        let ctx = egui::Context::default();
        f.theme.apply(&ctx);
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1280., 800.))), ..Default::default() };
        let mut out = Actions::default();
        let v = f.view();
        let full = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, ui.max_rect(), &v, &mut out, None);
            });
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
        assert!(texts.iter().any(|t| t.contains("1000 mm")), "expected 1000 mm in {texts:?}");
    }
    #[test] fn measurement_readout_rounds_to_nearest_millimetre() {
        use crate::units::{LengthUnit, UnitSettings};
        let mm = UnitSettings::default();
        assert_eq!(format_measurement_distance(0.001499, mm), "1 mm");
        assert_eq!(format_measurement_distance(0.001501, mm), "2 mm");
        assert_eq!(format_measurement_distance(0.0004, mm), "0 mm");
        assert_eq!(format_measurement_distance(0.1499, UnitSettings::new(LengthUnit::Mm, 0.01)), "1 mm");
        assert_eq!(format_measurement_distance(1.2346, UnitSettings::new(LengthUnit::M, 1.)), "1.235 m");
    }
    #[test] fn measurement_anchor_tracks_point_and_detaches_on_topology_change() {
        let doc=json!({"nodes":{"box1":{"measurement_identity":"stable"}}});
        let row=json!({"points":[[1.,2.,3.],[0.,0.,0.]],"refs":[{"node":"box1","identity":"stable","point":1,"point_count":2},null]});
        assert_eq!(measurement_endpoint(&row,0,&doc,"box1",Some(&[0.,0.,0.,4.,5.,6.])),Some(([4.,5.,6.],false)));
        assert_eq!(measurement_endpoint(&row,0,&doc,"box1",Some(&[0.,0.,0.])),Some(([1.,2.,3.],true)));
    }
    #[test] fn measurement_selection_and_delete_key_remove_measurement() {
        let mut f = crate::ui::testing::Fixture::new();
        f.camera.projection = [[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]];
        let mut doc = f.doc.value().clone();
        doc["measurements"] = serde_json::json!([
            { "a": [0.0, 0.0, 0.0], "b": [1.0, 0.0, 0.0] }
        ]);
        f.doc = minihou_app::Document::parse(doc.to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        let ctx = egui::Context::default();
        f.theme.apply(&ctx);
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(1280., 800.));
        let v = f.view();
        let frame = |events: Vec<egui::Event>| {
            let mut out = Actions::default();
            let _ = ctx.run(egui::RawInput { screen_rect: Some(viewport), events, ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    show(ui, viewport, &v, &mut out, None);
                });
            });
            out
        };
        let click = |p: Pos2| {
            frame(vec![egui::Event::PointerMoved(p), egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }]);
            frame(vec![egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }])
        };

        frame(vec![]);
        assert_eq!(selected_measurement(&ctx), None);

        // Click the measurement label target: (a+b)/2 = (840, 400), label is at (848, 392), target is (844..1004, 372..396).
        click(pos2(924., 384.));
        assert_eq!(selected_measurement(&ctx), Some(0));

        // Pressing Delete key removes the selected measurement
        let out = frame(vec![egui::Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        assert!(matches!(out.0.as_slice(), [UiAction::DeleteMeasurement(0)]));
        assert_eq!(selected_measurement(&ctx), None);
    }
    #[test] fn measurement_clear_button_emits_clear_action() {
        let mut f = crate::ui::testing::Fixture::new();
        f.camera.projection = [[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]];
        let mut doc = f.doc.value().clone();
        doc["measurements"] = serde_json::json!([
            { "a": [0.0, 0.0, 0.0], "b": [1.0, 0.0, 0.0] }
        ]);
        f.doc = minihou_app::Document::parse(doc.to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        let ctx = egui::Context::default();
        f.theme.apply(&ctx);
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(1280., 800.));
        let v = f.view();
        let frame = |events: Vec<egui::Event>| {
            let mut out = Actions::default();
            let _ = ctx.run(egui::RawInput { screen_rect: Some(viewport), events, ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    show(ui, viewport, &v, &mut out, None);
                });
            });
            out
        };
        let click = |p: Pos2| {
            frame(vec![egui::Event::PointerMoved(p), egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }]);
            frame(vec![egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }])
        };

        frame(vec![]);
        // Clear button center is (113, 724)
        let out = click(pos2(113., 724.));
        assert!(matches!(out.0.as_slice(), [UiAction::ClearMeasurements]));
    }
    #[test] fn measurement_stale_selection_is_cleared() {
        let mut f = crate::ui::testing::Fixture::new();
        let mut doc = f.doc.value().clone();
        doc["measurements"] = serde_json::json!([
            { "a": [0.0, 0.0, 0.0], "b": [1.0, 0.0, 0.0] }
        ]);
        f.doc = minihou_app::Document::parse(doc.to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        let ctx = egui::Context::default();
        f.theme.apply(&ctx);
        // Simulate selection pointing to out-of-bounds index 5
        ctx.data_mut(|d| d.insert_temp(selected_id(), 5usize));
        assert_eq!(selected_measurement(&ctx), Some(5));

        let mut out = Actions::default();
        let v = f.view();
        let _ = ctx.run(egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1280., 800.))), ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, ui.max_rect(), &v, &mut out, None);
            });
        });
        assert_eq!(selected_measurement(&ctx), None);
    }
