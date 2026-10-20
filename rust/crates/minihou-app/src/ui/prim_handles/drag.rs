use super::*;

#[derive(Clone, Debug)]
pub struct PrimDrag {
    pub(super) node: String,
    pub(super) start_pointer: Pos2,
    pub(super) screen_dir: Vec2,
    pub(super) span: f32,
    pub(super) start_params: Value,
    pub(super) action: GripAction,
    pub(super) world_dir: [f32; 3],
    pub(super) camera: CameraInfo,
    pub(super) rect: Rect,
}

pub fn active(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<PrimDrag>(Id::new("prim_handle_drag")).is_some())
}

pub fn finish_input(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<PrimDrag>(Id::new("prim_handle_drag")));
}
pub(super) fn selected_node<'a>(v: &'a View) -> Option<(&'a str, &'a str, &'a Value)> {
    if v.selected.is_empty() { return None; }
    let node = v.doc.get("nodes")?.get(v.selected)?;
    let ty = node.get("type")?.as_str()?;
    let params = node.get("params").unwrap_or(&Value::Null);
    Some((v.selected, ty, params))
}

pub fn hit(rect: Rect, v: &View, pointer: Pos2) -> bool {
    if v.editor.tool != Tool::Select { return false; }
    let Some((node_id, ty, params)) = selected_node(v) else { return false; };
    let (grips, _) = handles_for_node(node_id, ty, params);
    for grip in &grips {
        if let Some(pos) = project(v.camera, rect, grip.world_pos) {
            let hit_radius = if grip.action == GripAction::TubeCapsToggle { 22.0 } else { 9.0 };
            if (pointer - pos).length() <= hit_radius {
                return true;
            }
        }
    }
    false
}

