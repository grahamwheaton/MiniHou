//! Headless replays of the live viewport paths (scene -> pick -> node params -> cook), without a window.
use super::*;

fn cook(doc: &Value, node: &str) -> std::sync::Arc<minihou_core::Geometry> {
    Graph::from_value(doc).unwrap().cook(Some(node)).unwrap()
}

#[test]
fn solid_box_selection_does_not_reach_back_elements() {
    use eframe::egui::{pos2, Rect};
    let doc = json!({"version":1,"display":"box1","nodes":{"box1":{"type":"box"}}});
    let scene = Scene::new(&cook(&doc, "box1"));
    let mut camera = Camera::frame(scene.center, scene.radius);
    camera.set_view(StandardView::Front);
    let (yaw, pitch) = camera.angles();
    let (projection, _) = camera.matrices(1.);
    let (ray_origin, ray_forward) = camera.ray(0., 0., 1.);
    let (rx, _) = camera.ray(1., 0., 1.);
    let (ry, _) = camera.ray(0., 1., 1.);
    let info = CameraInfo { yaw, pitch, ortho:true, projection, ray_origin, ray_forward,
        ray_right:[0,1,2].map(|k|rx[k]-ray_origin[k]),
        ray_up:[0,1,2].map(|k|ry[k]-ray_origin[k]) };
    let rect=Rect::from_min_max(pos2(0.,0.),pos2(800.,800.));
    let (lo,hi)=minihou_core::preview::bounds(scene.positions());
    let front=(0..scene.positions().len()/3).find(|&i| {
        let p=picking::at(scene.positions(),i as u32);
        (p[0]-lo[0]).abs()<1e-5 && (p[1]-lo[1]).abs()<1e-5 && (p[2]-lo[2]).abs()<1e-5
    }).unwrap() as u32;
    let back=(0..scene.positions().len()/3).find(|&i| {
        let p=picking::at(scene.positions(),i as u32);
        (p[0]-lo[0]).abs()<1e-5 && (p[1]-hi[1]).abs()<1e-5 && (p[2]-lo[2]).abs()<1e-5
    }).unwrap() as u32;
    let front_pos=picking::at(scene.positions(),front);
    let back_pos=picking::at(scene.positions(),back);
    let pointer=ui::transform_gizmo::project(info,rect,front_pos).unwrap();
    assert_eq!(pointer,ui::transform_gizmo::project(info,rect,back_pos).unwrap());
    let range=vec![("box1".to_owned(),0..scene.positions().len() as u32/3)];
    let selected=std::collections::BTreeSet::from(["box1".to_owned()]);
    let editor=EditorState::default();
    assert!(!editor.xray && editor.overlays.enabled && editor.overlays.wireframe);
    let ignore=picking::ignores_surface_occlusion(Mode::Solid,editor.overlays.enabled,editor.xray);
    assert!(!ignore);
    assert!(element_pick_glue::element_visible_at(&scene,info,rect,front_pos,
        viewport::ObjectVisibility::HideOthers,&range,&selected,ignore));
    assert!(!element_pick_glue::element_visible_at(&scene,info,rect,back_pos,
        viewport::ObjectVisibility::HideOthers,&range,&selected,ignore));
    // The region path checks edge endpoints and triangle centres with the same visibility query.
    let back_edge_end=[hi[0],hi[1],lo[2]];
    let back_face_center=[(lo[0]+hi[0])*0.5,hi[1],(lo[2]+hi[2])*0.5];
    for candidate in [back_edge_end,back_face_center] {
        assert!(!element_pick_glue::element_visible_at(&scene,info,rect,candidate,
            viewport::ObjectVisibility::HideOthers,&range,&selected,ignore));
    }
    for mode in [Mode::Wireframe,Mode::Solid] {
        let through=picking::ignores_surface_occlusion(mode,true,mode==Mode::Solid);
        assert!(element_pick_glue::element_visible_at(&scene,info,rect,back_face_center,
            viewport::ObjectVisibility::HideOthers,&range,&selected,through));
    }
    let hit=pick_displayed(&scene,ui::transform_gizmo::ray(info,rect,pointer).0,
        ui::transform_gizmo::ray(info,rect,pointer).1,viewport::ObjectVisibility::HideOthers,&range,&selected).unwrap();
    assert!(scene.triangle_indices()[hit.tri*3..hit.tri*3+3].iter().all(|&id|
        (picking::at(scene.positions(),id)[1]-lo[1]).abs()<1e-5));
    let topo=element_select::Topology::new(scene.positions(),scene.triangle_indices(),scene.edge_indices());
    let (o,d)=ui::transform_gizmo::ray(info,rect,pointer);
    let point=[0,1,2].map(|k|o[k]+d[k]*hit.t);
    for mode in [ui::SelectionMode::Points,ui::SelectionMode::Edges,ui::SelectionMode::Faces] {
        let picked=topo.pick(scene.positions(),scene.triangle_indices(),hit,point,mode);
        let ids=selected_point_ids(&picked,mode,scene.triangle_indices());
        assert!(!ids.is_empty(),"{mode:?}");
        assert!(ids.iter().all(|&id|(picking::at(scene.positions(),id)[1]-lo[1]).abs()<1e-5),"{mode:?}: {picked:?}");
    }
}
#[test]
fn loop_cut_confirm_selects_the_cooked_loop() {
    let base=stepped_doc();
    let (value,_) = graph_edit::add_viewport_cut(base.value(),"out","polysplit",json!({
        "mode":"edge_loop","split_edges":"p0-1","loops":2,"position":0.5,"factor":0.4,
        "outgrp_edges":true,"outgrp_edges_name":"viewport_loop_cut"
    })).unwrap();
    let g=cook(&value,"out");
    let selection=loop_cut_selection(&g,&(0..g.npoints as u32));
    assert_eq!(selection.mode,ui::SelectionMode::Edges);
    assert_eq!(selection.edges.len(),8);
    let scene=Scene::new(&g);
    assert!(selection.edges.iter().all(|edge|scene.edge_indices().chunks_exact(2)
        .any(|pair|(pair[0],pair[1])==*edge)));
}

