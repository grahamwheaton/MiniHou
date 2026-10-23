use super::*;

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
                action: GripAction::IntStep { param: "columns", default: 32, min: 3, max: 4096, step_scale: 1.0 },
                label: Some(format!("Cols: {columns}")),
            });

            let row_pos = local_to_world([0.0, ring_r, ring_z], center, rotate, scale, uniform_scale);
            let row_dir = rotate_dir([-1.0, 0.0, 0.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_sphere_rows"),
                world_pos: row_pos,
                world_dir: row_dir,
                shape: GripShape::Circle,
                action: GripAction::IntStep { param: "rows", default: 16, min: 2, max: 4096, step_scale: 1.0 },
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
                action: GripAction::IntStep { param: "columns", default: 24, min: 3, max: 4096, step_scale: 1.0 },
                label: Some(format!("Cols: {columns}")),
            });

            let row_pos = local_to_world([0.0, ring_r, ring_z], center, rotate, scale, uniform_scale);
            let row_dir = rotate_dir([-1.0, 0.0, 0.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_tube_rows"),
                world_pos: row_pos,
                world_dir: row_dir,
                shape: GripShape::Circle,
                action: GripAction::IntStep { param: "rows", default: 1, min: 1, max: 4096, step_scale: 1.0 },
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
                action: GripAction::IntStep { param: "columns", default: 32, min: 3, max: 4096, step_scale: 1.0 },
                label: Some(format!("Cols: {columns}")),
            });

            let row_pos = local_to_world([0.0, ring_r, ring_z], center, rotate, scale, uniform_scale);
            let row_dir = rotate_dir([-1.0, 0.0, 0.0], rotate);
            grips.push(Grip {
                id: format!("{node_id}_torus_rows"),
                world_pos: row_pos,
                world_dir: row_dir,
                shape: GripShape::Circle,
                action: GripAction::IntStep { param: "rows", default: 12, min: 3, max: 4096, step_scale: 1.0 },
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
                action: GripAction::IntStep { param: "columns", default: 10, min: 2, max: 20000, step_scale: 1.0 },
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
                action: GripAction::IntStep { param: "rows", default: 10, min: 2, max: 20000, step_scale: 1.0 },
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
                action: GripAction::IntStep { param: "divisions", default: 32, min: 3, max: 100000, step_scale: 1.0 },
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
                action: GripAction::IntStep { param: "points", default: 10, min: 2, max: 1000000, step_scale: 1.0 },
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
