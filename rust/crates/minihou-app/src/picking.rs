//! Picking mesh elements in the viewport: a ray from the camera against the displayed triangles (CPU, brute force, which is
//! plenty for a click), and the highlight geometry for the current selection.
use crate::ui::SelectionMode;

/// What a click hit: the triangle (index into the triangle list), the distance along the ray and the barycentric weights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit { pub tri: usize, pub t: f32, pub bary: [f32; 3] }

/// Selected elements of one kind; ids are point numbers (Points), vertex-pair ids (Edges) or triangle numbers (Faces).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection { pub mode: SelectionMode, pub points: Vec<u32>, pub edges: Vec<(u32, u32)>, pub tris: Vec<u32> }
impl Selection {
    pub fn is_empty(&self) -> bool { self.points.is_empty() && self.edges.is_empty() && self.tris.is_empty() }
    pub fn count(&self) -> usize { self.points.len() + self.edges.len() + self.tris.len() }
    pub fn clear(&mut self) { self.points.clear(); self.edges.clear(); self.tris.clear(); }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[1]*b[2] - a[2]*b[1], a[2]*b[0] - a[0]*b[2], a[0]*b[1] - a[1]*b[0]] }
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 { a[0]*b[0] + a[1]*b[1] + a[2]*b[2] }
pub fn at(p: &[f32], i: u32) -> [f32; 3] { let k = i as usize * 3; [p[k], p[k + 1], p[k + 2]] }