#[test]
fn loop_cut_slide_keeps_the_whole_ring_selected_at_any_factor() {
    let base=stepped_doc();
    let mut counts=Vec::new();
    for factor in [0.,0.4,-0.7,1.] {
        let (value,_)=graph_edit::add_viewport_cut(base.value(),"out","polysplit",json!({
            "mode":"edge_loop","split_edges":"p0-1","loops":1,"position":0.5,"factor":factor,
            "outgrp_edges":true,"outgrp_edges_name":"viewport_loop_cut"})).unwrap();
        let g=cook(&value,"out");
        assert!(loop_cut_has_marks(&g));
        counts.push(loop_cut_selection(&g,&(0..g.npoints as u32)).edges.len());
    }
    assert!(counts.iter().all(|&c|c==counts[0]&&c>0),"ring size must not depend on the slide factor: {counts:?}");
}

#[test]
fn undo_restores_face_selection_for_another_extrude() {
    let mut doc=stepped_doc();
    let original=cook(doc.value(),"out");
    let scene=Scene::new(&original);
    let topo=element_select::Topology::new(scene.positions(),scene.triangle_indices(),scene.edge_indices());
    let face=topo.polygons().next().unwrap();
    let selection=picking::Selection {mode:ui::SelectionMode::Faces,tris:topo.poly_tris(face).to_vec(),..Default::default()};
    let mut memory=element_select::SelectionMemory::default();
    let key=element_select::topology_key(scene.positions().len()/3,scene.triangle_indices(),scene.edge_indices());
    assert_eq!(memory.retarget(key,(selection.clone(),picking::Selection::default())).0,selection);
    let mut history=minihou_app::History::default();
    let (next,_)=ui::modal_model::insert_graph(&doc,"out",ModelKind::Extrude,json!({"group":face.to_string(),"distance":0.2})).unwrap();
    let saved_selection=selection_state(&selection,&picking::Selection::default());
    history.record_with_selection(doc.clone(),&next,Some(saved_selection.clone()),Some(saved_selection)); doc=next;
    let changed=Scene::new(&cook(doc.value(),"out"));
    let changed_key=element_select::topology_key(changed.positions().len()/3,changed.triangle_indices(),changed.edge_indices());
    memory.retarget(changed_key,(selection.clone(),picking::Selection::default()));
    let HistoryOutcome::DocEdit { selection:Some(saved),.. }=history.undo_outcome(&mut doc).unwrap() else { panic!("expected document step") };
    let _=memory.retarget(key,(picking::Selection::default(),picking::Selection::default()));
    let restored=restore(saved.selection);
    assert_eq!(restored,selection);
    let faces=topo.polys_of(&restored.tris);
    let group=faces.iter().map(u32::to_string).collect::<Vec<_>>().join(" ");
    let (repeated,_)=ui::modal_model::insert_graph(&doc,"out",ModelKind::Extrude,json!({"group":group,"distance":0.3})).unwrap();
    cook(repeated.value(),"out");
}
/// The polygon whose centroid sits at the centre of a bounding face (the inset's inner face on that side).
fn centre_face(scene: &Scene, topo: &element_select::Topology) -> u32 {
    let p = scene.positions(); let tris = scene.triangle_indices();
    let (lo, hi) = minihou_core::preview::bounds(p);
    let mid = [0, 1, 2].map(|k| (lo[k] + hi[k]) * 0.5);
    topo.polygons().find(|&f| {
        let verts = topo.polygon_vertices(f);
        let c = [0, 1, 2].map(|k| verts.iter().map(|&v| p[v as usize * 3 + k]).sum::<f32>() / verts.len() as f32);
        let on_top = (c[1] - hi[1]).abs() < 1e-4;
        on_top && (c[0] - mid[0]).abs() < 1e-4 && (c[2] - mid[2]).abs() < 1e-4 && topo.poly_tris(f).len() == 2
            && tris.len() > 0
    }).expect("inner top face")
}

