//! Picking mesh elements in the viewport and highlight geometry for the current selection.
use crate::ui::SelectionMode;

#[derive(Clone, Copy, Debug)]
pub struct SnapFeature { pub point: [f32; 3], pub triangle: [[f32; 3]; 3], pub kind: crate::ui::SnapTo }

/// Compare every enabled feature in pixels, including vertices outside the ray-hit triangle.
pub fn snap_feature(positions: &[f32], indices: &[u32], camera: crate::ui::CameraInfo,
    rect: eframe::egui::Rect, pointer: eframe::egui::Pos2, targets: crate::ui::SnapTargets,
    backface_culling: bool) -> Option<SnapFeature> {
    use crate::ui::{SnapTo, transform_gizmo::{project, ray}};
    let (origin, direction) = ray(camera, rect, pointer);
    let mut best: Option<(f32, f32, SnapFeature)> = None;
    for ids in indices.chunks_exact(3) {
        if ids.iter().any(|&id| id as usize >= positions.len() / 3) { continue; }
        let tri = [at(positions, ids[0]), at(positions, ids[1]), at(positions, ids[2])];
        if backface_culling && dot(cross(sub(tri[1], tri[0]), sub(tri[2], tri[0])), direction) >= 0. { continue; }
        let screens = tri.map(|p| project(camera, rect, p));
        let mut consider = |point: [f32; 3], kind| {
            let Some(screen) = project(camera, rect, point) else { return; };
            let distance = screen.distance(pointer);
            let depth = dot(sub(point, origin), direction);
            if distance <= 14. && depth > 0. && best.as_ref().is_none_or(|(d,z,_)|
                distance < *d - 0.001 || ((distance - *d).abs() <= 0.001 && depth < *z)) {
                best = Some((distance, depth, SnapFeature { point, triangle: tri, kind }));
            }
        };
        if targets.contains(SnapTo::Vertex) { for p in tri { consider(p, SnapTo::Vertex); } }
        for (a,b) in [(0,1),(1,2),(2,0)] {
            if targets.contains(SnapTo::EdgeCenter) { consider([0,1,2].map(|k| (tri[a][k]+tri[b][k])*0.5), SnapTo::EdgeCenter); }
            if targets.contains(SnapTo::Edge) {
                if let (Some(sa), Some(sb)) = (screens[a], screens[b]) {
                    let ab = sb-sa;
                    let t = ((pointer-sa).dot(ab)/ab.length_sq().max(1e-8)).clamp(0.,1.);
                    // Perspective-correct interpolation along the projected segment.
                    let w = |p: [f32;3]| camera.projection[0][3]*p[0]+camera.projection[1][3]*p[1]+camera.projection[2][3]*p[2]+camera.projection[3][3];
                    let (wa,wb) = (w(tri[a]),w(tri[b]));
                    let t = t*wa / ((1.-t)*wb+t*wa).max(1e-8);
                    consider([0,1,2].map(|k| tri[a][k]+(tri[b][k]-tri[a][k])*t), SnapTo::Edge);
                }
            }
        }
        if targets.contains(SnapTo::Face) || targets.contains(SnapTo::Volume) {
            if let Some(hit) = pick_triangle(tri.as_flattened(), &[0,1,2], origin, direction) {
                let point = [0,1,2].map(|k| origin[k]+direction[k]*hit.t);
                if targets.contains(SnapTo::Face) { consider(point, SnapTo::Face); }
                if targets.contains(SnapTo::Volume) {
                    // Enter the surface and find the next exit along the same ray.
                    let inside = [0,1,2].map(|k| point[k]+direction[k]*1e-4);
                    if let Some(exit) = pick_triangle(positions, indices, inside, direction) {
                        consider([0,1,2].map(|k| point[k]+direction[k]*(exit.t+1e-4)*0.5), SnapTo::Volume);
                    }
                }
            }
        }
    }
    best.map(|(_,_,feature)| feature)
}