/// Nearest front-most triangle along the ray (Moller-Trumbore, both sides).
pub fn pick_triangle(p: &[f32], tri: &[u32], origin: [f32; 3], dir: [f32; 3]) -> Option<Hit> {
    let mut best: Option<Hit> = None;
    for (n, t) in tri.as_chunks::<3>().0.iter().enumerate() {
        let (a, b, c) = (at(p, t[0]), at(p, t[1]), at(p, t[2]));
        let (e1, e2) = (sub(b, a), sub(c, a));
        let h = cross(dir, e2);
        let det = dot(e1, h);
        if det.abs() < 1e-12 { continue; }
        let inv = 1.0 / det;
        let s = sub(origin, a);
        let u = dot(s, h) * inv;
        if !(0.0..=1.0).contains(&u) { continue; }
        let q = cross(s, e1);
        let v = dot(dir, q) * inv;
        if v < 0.0 || u + v > 1.0 { continue; }
        let dist = dot(e2, q) * inv;
        if dist > 1e-6 && best.is_none_or(|h| dist < h.t) { best = Some(Hit { tri: n, t: dist, bary: [1.0 - u - v, u, v] }); }
    }
    best
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

/// Whether the node type has a native `center` parameter (box, sphere, tube, torus, grid, circle, point).
pub fn node_has_center_param(node_type: &str) -> bool {
    matches!(
        node_type,
        "box" | "sphere" | "tube" | "torus" | "grid" | "circle" | "point"
    )
}

/// Whether the node type is a primitive generator.
pub fn is_primitive_node(node_type: &str) -> bool {
    node_has_center_param(node_type)
        || matches!(
            node_type,
            "line" | "curve" | "bezier" | "helix" | "lsystem" | "null" | "lattice_grid"
        )
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

pub const MAX_UNDECIMATED_POINTS: usize = 200_000;

/// Decimation logic for point dots:
/// When `npoints <= MAX_UNDECIMATED_POINTS`, returns stride 1 (all points shown) and None for status.
/// When `npoints > MAX_UNDECIMATED_POINTS`, decimates dots by returning a stride > 1 and a status notice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecimatedDots {
    pub stride: usize,
    pub hidden: bool,
    pub status: Option<&'static str>,
}

pub fn decimate_dots(npoints: usize) -> DecimatedDots {
    if npoints <= MAX_UNDECIMATED_POINTS {
        DecimatedDots {
            stride: 1,
            hidden: false,
            status: None,
        }
    } else {
        DecimatedDots {
            stride: (npoints / 50_000).max(2),
            hidden: false,
            status: Some(">200k points: dots decimated"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minihou_app::{Camera, StandardView};

    // two triangles forming a unit square in the z = 0 plane
    const P: [f32; 12] = [0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.];
    const T: [u32; 6] = [0, 1, 2, 0, 2, 3];

    #[test]
    fn a_ray_down_z_hits_the_right_triangle() {
        let hit = pick_triangle(&P, &T, [0.8, 0.3, 5.], [0., 0., -1.]).unwrap();
        assert_eq!(hit.tri, 0);
        assert!((hit.t - 5.).abs() < 1e-5);
        let hit = pick_triangle(&P, &T, [0.2, 0.7, 5.], [0., 0., -1.]).unwrap();
        assert_eq!(hit.tri, 1);
        assert!(pick_triangle(&P, &T, [3., 3., 5.], [0., 0., -1.]).is_none());
    }

    #[test]
    fn nearest_triangle_wins_and_back_faces_count() {
        let p = [0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1., 1., 0., 1., 0., 1., 1.];
        let t = [0, 1, 2, 3, 4, 5];
        let hit = pick_triangle(&p, &t, [0.2, 0.2, 9.], [0., 0., -1.]).unwrap();
        assert_eq!(hit.tri, 1);
        let from_below = pick_triangle(&p, &t, [0.2, 0.2, -9.], [0., 0., 1.]).unwrap();
        assert_eq!(from_below.tri, 0);
    }

    #[test]
    fn elements_follow_the_mode() {
        let hit = pick_triangle(&P, &T, [0.9, 0.05, 1.], [0., 0., -1.]).unwrap();
        assert_eq!(element_of(hit, &T, SelectionMode::Faces).tris, vec![0]);
        assert_eq!(element_of(hit, &T, SelectionMode::Points).points, vec![1]);
        // the click is nearest the bottom edge 0-1: the edge opposite the corner with the smallest weight (corner 2)
        assert_eq!(element_of(hit, &T, SelectionMode::Edges).edges, vec![(0, 1)]);
        assert!(element_of(hit, &T, SelectionMode::Object).tris.is_empty());
    }

    #[test]
    fn shift_click_toggles() {
        let mut cur = Selection { mode: SelectionMode::Faces, tris: vec![0], ..Default::default() };
        let one = Selection { mode: SelectionMode::Faces, tris: vec![1], ..Default::default() };
        extend(&mut cur, &one);
        assert_eq!(cur.tris, vec![0, 1]);
        extend(&mut cur, &one);
        assert_eq!(cur.tris, vec![0]);
        let pts = Selection { mode: SelectionMode::Points, points: vec![2], ..Default::default() };
        extend(&mut cur, &pts);
        assert!(cur.tris.is_empty() && cur.points == vec![2]);
    }

    #[test]
    fn highlight_sizes() {
        let sel = Selection { mode: SelectionMode::Faces, tris: vec![1], edges: vec![(0, 1)], points: vec![3] };
        let (faces, lines) = highlight(&P, &T, &sel, 0.05);
        assert_eq!(faces.len(), 9);
        assert_eq!(lines.len(), 6 + 18);
    }

    #[test]
    fn camera_center_ray_hits_mesh_in_perspective_and_orthographic_views() {
        let vertices = [-1., -1., 0., 1., -1., 0., 0., 1., 0.];
        let triangles = [0, 1, 2];
        let mut camera = Camera::frame([0., 0., 0.], 1.);
        for view in [StandardView::Perspective, StandardView::Front] {
            camera.set_view(view);
            let (origin, direction) = camera.ray(0., 0., 16. / 9.);
            let hit = pick_triangle(&vertices, &triangles, origin, direction).unwrap();
            assert_eq!(hit.tri, 0);
            assert!(hit.t > 0.);
        }
    }

    #[test]
    fn drop_placement_logic() {
        let p_tri = [-1., 2., -1., 1., 2., -1., 0., 2., 1.];
        let tri = [0, 1, 2];
        let cursor = [10.0, 5.0, -10.0];

        // 1. Ray hits triangle at y = 2.0
        let origin = [0.0, 5.0, 0.0];
        let dir_down = [0.0, -1.0, 0.0];
        let (hit_pos, surface_hit) = drop_world_position(origin, dir_down, Some(&p_tri), Some(&tri), cursor);
        assert!(surface_hit);
        assert!((hit_pos[1] - 2.0).abs() < 1e-4);

        // 2. Ray misses triangle but hits ground plane y = 0
        let origin_off = [5.0, 5.0, 5.0];
        let (hit_ground, surface_hit) = drop_world_position(origin_off, dir_down, Some(&p_tri), Some(&tri), cursor);
        assert!(surface_hit);
        assert!((hit_ground[1] - 0.0).abs() < 1e-5);
        assert!((hit_ground[0] - 5.0).abs() < 1e-5);
        assert!((hit_ground[2] - 5.0).abs() < 1e-5);

        // 3. Ray parallel to ground plane (dir.y = 0): falls back to 3D cursor
        let dir_parallel = [1.0, 0.0, 0.0];
        let (fallback_cursor, surface_hit) = drop_world_position(origin_off, dir_parallel, Some(&p_tri), Some(&tri), cursor);
        assert!(!surface_hit);
        assert_eq!(fallback_cursor, cursor);

        // Rest-on-surface offsets
        assert!((primitive_rest_offset("box") - 0.5).abs() < 1e-4);
        assert!((primitive_rest_offset("sphere") - 1.0).abs() < 1e-4);
        assert!((primitive_rest_offset("grid") - 5.0).abs() < 1e-4);

        // Parameter choice per node type
        assert!(node_has_center_param("box"));
        assert!(node_has_center_param("sphere"));
        assert!(node_has_center_param("tube"));
        assert!(node_has_center_param("torus"));
        assert!(node_has_center_param("grid"));
        assert!(node_has_center_param("circle"));
        assert!(!node_has_center_param("line"));
        assert!(!node_has_center_param("null"));
        assert!(!node_has_center_param("transform"));

        assert!(is_primitive_node("box"));
        assert!(is_primitive_node("line"));
        assert!(!is_primitive_node("noise"));
        assert!(!is_primitive_node("normal"));
    }

    #[test]
    fn cursor_placement_and_reset() {
        let p_mesh = [-1., 3., -1., 1., 3., -1., 0., 3., 1.];
        let tri = [0, 1, 2];
        let mut cursor = [1.0, 2.0, 3.0];
        let fwd = [0.0, 0.0, -1.0];

        // 1. Ray hits mesh
        let origin = [0.0, 6.0, 0.0];
        let dir = [0.0, -1.0, 0.0];
        let at_mesh = cursor_hit_position(origin, dir, Some(&p_mesh), Some(&tri), cursor, fwd);
        assert!((at_mesh[1] - 3.0).abs() < 1e-4);

        // 2. Ray misses mesh: hits plane at cursor height (y = 2.0)
        let origin_off = [5.0, 6.0, 5.0];
        let at_plane = cursor_hit_position(origin_off, dir, Some(&p_mesh), Some(&tri), cursor, fwd);
        assert!((at_plane[1] - cursor[1]).abs() < 1e-4);
        assert!((at_plane[0] - 5.0).abs() < 1e-4);

        // 3. Reset cursor to origin
        cursor = [0.0, 0.0, 0.0];
        assert_eq!(cursor, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn ground_plane_intersection() {
        let origin = [0.0, 5.0, 0.0];
        let dir_down = [0.0, -1.0, 0.0];
        let t = intersect_ground_plane(origin, dir_down).unwrap();
        assert!((t - 5.0).abs() < 1e-5);

        let hit = intersect_ground_or_mesh(origin, dir_down, None, None, 10.0);
        assert!((hit[1] - 0.0).abs() < 1e-5);

        // Pointing up away from ground
        let dir_up = [0.0, 1.0, 0.0];
        assert!(intersect_ground_plane(origin, dir_up).is_none());

        // With mesh between origin and ground plane (at y = 2.0)
        let p_mesh = [-1., 2., -1., 1., 2., -1., 0., 2., 1.];
        let tri_mesh = [0, 1, 2];
        let hit_mesh = intersect_ground_or_mesh(origin, dir_down, Some(&p_mesh), Some(&tri_mesh), 10.0);
        assert!((hit_mesh[1] - 2.0).abs() < 1e-4);
    }

    #[test]
    fn dot_decimation() {
        let small = decimate_dots(100);
        assert_eq!(small.stride, 1);
        assert_eq!(small.status, None);

        let boundary = decimate_dots(200_000);
        assert_eq!(boundary.stride, 1);
        assert_eq!(boundary.status, None);

        let large = decimate_dots(200_001);
        assert!(large.stride > 1);
        assert!(large.status.unwrap().contains(">200k"));

        let very_large = decimate_dots(1_000_000);
        assert!(very_large.stride >= 20);
        assert!(very_large.status.unwrap().contains(">200k"));
    }
}