#[test]
fn live_pushpull_group_targets_and_through_hole() {
    let doc = json!({"version":1,"display":"out","nodes":{
        "box1":{"type":"box"},
        "in1":{"type":"inset","inputs":{"geo":"box1"},"params":{"group":"","amount":0.25}},
        "out":{"type":"output","inputs":{"geo":"in1"}}}});
    // inset every face first, then use the top inner face
    let g0 = cook(&doc, "box1");
    let mut doc = doc;
    // top face of the default box: find it from the scene of box1
    let s0 = Scene::new(&g0);
    let t0 = element_select::Topology::new(s0.positions(), s0.triangle_indices(), s0.edge_indices());
    let (_, hi) = minihou_core::preview::bounds(s0.positions());
    let top = t0.polygons().find(|&f| t0.polygon_vertices(f).iter().all(|&v| (s0.positions()[v as usize * 3 + 1] - hi[1]).abs() < 1e-5)).unwrap();
    doc["nodes"]["in1"]["params"]["group"] = json!(top.to_string());
    let g = cook(&doc, "out");
    let scene = Scene::new(&g);
    let topo = element_select::Topology::new(scene.positions(), scene.triangle_indices(), scene.edge_indices());
    let face = centre_face(&scene, &topo);
    let selection_tris = topo.poly_tris(face).to_vec();
    let group = group_text(&topo.polys_of(&selection_tris));
    eprintln!("face {face} tris {selection_tris:?} group {group} nprims {}", g.nprims());
    let normal = [0., 1., 0.];
    let targets = ui::modal_model::pushpull_targets(scene.positions(), scene.triangle_indices(), &selection_tris, normal);
    eprintln!("targets {targets:?}");
    let through = targets.iter().find(|t| t.kind == ui::modal_model::SnapKind::Through).expect("through target").distance;
    let document = Document::parse(doc.to_string()).unwrap();
    let source = modeling_source(document.value(), "in1", "out");
    let (mut next, id) = ui::modal_model::insert_graph(&document, &source, ModelKind::PushPull, json!({"group":group,"distance":0.})).unwrap();
    next.edit(&Edit::Param { node: id.clone(), name: "distance".into(), value: json!(through) }).unwrap();
    let out = cook(next.value(), "out");
    let edges = minihou_core::topo::edges(&out.topology, out.npoints);
    assert_eq!(out.npoints as i64 - edges.len() as i64 + out.nprims() as i64, 0, "through-hole is a torus");
}

