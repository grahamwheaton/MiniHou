#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SketchMode { Line, Pen, Polygon, Rectangle, Circle, Tape, Bezier, Arc }
impl SketchMode {
    /// The drawing modes of the old sketch panel, in its button order (Tape is the Measure tool).
    pub const DRAW: [Self; 5] = [Self::Line, Self::Pen, Self::Polygon, Self::Rectangle, Self::Circle];
    /// Command id that starts this mode (registered in `keymap.rs`, unbound by default like the old build).
    pub fn command_id(self) -> &'static str { match self { Self::Line => "sketch_line", Self::Pen => "sketch_polyline", Self::Polygon => "sketch_polygon", Self::Rectangle => "sketch_rectangle", Self::Circle => "sketch_circle", Self::Tape => "sketch_measure", Self::Bezier => "sketch_bezier", Self::Arc => "sketch_arc" } }
    pub fn name(self) -> &'static str { match self { Self::Line => "Line", Self::Pen => "Polyline", Self::Polygon => "Polygon", Self::Rectangle => "Rectangle", Self::Circle => "Circle", Self::Tape => "Measure", Self::Bezier => "Bezier", Self::Arc => "Arc" } }
    pub fn tool(self) -> super::super::Tool { match self {
        Self::Line => super::super::Tool::Line, Self::Pen => super::super::Tool::Polyline,
        Self::Polygon => super::super::Tool::Polygon, Self::Rectangle => super::super::Tool::Rectangle,
        Self::Circle => super::super::Tool::Circle, Self::Tape => super::super::Tool::Measure,
        Self::Bezier | Self::Arc => super::super::Tool::Polyline,
    } }
    pub fn from_tool(tool: super::super::Tool) -> Option<Self> {
        Self::DRAW.into_iter().chain([Self::Tape]).find(|m| m.tool() == tool)
    }
    pub(super) fn short(self) -> &'static str { match self { Self::Line => "Line", Self::Pen => "Poly", Self::Polygon => "Polygon", Self::Rectangle => "Rect", Self::Circle => "Circle", Self::Tape => "Tape", Self::Bezier => "Bezier", Self::Arc => "Arc" } }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Plane { #[default] XY, XZ, YZ }
impl Plane {
    pub fn basis(self) -> ([f32; 3], [f32; 3], [f32; 3]) { match self {
        Self::XY => ([1.,0.,0.], [0.,1.,0.], [0.,0.,1.]),
        Self::XZ => ([1.,0.,0.], [0.,0.,1.], [0.,-1.,0.]),
        Self::YZ => ([0.,1.,0.], [0.,0.,1.], [1.,0.,0.]),
    } }
    pub(super) fn label(self) -> &'static str { match self { Self::XY => "XY", Self::XZ => "XZ", Self::YZ => "YZ" } }
}
pub(super) fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [0,1,2].map(|i| a[i] + b[i]) }
pub(super) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [0,1,2].map(|i| a[i] - b[i]) }
pub(super) fn mul(a: [f32; 3], t: f32) -> [f32; 3] { a.map(|x| x * t) }
pub(super) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 { (0..3).map(|i| a[i] * b[i]).sum() }
pub(super) fn length(a: [f32; 3]) -> f32 { dot(a, a).sqrt() }
pub fn grid_snap(p: [f32; 3], plane: Plane, step: f32) -> [f32; 3] {
    if step <= 0. { return p; }
    let (_, _, n) = plane.basis();
    [0,1,2].map(|i| if n[i] != 0. { p[i] } else { (p[i] / step).round() * step })
}
pub fn axis_lock(p: [f32; 3], anchor: [f32; 3], axis: usize, plane: Plane) -> [f32; 3] {
    let (_, _, n) = plane.basis();
    if axis > 2 || n[axis] != 0. { return p; }
    [0,1,2].map(|i| if i == axis { p[i] } else { anchor[i] })
}
pub fn numeric_length(p: [f32; 3], anchor: [f32; 3], amount: f32) -> [f32; 3] {
    let d = sub(p, anchor); let len = length(d);
    if len < 1e-8 || amount <= 0. { p } else { add(anchor, mul(d, amount / len)) }
}
pub fn shape(mode: SketchMode, points: &[[f32; 3]], plane: Plane, segments: usize) -> Vec<[f32; 3]> {
    if points.len() < 2 { return points.to_vec(); }
    let (u, v, _) = plane.basis(); let a = points[0]; let d = sub(*points.last().unwrap(), a);
    match mode {
        SketchMode::Rectangle => { let x = mul(u, dot(d, u)); let y = mul(v, dot(d, v)); vec![a, add(a, x), add(add(a, x), y), add(a, y)] }
        SketchMode::Circle => { let r = length(d); (0..segments.clamp(3, 256)).map(|i| { let t = i as f32 * std::f32::consts::TAU / segments.clamp(3, 256) as f32; add(a, add(mul(u, r * t.cos()), mul(v, r * t.sin()))) }).collect() }
        SketchMode::Arc => {
            let mid = mul(add(a, *points.last().unwrap()), 0.5);
            let chord = sub(*points.last().unwrap(), a);
            let r = length(chord) * 0.5;
            if r < 1e-7 { return points.to_vec(); }
            let chord_dir = mul(chord, 1.0 / (r * 2.0));
            let (_, _, n) = plane.basis();
            let perp = [
                n[1] * chord_dir[2] - n[2] * chord_dir[1],
                n[2] * chord_dir[0] - n[0] * chord_dir[2],
                n[0] * chord_dir[1] - n[1] * chord_dir[0],
            ];
            let n_segs = segments.clamp(4, 64);
            (0..=n_segs).map(|i| {
                let angle = std::f32::consts::PI * (1.0 - i as f32 / n_segs as f32);
                add(mid, add(mul(chord_dir, -r * angle.cos()), mul(perp, r * angle.sin())))
            }).collect()
        }
        _ => points.to_vec(),
    }
}
pub fn specification(mode: SketchMode, points: &[[f32; 3]], plane: Plane) -> Option<(Vec<[f32; 3]>, bool)> {
    let closed = matches!(mode, SketchMode::Polygon | SketchMode::Rectangle | SketchMode::Circle);
    let points = shape(mode, points, plane, 32);
    if points.len() < if closed { 3 } else { 2 } || points.iter().flatten().any(|x| !x.is_finite()) { return None; }
    if points.iter().enumerate().any(|(i,p)| points[..i].iter().any(|q| length(sub(*p,*q)) < 1e-7)) { return None; }
    if closed {
        let (u,v,n) = plane.basis(); let first = points[0]; let mut area = 0.;
        for (i,p) in points.iter().enumerate() {
            if dot(sub(*p,first),n).abs() > 1e-5 { return None; }
            let q = points[(i+1)%points.len()]; area += dot(*p,u)*dot(q,v) - dot(*p,v)*dot(q,u);
        }
        if area.abs() < 1e-8 { return None; }
        let xy: Vec<[f32; 2]> = points.iter().map(|p| [dot(*p,u), dot(*p,v)]).collect();
        if outline_crosses(&xy) { return None; }
    }
    Some((points,closed))
}
fn outline_crosses(points: &[[f32; 2]]) -> bool {
    let cross = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| (b[0]-a[0])*(c[1]-a[1]) - (b[1]-a[1])*(c[0]-a[0]);
    for i in 0..points.len() {
        for j in i+2..points.len() {
            if i == 0 && j == points.len()-1 { continue; }
            let (a,b,c,d) = (points[i], points[(i+1)%points.len()], points[j], points[(j+1)%points.len()]);
            if cross(a,b,c)*cross(a,b,d) < 0. && cross(c,d,a)*cross(c,d,b) < 0. { return true; }
        }
    }
    false
}
fn plane_hit(ray: ([f32; 3], [f32; 3]), plane: Plane, origin: [f32; 3]) -> Option<[f32; 3]> {
    let (_, _, n) = plane.basis(); let denom = dot(ray.1, n);
    if denom.abs() < 1e-5 { return None; }
    let t = dot(sub(origin, ray.0), n) / denom;
    (t >= 0.).then(|| add(ray.0, mul(ray.1, t)))
}
/// Evaluated curve topology is what the viewport draws, including sampled Beziers.
pub(super) fn curve_snap_geometry(g: &minihou_core::Geometry) -> (Vec<u32>, Vec<[f32; 3]>) {
    let t = &g.topology; let mut lines = Vec::new(); let mut centers = Vec::new();
    let p = |id: i32| { let i = id as usize * 3; [g.p()[i], g.p()[i + 1], g.p()[i + 2]] };
    for prim in 0..t.nprims() {
        let marked = g.prim.get("__viewport_curve").and_then(|a| a.as_i32()).is_some_and(|v| v.get(prim) == Some(&1));
        if !marked && t.is_closed(prim) { continue; }
        let ids = &t.vertex_point[t.prim_offsets[prim] as usize..t.prim_offsets[prim + 1] as usize];
        for pair in ids.windows(2) { lines.extend([pair[0] as u32, pair[1] as u32]); }
        if t.is_closed(prim) && ids.len() > 2 { lines.extend([*ids.last().unwrap() as u32, ids[0] as u32]); }
        if ids.len() < 8 { continue; }
        let (a,b,c) = (p(ids[0]), p(ids[ids.len()/3]), p(ids[2*ids.len()/3]));
        let u = sub(b,a); let v = sub(c,a);
        let cross = |x:[f32;3],y:[f32;3]| [x[1]*y[2]-x[2]*y[1],x[2]*y[0]-x[0]*y[2],x[0]*y[1]-x[1]*y[0]];
        let w = cross(u,v); let den = 2. * dot(w,w);
        if den < 1e-10 { continue; }
        let center = add(a,mul(add(mul(cross(w,u),dot(v,v)),mul(cross(v,w),dot(u,u))),1./den));
        let radius = length(sub(a,center));
        if radius > 1e-6 && ids.iter().all(|&id| (length(sub(p(id),center))-radius).abs() <= radius*0.01) { centers.push(center); }
    }
    (lines, centers)
}
