//! Editable Bezier controls in the native viewport. Geometry edits are committed once on release.
use super::{Actions, UiAction, View};
use eframe::egui::{self, Color32, Id, Pos2, Rect, Stroke, Vec2};
use minihou_app::Edit;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Control { pub p:[f64;3], pub left:[f64;3], pub right:[f64;3], pub mode:String }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part { Anchor, Left, Right }
#[derive(Clone, Copy, Debug)]
pub struct Selected { pub index:usize, pub part:Part }
#[derive(Clone, Debug)]
struct Drag { node:String, before:Vec<Control>, current:Vec<Control>, selected:Selected, start:Pos2, base:[f64;3], changed:bool }
fn selection_id(node:&str)->Id { Id::new(("curve_selection",node)) }
fn drag_id()->Id { Id::new("curve_tool_drag") }
pub fn selected(ctx:&egui::Context,node:&str)->Selected { ctx.data(|d|d.get_temp(selection_id(node)).unwrap_or(Selected{index:0,part:Part::Anchor})) }
pub fn set_selected(ctx:&egui::Context,node:&str,selection:Selected) { ctx.data_mut(|d|d.insert_temp(selection_id(node),selection)); }
pub fn active(ctx:&egui::Context)->bool { ctx.data(|d|d.get_temp::<Drag>(drag_id()).is_some()) }
pub fn cancel(ctx:&egui::Context) { ctx.data_mut(|d|d.remove::<Drag>(drag_id())); }
pub fn is_active_node(v:&View)->bool { v.doc["nodes"][v.selected]["type"]=="bezier" && minihou_app::display_nodes(v.doc).iter().any(|id|id==v.selected) }
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
pub fn hit(rect:Rect,v:&View,pointer:Pos2,ctx:&egui::Context)->bool {
    if !is_active_node(v){return false;}
    parse(&v.doc["nodes"][v.selected]["params"]["controls"]).is_some_and(|cs|hit_control(v,rect,&cs,selected(ctx,v.selected),pointer).is_some())
}
pub fn show(ui:&mut egui::Ui,rect:Rect,v:&View,out:&mut Actions,double_click:bool) {
    if !is_active_node(v){cancel(ui.ctx());return;}
    let Some(doc_cs)=parse(&v.doc["nodes"][v.selected]["params"]["controls"]) else{return;};
    let mut drag=ui.ctx().data(|d|d.get_temp::<Drag>(drag_id())).filter(|d|d.node==v.selected);
    let mut cs=drag.as_ref().map(|d|d.current.clone()).unwrap_or_else(||doc_cs.clone());
    let mut selected=selected(ui.ctx(),v.selected);selected.index=selected.index.min(cs.len()-1);
    let pointer=ui.input(|i|i.pointer.latest_pos());
    if let Some(mut d)=drag.take() {
        if ui.input(|i|i.key_pressed(egui::Key::Escape)||i.pointer.secondary_pressed()) {cancel(ui.ctx());}
        else if !ui.input(|i|i.pointer.primary_down()) {
            cancel(ui.ctx());
            if d.changed {out.push(UiAction::Edit{edit:Edit::Param{node:d.node,name:"controls".into(),value:write(&d.current)},continuous:false});}
        } else {
            if let Some(pointer)=pointer {
                if let (Some(now),Some(start))=(plane_point(v,rect,pointer,d.base),plane_point(v,rect,d.start,d.base)) {
                    let precision=if ui.input(|i|i.modifiers.shift){0.1}else{1.};
                    let mut target=add(d.base,mul(sub(now,start),precision));
                    if v.editor.snap.enabled != ui.input(|i|i.modifiers.ctrl) {
                        let step=v.editor.snap.increment as f64;
                        if step>0. {target=target.map(|x|(x/step).round()*step);}
                    }
                    d.current=move_part(&d.before,d.selected.index,d.selected.part,target);
                    d.changed=d.current!=d.before;cs=d.current.clone();
                }
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
                selected=hit;set_selected(ui.ctx(),v.selected,hit);
                let resolved=resolved(&cs,v.doc["nodes"][v.selected]["params"]["closed"].as_bool().unwrap_or(false));
                let c=&resolved[hit.index];let base=match hit.part {Part::Anchor=>c.p,Part::Left=>c.left,Part::Right=>c.right};
                ui.ctx().data_mut(|data|data.insert_temp(drag_id(),Drag{node:v.selected.into(),before:resolved.clone(),current:resolved,selected:hit,start:pointer,base,changed:false}));
            }
        }
    }
    let closed=v.doc["nodes"][v.selected]["params"]["closed"].as_bool().unwrap_or(false);
    let cs=resolved(&cs,closed);let paint=ui.painter_at(rect);
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
}