#[test]
fn live_pushpull_on_face_target_on_stepped_solid() {
    let mut doc = json!({"version":1,"display":"out","nodes":{
        "box1":{"type":"box"},
        "in1":{"type":"inset","inputs":{"geo":"box1"},"params":{"group":"","amount":0.25}},
        "ex1":{"type":"polyextrude","inputs":{"geo":"in1"},"params":{"group":"extrudeFront","distance":0.5}},
        "out":{"type":"output","inputs":{"geo":"ex1"}}}});
    let g0 = cook(&doc, "box1");
    let s0 = Scene::new(&g0);
    let t0 = element_select::Topology::new(s0.positions(), s0.triangle_indices(), s0.edge_indices());
    let (_, hi) = minihou_core::preview::bounds(s0.positions());
    let top = t0.polygons().find(|&f| t0.polygon_vertices(f).iter().all(|&v| (s0.positions()[v as usize * 3 + 1] - hi[1]).abs() < 1e-5)).unwrap();
    doc["nodes"]["in1"]["params"]["group"] = json!(top.to_string());
    let g = cook(&doc, "out");
    let scene = Scene::new(&g);
    let p = scene.positions();
    let topo = element_select::Topology::new(p, scene.triangle_indices(), scene.edge_indices());
    // a frame face of the lower step (all corners at the old top height)
    let face = topo.polygons().find(|&f| topo.polygon_vertices(f).iter().all(|&v| (p[v as usize * 3 + 1] - hi[1]).abs() < 1e-5)).expect("lower step face");
    let tris = topo.poly_tris(face).to_vec();
    let targets = ui::modal_model::pushpull_targets(p, scene.triangle_indices(), &tris, [0., 1., 0.]);
    eprintln!("step targets {targets:?}");
    assert!(targets.iter().any(|t| t.kind == ui::modal_model::SnapKind::OnFace && (t.distance - 0.5).abs() < 1e-4), "{targets:?}");
    let document = Document::parse(doc.to_string()).unwrap();
    let (mut next, id) = ui::modal_model::insert_graph(&document, "out", ModelKind::PushPull, json!({"group":face.to_string(),"distance":0.})).unwrap();
    next.edit(&Edit::Param { node: id, name: "distance".into(), value: json!(0.5) }).unwrap();
    let out = cook(next.value(), "out");
    let edges = minihou_core::topo::edges(&out.topology, out.npoints);
    assert_eq!(out.npoints as i64 - edges.len() as i64 + out.nprims() as i64, 2);
}

fn stepped_doc() -> Document {
    Document::parse(json!({"version":1,"display":"out","nodes":{
        "box1":{"type":"box"},
        "xf1":{"type":"transform","inputs":{"geo":"box1"}},
        "out":{"type":"output","inputs":{"geo":"xf1"}}}}).to_string()).unwrap()
}

/// Live report 8: an action, Ctrl+Z, then a second action failed until the output node was clicked again, because
/// the selected node was the one undo removed. Each modelling kind and a transform splice, undone, then a new action.
#[test]
fn undo_then_next_action_uses_a_live_source() {
    let base = stepped_doc();
    let actions: [(ModelKind, Value); 4] = [
        (ModelKind::Extrude, json!({"group":"0","distance":0.3})),
        (ModelKind::Inset, json!({"group":"0","amount":0.1})),
        (ModelKind::Bevel, json!({"edges":"0-1","distance":0.05,"divisions":1})),
        (ModelKind::PushPull, json!({"group":"0","distance":0.2})),
    ];
    for (first, params) in &actions { for (second, params2) in &actions {
        let mut history = minihou_app::History::default();
        let mut doc = base.clone();
        let (selected, display) = ("out".to_owned(), "out".to_owned());
        let source = modeling_source(doc.value(), &selected, &display);
        let (next, id) = ui::modal_model::insert_graph(&doc, &source, *first, params.clone()).unwrap();
        history.record(doc.clone(), &next); doc = next;
        let selected = id; // the app selects the new node
        assert!(history.undo(&mut doc).is_some());
        assert!(doc.value()["nodes"].get(&selected).is_none(), "undo removed {selected}");
        let selected = surviving_selection(doc.value(), &selected, &display);
        assert_eq!(selected, "out");
        let source = modeling_source(doc.value(), &selected, &display);
        let (next, id2) = ui::modal_model::insert_graph(&doc, &source, *second, params2.clone())
            .unwrap_or_else(|| panic!("{first:?} then undo then {second:?}"));
        cook(next.value(), "out");
        assert_eq!(next.value()["nodes"]["out"]["inputs"]["geo"], id2.as_str());
    } }
    // transform (object mode G) after an undone action: the splice still lands before the output
    let mut value = base.value().clone();
    let id = ensure_transform_target(&mut value, &surviving_selection(base.value(), "gone", "out"), true, Some("gone")).unwrap();
    assert_eq!(id, "xf1");
    // a stale upstream selection does not take ids picked on the displayed scene
    assert_eq!(modeling_source(base.value(), "box1", "out"), "out");
    assert_eq!(modeling_source(base.value(), "xf1", "out"), "xf1");
    assert_eq!(modeling_source(base.value(), "missing", "out"), "out");
}

