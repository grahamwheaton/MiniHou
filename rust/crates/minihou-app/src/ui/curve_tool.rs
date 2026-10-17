//! Editable Bezier controls in the native viewport. Geometry edits are committed once on release.
use super::{Actions, Orientation, Tool, UiAction, View};
use eframe::egui::{self, Color32, Id, PointerButton, Pos2, Rect, Stroke, Vec2};
use minihou_app::Edit;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Control { pub p:[f64;3], pub left:[f64;3], pub right:[f64;3], pub mode:String }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part { Anchor, Left, Right }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selected { pub index:usize, pub part:Part }
#[derive(Clone, Debug)]
struct Drag { node:String, before:Vec<Control>, current:Vec<Control>, selected:Selected, start:Pos2, base:[f64;3],
    parts:Vec<Selected>,kind:Tool, axis:Option<usize>, plane:bool, modal:bool, changed:bool }
fn selection_id(node:&str)->Id { Id::new(("curve_selection",node)) }
fn selection_set_id(node:&str)->Id { Id::new(("curve_selection_set",node)) }
fn drag_id()->Id { Id::new("curve_tool_drag") }
pub fn selected(ctx:&egui::Context,node:&str)->Selected { ctx.data(|d|d.get_temp(selection_id(node)).unwrap_or(Selected{index:0,part:Part::Anchor})) }
fn selected_parts(ctx:&egui::Context,node:&str)->Vec<Selected> {ctx.data(|d|d.get_temp(selection_set_id(node))).unwrap_or_else(||vec![selected(ctx,node)])}
pub fn set_selected(ctx:&egui::Context,node:&str,selection:Selected) { ctx.data_mut(|d|{d.insert_temp(selection_id(node),selection);d.insert_temp(selection_set_id(node),vec![selection]);}); }
fn extend_selected(ctx:&egui::Context,node:&str,selection:Selected) {let mut parts=selected_parts(ctx,node);if !parts.contains(&selection){parts.push(selection);}ctx.data_mut(|d|{d.insert_temp(selection_id(node),selection);d.insert_temp(selection_set_id(node),parts);});}
pub fn active(ctx:&egui::Context)->bool { ctx.data(|d|d.get_temp::<Drag>(drag_id()).is_some()) }
pub fn cancel(ctx:&egui::Context) { ctx.data_mut(|d|d.remove::<Drag>(drag_id())); }
pub fn start_modal(ctx:&egui::Context,node:&str,controls:&Value,kind:super::modal_transform::Kind,pointer:Pos2)->bool {
    let Some(cs)=parse(controls) else {return false;};
    let selected=selected(ctx,node);let selected=Selected{index:selected.index.min(cs.len()-1),..selected};
    let c=&cs[selected.index];let base=part_position(c,selected.part);
    let parts=selected_parts(ctx,node);
    ctx.data_mut(|data|data.insert_temp(drag_id(),Drag{node:node.into(),before:cs.clone(),current:cs,selected,start:pointer,base,parts,
        kind:match kind {super::modal_transform::Kind::Move=>Tool::Move,super::modal_transform::Kind::Rotate=>Tool::Rotate,super::modal_transform::Kind::Scale=>Tool::Scale},
        axis:None,plane:false,modal:true,changed:false}));true
}
/// The selected node (network editor selection / active object) is a Bezier: its handles show and edit whether or not it is displayed.
pub fn is_active_node(v:&View)->bool { v.doc["nodes"][v.selected]["type"]=="bezier" }
pub fn node_controls(doc:&Value,node:&str)->Value {
    doc["nodes"][node]["params"]["controls"].as_str().map(|s|Value::String(s.into()))
        .unwrap_or_else(||(minihou_core::nodes::get("bezier").unwrap().params.iter().find(|p|p.name=="controls").unwrap().default)())
}
pub fn visible_controls(ctx:&egui::Context,doc:&Value,node:&str)->Option<Vec<Control>> {
    ctx.data(|d|d.get_temp::<Drag>(drag_id())).filter(|d|d.node==node).map(|d|d.current)
        .or_else(||parse(&node_controls(doc,node)))
}
pub fn parse(value:&Value)->Option<Vec<Control>> {
    let text=value.as_str()?;
    let cs:Vec<Control>=serde_json::from_str(text).ok()?;
    (cs.len()>=2 && cs.len()<=10000 && cs.iter().all(|c| [c.p,c.left,c.right].iter().flatten().all(|x|x.is_finite()) &&
        ["AUTO","VECTOR","ALIGNED","MIRRORED","FREE"].contains(&c.mode.as_str()))).then_some(cs)
}
pub fn write(cs:&[Control])->Value { Value::String(serde_json::to_string(cs).unwrap_or_default()) }
fn add(a:[f64;3],b:[f64;3])->[f64;3] { [0,1,2].map(|k|a[k]+b[k]) }
fn sub(a:[f64;3],b:[f64;3])->[f64;3] { [0,1,2].map(|k|a[k]-b[k]) }
fn mul(a:[f64;3],s:f64)->[f64;3] { a.map(|x|x*s) }
fn dot(a:[f64;3],b:[f64;3])->f64 { (0..3).map(|k|a[k]*b[k]).sum() }
fn len(a:[f64;3])->f64 { dot(a,a).sqrt() }
fn unit(a:[f64;3])->[f64;3] { mul(a,1./len(a).max(1e-12)) }
fn midpoint(a:[f64;3],b:[f64;3])->[f64;3] { mul(add(a,b),0.5) }
fn part_position(c:&Control,part:Part)->[f64;3] {match part {Part::Anchor=>c.p,Part::Left=>c.left,Part::Right=>c.right}}
pub fn resolved(cs:&[Control],closed:bool)->Vec<Control> {
    let mut out=cs.to_vec(); let n=out.len();
    for i in 0..n {
        let p=cs[i].p; let prev=if i>0||closed {cs[(i+n-1)%n].p} else {p};
        let next=if i+1<n||closed {cs[(i+1)%n].p} else {p};
        match cs[i].mode.as_str() {
            "VECTOR"=>{out[i].left=add(p,mul(sub(prev,p),1./3.));out[i].right=add(p,mul(sub(next,p),1./3.));},
            "AUTO"=>{let d=unit(sub(next,prev));out[i].left=sub(p,mul(d,len(sub(p,prev))/3.));out[i].right=add(p,mul(d,len(sub(next,p))/3.));},
            "ALIGNED"|"MIRRORED"=>{let d=sub(cs[i].right,p); if len(d)>1e-12 {out[i].left=sub(p,mul(unit(d),if cs[i].mode=="MIRRORED" {len(d)} else {len(sub(cs[i].left,p))}));}},
            _=>{}
        }
    }
    out
}
pub fn move_part(cs:&[Control],index:usize,part:Part,to:[f64;3])->Vec<Control> {
    let mut out=cs.to_vec(); let Some(c)=out.get_mut(index) else {return out};
    match part {
        Part::Anchor=>{let delta=sub(to,c.p);c.p=to;c.left=add(c.left,delta);c.right=add(c.right,delta);},
        Part::Left|Part::Right=>{
            let old_other=if part==Part::Left {c.right} else {c.left};
            if matches!(c.mode.as_str(),"AUTO"|"VECTOR") { c.mode="FREE".into(); }
            let direction=sub(to,c.p);
            let other=if c.mode=="MIRRORED" {sub(c.p,direction)} else {sub(c.p,mul(unit(direction),len(sub(old_other,c.p))))};
            if part==Part::Left {c.left=to;if matches!(c.mode.as_str(),"ALIGNED"|"MIRRORED") {c.right=other;}}
            else {c.right=to;if matches!(c.mode.as_str(),"ALIGNED"|"MIRRORED") {c.left=other;}}
        }
    }
    out
}
pub fn append(cs:&[Control],p:[f64;3])->Vec<Control> {
    let mut out=cs.to_vec();out.push(Control{p,left:sub(p,[0.33,0.,0.]),right:add(p,[0.33,0.,0.]),mode:"AUTO".into()});out
}
pub fn delete(cs:&[Control],index:usize)->Vec<Control> { let mut out=cs.to_vec();if out.len()>2&&index<out.len(){out.remove(index);}out }
pub fn split(cs:&[Control],index:usize,closed:bool)->Vec<Control> {
    if index>=cs.len() || (index+1==cs.len()&&!closed) {return cs.to_vec();}
    let mut out=resolved(cs,closed);let next=(index+1)%out.len();
    let (a,b)=(&out[index],&out[next]);
    let (q0,q1,q2)=(midpoint(a.p,a.right),midpoint(a.right,b.left),midpoint(b.left,b.p));
    let (r0,r1)=(midpoint(q0,q1),midpoint(q1,q2));
    out[index].right=q0;out[index].mode="FREE".into();out[next].left=q2;out[next].mode="FREE".into();
    out.insert(index+1,Control{p:midpoint(r0,r1),left:r0,right:r1,mode:"ALIGNED".into()});out
}
pub fn transform_part(cs:&[Control],index:usize,part:Part,kind:Tool,axis:Option<usize>,plane:bool,amount:f64,frame:[[f64;3];3])->Vec<Control> {
    let Some(c)=cs.get(index) else {return cs.to_vec()};
    let pivot=c.p;
    let transform=|point:[f64;3]| {
        let v=sub(point,pivot);
        if kind==Tool::Rotate {
            let n=frame[axis.unwrap_or(2)];let (sin,cos)=amount.to_radians().sin_cos();
            let cross=[n[1]*v[2]-n[2]*v[1],n[2]*v[0]-n[0]*v[2],n[0]*v[1]-n[1]*v[0]];
            add(pivot,add(add(mul(v,cos),mul(cross,sin)),mul(n,dot(n,v)*(1.-cos))))
        } else {
            let factor=amount.max(1e-6);
            let delta=match axis {None=>mul(v,factor-1.),Some(a) if plane=>mul(sub(v,mul(frame[a],dot(v,frame[a]))),factor-1.),Some(a)=>mul(frame[a],dot(v,frame[a])*(factor-1.))};
            add(point,delta)
        }
    };
    if part!=Part::Anchor {return move_part(cs,index,part,transform(part_position(c,part)));}
    let mut out=cs.to_vec();out[index].left=transform(c.left);out[index].right=transform(c.right);
    if matches!(c.mode.as_str(),"AUTO"|"VECTOR") {out[index].mode="FREE".into();}out
}
fn frame(v:&View)->[[f64;3];3] {
    match v.editor.orientation {
        Orientation::View=>[v.camera.ray_right,v.camera.ray_up,v.camera.ray_forward].map(|a|unit(a.map(f64::from))),
        Orientation::Cursor=>{
            let r=crate::transform_math::euler_xyz(v.editor.cursor_rotation.map(f64::from));
            crate::transform_math::Frame::from_rotation(&r).axes
        },
        _=>[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]],
    }
}
fn xyz(p:[f64;3])->[f32;3] {p.map(|x|x as f32)}
fn screen(v:&View,rect:Rect,p:[f64;3])->Option<Pos2> { super::transform_gizmo::project(v.camera,rect,xyz(p)) }
fn plane_point(v:&View,rect:Rect,pointer:Pos2,through:[f64;3])->Option<[f64;3]> {
    let (o,d)=super::transform_gizmo::ray(v.camera,rect,pointer);
    let n=v.camera.ray_forward.map(|x|x as f64);let o=o.map(|x|x as f64);let d=d.map(|x|x as f64);
    let denom=dot(d,n);if denom.abs()<1e-9{return None;}
    let t=dot(sub(through,o),n)/denom;
    (t>=0.).then(||add(o,mul(d,t)))
}
fn hit_control(v:&View,rect:Rect,cs:&[Control],selected:Selected,pointer:Pos2)->Option<Selected> {
    let resolved=resolved(cs,v.doc["nodes"][v.selected]["params"]["closed"].as_bool().unwrap_or(false));
    let selected_index=selected.index.min(resolved.len()-1);
    for part in [Part::Left,Part::Right] {
        let p=if part==Part::Left {resolved[selected_index].left} else {resolved[selected_index].right};
        if screen(v,rect,p).is_some_and(|s|s.distance(pointer)<=8.) {return Some(Selected{index:selected_index,part});}
    }
    resolved.iter().enumerate().find(|(_,c)|screen(v,rect,c.p).is_some_and(|s|s.distance(pointer)<=10.))
        .map(|(index,_)|Selected{index,part:Part::Anchor})
}
fn screen_distance(p:Pos2,a:Pos2,b:Pos2)->f32 {let ab=b-a;let t=((p-a).dot(ab)/ab.length_sq().max(1e-6)).clamp(0.,1.);(p-(a+ab*t)).length()}
fn gizmo(v:&View,rect:Rect,cs:&[Control],selected:Selected,pointer:Option<Pos2>,paint:Option<&egui::Painter>)->Option<(Option<usize>,bool)> {
    let tool=v.editor.tool;
    if !matches!(tool,Tool::Move|Tool::Rotate|Tool::Scale|Tool::Transform) {return None;}
    let c=cs.get(selected.index)?;let origin=if matches!(tool,Tool::Move|Tool::Transform) {part_position(c,selected.part)} else {c.p};
    let center=screen(v,rect,origin)?;let frame=frame(v);
    let colors=[Color32::from_rgb(241,107,103),Color32::from_rgb(131,207,114),Color32::from_rgb(118,170,255)];
    let mut dirs=[Vec2::ZERO;3];
    for axis in 0..3 {let tip=screen(v,rect,add(origin,frame[axis])).unwrap_or(center);
        let delta=tip-center;dirs[axis]=if delta.length()>2. {delta.normalized()*64.} else {Vec2::ZERO};
    }
    let mut hit=None;
    if tool==Tool::Rotate {
        for axis in 0..3 {
            let a=frame[(axis+1)%3];let b=frame[(axis+2)%3];
            let unit=screen(v,rect,add(origin,a)).map(|q|q.distance(center)).unwrap_or(60.).max(1.);
            let radius=60. / unit as f64;
            let mut last=None;
            for step in 0..=64 {let theta=step as f64*std::f64::consts::TAU/64.;
                let world=add(origin,mul(add(mul(a,theta.cos()),mul(b,theta.sin())),radius));
                if let Some(q)=screen(v,rect,world) {if let Some(prev)=last {
                    if pointer.is_some_and(|p|screen_distance(p,prev,q)<=7.) {hit=Some((Some(axis),false));}
                    if let Some(paint)=paint {paint.line_segment([prev,q],Stroke::new(2_f32,colors[axis]));}
                }last=Some(q);}
            }
        }
    } else {
        for axis in 0..3 {let tip=center+dirs[axis];if dirs[axis].length()<10.{continue;}
            if pointer.is_some_and(|p|p.distance(tip)<=9.) {hit=Some((Some(axis),false));}
            if let Some(paint)=paint {paint.line_segment([center+dirs[axis].normalized()*14.,tip],Stroke::new(2.5_f32,colors[axis]));
                if tool==Tool::Scale {paint.rect_filled(Rect::from_center_size(tip,Vec2::splat(10.)),1.,colors[axis]);}
                else {paint.circle_filled(tip,5.,colors[axis]);}}
        }
        if matches!(tool,Tool::Move|Tool::Transform) {
            for normal in 0..3 {let (a,b)=((normal+1)%3,(normal+2)%3);
                let corner=center+dirs[a]*0.35+dirs[b]*0.35;
                if pointer.is_some_and(|p|p.distance(corner)<=9.) {hit=Some((Some(normal),true));}
                if let Some(paint)=paint {paint.rect_filled(Rect::from_center_size(corner,Vec2::splat(10.)),1.,colors[normal].gamma_multiply(0.7));}
            }
        }
    }
    hit
}
pub fn hit(rect:Rect,v:&View,pointer:Pos2,ctx:&egui::Context)->bool {
    if !is_active_node(v){return false;}
    parse(&node_controls(v.doc,v.selected)).is_some_and(|cs| {
        let selected=selected(ctx,v.selected);
        hit_control(v,rect,&cs,selected,pointer).is_some() || gizmo(v,rect,&cs,selected,Some(pointer),None).is_some()
    })
}
pub fn region_select(ctx:&egui::Context,rect:Rect,v:&View,shape:&crate::picking::RegionShape,op:crate::picking::SelectOp)->bool {
    if !is_active_node(v){return false;}
    let Some(cs)=parse(&node_controls(v.doc,v.selected)) else{return false;};
    let cs=resolved(&cs,v.doc["nodes"][v.selected]["params"]["closed"].as_bool().unwrap_or(false));
    let active=selected(ctx,v.selected).index.min(cs.len()-1);
    let mut found=Vec::new();
    for (index,c) in cs.iter().enumerate() {
        if index==active {for part in [Part::Left,Part::Right] {
            if screen(v,rect,part_position(c,part)).is_some_and(|p|shape.contains(p)){found.push(Selected{index,part});}
        }}
        if screen(v,rect,c.p).is_some_and(|p|shape.contains(p)){found.push(Selected{index,part:Part::Anchor});}
    }
    let mut parts=if op==crate::picking::SelectOp::Replace {Vec::new()} else {selected_parts(ctx,v.selected)};
    for part in found {
        if op==crate::picking::SelectOp::Subtract {parts.retain(|p|*p!=part);}
        else if !parts.contains(&part){parts.push(part);}
    }
    let last=parts.last().copied().unwrap_or(Selected{index:0,part:Part::Anchor});
    ctx.data_mut(|d|{d.insert_temp(selection_id(v.selected),last);d.insert_temp(selection_set_id(v.selected),parts);});
    true
}
pub fn show(ui:&mut egui::Ui,rect:Rect,v:&View,out:&mut Actions,double_click:bool) {
    if !is_active_node(v){cancel(ui.ctx());return;}
    let Some(doc_cs)=parse(&node_controls(v.doc,v.selected)) else{return;};
    let mut drag=ui.ctx().data(|d|d.get_temp::<Drag>(drag_id())).filter(|d|d.node==v.selected);
    let mut cs=drag.as_ref().map(|d|d.current.clone()).unwrap_or_else(||doc_cs.clone());
    let mut selected=selected(ui.ctx(),v.selected);selected.index=selected.index.min(cs.len()-1);
    let pointer=ui.input(|i|i.pointer.latest_pos());
    if let Some(mut d)=drag.take() {
        if ui.input(|i|i.key_pressed(egui::Key::Escape)||i.pointer.secondary_pressed()) {cancel(ui.ctx());}
        else if (!d.modal && !ui.input(|i|i.pointer.primary_down())) || (d.modal && ui.input(|i|i.key_pressed(egui::Key::Enter)||i.pointer.button_clicked(PointerButton::Primary))) {
            cancel(ui.ctx());
            if d.changed {out.push(UiAction::Edit{edit:Edit::Param{node:d.node,name:"controls".into(),value:write(&d.current)},continuous:false});}
        } else {
            if d.modal {for axis in 0..3 {let key=[egui::Key::X,egui::Key::Y,egui::Key::Z][axis];
                if ui.input(|i|i.key_pressed(key)) {let plane=ui.input(|i|i.modifiers.shift);
                    if d.axis==Some(axis)&&d.plane==plane {d.axis=None;d.plane=false;} else {d.axis=Some(axis);d.plane=plane;}
                }
            }}
            if let Some(pointer)=pointer {
                let precision=if ui.input(|i|i.modifiers.shift){0.1}else{1.};
                let snap=v.editor.snap.enabled != ui.input(|i|i.modifiers.ctrl);
                let frame=frame(v);
                if d.kind==Tool::Move {
                    if let (Some(now),Some(start))=(plane_point(v,rect,pointer,d.base),plane_point(v,rect,d.start,d.base)) {
                        let mut delta=mul(sub(now,start),precision);
                        if let Some(axis)=d.axis {let along=mul(frame[axis],dot(delta,frame[axis]));delta=if d.plane {sub(delta,along)} else {along};}
                        if snap {let step=v.editor.snap.increment as f64;if step>0. {delta=delta.map(|x|(x/step).round()*step);}}
                        d.current=d.before.clone();
                        for part in &d.parts {if let Some(control)=d.before.get(part.index) {
                            d.current=move_part(&d.current,part.index,part.part,add(part_position(control,part.part),delta));
                        }}
                    }
                } else {
                    let raw=((pointer.x-d.start.x)-(pointer.y-d.start.y)) as f64*precision;
                    let mut amount=if d.kind==Tool::Rotate {raw*0.6} else {(raw/120.).exp()};
                    if snap {let step=if d.kind==Tool::Rotate {5.} else {0.1};
                        amount=if d.kind==Tool::Rotate {(amount/step).round()*step} else {1.+((amount-1.)/step).round()*step};}
                    d.current=d.before.clone();
                    for part in &d.parts {d.current=transform_part(&d.current,part.index,part.part,d.kind,d.axis,d.plane,amount,frame);}
                }
                d.changed=d.current!=d.before;cs=d.current.clone();
            }
            ui.ctx().data_mut(|data|data.insert_temp(drag_id(),d));
        }
    } else if let Some(pointer)=pointer.filter(|p|rect.contains(*p)) {
        if double_click && ui.input(|i|i.modifiers.shift) {
            if let Some(p)=plane_point(v,rect,pointer,cs.last().unwrap().p) {
                cs=append(&cs,p);selected=Selected{index:cs.len()-1,part:Part::Anchor};set_selected(ui.ctx(),v.selected,selected);
                out.push(UiAction::Edit{edit:Edit::Param{node:v.selected.into(),name:"controls".into(),value:write(&cs)},continuous:false});
            }
        } else if ui.input(|i|i.pointer.primary_pressed()) {
            if let Some(hit)=hit_control(v,rect,&cs,selected,pointer) {
                selected=hit;if ui.input(|i|i.modifiers.shift){extend_selected(ui.ctx(),v.selected,hit);}else{set_selected(ui.ctx(),v.selected,hit);}
                let resolved=resolved(&cs,v.doc["nodes"][v.selected]["params"]["closed"].as_bool().unwrap_or(false));
                let c=&resolved[hit.index];let base=part_position(c,hit.part);
                let parts=selected_parts(ui.ctx(),v.selected);
                ui.ctx().data_mut(|data|data.insert_temp(drag_id(),Drag{node:v.selected.into(),before:resolved.clone(),current:resolved,selected:hit,start:pointer,base,parts,
                    kind:Tool::Move,axis:None,plane:false,modal:false,changed:false}));
            } else if let Some((axis,plane))=gizmo(v,rect,&cs,selected,Some(pointer),None) {
                let resolved=resolved(&cs,v.doc["nodes"][v.selected]["params"]["closed"].as_bool().unwrap_or(false));
                let base=part_position(&resolved[selected.index],selected.part);
                let parts=selected_parts(ui.ctx(),v.selected);
                ui.ctx().data_mut(|data|data.insert_temp(drag_id(),Drag{node:v.selected.into(),before:resolved.clone(),current:resolved,selected,start:pointer,base,parts,
                    kind:if v.editor.tool==Tool::Transform {Tool::Move} else {v.editor.tool},axis,plane,modal:false,changed:false}));
            }
        }
    }
    let closed=v.doc["nodes"][v.selected]["params"]["closed"].as_bool().unwrap_or(false);
    let cs=resolved(&cs,closed);let paint=ui.painter_at(rect);
    gizmo(v,rect,&cs,selected,pointer,Some(&paint));
    let gold=Color32::from_rgb(232,181,101);let bright=Color32::from_rgb(255,176,77);
    for i in 0..(cs.len()-usize::from(!closed)) {
        let a=&cs[i];let b=&cs[(i+1)%cs.len()];let mut last=None;
        for step in 0..=32 {let t=step as f64/32.;let u=1.-t;
            let p=[0,1,2].map(|k|u*u*u*a.p[k]+3.*u*u*t*a.right[k]+3.*u*t*t*b.left[k]+t*t*t*b.p[k]);
            if let Some(q)=screen(v,rect,p) {if let Some(prev)=last {paint.line_segment([prev,q],Stroke::new(2_f32,gold));}last=Some(q);}
        }
    }
    for (i,c) in cs.iter().enumerate() {
        let Some(p)=screen(v,rect,c.p) else {continue;};
        if i==selected.index {for h in [c.left,c.right] {if let Some(q)=screen(v,rect,h) {
            paint.line_segment([p,q],Stroke::new(1.5_f32,bright));
            paint.rect_filled(Rect::from_center_size(q,Vec2::splat(10.)),1.,bright);
        }}}
        paint.circle_filled(p,if i==selected.index {7.} else {5.},if i==selected.index {bright} else {Color32::from_rgb(232,237,244)});
        paint.circle_stroke(p,if i==selected.index {7.} else {5.},Stroke::new(2_f32,Color32::from_rgb(23,32,44)));
    }
}