/// A region in viewport screen coordinates. Boundaries count as inside.
#[derive(Clone, Debug, PartialEq)]
pub enum RegionShape {
    Rect(eframe::egui::Rect),
    Circle { center: eframe::egui::Pos2, radius: f32 },
    /// One continuous brush path, evaluated in one mesh pass.
    CircleStroke { points: Vec<eframe::egui::Pos2>, radius: f32 },
    Lasso(Vec<eframe::egui::Pos2>),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectOp { #[default] Replace, Add, Subtract }
impl RegionShape {
    pub fn contains(&self, p: eframe::egui::Pos2) -> bool {
        match self {
            Self::Rect(r) => r.contains(p),
            Self::Circle { center, radius } => *radius >= 0. && center.distance_sq(p) <= radius * radius,
            Self::CircleStroke { points, radius } => *radius >= 0. && points.windows(2).any(|pair| {
                let delta=pair[1]-pair[0];
                let t=((p-pair[0]).dot(delta)/delta.length_sq().max(f32::MIN_POSITIVE)).clamp(0.,1.);
                p.distance_sq(pair[0]+delta*t) <= radius*radius
            }),
            Self::Lasso(points) => {
                if points.len() < 3 { return false; }
                let mut inside = false;
                for (a, b) in points.iter().zip(points.iter().cycle().skip(1)).take(points.len()) {
                    // Use f64 to retain the reference's boundary tolerance in screen coordinates.
                    let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
                    let (px, py) = ((p.x - a.x) as f64, (p.y - a.y) as f64);
                    let length = dx * dx + dy * dy;
                    let t = if length == 0. { 0. } else { ((px * dx + py * dy) / length).clamp(0., 1.) };
                    if (px - t * dx).hypot(py - t * dy) < 1e-7 { return true; }
                    if (a.y > p.y) != (b.y > p.y) && px < dx * py / dy { inside = !inside; }
                }
                inside
            }
        }
    }
    /// Edges count only when both projected endpoints are inside (browser editor parity).
    pub fn contains_edge(&self, a: eframe::egui::Pos2, b: eframe::egui::Pos2) -> bool {
        self.contains(a) && self.contains(b)
    }
}

/// What a click hit: the triangle (index into the triangle list), the distance along the ray and the barycentric weights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit { pub tri: usize, pub t: f32, pub bary: [f32; 3] }

/// Cook-time triangle accelerator. Navigation depth picks never scan the whole mesh.
pub struct Bvh { order: Vec<usize>, nodes: Vec<BvhNode> }
struct BvhNode { lo: [f32; 3], hi: [f32; 3], left: usize, right: usize, start: usize, end: usize }
impl Bvh {
    pub fn new(p: &[f32], indices: &[u32]) -> Self {
        let mut b = Self { order: (0..indices.len()/3).collect(), nodes: Vec::new() };
        if !b.order.is_empty() { b.build(p, indices, 0, b.order.len()); }
        b
    }
    fn build(&mut self, p: &[f32], indices: &[u32], start: usize, end: usize) -> usize {
        let mut lo = [f32::INFINITY; 3]; let mut hi = [f32::NEG_INFINITY; 3];
        for &id in &self.order[start..end] { for &v in &indices[id*3..id*3+3] {
            if let Some(pt) = p.get(v as usize*3..v as usize*3+3) { for k in 0..3 { lo[k]=lo[k].min(pt[k]); hi[k]=hi[k].max(pt[k]); } }
        } }
        let node = self.nodes.len();
        self.nodes.push(BvhNode { lo, hi, left:0, right:0, start, end });
        if end-start > 8 {
            let axis = (0..3).max_by(|&a,&b| (hi[a]-lo[a]).total_cmp(&(hi[b]-lo[b]))).unwrap();
            self.order[start..end].sort_unstable_by(|&a,&b| {
                let centroid = |id: usize| indices[id*3..id*3+3].iter().map(|&v| p.get(v as usize*3+axis).copied().unwrap_or(0.)).sum::<f32>();
                centroid(a).total_cmp(&centroid(b))
            });
            let mid=(start+end)/2;
            let left=self.build(p,indices,start,mid); let right=self.build(p,indices,mid,end);
            self.nodes[node].left=left; self.nodes[node].right=right;
        }
        node
    }
    pub fn pick(&self, p:&[f32], indices:&[u32], origin:[f32;3], dir:[f32;3], mut visible:impl FnMut(&[u32])->bool) -> Option<Hit> {
        let mut best:Option<Hit>=None;
        if self.nodes.is_empty() { return None; }
        let mut stack=vec![0];
        while let Some(id)=stack.pop() {
            let n=&self.nodes[id];
            let mut near=0_f32; let mut far=best.map_or(f32::INFINITY,|h|h.t);
            let mut missed=false;
            for k in 0..3 {
                if dir[k].abs()<1e-12 { if origin[k]<n.lo[k] || origin[k]>n.hi[k] { missed=true; break; } }
                else { let a=(n.lo[k]-origin[k])/dir[k]; let b=(n.hi[k]-origin[k])/dir[k]; near=near.max(a.min(b)); far=far.min(a.max(b)); }
            }
            if missed || near>far { continue; }
            if n.left!=0 { stack.push(n.left); stack.push(n.right); continue; }
            for &tri_id in &self.order[n.start..n.end] {
                let ids=&indices[tri_id*3..tri_id*3+3];
                if !visible(ids) { continue; }
                if let Some(mut hit)=pick_triangle(p,ids,origin,dir) {
                    if best.is_none_or(|h|hit.t<h.t) { hit.tri=tri_id; best=Some(hit); }
                }
            }
        }
        best
    }
}

#[cfg(test)] mod bvh_navigation_tests {
    use super::*;
    #[test] fn navigation_depth_pick_uses_cooked_bvh_and_empty_fallback() {
        let p=[-1.,-1.,0., 1.,-1.,0., 0.,1.,0.]; let tri=[0,1,2];
        let bvh=Bvh::new(&p,&tri);
        assert_eq!(bvh.pick(&p,&tri,[0.,0.,2.],[0.,0.,-1.],|_|true).unwrap().tri,0);
        assert!(bvh.pick(&p,&tri,[5.,0.,2.],[0.,0.,-1.],|_|true).is_none());
        assert!(bvh.pick(&p,&tri,[0.,0.,2.],[0.,0.,-1.],|_|false).is_none());
    }
    #[test] fn million_triangle_navigation_pick_is_under_five_ms() {
        let (w,h)=(1000_u32,500_u32);
        let mut p=Vec::with_capacity(((w+1)*(h+1)*3) as usize);
        for y in 0..=h {for x in 0..=w {p.extend([x as f32,y as f32,0.]);}}
        let mut tri=Vec::with_capacity((w*h*6) as usize);
        for y in 0..h {for x in 0..w {let a=y*(w+1)+x;tri.extend([a,a+1,a+w+2,a,a+w+2,a+w+1]);}}
        let bvh=Bvh::new(&p,&tri);
        let mut times=Vec::new();
        for _ in 0..7 {
            let now=std::time::Instant::now();
            assert!(bvh.pick(&p,&tri,[511.25,241.5,10.],[0.,0.,-1.],|_|true).is_some());
            times.push(now.elapsed());
        }
        times.sort();
        eprintln!("million-triangle BVH median pick: {:?}",times[3]);
        assert!(times[3]<std::time::Duration::from_millis(5),"median pick: {:?}",times[3]);
    }
}