/// Live report 7: Alt+E with faces selected starts at once; with nothing selected it waits for a pick.
#[test]
fn alt_e_with_face_selection_starts_immediately() {
    use ui::SelectionMode as M;
    let faces = picking::Selection { mode: M::Faces, tris: vec![8, 9], ..Default::default() };
    assert!(pushpull_starts_now(M::Faces, &faces));
    assert!(!pushpull_starts_now(M::Faces, &picking::Selection { mode: M::Faces, ..Default::default() }));
    assert!(!pushpull_starts_now(M::Object, &faces));
    assert!(!pushpull_starts_now(M::Edges, &picking::Selection { mode: M::Edges, edges: vec![(0, 1)], ..Default::default() }));
}

/// One displayed object, Loop Cut confirmed with Enter: the cooked loop must be the selection
/// afterwards, computed from the same range the app uses for the active object.
#[test]
fn loop_cut_confirm_keeps_loop_selected_for_single_object() {
    let base=stepped_doc();
    let (value,id)=graph_edit::add_viewport_cut(base.value(),"out","polysplit",json!({
        "mode":"edge_loop","split_edges":"p0-1","loops":2,"position":0.5,"factor":0.4,
        "outgrp_edges":true,"outgrp_edges_name":"viewport_loop_cut"
    })).unwrap();
    let g=cook(&value,&id);
    // The app derives the active object's range from the displayed geometry (single object: all points).
    let range=0..g.npoints as u32;
    let selection=loop_cut_selection(&g,&range);
    assert!(!selection.edges.is_empty(),"confirm must leave the cooked loop selected");
    assert_eq!(selection.mode,ui::SelectionMode::Edges);
}

#[test]
fn extrude_keeps_original_face_selected() {
    let doc = json!({"version":1,"display":"out","nodes":{
        "box1":{"type":"box"},
        "ex1":{"type":"polyextrude","inputs":{"geo":"box1"},"params":{"group":"0","distance":0.5}},
        "out":{"type":"output","inputs":{"geo":"ex1"}}}});
    let s0 = Scene::new(&cook(&doc, "box1"));
    let topo = element_select::Topology::new(s0.positions(), s0.triangle_indices(), s0.edge_indices());
    let tris: Vec<u32> = (0..s0.triangle_indices().len() as u32 / 3).filter(|&t| topo.poly_of_tri(t) == Some(0)).collect();
    assert!(!tris.is_empty());
    let sel = crate::picking::Selection { mode: crate::ui::SelectionMode::Faces, tris: tris.clone(), ..Default::default() };
    let scene = Scene::new(&cook(&doc, "out"));
    let kept = crate::keep_extrude_selection(&sel, &scene).expect("selection kept");
    assert_eq!(kept.tris, tris);
}

