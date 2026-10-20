use crate::ui::SelectionMode;

use super::spatial::{sub, dot, Hit};

pub use super::spatial::{at, try_at, pick_triangle, pick_triangle_filtered, pick_triangle_culled};

/// Selected elements of one kind; ids are point numbers (Points), vertex-pair ids (Edges) or triangle numbers (Faces).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection { pub mode: SelectionMode, pub points: Vec<u32>, pub edges: Vec<(u32, u32)>, pub tris: Vec<u32> }
impl Selection {
    pub fn is_empty(&self) -> bool { self.points.is_empty() && self.edges.is_empty() && self.tris.is_empty() }
    pub fn count(&self) -> usize { self.points.len() + self.edges.len() + self.tris.len() }
    pub fn clear(&mut self) { self.points.clear(); self.edges.clear(); self.tris.clear(); }
}


/// The element a hit means in `mode`: the corner with the largest weight, the edge opposite the smallest weight, or the face.
pub fn element_of(hit: Hit, tri: &[u32], mode: SelectionMode) -> Selection {
    let t = &tri[hit.tri * 3..hit.tri * 3 + 3];
    let mut s = Selection { mode, ..Default::default() };
    match mode {
        SelectionMode::Object => {}
        SelectionMode::Faces => s.tris.push(hit.tri as u32),
        SelectionMode::Points => {
            let k = (0..3).max_by(|&a, &b| hit.bary[a].total_cmp(&hit.bary[b])).unwrap_or(0);
            s.points.push(t[k]);
        }
        SelectionMode::Edges => {
            let far = (0..3).min_by(|&a, &b| hit.bary[a].total_cmp(&hit.bary[b])).unwrap_or(0);
            let (a, b) = (t[(far + 1) % 3], t[(far + 2) % 3]);
            s.edges.push((a.min(b), a.max(b)));
        }
    }
    s
}

/// Add `new` to `current` (or toggle its elements off when they are already selected), as shift-click does.
pub fn extend(current: &mut Selection, new: &Selection) {
    if current.mode != new.mode { current.clear(); current.mode = new.mode; }
    fn toggle<T: PartialEq + Copy>(list: &mut Vec<T>, items: &[T]) {
        for &i in items { if let Some(k) = list.iter().position(|x| *x == i) { list.remove(k); } else { list.push(i); } }
    }
    toggle(&mut current.points, &new.points);
    toggle(&mut current.edges, &new.edges);
    toggle(&mut current.tris, &new.tris);
}

/// Highlight geometry for a selection: triangle corners (3 positions per selected face) and line end points (2 per segment;
/// a point is drawn as a small 3-axis cross of size `size`).
pub fn highlight(p: &[f32], tri: &[u32], sel: &Selection, size: f32) -> (Vec<f32>, Vec<f32>) {
    let (mut faces, mut lines) = (Vec::new(), Vec::new());
    for &t in &sel.tris {
        let k = t as usize * 3;
        if k + 3 <= tri.len() { for &v in &tri[k..k + 3] { faces.extend_from_slice(&at(p, v)); } }
    }
    for &(a, b) in &sel.edges {
        if (a.max(b) as usize + 1) * 3 <= p.len() { lines.extend_from_slice(&at(p, a)); lines.extend_from_slice(&at(p, b)); }
    }
    for &i in &sel.points {
        if (i as usize + 1) * 3 > p.len() { continue; }
        let c = at(p, i);
        for axis in 0..3 {
            let (mut lo, mut hi) = (c, c);
            lo[axis] -= size; hi[axis] += size;
            lines.extend_from_slice(&lo); lines.extend_from_slice(&hi);
        }
    }
    (faces, lines)
}

/// Intersects ray with a horizontal plane at y = plane_y.
pub fn intersect_horizontal_plane(origin: [f32; 3], dir: [f32; 3], plane_y: f32) -> Option<f32> {
    if dir[1].abs() > 1e-6 {
        let t = (plane_y - origin[1]) / dir[1];
        if t > 0.0 {
            return Some(t);
        }
    }
    None
}

