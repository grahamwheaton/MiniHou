//! Viewport hints for the active modal operator.
use eframe::egui::{self, pos2, Color32, Pos2, Rect, Stroke};
use super::{modal_transform::{BaseStage, CageHandle, Constraint, Kind, Modal, SketchStage}, transform_gizmo::project, SnapTo, View};

fn line(ui: &egui::Ui, rect: Rect, a: Pos2, b: Pos2, color: Color32, width: f32) {
    ui.painter_at(rect).line_segment([a,b], Stroke::new(width,color));
}
pub fn show(ui: &egui::Ui, rect: Rect, v: &View<'_>, modal: &Modal) {
    let painter = ui.painter_at(rect);
    let center = project(v.camera,rect,modal.pivot).unwrap_or(rect.center());
    let colors = [v.theme.x_axis, v.theme.y_axis, v.theme.z_axis];
    let orientation=if matches!(modal.constraint,Constraint::Axis(_,true)|Constraint::Plane(_,true))
        { super::Orientation::Local } else { v.editor.orientation };
    let frame = super::modal_transform::orientation_frame(orientation,v.camera,modal.rotation);
    if let Constraint::Axis(axis, _) | Constraint::Plane(axis, _) = modal.constraint {
        let offset = frame[axis].map(|x| x * v.bounds.1.max(0.1) * 0.2);
        let b = [0,1,2].map(|i| modal.pivot[i]+offset[i]);
        if let Some(b)=project(v.camera,rect,b) {
            let delta=b-center;
            if delta.length_sq()>1. { let span=delta.normalized()*rect.size().length();
                line(ui,rect,center-span,center+span,colors[axis],1.5); }
        }
    }
    if modal.proportional {
        let extent = [modal.pivot[0]+modal.radius,modal.pivot[1],modal.pivot[2]];
        let radius = project(v.camera,rect,extent).map(|p|(p-center).length()).unwrap_or(modal.radius*40.).max(3.);
        painter.circle_stroke(center,radius,Stroke::new(1_f32,v.theme.accent));
    }
    if let Some(target) = modal.target.and_then(|p|project(v.camera,rect,p)) {
        if let Some(shape)=modal.target_shape {
            let points=shape.map(|p|project(v.camera,rect,p));
            match modal.target_kind {
                Some(SnapTo::Edge | SnapTo::EdgeCenter) => {
                    let pairs=[(0,1),(1,2),(2,0)];
                    let distance=|a: Pos2,b: Pos2| { let ab=b-a;
                        let t=((target-a).dot(ab)/ab.length_sq().max(1e-8)).clamp(0.,1.);
                        (target-(a+ab*t)).length_sq() };
                    if let Some((a,b))=pairs.into_iter().filter_map(|(a,b)|Some((points[a]?,points[b]?)))
                        .min_by(|(a,b),(c,d)| distance(*a,*b).total_cmp(&distance(*c,*d))) {
                        line(ui,rect,a,b,v.theme.accent,3.);
                    }
                }
                Some(SnapTo::Face) => if let [Some(a),Some(b),Some(c)]=points {
                    painter.add(egui::Shape::convex_polygon(vec![a,b,c],v.theme.accent.gamma_multiply(0.25),
                        Stroke::new(2_f32,v.theme.accent)));
                },
                _ => {}
            }
        }
        painter.circle_filled(target,6.,v.theme.accent);
        painter.circle_stroke(target,9.,Stroke::new(1.5_f32,Color32::WHITE));
    }
    if modal.base_stage!=BaseStage::Off {
        if modal.kind==Kind::Rotate {
            if let (Some(base),Some(source))=(modal.base_point,modal.source_direction) {
                let axis=modal.rotation_axis(frame,v.camera);
                let delta=super::modal_transform::base_snap_offset(base,source);
                let along=(0..3).map(|i|delta[i]*axis[i]).sum::<f32>();
                let tangent=[0,1,2].map(|i|delta[i]-axis[i]*along);
                let length=tangent.iter().map(|x|x*x).sum::<f32>().sqrt();
                if length>1e-6 {
                    let tangent=tangent.map(|x|x/length);
                    let side=[axis[1]*tangent[2]-axis[2]*tangent[1],
                        axis[2]*tangent[0]-axis[0]*tangent[2],axis[0]*tangent[1]-axis[1]*tangent[0]];
                    let offset=modal.base_target.map(|target|super::modal_transform::base_snap_offset(base,target)).unwrap_or([0.;3]);
                    let origin=[0,1,2].map(|i|base[i]+offset[i]);
                    let radius=v.bounds.1.max(0.1)*0.3;
                    let at=|angle: f32,scale: f32| {
                        let (sin,cos)=angle.to_radians().sin_cos();
                        project(v.camera,rect,[0,1,2].map(|i|origin[i]+radius*scale*(tangent[i]*cos+side[i]*sin)))
                    };
                    for i in 0..72 {
                        let angle=i as f32*5.;
                        let major=i%3==0;
                        if let (Some(a),Some(b))=(at(angle,if major {0.87} else {0.94}),at(angle,1.)) {
                            line(ui,rect,a,b,if major {Color32::WHITE} else {Color32::GRAY},if major {1.4} else {0.7});
                        }
                    }
                    let angle=modal.base_angle.unwrap_or(0.);
                    let steps=(angle.abs()/5.).ceil().clamp(1.,144.) as usize;
                    let arc: Vec<Pos2>=(0..=steps).filter_map(|i|at(angle*i as f32/steps as f32,0.78)).collect();
                    if arc.len()>1 {painter.add(egui::Shape::line(arc,Stroke::new(2.,v.theme.accent)));}
                    if let Some(start)=at(0.,1.) {line(ui,rect,project(v.camera,rect,origin).unwrap_or(center),start,v.theme.accent,1.5);}
                    if let Some(end)=at(angle,1.) {line(ui,rect,project(v.camera,rect,origin).unwrap_or(center),end,Color32::WHITE,2.);}
                    painter.text(modal.pointer+egui::vec2(16.,-16.),egui::Align2::LEFT_BOTTOM,
                        format!("{angle:.1}°"),egui::FontId::proportional(14.),Color32::WHITE);
                }
            }
        }
        let mark=|world: Option<[f32;3]>, label: &str, color: Color32| {
            if let Some(pos)=world.and_then(|p|project(v.camera,rect,p)) {
                painter.circle_stroke(pos,8.,Stroke::new(2.,color));
                painter.text(pos+egui::vec2(11.,-11.),egui::Align2::LEFT_BOTTOM,label,
                    egui::FontId::proportional(13.),color);
            }
        };
        mark(modal.base_point,"Base",v.theme.accent);
        mark(modal.base_target,"Target",Color32::WHITE);
        if let Some(base)=modal.base_point.and_then(|p|project(v.camera,rect,p)) {
            // Rotate and scale: pivot -> reference (accent) and pivot -> destination (white), no translation.
            if let Some(reference)=modal.base_target.and_then(|p|project(v.camera,rect,p)) {
                if modal.kind!=super::modal_transform::Kind::Move { line(ui,rect,base,reference,v.theme.accent,2.); }
            }
            if let Some(target)=modal.target_direction.and_then(|p|project(v.camera,rect,p)) {
                line(ui,rect,base,target,Color32::WHITE,2.);
            }
            if modal.base_stage==BaseStage::Target { line(ui,rect,base,modal.pointer,Color32::LIGHT_GRAY,1.); }
            if modal.base_stage==BaseStage::Destination { line(ui,rect,base,modal.pointer,Color32::WHITE,1.); }
        }
        let move_kind=modal.kind==super::modal_transform::Kind::Move;
        let label=match modal.base_stage {
            BaseStage::Base=>if move_kind {"Pick base point"} else {"Pick pivot point"},
            BaseStage::Target=>if move_kind {"Pick target point"} else {"Pick reference point"},
            BaseStage::Destination=>"Pick destination point",
            BaseStage::Done=>"Base snap",BaseStage::Off=>"" };
        painter.text(modal.pointer+egui::vec2(16.,16.),egui::Align2::LEFT_TOP,label,
            egui::FontId::proportional(13.),Color32::WHITE);
    }
    if modal.sketch != SketchStage::Off {
        let pivot = modal.sketch_pivot.and_then(|p|project(v.camera,rect,p)).unwrap_or(modal.pointer);
        painter.circle_stroke(pivot,8.,Stroke::new(2_f32,v.theme.accent));
        line(ui,rect,pivot,pos2(pivot.x+15.,pivot.y),colors[0],2.);
        line(ui,rect,pivot,pos2(pivot.x,pivot.y-15.),colors[1],2.);
        if let Some(reference) = modal.sketch_reference {
            line(ui,rect,pivot,reference,Color32::LIGHT_GRAY,1.5);
            if modal.sketch == SketchStage::Angle {
                line(ui,rect,pivot,modal.pointer,v.theme.accent,2.);
                let angle = super::modal_transform::angle_between(reference,modal.pointer,pivot);
                let start=(reference-pivot).angle();
                let radius=28.;
                let arc: Vec<Pos2>=(0..=24).map(|i| {
                    let theta=start+angle.to_radians()*i as f32/24.;
                    pivot+egui::vec2(theta.cos(),theta.sin())*radius
                }).collect();
                painter.add(egui::Shape::line(arc,Stroke::new(1.5_f32,v.theme.accent)));
                painter.text(modal.pointer+egui::vec2(12.,-12.),egui::Align2::LEFT_BOTTOM,
                    format!("{angle:.1}°"),egui::FontId::proportional(13.),Color32::WHITE);
            }
        }
    }
    if modal.cage {
        let (lo,hi)=modal.bounds;
        let corners: Vec<Option<Pos2>> = (0..8).map(|i|project(v.camera,rect,[
            if i&1 != 0 { hi[0] } else { lo[0] }, if i&2 != 0 { hi[1] } else { lo[1] }, if i&4 != 0 { hi[2] } else { lo[2] }])).collect();
        for i in 0..8 { for bit in [1,2,4] { if i&bit == 0 {
            if let (Some(a),Some(b))=(corners[i],corners[i|bit]) { line(ui,rect,a,b,v.theme.accent,1.); }
        } } }
        for (i,point) in corners.into_iter().enumerate() { if let Some(point)=point {
            let active = modal.cage_handle == Some(CageHandle::Corner([i&1!=0,i&2!=0,i&4!=0]));
            painter.circle_filled(point,if active { 6. } else { 4. },if active { Color32::WHITE } else { v.theme.accent });
        } }
        for axis in 0..3 { for high in [false,true] {
            let mut point = [0,1,2].map(|i|(lo[i]+hi[i])*0.5); point[axis]=if high {hi[axis]} else {lo[axis]};
            if let Some(point)=project(v.camera,rect,point) {
                let active=modal.cage_handle==Some(CageHandle::Face(axis,high));
                painter.rect_filled(Rect::from_center_size(point,egui::vec2(8.,8.)),1.,if active {Color32::WHITE} else {colors[axis]});
            }
        } }
    }
}

pub fn cage_hit(rect: Rect, camera: super::CameraInfo, modal: &Modal, pointer: Pos2) -> Option<CageHandle> {
    let (lo,hi)=modal.bounds;
    let mut candidates=Vec::new();
    for i in 0..8 { let p=[if i&1!=0 {hi[0]} else {lo[0]},if i&2!=0 {hi[1]} else {lo[1]},if i&4!=0 {hi[2]} else {lo[2]}];
        if let Some(q)=project(camera,rect,p) { candidates.push(((q-pointer).length(),CageHandle::Corner([i&1!=0,i&2!=0,i&4!=0]))); } }
    for axis in 0..3 { for high in [false,true] {
        let mut p=[0,1,2].map(|i|(lo[i]+hi[i])*0.5); p[axis]=if high {hi[axis]} else {lo[axis]};
        if let Some(q)=project(camera,rect,p) { candidates.push(((q-pointer).length(),CageHandle::Face(axis,high))); }
    } }
    candidates.into_iter().filter(|(distance,_)|*distance<13.).min_by(|a,b|a.0.total_cmp(&b.0)).map(|(_,handle)|handle)
}