pub fn compute_edits_for_drag(
    drag: &PrimDrag,
    cur_pointer: Pos2,
    shift: bool,
) -> Vec<(&'static str, Value)> {
    let delta = cur_pointer - drag.start_pointer;
    let dir = if drag.screen_dir.length_sq() > 1e-4 { drag.screen_dir.normalized() } else { Vec2::X };
    let pixels = delta.dot(dir);
    let world_delta = (pixels / drag.span.max(5.0)) as f64;

    let mut out = Vec::new();
    match drag.action {
        GripAction::BoxFace { axis, .. } => {
            let start_size = get_v3(&drag.start_params, "size", [1.0, 1.0, 1.0]);
            let start_center = get_v3(&drag.start_params, "center", [0.0, 0.0, 0.0]);
            let us = get_f64(&drag.start_params, "uniform_scale", 1.0);
            let local_delta = world_delta / us.max(1e-4);

            if shift {
                let new_size_val = (start_size[axis] + local_delta).max(0.0);
                let actual_delta = new_size_val - start_size[axis];
                let mut new_size = start_size;
                new_size[axis] = new_size_val;
                let mut new_center = start_center;
                for (k, c) in new_center.iter_mut().enumerate() {
                    *c += drag.world_dir[k] as f64 * (actual_delta * us * 0.5);
                }
                out.push(("size", json!(new_size)));
                out.push(("center", json!(new_center)));
            } else {
                let new_size_val = (start_size[axis] + 2.0 * local_delta).max(0.0);
                let mut new_size = start_size;
                new_size[axis] = new_size_val;
                out.push(("size", json!(new_size)));
            }
        }
        GripAction::BoxCorner => {
            let start_us = get_f64(&drag.start_params, "uniform_scale", 1.0);
            let factor = ((drag.span + pixels) / drag.span.max(10.0)).max(0.01) as f64;
            let new_us = (start_us * factor).max(0.0);
            out.push(("uniform_scale", json!(new_us)));
        }
        GripAction::SphereRadius => {
            let start_r = get_f64(&drag.start_params, "radius", 1.0);
            let us = get_f64(&drag.start_params, "uniform_scale", 1.0);
            let scale = get_v3(&drag.start_params, "scale", [1.0, 1.0, 1.0]);
            let local_delta = world_delta / (scale[0] * us).max(1e-4);
            let new_r = (start_r + local_delta).max(0.0);
            out.push(("radius", json!(new_r)));
        }
        GripAction::IntStep { param, min, max, step_scale } => {
            let start_val = get_i64(&drag.start_params, param, min);
            let steps = (pixels / (10.0 * step_scale)).round() as i64;
            let new_val = (start_val + steps).clamp(min, max);
            out.push((param, json!(new_val)));
        }
        GripAction::TubeRadiusTop => {
            let start_r = get_f64(&drag.start_params, "radius_top", 1.0);
            let us = get_f64(&drag.start_params, "uniform_scale", 1.0);
            let scale = get_v3(&drag.start_params, "scale", [1.0, 1.0, 1.0]);
            let local_delta = world_delta / (scale[0] * us).max(1e-4);
            let new_r = (start_r + local_delta).max(0.0);
            out.push(("radius_top", json!(new_r)));
        }
        GripAction::TubeRadiusBottom => {
            let start_r = get_f64(&drag.start_params, "radius_bottom", 1.0);
            let us = get_f64(&drag.start_params, "uniform_scale", 1.0);
            let scale = get_v3(&drag.start_params, "scale", [1.0, 1.0, 1.0]);
            let local_delta = world_delta / (scale[0] * us).max(1e-4);
            let new_r = (start_r + local_delta).max(0.0);
            out.push(("radius_bottom", json!(new_r)));
        }
        GripAction::TubeHeight => {
            let start_h = get_f64(&drag.start_params, "height", 2.0);
            let us = get_f64(&drag.start_params, "uniform_scale", 1.0);
            let scale = get_v3(&drag.start_params, "scale", [1.0, 1.0, 1.0]);
            let local_delta = world_delta / (scale[2] * us).max(1e-4);
            let new_h = (start_h + 2.0 * local_delta).max(0.0);
            out.push(("height", json!(new_h)));
        }
        GripAction::TubeCapsToggle => {}
        GripAction::TorusRadius => {
            let start_r = get_f64(&drag.start_params, "radius", 1.0);
            let us = get_f64(&drag.start_params, "uniform_scale", 1.0);
            let scale = get_v3(&drag.start_params, "scale", [1.0, 1.0, 1.0]);
            let local_delta = world_delta / (scale[0] * us).max(1e-4);
            let new_r = (start_r + local_delta).max(0.0);
            out.push(("radius", json!(new_r)));
        }
        GripAction::TorusThickness => {
            let start_t = get_f64(&drag.start_params, "thickness", 0.25);
            let us = get_f64(&drag.start_params, "uniform_scale", 1.0);
            let scale = get_v3(&drag.start_params, "scale", [1.0, 1.0, 1.0]);
            let local_delta = world_delta / (scale[2] * us).max(1e-4);
            let new_t = (start_t + local_delta).max(0.0);
            out.push(("thickness", json!(new_t)));
        }
        GripAction::GridSize { axis } => {
            let param_name = if axis == 0 { "size_x" } else { "size_y" };
            let start_s = get_f64(&drag.start_params, param_name, 10.0);
            let us = get_f64(&drag.start_params, "uniform_scale", 1.0);
            let scale = get_v3(&drag.start_params, "scale", [1.0, 1.0, 1.0]);
            let local_delta = world_delta / (scale[axis] * us).max(1e-4);
            let new_s = (start_s + 2.0 * local_delta).max(0.0);
            out.push((param_name, json!(new_s)));
        }
        GripAction::CircleRadius => {
            let start_r = get_f64(&drag.start_params, "radius", 1.0);
            let new_r = (start_r + world_delta).max(0.0);
            out.push(("radius", json!(new_r)));
        }
        GripAction::LineLength => {
            let start_l = get_f64(&drag.start_params, "length", 1.0);
            let new_l = (start_l + world_delta).max(0.0);
            out.push(("length", json!(new_l)));
        }
        GripAction::CurvePoint(index) => {
            let default="0,0,0  1,0,0  1,1,0  2,1,1";
            if let Some(mut points)=parse_curve_points(drag.start_params["points"].as_str().unwrap_or(default)) {
                if let Some(base)=points.get(index).copied() {
                    let plane=|pointer| {
                        let (origin,dir)=ray(drag.camera,drag.rect,pointer);
                        let n=drag.camera.ray_forward;
                        let denom=(0..3).map(|k|dir[k]*n[k]).sum::<f32>() as f64;
                        if denom.abs()<1e-8 {return None;}
                        let t=(0..3).map(|k|(base[k]-origin[k] as f64)*n[k] as f64).sum::<f64>()/denom;
                        Some([0,1,2].map(|k|origin[k] as f64+dir[k] as f64*t))
                    };
                    if let (Some(start),Some(end))=(plane(drag.start_pointer),plane(cur_pointer)) {
                        let precision=if shift {0.1}else{1.};
                        points[index]=[0,1,2].map(|k|base[k]+(end[k]-start[k])*precision);
                        out.push(("points",json!(write_curve_points(&points))));
                    }
                }
            }
        }
        GripAction::HelixValue{param,min} => {
            let default=match param {"radius"=>1.,"radius_end"=>get_f64(&drag.start_params,"radius",1.),"height"=>2.,"turns"=>3.,_=>0.};
            let start=get_f64(&drag.start_params,param,default);
            let start=if param=="radius_end"&&start<0. {default}else{start};
            out.push((param,json!((start+world_delta).max(min))));
        }
    }
    out
}