/// Compute world position for drop placement:
/// Ray from pointer against displayed mesh (first hit, `pick_triangle`) else ground plane y = 0,
/// else (ray parallel) the 3D cursor.
/// Returns (position, is_surface_hit).
pub fn drop_world_position(
    origin: [f32; 3],
    dir: [f32; 3],
    mesh_p: Option<&[f32]>,
    mesh_tri: Option<&[u32]>,
    cursor: [f32; 3],
) -> ([f32; 3], bool) {
    if let (Some(p), Some(tri)) = (mesh_p, mesh_tri) {
        if !p.is_empty() && !tri.is_empty() {
            if let Some(hit) = pick_triangle(p, tri, origin, dir) {
                return (
                    [
                        origin[0] + hit.t * dir[0],
                        origin[1] + hit.t * dir[1],
                        origin[2] + hit.t * dir[2],
                    ],
                    true,
                );
            }
        }
    }
    if let Some(t) = intersect_ground_plane(origin, dir) {
        return (
            [origin[0] + t * dir[0], 0.0, origin[2] + t * dir[2]],
            true,
        );
    }
    (cursor, false)
}

/// Computes 3D cursor placement under the pointer:
/// Surface hit, else ground plane at cursor's current height projected along the view.
pub fn cursor_hit_position(
    origin: [f32; 3],
    dir: [f32; 3],
    mesh_p: Option<&[f32]>,
    mesh_tri: Option<&[u32]>,
    cursor: [f32; 3],
    cam_forward: [f32; 3],
) -> [f32; 3] {
    if let (Some(p), Some(tri)) = (mesh_p, mesh_tri) {
        if !p.is_empty() && !tri.is_empty() {
            if let Some(hit) = pick_triangle(p, tri, origin, dir) {
                return [
                    origin[0] + hit.t * dir[0],
                    origin[1] + hit.t * dir[1],
                    origin[2] + hit.t * dir[2],
                ];
            }
        }
    }
    if let Some(t) = intersect_horizontal_plane(origin, dir, cursor[1]) {
        return [
            origin[0] + t * dir[0],
            cursor[1],
            origin[2] + t * dir[2],
        ];
    }
    let denom = dot(dir, cam_forward);
    if denom.abs() > 1e-6 {
        let t = dot(sub(cursor, origin), cam_forward) / denom;
        if t > 0.0 {
            return [
                origin[0] + t * dir[0],
                origin[1] + t * dir[1],
                origin[2] + t * dir[2],
            ];
        }
    }
    cursor
}

/// Vertical offset to rest a primitive on a surface so its bounding box sits on the hit point.
pub fn primitive_rest_offset(node_type: &str) -> f32 {
    let val = serde_json::json!({
        "nodes": {
            "n": { "type": node_type }
        }
    });
    if let Ok(graph) = minihou_core::Graph::from_value(&val) {
        if let Ok(geo) = graph.cook(Some("n")) {
            if geo.npoints > 0 {
                let (lo, _hi) = minihou_core::preview::bounds(geo.p());
                return -lo[1];
            }
        }
    }
    match node_type {
        "box" => 0.5,
        "sphere" | "tube" => 1.0,
        "torus" => 0.25,
        _ => 0.0,
    }
}

/// Intersects a ray with the ground plane y = 0.
/// Returns distance t along the ray if hit in front (t > 0).
pub fn intersect_ground_plane(origin: [f32; 3], dir: [f32; 3]) -> Option<f32> {
    if dir[1].abs() > 1e-6 {
        let t = -origin[1] / dir[1];
        if t > 0.0 {
            return Some(t);
        }
    }
    None
}

/// Intersects the pointer ray with the ground plane y = 0, or with the displayed mesh if it is hit first.
/// Returns the 3D world position of the hit.
#[allow(dead_code)]
pub fn intersect_ground_or_mesh(
    origin: [f32; 3],
    dir: [f32; 3],
    mesh_positions: Option<&[f32]>,
    mesh_triangles: Option<&[u32]>,
    default_dist: f32,
) -> [f32; 3] {
    let t_mesh = match (mesh_positions, mesh_triangles) {
        (Some(p), Some(tri)) if !p.is_empty() && !tri.is_empty() => {
            pick_triangle(p, tri, origin, dir).map(|h| h.t)
        }
        _ => None,
    };
    let t_plane = intersect_ground_plane(origin, dir);
    let t = match (t_mesh, t_plane) {
        (Some(tm), Some(tp)) => tm.min(tp),
        (Some(tm), None) => tm,
        (None, Some(tp)) => tp,
        (None, None) => default_dist.max(1.0),
    };
    [origin[0] + t * dir[0], origin[1] + t * dir[1], origin[2] + t * dir[2]]
}

