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
    IntStep { param: &'static str, min: i64, max: i64, step_scale: f32 },
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

pub fn handles_for_node(node_id: &str, node_type: &str, params: &Value) -> (Vec<Grip>, Option<SpinnerRing>) {
    let mut grips = Vec::new();
    let mut ring = None;

    match node_type {
        "box" => {
            let size = get_v3(params, "size", [1.0, 1.0, 1.0]);
            let center = get_v3(params, "center", [0.0, 0.0, 0.0]);
            let rotate = get_v3(params, "rotate", [0.0, 0.0, 0.0]);
            let uniform_scale = get_f64(params, "uniform_scale", 1.0);
            let scale = [1.0, 1.0, 1.0];

            let face_defs = [
                (0, 1.0, [size[0] * 0.5, 0.0, 0.0], [1.0, 0.0, 0.0], "+X"),
                (0, -1.0, [-size[0] * 0.5, 0.0, 0.0], [-1.0, 0.0, 0.0], "-X"),
                (1, 1.0, [0.0, size[1] * 0.5, 0.0], [0.0, 1.0, 0.0], "+Y"),
                (1, -1.0, [0.0, -size[1] * 0.5, 0.0], [0.0, -1.0, 0.0], "-Y"),
                (2, 1.0, [0.0, 0.0, size[2] * 0.5], [0.0, 0.0, 1.0], "+Z"),
                (2, -1.0, [0.0, 0.0, -size[2] * 0.5], [0.0, 0.0, -1.0], "-Z"),
            ];

            for (axis, sign, local_p, local_d, name) in face_defs {
                let world_pos = local_to_world(local_p, center, rotate, scale, uniform_scale);
                let world_dir = rotate_dir(local_d, rotate);
                grips.push(Grip {
                    id: format!("{node_id}_box_face_{name}"),
                    world_pos,
                    world_dir,
                    shape: GripShape::Square,
                    action: GripAction::BoxFace { axis, sign },
                    label: Some(format!("{}: {:.2}", ["X", "Y", "Z"][axis], size[axis])),
                });
            }

            let corner_p = [size[0] * 0.5, size[1] * 0.5, size[2] * 0.5];
            let corner_d = [1.0, 1.0, 1.0];
            let world_pos = local_to_world(corner_p, center, rotate, scale, uniform_scale);
            let world_dir = rotate_dir(corner_d, rotate);
            grips.push(Grip {
                id: format!("{node_id}_box_corner"),
                world_pos,
                world_dir,
                shape: GripShape::Circle,
                action: GripAction::BoxCorner,
                label: Some(format!("Scale: {uniform_scale:.2}")),
            });
        }
        "sphere" => {
            let radius = get_f64(params, "radius", 1.0);
            let rows = get_i64(params, "rows", 16);
            let columns = get_i64(params, "columns", 32);
            let center = get_v3(params, "center", [0.0, 0.0, 0.0]);
            let rotate = get_v3(params, "rotate", [0.0, 0.0, 0.0]);
            let scale = get_v3(params, "scale", [1.0, 1.0, 1.0]);
            let uniform_scale = get_f64(params, "uniform_scale", 1.0);

            // Equator radius grip
            let world_pos = local_to_world([radius, 0.0, 0.0], center, rotate, scale, uniform_scale);
            let world_dir = rotate_dir([1.0, 0.0, 0.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_sphere_radius"),
                world_pos,
                world_dir,
                shape: GripShape::Circle,
                action: GripAction::SphereRadius,
                label: Some(format!("Radius: {radius:.2}")),
            });

            // Spinner ring above
            let ring_z = radius * 1.25;
            let ring_r = (radius * 0.4).max(0.2);
            let ring_center = local_to_world([0.0, 0.0, ring_z], center, rotate, scale, uniform_scale);
            let ring_normal = rotate_dir([0.0, 0.0, 1.0], rotate);
            ring = Some(SpinnerRing {
                center: ring_center,
                radius: (ring_r * scale[0] * uniform_scale) as f32,
                normal: ring_normal,
                label: format!("{rows} x {columns}"),
            });

            let col_pos = local_to_world([ring_r, 0.0, ring_z], center, rotate, scale, uniform_scale);
            let col_dir = rotate_dir([0.0, 1.0, 0.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_sphere_cols"),
                world_pos: col_pos,
                world_dir: col_dir,
                shape: GripShape::Circle,
                action: GripAction::IntStep { param: "columns", min: 3, max: 4096, step_scale: 1.0 },
                label: Some(format!("Cols: {columns}")),
            });

            let row_pos = local_to_world([0.0, ring_r, ring_z], center, rotate, scale, uniform_scale);
            let row_dir = rotate_dir([-1.0, 0.0, 0.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_sphere_rows"),
                world_pos: row_pos,
                world_dir: row_dir,
                shape: GripShape::Circle,
                action: GripAction::IntStep { param: "rows", min: 2, max: 4096, step_scale: 1.0 },
                label: Some(format!("Rows: {rows}")),
            });
        }
        "tube" => {
            let radius_bottom = get_f64(params, "radius_bottom", 1.0);
            let radius_top = get_f64(params, "radius_top", 1.0);
            let height = get_f64(params, "height", 2.0);
            let rows = get_i64(params, "rows", 1);
            let columns = get_i64(params, "columns", 24);
            let caps = get_bool(params, "caps", true);
            let center = get_v3(params, "center", [0.0, 0.0, 0.0]);
            let rotate = get_v3(params, "rotate", [0.0, 0.0, 0.0]);
            let scale = get_v3(params, "scale", [1.0, 1.0, 1.0]);
            let uniform_scale = get_f64(params, "uniform_scale", 1.0);

            // Top radius
            let top_pos = local_to_world([radius_top, 0.0, height * 0.5], center, rotate, scale, uniform_scale);
            let top_dir = rotate_dir([1.0, 0.0, 0.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_tube_radius_top"),
                world_pos: top_pos,
                world_dir: top_dir,
                shape: GripShape::Circle,
                action: GripAction::TubeRadiusTop,
                label: Some(format!("R top: {radius_top:.2}")),
            });

            // Bottom radius
            let bot_pos = local_to_world([radius_bottom, 0.0, -height * 0.5], center, rotate, scale, uniform_scale);
            let bot_dir = rotate_dir([1.0, 0.0, 0.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_tube_radius_bottom"),
                world_pos: bot_pos,
                world_dir: bot_dir,
                shape: GripShape::Circle,
                action: GripAction::TubeRadiusBottom,
                label: Some(format!("R bot: {radius_bottom:.2}")),
            });

            // Height
            let height_pos = local_to_world([0.0, 0.0, height * 0.5], center, rotate, scale, uniform_scale);
            let height_dir = rotate_dir([0.0, 0.0, 1.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_tube_height"),
                world_pos: height_pos,
                world_dir: height_dir,
                shape: GripShape::Square,
                action: GripAction::TubeHeight,
                label: Some(format!("Height: {height:.2}")),
            });

            // Caps toggle button near top
            let caps_pos = local_to_world([-radius_top * 0.5, 0.0, height * 0.5], center, rotate, scale, uniform_scale);
            grips.push(Grip {
                id: format!("{node_id}_tube_caps"),
                world_pos: caps_pos,
                world_dir: rotate_dir([0.0, 0.0, 1.0], rotate),
                shape: GripShape::Square,
                action: GripAction::TubeCapsToggle,
                label: Some(if caps { "Caps: On".into() } else { "Caps: Off".into() }),
            });

            // Spinner ring above
            let ring_z = height * 0.5 + (height * 0.25).max(0.3);
            let ring_r = (radius_top * 0.4).max(0.2);
            let ring_center = local_to_world([0.0, 0.0, ring_z], center, rotate, scale, uniform_scale);
            let ring_normal = rotate_dir([0.0, 0.0, 1.0], rotate);
            ring = Some(SpinnerRing {
                center: ring_center,
                radius: (ring_r * scale[0] * uniform_scale) as f32,
                normal: ring_normal,
                label: format!("{rows} x {columns}"),
            });

            let col_pos = local_to_world([ring_r, 0.0, ring_z], center, rotate, scale, uniform_scale);
            let col_dir = rotate_dir([0.0, 1.0, 0.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_tube_cols"),
                world_pos: col_pos,
                world_dir: col_dir,
                shape: GripShape::Circle,
                action: GripAction::IntStep { param: "columns", min: 3, max: 4096, step_scale: 1.0 },
                label: Some(format!("Cols: {columns}")),
            });

            let row_pos = local_to_world([0.0, ring_r, ring_z], center, rotate, scale, uniform_scale);
            let row_dir = rotate_dir([-1.0, 0.0, 0.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_tube_rows"),
                world_pos: row_pos,
                world_dir: row_dir,
                shape: GripShape::Circle,
                action: GripAction::IntStep { param: "rows", min: 1, max: 4096, step_scale: 1.0 },
                label: Some(format!("Rows: {rows}")),
            });
        }
        "torus" => {
            let radius = get_f64(params, "radius", 1.0);
            let thickness = get_f64(params, "thickness", 0.25);
            let rows = get_i64(params, "rows", 12);
            let columns = get_i64(params, "columns", 32);
            let center = get_v3(params, "center", [0.0, 0.0, 0.0]);
            let rotate = get_v3(params, "rotate", [0.0, 0.0, 0.0]);
            let scale = get_v3(params, "scale", [1.0, 1.0, 1.0]);
            let uniform_scale = get_f64(params, "uniform_scale", 1.0);

            // Major radius (outer ring grip)
            let major_pos = local_to_world([radius + thickness, 0.0, 0.0], center, rotate, scale, uniform_scale);
            let major_dir = rotate_dir([1.0, 0.0, 0.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_torus_radius"),
                world_pos: major_pos,
                world_dir: major_dir,
                shape: GripShape::Circle,
                action: GripAction::TorusRadius,
                label: Some(format!("Major R: {radius:.2}")),
            });

            // Minor radius (tube grip)
            let minor_pos = local_to_world([radius, 0.0, thickness], center, rotate, scale, uniform_scale);
            let minor_dir = rotate_dir([0.0, 0.0, 1.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_torus_thickness"),
                world_pos: minor_pos,
                world_dir: minor_dir,
                shape: GripShape::Circle,
                action: GripAction::TorusThickness,
                label: Some(format!("Minor r: {thickness:.2}")),
            });

            // Spinner ring above
            let ring_z = thickness + (radius * 0.35).max(0.3);
            let ring_r = (radius * 0.35).max(0.2);
            let ring_center = local_to_world([0.0, 0.0, ring_z], center, rotate, scale, uniform_scale);
            let ring_normal = rotate_dir([0.0, 0.0, 1.0], rotate);
            ring = Some(SpinnerRing {
                center: ring_center,
                radius: (ring_r * scale[0] * uniform_scale) as f32,
                normal: ring_normal,
                label: format!("{rows} x {columns}"),
            });

            let col_pos = local_to_world([ring_r, 0.0, ring_z], center, rotate, scale, uniform_scale);
            let col_dir = rotate_dir([0.0, 1.0, 0.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_torus_cols"),
                world_pos: col_pos,
                world_dir: col_dir,
                shape: GripShape::Circle,
                action: GripAction::IntStep { param: "columns", min: 3, max: 4096, step_scale: 1.0 },
                label: Some(format!("Cols: {columns}")),
            });

            let row_pos = local_to_world([0.0, ring_r, ring_z], center, rotate, scale, uniform_scale);
            let row_dir = rotate_dir([-1.0, 0.0, 0.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_torus_rows"),
                world_pos: row_pos,
                world_dir: row_dir,
                shape: GripShape::Circle,
                action: GripAction::IntStep { param: "rows", min: 3, max: 4096, step_scale: 1.0 },
                label: Some(format!("Rows: {rows}")),
            });
        }
        "grid" => {
            let size_x = get_f64(params, "size_x", 10.0);
            let size_y = get_f64(params, "size_y", 10.0);
            let columns = get_i64(params, "columns", 10);
            let rows = get_i64(params, "rows", 10);
            let center = get_v3(params, "center", [0.0, 0.0, 0.0]);
            let rotate = get_v3(params, "rotate", [0.0, 0.0, 0.0]);
            let scale = get_v3(params, "scale", [1.0, 1.0, 1.0]);
            let uniform_scale = get_f64(params, "uniform_scale", 1.0);

            // size_x edge grip
            let sx_pos = local_to_world([size_x * 0.5, 0.0, 0.0], center, rotate, scale, uniform_scale);
            let sx_dir = rotate_dir([1.0, 0.0, 0.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_grid_size_x"),
                world_pos: sx_pos,
                world_dir: sx_dir,
                shape: GripShape::Square,
                action: GripAction::GridSize { axis: 0 },
                label: Some(format!("Size X: {size_x:.1}")),
            });

            // size_y edge grip
            let sy_pos = local_to_world([0.0, size_y * 0.5, 0.0], center, rotate, scale, uniform_scale);
            let sy_dir = rotate_dir([0.0, 1.0, 0.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_grid_size_y"),
                world_pos: sy_pos,
                world_dir: sy_dir,
                shape: GripShape::Square,
                action: GripAction::GridSize { axis: 1 },
                label: Some(format!("Size Y: {size_y:.1}")),
            });

            // columns division grip
            let col_pos = local_to_world([size_x * 0.25, size_y * 0.5, 0.0], center, rotate, scale, uniform_scale);
            let col_dir = rotate_dir([1.0, 0.0, 0.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_grid_cols"),
                world_pos: col_pos,
                world_dir: col_dir,
                shape: GripShape::Circle,
                action: GripAction::IntStep { param: "columns", min: 2, max: 20000, step_scale: 1.0 },
                label: Some(format!("{columns} cols")),
            });

            // rows division grip
            let row_pos = local_to_world([size_x * 0.5, size_y * 0.25, 0.0], center, rotate, scale, uniform_scale);
            let row_dir = rotate_dir([0.0, 1.0, 0.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_grid_rows"),
                world_pos: row_pos,
                world_dir: row_dir,
                shape: GripShape::Circle,
                action: GripAction::IntStep { param: "rows", min: 2, max: 20000, step_scale: 1.0 },
                label: Some(format!("{rows} rows")),
            });
        }
        "circle" => {
            let radius = get_f64(params, "radius", 1.0);
            let divisions = get_i64(params, "divisions", 32);
            let center = get_v3(params, "center", [0.0, 0.0, 0.0]);
            let rotate = get_v3(params, "rotate", [0.0, 0.0, 0.0]);
            let scale = [1.0, 1.0, 1.0];
            let uniform_scale = 1.0;

            // Radius grip
            let rad_pos = local_to_world([radius, 0.0, 0.0], center, rotate, scale, uniform_scale);
            let rad_dir = rotate_dir([1.0, 0.0, 0.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_circle_radius"),
                world_pos: rad_pos,
                world_dir: rad_dir,
                shape: GripShape::Circle,
                action: GripAction::CircleRadius,
                label: Some(format!("Radius: {radius:.2}")),
            });

            // Point-count grip
            let pts_pos = local_to_world([0.0, radius, 0.0], center, rotate, scale, uniform_scale);
            let pts_dir = rotate_dir([-1.0, 0.0, 0.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_circle_divs"),
                world_pos: pts_pos,
                world_dir: pts_dir,
                shape: GripShape::Circle,
                action: GripAction::IntStep { param: "divisions", min: 3, max: 100000, step_scale: 1.0 },
                label: Some(format!("{divisions} pts")),
            });
        }
        "line" => {
            let origin = get_v3(params, "origin", [0.0, 0.0, 0.0]);
            let direction = get_v3(params, "direction", [0.0, 0.0, 1.0]);
            let length = get_f64(params, "length", 1.0);
            let points = get_i64(params, "points", 10);

            let d_len = (direction[0]*direction[0] + direction[1]*direction[1] + direction[2]*direction[2]).sqrt().max(1e-6);
            let norm_dir = [direction[0] / d_len, direction[1] / d_len, direction[2] / d_len];

            // Length grip at end of line
            let end_pos = [
                (origin[0] + norm_dir[0] * length) as f32,
                (origin[1] + norm_dir[1] * length) as f32,
                (origin[2] + norm_dir[2] * length) as f32,
            ];
            let dir_f32 = [norm_dir[0] as f32, norm_dir[1] as f32, norm_dir[2] as f32];
            grips.push(Grip {
                id: format!("{node_id}_line_length"),
                world_pos: end_pos,
                world_dir: dir_f32,
                shape: GripShape::Square,
                action: GripAction::LineLength,
                label: Some(format!("Length: {length:.2}")),
            });

            // Point count grip at mid-point
            let mid_pos = [
                (origin[0] + norm_dir[0] * (length * 0.5)) as f32,
                (origin[1] + norm_dir[1] * (length * 0.5)) as f32,
                (origin[2] + norm_dir[2] * (length * 0.5)) as f32,
            ];
            grips.push(Grip {
                id: format!("{node_id}_line_points"),
                world_pos: mid_pos,
                world_dir: dir_f32,
                shape: GripShape::Circle,
                action: GripAction::IntStep { param: "points", min: 2, max: 1000000, step_scale: 1.0 },
                label: Some(format!("{points} pts")),
            });
        }
        "curve" => {
            let default="0,0,0  1,0,0  1,1,0  2,1,1";
            if let Some(points)=parse_curve_points(params["points"].as_str().unwrap_or(default)) {
                for (index,p) in points.into_iter().enumerate() {
                    grips.push(Grip{id:format!("{node_id}_curve_point_{index}"),world_pos:p.map(|x|x as f32),world_dir:[1.,0.,0.],
                        shape:GripShape::Circle,action:GripAction::CurvePoint(index),label:Some(format!("Point {}",index+1))});
                }
            }
        }
        "helix" => {
            let radius=get_f64(params,"radius",1.);
            let top=get_f64(params,"radius_end",-1.);let top=if top<0.{radius}else{top};
            let height=get_f64(params,"height",2.);let turns=get_f64(params,"turns",3.);
            let angle=turns*std::f64::consts::TAU;
            for (name,param,p,dir,min) in [
                ("Radius","radius",[radius,0.,0.],[1.,0.,0.],0.),
                ("Radius top","radius_end",[top*angle.cos(),top*angle.sin(),height],[angle.cos(),angle.sin(),0.],0.),
                ("Height","height",[0.,0.,height],[0.,0.,1.],0.),
                ("Turns","turns",[top*angle.cos(),top*angle.sin(),height+0.3],[-angle.sin(),angle.cos(),0.],0.01),
            ] {
                grips.push(Grip{id:format!("{node_id}_helix_{param}"),world_pos:p.map(|x|x as f32),world_dir:dir.map(|x|x as f32),
                    shape:GripShape::Square,action:GripAction::HelixValue{param,min},label:Some(format!("{name}: {:.2}",if param=="radius_end" {top}else{get_f64(params,param,if param=="turns" {3.}else if param=="height" {2.}else{1.})}))});
            }
        }
        _ => {}
    }

    (grips, ring)
}

#[derive(Clone, Debug)]
pub struct PrimDrag {
    node: String,
    start_pointer: Pos2,
    screen_dir: Vec2,
    span: f32,
    start_params: Value,
    action: GripAction,
    world_dir: [f32; 3],
    camera: CameraInfo,
    rect: Rect,
}

pub fn active(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<PrimDrag>(Id::new("prim_handle_drag")).is_some())
}