#[test]
fn e_after_inset_starts_the_extrude_modal_on_the_inner_face() {
    use crate::ui::{modal_model::{Kind, State}, SelectionMode};
    let doc = json!({"version":1,"display":"out","nodes":{
        "box1":{"type":"box"},
        "in1":{"type":"inset","inputs":{"geo":"box1"},"params":{"group":"0","amount":0.1}},
        "out":{"type":"output","inputs":{"geo":"in1"}}}});
    let s0 = Scene::new(&cook(&doc, "box1"));
    let topo = element_select::Topology::new(s0.positions(), s0.triangle_indices(), s0.edge_indices());
    let tris: Vec<u32> = (0..s0.triangle_indices().len() as u32 / 3).filter(|&t| topo.poly_of_tri(t) == Some(0)).collect();
    let sel = crate::picking::Selection { mode: SelectionMode::Faces, tris: tris.clone(), ..Default::default() };
    // Inset keeps the face selection through its cooks, like Extrude.
    assert!(crate::keeps_face_selection(Kind::Inset, SelectionMode::Faces));
    assert!(crate::keeps_face_selection(Kind::Extrude, SelectionMode::Faces));
    let scene = Scene::new(&cook(&doc, "out"));
    let kept = crate::keep_extrude_selection(&sel, &scene).expect("selection kept after inset");
    // The kept ids are the inset's inner face: same centre, smaller area.
    let (p0, p1) = (s0.positions(), scene.positions());
    let stats = |p: &[f32], idx: &[u32]| { let mut c = [0f32; 3]; let mut area = 0f32;
        for &t in &tris { let ids: [u32; 3] = idx[t as usize * 3..t as usize * 3 + 3].try_into().unwrap();
            let v = ids.map(|i| [p[i as usize * 3], p[i as usize * 3 + 1], p[i as usize * 3 + 2]]);
            let (u, w) = ([0, 1, 2].map(|k| v[1][k] - v[0][k]), [0, 1, 2].map(|k| v[2][k] - v[0][k]));
            let n = [u[1]*w[2]-u[2]*w[1], u[2]*w[0]-u[0]*w[2], u[0]*w[1]-u[1]*w[0]];
            area += (n[0]*n[0]+n[1]*n[1]+n[2]*n[2]).sqrt() / 2.;
            for k in 0..3 { c[k] += (v[0][k] + v[1][k] + v[2][k]) / 3. / tris.len() as f32; } }
        (c, area) };
    let ((c0, a0), (c1, a1)) = (stats(p0, s0.triangle_indices()), stats(p1, scene.triangle_indices()));
    assert!((0..3).all(|k| (c0[k] - c1[k]).abs() < 1e-4), "{c0:?} vs {c1:?}");
    assert!(a1 < a0 * 0.95 && a1 > 0.);
    // E now starts the modal (not the fixed-distance default node) ...
    assert!(crate::model_key_starts_modal(Kind::Extrude, SelectionMode::Faces, &kept));
    // ... while an empty selection (the old post-inset state) took the default path.
    let empty = crate::picking::Selection { mode: SelectionMode::Faces, ..Default::default() };
    assert!(!crate::model_key_starts_modal(Kind::Extrude, SelectionMode::Faces, &empty));
    // The modal distance starts at 0 and comes from the mouse, or from typed input.
    let mut s = State::new(Kind::Extrude, eframe::egui::pos2(0., 0.), 0.01);
    s.screen_axis = eframe::egui::vec2(0., -1.);
    assert_eq!(s.value, 0.);
    s.move_to(eframe::egui::pos2(0., -20.), false, false); assert!((s.value - 0.2).abs() < 1e-5);
    s.key_event(eframe::egui::Key::Num3, false); assert_eq!(s.value, 3.);
}

#[test]
fn undo_of_a_removed_node_selects_the_node_that_fed_it() {
    let old=json!({"nodes":{"box1":{"type":"box"},"pp":{"type":"pushpull","inputs":{"geo":"box1"}},"other":{"type":"box"},"out":{"type":"output","inputs":{"geo":"pp"}}}});
    let new=json!({"nodes":{"box1":{"type":"box"},"other":{"type":"box"},"out":{"type":"output","inputs":{"geo":"box1"}}}});
    assert_eq!(surviving_selection_from(&old,&new,"pp","other"),"box1");
    assert_eq!(surviving_selection_from(&old,&new,"box1","other"),"box1");
    assert_eq!(surviving_selection_from(&old,&new,"gone","other"),"other");
}
