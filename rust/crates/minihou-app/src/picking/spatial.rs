/// Bounds of the viewport point buffer, including an appended selected preview.
pub fn position_bounds(positions: &[f32]) -> ([f32; 3], [f32; 3]) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for point in positions.chunks_exact(3) {
        for k in 0..3 { lo[k] = lo[k].min(point[k]); hi[k] = hi[k].max(point[k]); }
    }
    if positions.len() < 3 { ([0.; 3], [0.; 3]) } else { (lo, hi) }
}

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


/// A point or open-wire segment picked in viewport pixels. The indices remain geometry point ids.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WireHit { Point(u32), Edge(u32, u32) }

pub fn pick_wire(p: &[f32], lines: &[u32], pointer: eframe::egui::Pos2,
    mut project: impl FnMut([f32; 3]) -> Option<(eframe::egui::Pos2, f32)>,
    visible: impl FnMut(u32) -> bool, points: bool, edges: bool) -> Option<WireHit> {
    pick_wire_occluded(p,lines,pointer,project,visible,points,edges,f32::INFINITY)
}
/// As `pick_wire`; elements deeper than `max_depth` (the mesh surface under the pointer) are hidden behind it.
#[allow(clippy::too_many_arguments)]
pub fn pick_wire_occluded(p: &[f32], lines: &[u32], pointer: eframe::egui::Pos2,
    mut project: impl FnMut([f32; 3]) -> Option<(eframe::egui::Pos2, f32)>,
    mut visible: impl FnMut(u32) -> bool, points: bool, edges: bool, max_depth: f32) -> Option<WireHit> {
    let mut best: Option<(f32, f32, WireHit)> = None;
    let mut consider = |distance: f32, depth: f32, hit| {
        if depth <= 0. || !depth.is_finite() || depth > max_depth { return; }
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


pub(super) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
pub(super) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[1]*b[2] - a[2]*b[1], a[2]*b[0] - a[0]*b[2], a[0]*b[1] - a[1]*b[0]] }
pub(super) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 { a[0]*b[0] + a[1]*b[1] + a[2]*b[2] }
/// Point position; NaN when the id is outside `p` (a stale id must never crash the viewport, and NaN never hits a ray).
pub fn at(p: &[f32], i: u32) -> [f32; 3] { try_at(p, i).unwrap_or([f32::NAN; 3]) }
/// Like `at`, but None when the point id is outside `p` (stale element ids after the displayed geometry changed).
pub fn try_at(p: &[f32], i: u32) -> Option<[f32; 3]> { let k = i as usize * 3; (k + 3 <= p.len()).then(|| [p[k], p[k + 1], p[k + 2]]) }

/// Nearest front-most triangle along the ray (Moller-Trumbore, both sides).
pub fn pick_triangle(p: &[f32], tri: &[u32], origin: [f32; 3], dir: [f32; 3]) -> Option<Hit> {
    pick_triangle_filtered(p,tri,origin,dir,|_| true)
}
/// Filter before ray testing so an unpickable foreground ghost cannot block a selected object.
pub fn pick_triangle_filtered(p:&[f32],tri:&[u32],origin:[f32;3],dir:[f32;3],visible:impl FnMut(&[u32])->bool)->Option<Hit> {
    pick_triangle_culled(p,tri,origin,dir,false,visible)
}
/// As `pick_triangle_filtered`; with `cull` triangles facing away from the viewer (normal . dir >= 0) are skipped.
pub fn pick_triangle_culled(p:&[f32],tri:&[u32],origin:[f32;3],dir:[f32;3],cull:bool,mut visible:impl FnMut(&[u32])->bool)->Option<Hit> {
    let mut best: Option<Hit> = None;
    for (n, t) in tri.as_chunks::<3>().0.iter().enumerate() {
        if !visible(t) { continue; }
        let (a, b, c) = (at(p, t[0]), at(p, t[1]), at(p, t[2]));
        let (e1, e2) = (sub(b, a), sub(c, a));
        if cull && dot(cross(e1, e2), dir) >= 0. { continue; }
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