/// A point or open-wire segment picked in viewport pixels. The indices remain geometry point ids.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WireHit { Point(u32), Edge(u32, u32) }

pub fn pick_wire(p: &[f32], lines: &[u32], pointer: eframe::egui::Pos2,
    mut project: impl FnMut([f32; 3]) -> Option<(eframe::egui::Pos2, f32)>,
    mut visible: impl FnMut(u32) -> bool, points: bool, edges: bool) -> Option<WireHit> {
    let mut best: Option<(f32, f32, WireHit)> = None;
    let mut consider = |distance: f32, depth: f32, hit| {
        if depth <= 0. || !depth.is_finite() { return; }
        if best.as_ref().is_none_or(|(d,z,_)| distance < *d - 0.001 ||
            ((distance-*d).abs() <= 0.001 && depth < *z)) { best = Some((distance,depth,hit)); }
    };
    if points {
        for id in 0..(p.len()/3) as u32 {
            if !visible(id) { continue; }
            if let Some((screen,depth)) = project(at(p,id)) {
                let distance = screen.distance(pointer);
                if distance <= 12. { consider(distance,depth,WireHit::Point(id)); }
            }
        }
    }
    if edges {
        for pair in lines.chunks_exact(2) {
            let (a,b) = (pair[0],pair[1]);
            if a == b || a as usize >= p.len()/3 || b as usize >= p.len()/3 || !visible(a) || !visible(b) { continue; }
            let (Some((sa,za)),Some((sb,zb))) = (project(at(p,a)),project(at(p,b))) else { continue; };
            let ab = sb-sa;
            let t = ((pointer-sa).dot(ab)/ab.length_sq().max(1e-8)).clamp(0.,1.);
            let distance = (pointer-(sa+ab*t)).length();
            if distance <= 8. { consider(distance,za+(zb-za)*t,WireHit::Edge(a.min(b),a.max(b))); }
        }
    }
    best.map(|(_,_,hit)| hit)
}