pub fn finish_input(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<PrimDrag>(Id::new("prim_handle_drag")));
}
fn is_displayed(v: &View) -> bool {
    if v.selected.is_empty() { return false; }
    if v.display == v.selected { return true; }
    minihou_app::display_nodes(v.doc).iter().any(|n| n == v.selected)
}

fn selected_node<'a>(v: &'a View) -> Option<(&'a str, &'a str, &'a Value)> {
    if v.selected.is_empty() { return None; }
    let node = v.doc.get("nodes")?.get(v.selected)?;
    let ty = node.get("type")?.as_str()?;
    let params = node.get("params").unwrap_or(&Value::Null);
    Some((v.selected, ty, params))
}

pub fn hit(rect: Rect, v: &View, pointer: Pos2) -> bool {
    if v.editor.tool != Tool::Select || !is_displayed(v) { return false; }
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

pub fn show(ui: &mut egui::Ui, rect: Rect, v: &View, out: &mut Actions) -> bool {
    if v.editor.tool != Tool::Select || !is_displayed(v) { return false; }
    let Some((node_id, ty, params)) = selected_node(v) else { return false; };

    let (grips, ring) = handles_for_node(node_id, ty, params);
    if grips.is_empty() { return false; }

    let p = ui.painter_at(rect);
    let drag_id = Id::new("prim_handle_drag");
    let mut current_drag: Option<PrimDrag> = ui.ctx().data(|d| d.get_temp(drag_id));

    let pointer = ui.input(|i| i.pointer.hover_pos());
    let primary_down = ui.input(|i| i.pointer.primary_down());
    let primary_clicked = ui.input(|i| i.pointer.primary_pressed());
    let shift = ui.input(|i| i.modifiers.shift);
    if ty=="curve" && shift && ui.input(|i|i.pointer.button_double_clicked(PointerButton::Primary)) && current_drag.is_none() {
        if let Some(pointer)=pointer.filter(|p|rect.contains(*p)) {
            let default="0,0,0  1,0,0  1,1,0  2,1,1";
            if let Some(mut points)=parse_curve_points(params["points"].as_str().unwrap_or(default)) {
                let base=*points.last().unwrap();let (origin,dir)=ray(v.camera,rect,pointer);let n=v.camera.ray_forward;
                let denom=(0..3).map(|k|dir[k]*n[k]).sum::<f32>() as f64;
                if denom.abs()>1e-8 {let t=(0..3).map(|k|(base[k]-origin[k] as f64)*n[k] as f64).sum::<f64>()/denom;
                    points.push([0,1,2].map(|k|origin[k] as f64+dir[k] as f64*t));
                    out.push(UiAction::Edit{edit:Edit::Param{node:node_id.into(),name:"points".into(),value:json!(write_curve_points(&points))},continuous:false});
                }
            }
        }
    }

    // Render spinner ring if present
    if let Some(r) = ring {
        let center = r.center;
        let ring_radius = r.radius;
        let mut screen_pts = Vec::new();
        for i in 0..=32 {
            let theta = i as f32 * std::f32::consts::TAU / 32.0;
            let offset = [ring_radius * theta.cos(), ring_radius * theta.sin(), 0.0];
            let pt = [center[0] + offset[0], center[1] + offset[1], center[2] + offset[2]];
            if let Some(sp) = project(v.camera, rect, pt) {
                screen_pts.push(sp);
            }
        }
        if screen_pts.len() >= 2 {
            for pair in screen_pts.windows(2) {
                p.line_segment([pair[0], pair[1]], Stroke::new(1.2_f32, Color32::from_rgba_unmultiplied(79, 156, 240, 160)));
            }
        }
        if let Some(center_proj) = project(v.camera, rect, center) {
            p.text(
                center_proj + Vec2::new(0.0, -12.0),
                Align2::CENTER_BOTTOM,
                r.label,
                FontId::monospace(11.0),
                v.theme.text,
            );
        }
    }

    let accent = v.theme.accent;
    let accent_bright = Color32::from_rgb(140, 195, 255);

    let mut hit_any = false;

    // Handle active drag
    if let Some(drag) = &current_drag {
        if primary_down {
            if let Some(pos) = pointer {
                let edits = compute_edits_for_drag(drag, pos, shift);
                for (name, val) in edits {
                    out.push(UiAction::Edit {
                        edit: Edit::Param {
                            node: drag.node.clone(),
                            name: name.to_string(),
                            value: val,
                        },
                        continuous: true,
                    });
                }
            }
        } else {
            // Drag finished
            ui.ctx().data_mut(|d| d.remove::<PrimDrag>(drag_id));
            current_drag = None;
        }
    }

    // Process grips
    for grip in &grips {
        let Some(pos) = project(v.camera, rect, grip.world_pos) else { continue; };
        let end_pt = [
            grip.world_pos[0] + grip.world_dir[0] * 0.5,
            grip.world_pos[1] + grip.world_dir[1] * 0.5,
            grip.world_pos[2] + grip.world_dir[2] * 0.5,
        ];
        let end_pos = project(v.camera, rect, end_pt).unwrap_or(pos + Vec2::X * 30.0);
        let screen_dir = end_pos - pos;
        let span = screen_dir.length();

        if grip.action == GripAction::TubeCapsToggle {
            // Caps toggle chip
            let text = grip.label.as_deref().unwrap_or("Caps");
            let galley = p.layout_no_wrap(text.to_owned(), FontId::monospace(11.0), Color32::WHITE);
            let btn_rect = Rect::from_center_size(pos, galley.size() + Vec2::new(12.0, 6.0));
            let hovered = pointer.is_some_and(|pt| btn_rect.contains(pt));
            if hovered { hit_any = true; }

            let bg = if hovered { accent } else { Color32::from_rgba_unmultiplied(30, 36, 48, 220) };
            p.rect(btn_rect, 4.0, bg, Stroke::new(1.0_f32, accent), StrokeKind::Inside);
            p.galley(btn_rect.min + Vec2::new(6.0, 3.0), galley, Color32::WHITE);

            if hovered && primary_clicked && current_drag.is_none() {
                let caps = get_bool(params, "caps", true);
                out.push(UiAction::Edit {
                    edit: Edit::Param {
                        node: node_id.to_string(),
                        name: "caps".into(),
                        value: json!(!caps),
                    },
                    continuous: false,
                });
            }
            continue;
        }

        let hovered = pointer.is_some_and(|pt| (pt - pos).length() <= 9.0);
        if hovered { hit_any = true; }

        let color = if hovered { accent_bright } else { accent };
        let stroke = Stroke::new(1.2_f32, Color32::WHITE);

        match grip.shape {
            GripShape::Square => {
                let r = Rect::from_center_size(pos, Vec2::splat(9.0));
                p.rect(r, 1.5, color, stroke, StrokeKind::Inside);
            }
            GripShape::Circle => {
                p.circle_filled(pos, 5.0, color);
                p.circle_stroke(pos, 5.0, stroke);
            }
        }

        // Draw label on hover or if dragging this grip
        if let Some(text) = &grip.label {
            if hovered || current_drag.as_ref().is_some_and(|d| d.action == grip.action) {
                let galley = p.layout_no_wrap(text.clone(), FontId::monospace(10.5), Color32::WHITE);
                let chip_rect = Rect::from_min_size(pos + Vec2::new(8.0, -18.0), galley.size() + Vec2::new(8.0, 4.0));
                p.rect_filled(chip_rect, 3.0, Color32::from_rgba_unmultiplied(20, 24, 32, 230));
                p.galley(chip_rect.min + Vec2::new(4.0, 2.0), galley, Color32::WHITE);
            }
        }

        // Initiate drag
        if hovered && primary_clicked && current_drag.is_none() {
            let drag = PrimDrag {
                node: node_id.to_string(),
                start_pointer: pointer.unwrap_or(pos),
                screen_dir,
                span,
                start_params: params.clone(),
                action: grip.action.clone(),
                world_dir: grip.world_dir,
                camera: v.camera,
                rect,
            };
            ui.ctx().data_mut(|d| d.insert_temp(drag_id, drag));
            current_drag = ui.ctx().data(|d| d.get_temp(drag_id));
        }
    }

    hit_any || current_drag.is_some()
}

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
            action: GripAction::IntStep { param: "columns", min: 3, max: 4096, step_scale: 1.0 },
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