#[cfg(test)] mod tests {
    use super::*;
    fn controls()->Vec<Control> {vec![Control{p:[0.,0.,0.],left:[-1.,0.,0.],right:[1.,0.,0.],mode:"FREE".into()},Control{p:[3.,0.,0.],left:[2.,0.,0.],right:[4.,0.,0.],mode:"FREE".into()}]}
    #[test] fn round_trip_and_coupling() {
        let mut cs=controls();cs[0].mode="MIRRORED".into();
        let moved=move_part(&cs,0,Part::Right,[0.,2.,0.]);assert_eq!(moved[0].left,[0.,-2.,0.]);
        cs[0].mode="ALIGNED".into();let moved=move_part(&cs,0,Part::Right,[0.,2.,0.]);assert_eq!(moved[0].left,[0.,-1.,0.]);
        cs[0].mode="AUTO".into();assert_eq!(move_part(&cs,0,Part::Left,[-2.,0.,0.])[0].mode,"FREE");
        assert_eq!(parse(&write(&controls())),Some(controls()));
    }
    #[test] fn split_preserves_cubic() {
        let cs=controls();let split=split(&cs,0,false);assert_eq!(split.len(),3);
        assert_eq!(split[0].right,[0.5,0.,0.]);assert_eq!(split[1].p,[1.5,0.,0.]);
        assert_eq!(split[2].left,[2.5,0.,0.]);
        assert_eq!(delete(&split,1).len(),2);
    }
    #[test] fn anchor_rotate_scale_and_handle_promotion() {
        let frame=[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]];
        let mut cs=controls();cs[0].mode="VECTOR".into();
        let rotated=transform_part(&cs,0,Part::Right,Tool::Rotate,Some(2),false,90.,frame);
        assert_eq!(rotated[0].mode,"FREE");
        assert!((rotated[0].right[1]-1.).abs()<1e-9);
        let scaled=transform_part(&controls(),0,Part::Anchor,Tool::Scale,None,false,2.,frame);
        assert_eq!(scaled[0].left,[-2.,0.,0.]);assert_eq!(scaled[0].right,[2.,0.,0.]);
    }
    #[test] fn selected_bezier_shows_handles_when_not_displayed() {
        use crate::ui::testing::Fixture;
        let mut f=Fixture::new();
        let cs=controls();
        f.doc=minihou_app::Document::parse(serde_json::json!({"version":1,"nodes":{"curve1":{"type":"bezier","params":{"controls":write(&cs)}},"xform1":{"type":"transform","inputs":{"geo":"curve1"}}},"display":"xform1"}).to_string()).unwrap();
        f.graph=minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected="curve1".into();f.selected_set=std::collections::BTreeSet::from([f.selected.clone()]);f.display="xform1".into();
        f.camera=crate::ui::CameraInfo{ortho:true,projection:[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]],ray_origin:[0.,0.,10.],ray_forward:[0.,0.,-1.],ray_right:[1.,0.,0.],ray_up:[0.,1.,0.],..Default::default()};
        let ctx=egui::Context::default();let rect=Rect::from_min_size(Pos2::ZERO,egui::vec2(800.,600.));
        let v=f.view();assert!(is_active_node(&v));
        let handle=screen(&v,rect,cs[0].right).unwrap();
        assert!(hit(rect,&v,handle,&ctx),"the selected, non-displayed bezier's handle is hit-testable");
        f.selected="xform1".into();assert!(!is_active_node(&f.view()));
    }
    #[test] fn raw_pointer_routes_handle_press_drag_and_single_edit() {
        use crate::ui::testing::Fixture;
        use eframe::egui::{vec2,Event,Modifiers,PointerButton,RawInput};
        let mut f=Fixture::new();
        let mut cs=controls();cs[1].p=[-0.5,0.5,0.];cs[1].left=[-0.8,0.5,0.];cs[1].right=[-0.2,0.5,0.];
        f.doc=minihou_app::Document::parse(serde_json::json!({"version":1,"nodes":{"curve1":{"type":"bezier","params":{"controls":write(&cs)}}},"display":"curve1"}).to_string()).unwrap();
        f.graph=minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected="curve1".into();f.selected_set=std::collections::BTreeSet::from([f.selected.clone()]);f.display=f.selected.clone();
        f.editor.tool=Tool::Move;
        f.camera=crate::ui::CameraInfo{ortho:true,projection:[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]],ray_origin:[0.,0.,10.],ray_forward:[0.,0.,-1.],ray_right:[1.,0.,0.],ray_up:[0.,1.,0.],..Default::default()};
        let ctx=egui::Context::default();let rect=Rect::from_min_size(Pos2::ZERO,vec2(800.,600.));
        let handle=screen(&f.view(),rect,cs[0].right).unwrap();
        let mut frame=|events:Vec<Event>| {
            let raw=RawInput{screen_rect:Some(rect),events,..Default::default()};
            let mut actions=Actions::default();let view=f.view();
            let _=ctx.run(raw,|ctx| {egui::CentralPanel::default().show(ctx,|ui| {
                assert!(!crate::ui::selection_region::handle(ctx,rect,&view,&mut actions));
                show(ui,rect,&view,&mut actions,false);
            });});
            actions.0
        };
        let button=|pos,pressed|Event::PointerButton{pos,button:PointerButton::Primary,pressed,modifiers:Modifiers::NONE};
        frame(vec![Event::PointerMoved(handle),button(handle,true)]);
        assert_eq!(selected(&ctx,"curve1").part,Part::Right);
        assert!(active(&ctx));
        frame(vec![Event::PointerMoved(handle+vec2(30.,20.))]);
        let edits=frame(vec![button(handle+vec2(30.,20.),false)]);
        assert_eq!(edits.iter().filter(|a|matches!(a,UiAction::Edit{..})).count(),1);
        assert!(!active(&ctx));
        let anchor=screen(&f.view(),rect,cs[1].p).unwrap();
        let shape=crate::picking::RegionShape::Rect(Rect::from_center_size(anchor,vec2(20.,20.)));
        assert!(region_select(&ctx,rect,&f.view(),&shape,crate::picking::SelectOp::Add));
        assert!(selected_parts(&ctx,"curve1").contains(&Selected{index:0,part:Part::Right}));
        assert!(selected_parts(&ctx,"curve1").contains(&Selected{index:1,part:Part::Anchor}));
    }
}