#[cfg(test)] mod wire_tests {
    use super::*;
    use eframe::egui::pos2;
    #[test]
    fn swept_circle_covers_fast_and_bent_strokes_without_gaps() {
        use eframe::egui::pos2;
        let stroke=RegionShape::CircleStroke {points:vec![pos2(0.,0.),pos2(100.,0.),pos2(100.,100.)],radius:5.};
        for p in [pos2(-5.,0.),pos2(50.,5.),pos2(100.,50.),pos2(100.,105.)] {assert!(stroke.contains(p));}
        assert!(!stroke.contains(pos2(50.,50.)),"do not cut across a bent stroke");
        assert!(!stroke.contains(pos2(50.,5.1)));
        assert!(RegionShape::CircleStroke {points:vec![pos2(0.,0.);2],radius:5.}.contains(pos2(3.,4.)));
        assert!(!RegionShape::CircleStroke {points:vec![],radius:5.}.contains(pos2(0.,0.)));
    }
    #[test] fn wire_pixels_and_visibility() {
        let p=[0.,0.,1., 100.,0.,1.];
        let project=|p:[f32;3]|Some((pos2(p[0],p[1]),p[2]));
        assert_eq!(pick_wire(&p,&[0,1],pos2(3.,11.),project,|_|true,true,false),Some(WireHit::Point(0)));
        assert_eq!(pick_wire(&p,&[0,1],pos2(50.,7.),project,|_|true,false,true),Some(WireHit::Edge(0,1)));
        assert_eq!(pick_wire(&p,&[0,1],pos2(50.,9.),project,|_|true,false,true),None);
        assert_eq!(pick_wire(&p,&[0,1],pos2(0.,0.),project,|_|false,true,true),None);
    }
}

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
    pick_triangle_filtered(p,tri,origin,dir,|_| true)
}
/// Filter before ray testing so an unpickable foreground ghost cannot block a selected object.
pub fn pick_triangle_filtered(p:&[f32],tri:&[u32],origin:[f32;3],dir:[f32;3],mut visible:impl FnMut(&[u32])->bool)->Option<Hit> {
    let mut best: Option<Hit> = None;
    for (n, t) in tri.as_chunks::<3>().0.iter().enumerate() {
        if !visible(t) { continue; }
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
    use minihou_app::graph_edit::{is_primitive_node, node_has_center_param};

    #[test]
    fn snap_candidates_use_screen_distance_and_edges_are_not_midpoints() {
        use crate::ui::{CameraInfo,SnapTargets,SnapTo};
        use eframe::egui::{pos2,vec2,Rect};
        let camera=CameraInfo { projection:[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]],
            ortho:true,ray_origin:[0.,0.,5.],ray_right:[1.,0.,0.],ray_up:[0.,1.,0.],ray_forward:[0.,0.,-1.],..Default::default() };
        let rect=Rect::from_min_size(pos2(0.,0.),vec2(200.,200.));
        let p=[0.,0.,0., 1.,0.,0., 0.,1.,0.];
        let mut targets=SnapTargets::default(); targets.toggle(SnapTo::Edge); targets.toggle(SnapTo::EdgeCenter);
        let edge=snap_feature(&p,&[0,1,2],camera,rect,pos2(125.,102.),targets,false).unwrap();
        assert_eq!(edge.kind,SnapTo::Edge);
        assert!((edge.point[0]-0.25).abs()<1e-5 && edge.point[1].abs()<1e-5);
        let vertex=snap_feature(&p,&[0,1,2],camera,rect,pos2(96.,104.),SnapTo::Vertex.into(),false).unwrap();
        assert_eq!(vertex.point,[0.;3]); // Pointer need not hit a face.
        assert!(snap_feature(&p,&[0,1,2],camera,rect,pos2(150.,150.),SnapTo::Vertex.into(),false).is_none());
        targets.toggle(SnapTo::Face);
        let face=snap_feature(&p,&[0,1,2],camera,rect,pos2(120.,90.),targets,false).unwrap();
        assert_eq!(face.kind,SnapTo::Face);
        assert!((face.point[0]-0.2).abs()<1e-5 && (face.point[1]-0.1).abs()<1e-5);
        assert!(snap_feature(&p,&[0,2,1],camera,rect,pos2(120.,90.),SnapTo::Face.into(),true).is_none());
    }

    #[test]
    fn volume_snap_uses_midpoint_between_entry_and_exit() {
        use crate::ui::{CameraInfo,SnapTo};
        use eframe::egui::{pos2,vec2,Rect};
        let camera=CameraInfo { projection:[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]],
            ortho:true,ray_origin:[0.,0.,5.],ray_right:[1.,0.,0.],ray_up:[0.,1.,0.],ray_forward:[0.,0.,-1.],..Default::default() };
        let p=[0.,0.,1., 1.,0.,1., 0.,1.,1., 0.,0.,-1., 0.,1.,-1., 1.,0.,-1.];
        let feature=snap_feature(&p,&[0,1,2,3,4,5],camera,Rect::from_min_size(pos2(0.,0.),vec2(200.,200.)),pos2(120.,90.),SnapTo::Volume.into(),false).unwrap();
        assert!(feature.point[2].abs()<1e-4);
    }

    #[test]
    fn region_contains_boundaries_and_concave_lassos() {
        use eframe::egui::{pos2, Rect};
        let rect = RegionShape::Rect(Rect::from_min_max(pos2(0., 0.), pos2(10., 10.)));
        assert!(rect.contains(pos2(0., 10.)));
        assert!(!rect.contains(pos2(11., 5.)));
        let circle = RegionShape::Circle { center: pos2(0., 0.), radius: 5. };
        assert!(circle.contains(pos2(3., 4.)));
        assert!(!circle.contains(pos2(4., 4.)));
        let points = vec![pos2(0., 0.), pos2(10., 0.), pos2(10., 4.), pos2(4., 4.), pos2(4., 10.), pos2(0., 10.)];
        for polygon in [points.clone(), points.into_iter().rev().collect()] {
            let lasso = RegionShape::Lasso(polygon);
            assert!(lasso.contains(pos2(2., 8.)));
            assert!(lasso.contains(pos2(4., 7.)));
            assert!(lasso.contains(pos2(10., 0.)));
            assert!(!lasso.contains(pos2(7., 7.)));
            assert!(!lasso.contains(pos2(-1., 0.)));
        }
        assert!(!RegionShape::Lasso(vec![]).contains(pos2(0., 0.)));
        assert!(!RegionShape::Lasso(vec![pos2(0., 0.); 2]).contains(pos2(0., 0.)));
    }

    #[test]
    fn region_edges_require_both_endpoints_even_when_midpoint_is_inside() {
        use eframe::egui::{pos2, Rect};
        for shape in [RegionShape::Rect(Rect::from_min_max(pos2(-5., -5.), pos2(5., 5.))),
            RegionShape::Circle { center: pos2(0., 0.), radius: 5. },
            RegionShape::Lasso(vec![pos2(-5., -5.), pos2(5., -5.), pos2(5., 5.), pos2(-5., 5.)])] {
            assert!(shape.contains(pos2(0., 0.)));
            assert!(!shape.contains_edge(pos2(-10., 0.), pos2(10., 0.)));
            assert!(!shape.contains_edge(pos2(0., 0.), pos2(6., 0.)));
            assert!(shape.contains_edge(pos2(-2., 0.), pos2(5., 0.)));
        }
    }

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
        let vertices = [-1., 0., -1., 1., 0., -1., 0., 0., 1.];
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

#[test]
fn filtered_pick_ignores_foreground_ghost_and_preserves_triangle_id() {
    let p=[0.,0.,1., 1.,0.,1., 0.,1.,1., 0.,0.,0., 1.,0.,0., 0.,1.,0.];
    let tri=[0,1,2,3,4,5];
    let hit=pick_triangle_filtered(&p,&tri,[0.2,0.2,3.],[0.,0.,-1.],|t| t.iter().all(|id| (3..6).contains(id))).unwrap();
    assert_eq!(hit.tri,1); assert_eq!(hit.t,3.);
    assert!(pick_triangle_filtered(&p,&tri,[0.2,0.2,3.],[0.,0.,-1.],|_|false).is_none());
}
