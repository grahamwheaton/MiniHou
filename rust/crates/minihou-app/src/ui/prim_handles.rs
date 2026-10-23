//! Viewport handles on raw primitive nodes (box, sphere, tube, torus, grid, circle, line).
//! Pure maths + drawing: computes grips in local space, transforms by node transform params,
//! projects to screen using camera, and edits node parameters via UiAction::Edit.

use super::{Actions, CameraInfo, Tool, UiAction, View};
use eframe::egui::{self, pos2, Align2, Color32, FontId, Id, PointerButton, Pos2, Rect, Stroke, StrokeKind, Vec2};
use minihou_app::Edit;
use serde_json::{json, Value};

pub fn project(camera: CameraInfo, rect: Rect, point: [f32; 3]) -> Option<Pos2> {
    let m = camera.projection;
    let q = [point[0], point[1], point[2], 1.];
    let clip = [0, 1, 2, 3].map(|row| (0..4).map(|col| m[col][row] * q[col]).sum::<f32>());
    if clip[3] <= 1e-5 { return None; }
    let aspect = rect.width() / rect.height().max(1.);
    Some(pos2(
        rect.center().x + clip[0] / clip[3] / aspect * rect.width() * 0.5,
        rect.center().y - clip[1] / clip[3] * rect.height() * 0.5,
    ))
}

pub fn ray(camera: CameraInfo, rect: Rect, pos: Pos2) -> ([f32; 3], [f32; 3]) {
    let x = (pos.x - rect.center().x) * 2. / rect.width().max(1.);
    let y = (rect.center().y - pos.y) * 2. / rect.height().max(1.);
    let aspect = rect.width() / rect.height().max(1.);
    let combine = |base: [f32; 3]| [0, 1, 2].map(|k| base[k] + x * aspect * camera.ray_right[k] + y * camera.ray_up[k]);
    let norm = |a: [f32; 3]| {
        let n = (a[0]*a[0] + a[1]*a[1] + a[2]*a[2]).sqrt().max(1e-8);
        a.map(|v| v / n)
    };
    if camera.ortho {
        (combine(camera.ray_origin), camera.ray_forward)
    } else {
        (camera.ray_origin, norm(combine(camera.ray_forward)))
    }
}

pub fn local_to_world(
    p_local: [f64; 3],
    center: [f64; 3],
    rotate: [f64; 3],
    scale: [f64; 3],
    uniform_scale: f64,
) -> [f32; 3] {
    let r = minihou_core::math::euler_xyz(rotate);
    let scaled = [
        p_local[0] * scale[0] * uniform_scale,
        p_local[1] * scale[1] * uniform_scale,
        p_local[2] * scale[2] * uniform_scale,
    ];
    let rotated = minihou_core::math::apply(&r, scaled);
    [
        (rotated[0] + center[0]) as f32,
        (rotated[1] + center[1]) as f32,
        (rotated[2] + center[2]) as f32,
    ]
}

