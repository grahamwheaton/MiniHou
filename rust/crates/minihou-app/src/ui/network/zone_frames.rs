use super::*;
use minihou_core::graph::ZoneDef;

pub(super) fn zone_nodes(zone: &ZoneDef) -> Vec<String> {
    std::iter::once(&zone.start).chain(zone.members.iter()).chain(std::iter::once(&zone.end))
        .cloned().collect()
}

/// World-space frame, derived every draw from the zone's current cards.
pub(super) fn zone_bounds(zone: &ZoneDef, positions: &[(String, Pos2)]) -> Option<Rect> {
    let mut cards=zone_nodes(zone).into_iter().map(|id| {
        positions.iter().find(|(node,_)|node==&id).map(|(_,at)|Rect::from_min_size(*at,CARD))
    });
    let mut bounds=cards.next()??;
    for card in cards {bounds=bounds.union(card?);}
    Some(Rect::from_min_max(bounds.min-vec2(32.,40.),bounds.max+vec2(32.,28.)))
}

pub(super) fn zone_hit<'a>(zones: &'a [ZoneDef], positions: &[(String, Pos2)], at: Pos2) -> Option<&'a ZoneDef> {
    zones.iter().filter_map(|zone| {
        let rect=zone_bounds(zone,positions)?;
        let header=Rect::from_min_max(rect.min,pos2(rect.right(),rect.top()+30.));
        let border=rect.contains(at) && !rect.shrink(8.).contains(at);
        (header.contains(at) || border).then_some((zone,rect.area()))
    }).min_by(|a,b|a.1.total_cmp(&b.1)).map(|(zone,_)|zone)
}

pub(super) fn move_zone_document(before:&serde_json::Value, selected:&[(String,Pos2)], delta:Vec2) -> serde_json::Value {
    let mut next=before.clone();
    for (id,at) in selected {next["nodes"][id]["pos"]=serde_json::json!([at.x+delta.x,at.y+delta.y]);}
    next
}

pub(super) fn draw_zones(p:&egui::Painter, canvas:Rect, state:&State, v:&View, positions:&[(String,Pos2)]) {
    // Larger parent frames first, so nested frames remain legible.
    let mut frames:Vec<_>=v.graph.zones.iter().filter_map(|zone|zone_bounds(zone,positions).map(|rect|(zone,rect))).collect();
    frames.sort_by(|a,b|b.1.area().total_cmp(&a.1.area()));
    for (zone,world) in frames {
        let rect=Rect::from_min_max(screen(canvas,state,world.min),screen(canvas,state,world.max));
        if !rect.intersects(canvas) {continue;}
        let selected=zone_nodes(zone).iter().all(|id|v.selected_set.contains(id));
        let color=if selected {v.theme.sel} else {v.theme.accent};
        p.rect_filled(rect,8.*state.zoom,v.theme.accent_bg.gamma_multiply(0.15));
        p.rect_stroke(rect,8.*state.zoom,Stroke::new(if selected {2.}else{1.},color.gamma_multiply(0.65)),StrokeKind::Inside);
        let start=&v.doc["nodes"][&zone.start];
        let method=start["params"]["method"].as_str().unwrap_or("connected");
        let domain=start["params"]["domain"].as_str().unwrap_or("points");
        let label=format!("For Each  ·  {method} / {domain}");
        p.text(rect.left_top()+vec2(14.*state.zoom,19.*state.zoom),Align2::LEFT_CENTER,label,
            FontId::proportional(12.*state.zoom),color);
        // Boundary sockets mirror the real start input and end output; their hit targets
        // are added by canvas_ports and connect to those same nodes.
        let top=pos2(rect.center().x,rect.top());
        let bottom=pos2(rect.center().x,rect.bottom());
        for at in [top,bottom] {p.circle(at,6.*state.zoom,v.theme.base,Stroke::new(1.5,color));}
        p.text(top+vec2(10.*state.zoom,2.*state.zoom),Align2::LEFT_TOP,"Geometry",FontId::proportional(9.*state.zoom),v.theme.dim);
        p.text(bottom+vec2(10.*state.zoom,-2.*state.zoom),Align2::LEFT_BOTTOM,"Generated",FontId::proportional(9.*state.zoom),v.theme.dim);
    }
}

#[cfg(test)] mod tests {
    use super::*;
    use serde_json::json;
    #[test] fn bounds_and_hit_follow_members_and_nested_frames() {
        let outer=ZoneDef{id:"outer".into(),kind:"for_each".into(),start:"s".into(),end:"e".into(),members:vec!["body".into()],parent:None};
        let inner=ZoneDef{id:"inner".into(),kind:"for_each".into(),start:"body".into(),end:"e".into(),members:vec![],parent:Some("outer".into())};
        let positions=vec![("s".into(),pos2(0.,0.)),("body".into(),pos2(50.,100.)),("e".into(),pos2(0.,200.))];
        assert_eq!(zone_bounds(&outer,&positions),Some(Rect::from_min_max(pos2(-32.,-40.),pos2(302.,284.))));
        assert_eq!(zone_hit(&[outer.clone()],&positions,pos2(0.,-20.)).unwrap().id,"outer");
        assert!(zone_hit(&[outer.clone()],&positions,pos2(100.,60.)).is_none());
        let zones=[outer,inner];
        let hit=zone_hit(&zones,&positions,pos2(51.,69.)).unwrap();
        assert_eq!(hit.id,"inner");
    }
    #[test] fn frame_move_is_one_undoable_document_change() {
        let before=json!({"nodes":{"s":{"type":"for_each_start","pos":[0.,0.]},"b":{"type":"null","pos":[10.,100.]},"e":{"type":"for_each_end","pos":[0.,200.]}}});
        let selected=vec![("s".into(),pos2(0.,0.)),("b".into(),pos2(10.,100.)),("e".into(),pos2(0.,200.))];
        let next=move_zone_document(&before,&selected,vec2(40.,20.));
        assert_eq!(next["nodes"]["b"]["pos"],json!([50.,120.]));
        let mut doc=minihou_app::Document::parse(before.to_string()).unwrap();
        let mut history=minihou_app::History::default();
        history.replace(&mut doc,minihou_app::Document::parse(next.to_string()).unwrap());
        assert!(history.undo(&mut doc).is_some());
        assert_eq!(doc.value(),&before);
        assert!(!history.can_undo());
        assert!(history.redo(&mut doc).is_some());
        assert_eq!(doc.value(),&next);
    }
}