pub fn rotate_dir(dir_local: [f64; 3], rotate: [f64; 3]) -> [f32; 3] {
    let r = minihou_core::math::euler_xyz(rotate);
    let rot = minihou_core::math::apply(&r, dir_local);
    let n = (rot[0]*rot[0] + rot[1]*rot[1] + rot[2]*rot[2]).sqrt().max(1e-8);
    [
        (rot[0] / n) as f32,
        (rot[1] / n) as f32,
        (rot[2] / n) as f32,
    ]
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GripShape {
    Square,
    Circle,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GripAction {
    BoxFace { axis: usize, sign: f64 },
    BoxCorner,
    SphereRadius,
    IntStep { param: &'static str, default: i64, min: i64, max: i64, step_scale: f32 },
    TubeRadiusTop,
    TubeRadiusBottom,
    TubeHeight,
    TubeCapsToggle,
    TorusRadius,
    TorusThickness,
    GridSize { axis: usize },
    CircleRadius,
    LineLength,
    CurvePoint(usize),
    HelixValue { param: &'static str, min: f64 },
}

#[derive(Clone, Debug)]
pub struct Grip {
    pub id: String,
    pub world_pos: [f32; 3],
    pub world_dir: [f32; 3],
    pub shape: GripShape,
    pub action: GripAction,
    pub label: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SpinnerRing {
    pub center: [f32; 3],
    pub radius: f32,
    pub normal: [f32; 3],
    pub label: String,
}

fn get_f64(params: &Value, key: &str, default: f64) -> f64 {
    params.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
}

fn get_i64(params: &Value, key: &str, default: i64) -> i64 {
    params.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
}

fn get_bool(params: &Value, key: &str, default: bool) -> bool {
    params.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

fn get_v3(params: &Value, key: &str, default: [f64; 3]) -> [f64; 3] {
    if let Some(arr) = params.get(key).and_then(|v| v.as_array()) {
        [
            arr.first().and_then(|x| x.as_f64()).unwrap_or(default[0]),
            arr.get(1).and_then(|x| x.as_f64()).unwrap_or(default[1]),
            arr.get(2).and_then(|x| x.as_f64()).unwrap_or(default[2]),
        ]
    } else {
        default
    }
}

pub fn parse_curve_points(text:&str)->Option<Vec<[f64;3]>> {
    let points:Option<Vec<_>>=text.split_whitespace().map(|token| {
        let parts:Vec<_>=token.split(',').collect();if parts.len()!=3 {return None;}
        let xyz:Option<Vec<f64>>=parts.iter().map(|s|s.parse::<f64>().ok().filter(|x|x.is_finite())).collect();
        let xyz=xyz?;Some([xyz[0],xyz[1],xyz[2]])
    }).collect();
    points.filter(|p|p.len()>=2)
}
pub fn write_curve_points(points:&[[f64;3]])->String {
    points.iter().map(|p|format!("{},{},{}",p[0],p[1],p[2])).collect::<Vec<_>>().join(" ")
}

mod generator;
pub use generator::handles_for_node;

mod drag;
pub use drag::{PrimDrag, compute_edits_for_drag, active, finish_input, hit};

mod view;
pub use view::show;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn curve_points_round_trip_and_helix_grips() {
        let text="0,0,0 1.25,2,-3";
        let points=parse_curve_points(text).unwrap();
        assert_eq!(parse_curve_points(&write_curve_points(&points)),Some(points));
        let (grips,_)=handles_for_node("curve1","curve",&json!({"points":text}));
        assert_eq!(grips.len(),2);
        assert_eq!(grips[1].action,GripAction::CurvePoint(1));
        let (grips,_)=handles_for_node("helix1","helix",&json!({"radius":2.,"radius_end":1.,"height":4.,"turns":3.}));
        assert_eq!(grips.len(),4);
        assert!(grips.iter().any(|g|g.action==GripAction::HelixValue{param:"radius_end",min:0.}));
    }
    #[test] fn curve_point_drag_writes_xyz_text() {
        let drag=PrimDrag{node:"curve1".into(),start_pointer:pos2(100.,100.),screen_dir:Vec2::X,span:100.,
            start_params:json!({"points":"0,0,0 1,0,0"}),action:GripAction::CurvePoint(1),world_dir:[1.,0.,0.],
            camera:CameraInfo{ortho:true,ray_origin:[0.,0.,1.],ray_forward:[0.,0.,-1.],ray_right:[1.,0.,0.],ray_up:[0.,1.,0.],..Default::default()},
            rect:Rect::from_min_size(Pos2::ZERO,Vec2::splat(200.))};
        let edits=compute_edits_for_drag(&drag,pos2(150.,100.),false);
        assert_eq!(edits.len(),1);
        let points=parse_curve_points(edits[0].1.as_str().unwrap()).unwrap();
        assert_eq!(points[1],[1.5,0.,0.]);
    }

    #[test]
    fn test_box_face_drag_symmetric() {
        let start_params = json!({
            "size": [2.0, 4.0, 6.0],
            "center": [0.0, 0.0, 0.0],
            "uniform_scale": 1.0,
        });
        let drag = PrimDrag {
            node: "box1".into(),
            start_pointer: Pos2::new(100.0, 100.0),
            screen_dir: Vec2::new(50.0, 0.0),
            span: 50.0,
            start_params,
            action: GripAction::BoxFace { axis: 0, sign: 1.0 },
            world_dir: [1.0, 0.0, 0.0],
            camera: CameraInfo::default(),
            rect: Rect::NOTHING,
        };
        // Move pointer 50 pixels to the right along screen_dir: world_delta = 1.0
        // Symmetric mode (shift = false): size expands by 2 * delta = 2.0 -> size_x = 4.0
        let edits = compute_edits_for_drag(&drag, Pos2::new(150.0, 100.0), false);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].0, "size");
        let size = edits[0].1.as_array().unwrap();
        assert!((size[0].as_f64().unwrap() - 4.0).abs() < 1e-4);
        assert!((size[1].as_f64().unwrap() - 4.0).abs() < 1e-4);
    }

    #[test]
    fn test_box_face_drag_shift_opposite_fixed() {
        let start_params = json!({
            "size": [2.0, 4.0, 6.0],
            "center": [0.0, 0.0, 0.0],
            "uniform_scale": 1.0,
        });
        let drag = PrimDrag {
            node: "box1".into(),
            start_pointer: Pos2::new(100.0, 100.0),
            screen_dir: Vec2::new(50.0, 0.0),
            span: 50.0,
            start_params,
            action: GripAction::BoxFace { axis: 0, sign: 1.0 },
            world_dir: [1.0, 0.0, 0.0],
            camera: CameraInfo::default(),
            rect: Rect::NOTHING,
        };
        // Move pointer 50 pixels to the right with shift = true
        // size_x = start + 1.0 = 3.0, center moves by delta/2 = +0.5
        let edits = compute_edits_for_drag(&drag, Pos2::new(150.0, 100.0), true);
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0].0, "size");
        assert_eq!(edits[1].0, "center");
        let size = edits[0].1.as_array().unwrap();
        let center = edits[1].1.as_array().unwrap();
        assert!((size[0].as_f64().unwrap() - 3.0).abs() < 1e-4);
        assert!((center[0].as_f64().unwrap() - 0.5).abs() < 1e-4);
    }

    #[test]
    fn test_box_corner_scale_uniform() {
        let start_params = json!({
            "size": [2.0, 2.0, 2.0],
            "uniform_scale": 1.0,
        });
        let drag = PrimDrag {
            node: "box1".into(),
            start_pointer: Pos2::new(100.0, 100.0),
            screen_dir: Vec2::new(50.0, 50.0),
            span: 50.0,
            start_params,
            action: GripAction::BoxCorner,
            world_dir: [1.0, 1.0, 1.0],
            camera: CameraInfo::default(),
            rect: Rect::NOTHING,
        };
        // Move pointer 50px along span: factor = (50 + 50) / 50 = 2.0
        let edits = compute_edits_for_drag(&drag, Pos2::new(100.0 + 35.355, 100.0 + 35.355), false);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].0, "uniform_scale");
        let us = edits[0].1.as_f64().unwrap();
        assert!((us - 2.0).abs() < 0.05);
    }

    #[test]
    fn test_sphere_radius_and_ring() {
        let start_params = json!({
            "radius": 1.5,
            "rows": 16,
            "columns": 32,
            "uniform_scale": 1.0,
            "scale": [1.0, 1.0, 1.0],
        });
        let rad_drag = PrimDrag {
            node: "sphere1".into(),
            start_pointer: Pos2::new(100.0, 100.0),
            screen_dir: Vec2::new(40.0, 0.0),
            span: 40.0,
            start_params: start_params.clone(),
            action: GripAction::SphereRadius,
            world_dir: [1.0, 0.0, 0.0],
            camera: CameraInfo::default(),
            rect: Rect::NOTHING,
        };
        // Drag radius by +40px: delta = 1.0 -> radius = 2.5
        let edits = compute_edits_for_drag(&rad_drag, Pos2::new(140.0, 100.0), false);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].0, "radius");
        assert!((edits[0].1.as_f64().unwrap() - 2.5).abs() < 1e-4);

        // Spinner columns grip
        let col_drag = PrimDrag {
            node: "sphere1".into(),
            start_pointer: Pos2::new(100.0, 100.0),
            screen_dir: Vec2::new(0.0, 10.0),
            span: 10.0,
            start_params,
            action: GripAction::IntStep { param: "columns", default: 32, min: 3, max: 4096, step_scale: 1.0 },
            world_dir: [0.0, 1.0, 0.0],
            camera: CameraInfo::default(),
            rect: Rect::NOTHING,
        };
        // Drag columns by 20px -> +2 steps -> columns = 34
        let edits = compute_edits_for_drag(&col_drag, Pos2::new(100.0, 120.0), false);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].0, "columns");
        assert_eq!(edits[0].1.as_i64().unwrap(), 34);
    }

    #[test]
    fn primitive_count_drags_start_at_displayed_defaults() {
        for (node_type, rows, columns) in [
            ("sphere", 16, 32),
            ("tube", 1, 24),
            ("torus", 12, 32),
        ] {
            let params = json!({});
            let (grips, ring) = handles_for_node("primitive", node_type, &params);
            assert_eq!(ring.unwrap().label, format!("{rows} x {columns}"));
            for (param, start) in [("rows", rows), ("columns", columns)] {
                let grip = grips.iter().find(|grip| matches!(grip.action,
                    GripAction::IntStep { param: name, .. } if name == param)).unwrap();
                let GripAction::IntStep { min, .. } = &grip.action else { unreachable!() };
                let drag = PrimDrag {
                    node: "primitive".into(),
                    start_pointer: pos2(100.0, 100.0),
                    screen_dir: Vec2::X,
                    span: 10.0,
                    start_params: params.clone(),
                    action: grip.action.clone(),
                    world_dir: grip.world_dir,
                    camera: CameraInfo::default(),
                    rect: Rect::NOTHING,
                };
                for (delta, expected) in [(0.0, start), (20.0, start + 2), (-20.0, (start - 2).max(*min))] {
                    let edits = compute_edits_for_drag(&drag, pos2(100.0 + delta, 100.0), false);
                    assert_eq!(edits, vec![(param, json!(expected))], "{node_type} {param} at {delta}px");
                }
            }
        }
    }

    #[test]
    fn test_tube_radii_height() {
        let start_params = json!({
            "radius_top": 1.0,
            "radius_bottom": 1.5,
            "height": 3.0,
            "uniform_scale": 1.0,
            "scale": [1.0, 1.0, 1.0],
        });
        let h_drag = PrimDrag {
            node: "tube1".into(),
            start_pointer: Pos2::new(100.0, 100.0),
            screen_dir: Vec2::new(0.0, -50.0),
            span: 50.0,
            start_params,
            action: GripAction::TubeHeight,
            world_dir: [0.0, 0.0, 1.0],
            camera: CameraInfo::default(),
            rect: Rect::NOTHING,
        };
        // Drag height upward by 50px -> delta = 1.0 -> new height = 3.0 + 2*1.0 = 5.0
        let edits = compute_edits_for_drag(&h_drag, Pos2::new(100.0, 50.0), false);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].0, "height");
        assert!((edits[0].1.as_f64().unwrap() - 5.0).abs() < 1e-4);
    }

    #[test]
    fn test_torus_major_minor_radii() {
        let start_params = json!({
            "radius": 2.0,
            "thickness": 0.5,
            "uniform_scale": 1.0,
            "scale": [1.0, 1.0, 1.0],
        });
        let t_drag = PrimDrag {
            node: "torus1".into(),
            start_pointer: Pos2::new(100.0, 100.0),
            screen_dir: Vec2::new(0.0, -50.0),
            span: 50.0,
            start_params,
            action: GripAction::TorusThickness,
            world_dir: [0.0, 0.0, 1.0],
            camera: CameraInfo::default(),
            rect: Rect::NOTHING,
        };
        let edits = compute_edits_for_drag(&t_drag, Pos2::new(100.0, 50.0), false);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].0, "thickness");
        assert!((edits[0].1.as_f64().unwrap() - 1.5).abs() < 1e-4);
    }

    #[test]
    fn test_grid_size_and_divisions() {
        let start_params = json!({
            "size_x": 10.0,
            "size_y": 8.0,
            "columns": 5,
            "rows": 4,
            "uniform_scale": 1.0,
            "scale": [1.0, 1.0, 1.0],
        });
        let sx_drag = PrimDrag {
            node: "grid1".into(),
            start_pointer: Pos2::new(100.0, 100.0),
            screen_dir: Vec2::new(50.0, 0.0),
            span: 50.0,
            start_params,
            action: GripAction::GridSize { axis: 0 },
            world_dir: [1.0, 0.0, 0.0],
            camera: CameraInfo::default(),
            rect: Rect::NOTHING,
        };
        let edits = compute_edits_for_drag(&sx_drag, Pos2::new(150.0, 100.0), false);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].0, "size_x");
        assert!((edits[0].1.as_f64().unwrap() - 12.0).abs() < 1e-4);
    }

    #[test]
    fn test_circle_radius_and_divisions() {
        let start_params = json!({
            "radius": 2.0,
            "divisions": 32,
        });
        let rad_drag = PrimDrag {
            node: "circle1".into(),
            start_pointer: Pos2::new(100.0, 100.0),
            screen_dir: Vec2::new(50.0, 0.0),
            span: 50.0,
            start_params,
            action: GripAction::CircleRadius,
            world_dir: [1.0, 0.0, 0.0],
            camera: CameraInfo::default(),
            rect: Rect::NOTHING,
        };
        let edits = compute_edits_for_drag(&rad_drag, Pos2::new(150.0, 100.0), false);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].0, "radius");
        assert!((edits[0].1.as_f64().unwrap() - 3.0).abs() < 1e-4);
    }

    #[test]
    fn test_line_length_and_points() {
        let start_params = json!({
            "length": 5.0,
            "points": 10,
        });
        let len_drag = PrimDrag {
            node: "line1".into(),
            start_pointer: Pos2::new(100.0, 100.0),
            screen_dir: Vec2::new(0.0, 50.0),
            span: 50.0,
            start_params,
            action: GripAction::LineLength,
            world_dir: [0.0, 0.0, 1.0],
            camera: CameraInfo::default(),
            rect: Rect::NOTHING,
        };
        let edits = compute_edits_for_drag(&len_drag, Pos2::new(100.0, 150.0), false);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].0, "length");
        assert!((edits[0].1.as_f64().unwrap() - 6.0).abs() < 1e-4);
    }

    #[test]
    fn test_prim_handles_headless_frame() {
        use crate::ui::theme::Theme;
        use crate::ui::EditorState;
        use minihou_app::CookState;
        use minihou_core::Graph;
        use std::collections::HashMap;

        let demo_text = r#"{
            "nodes": {
                "box1": { "type": "box", "params": { "size": [2.0, 2.0, 2.0] } },
                "sphere1": { "type": "sphere", "params": { "radius": 1.0 } }
            },
            "display": "box1"
        }"#;
        let doc: Value = serde_json::from_str(demo_text).unwrap();
        let graph = Graph::from_value(&doc).unwrap();
        let editor = EditorState { tool: Tool::Select, ..Default::default() };
        let cooking = CookState::default();
        let theme = Theme::dark();
        let blender = crate::BlenderStatus::default();
        let flags = HashMap::new();
        let camera = CameraInfo {
            projection: [[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]],
            ray_origin: [0.,0.,5.],
            ray_forward: [0.,0.,-1.],
            ray_up: [0.,1.,0.],
            ray_right: [1.,0.,0.],
            ortho: true,
            ..Default::default()
        };
        let selected_set = std::collections::BTreeSet::from(["box1".to_owned()]);
        let view = View {
            graph: &graph,
            doc: &doc,
            path: std::path::Path::new("test.json"),
            dirty: false,
            can_undo: false,
            can_redo: false,
            selected: "box1",
            selected_set: &selected_set,
            display: "box1",
            cooking: &cooking,
            file_status: "",
            mode: crate::viewport::Mode::Shaded,
            theme: &theme,
            editor: &editor,
            camera,
            bounds: ([0.; 3], 1.0),
            selection_center: None,
            normal_frame: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            geo: None,
            blender: &blender,
            flags: &flags,
            units: crate::units::UnitSettings::default(),
            repeat_label: None,
        };

        let ctx = egui::Context::default();
        let mut actions = Actions::default();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));

        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let hit = show(ui, rect, &view, &mut actions);
                assert!(!hit); // No mouse input yet, so idle
            });
        });
        assert!(actions.0.is_empty());
    }
}
