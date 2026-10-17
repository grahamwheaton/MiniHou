//! Small vector helpers and the screen projection shared by every inference stage.
pub type V3 = [f32; 3];

pub fn add(a: V3, b: V3) -> V3 { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
pub fn sub(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
pub fn mul(a: V3, t: f32) -> V3 { [a[0] * t, a[1] * t, a[2] * t] }
pub fn dot(a: V3, b: V3) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
pub fn cross(a: V3, b: V3) -> V3 { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
pub fn len(a: V3) -> f32 { dot(a, a).sqrt() }
pub fn dist(a: V3, b: V3) -> f32 { len(sub(a, b)) }
pub fn lerp(a: V3, b: V3, t: f32) -> V3 { add(a, mul(sub(b, a), t)) }
pub fn norm(a: V3) -> Option<V3> { let l = len(a); (l > 1e-12 && l.is_finite()).then(|| mul(a, 1. / l)) }

/// Ray `(origin, unit direction)`.
pub type Ray = (V3, V3);

/// Parameters `(s on line a, t on line b)` of the closest points of two infinite lines.
/// None when they are parallel.
pub fn line_line(a0: V3, ad: V3, b0: V3, bd: V3) -> Option<(f32, f32)> {
    let r = sub(a0, b0);
    let (aa, ab, bb) = (dot(ad, ad), dot(ad, bd), dot(bd, bd));
    let (ar, br) = (dot(ad, r), dot(bd, r));
    let den = aa * bb - ab * ab;
    if den.abs() <= 1e-12 * aa * bb { return None; }
    Some(((ab * br - bb * ar) / den, (aa * br - ab * ar) / den))
}

/// Closest point on the segment `[a,b]` to the ray: `(segment param in 0..1, ray param)`.
pub fn segment_ray(a: V3, b: V3, ray: Ray) -> (f32, f32) {
    let d = sub(b, a);
    let s = match line_line(a, d, ray.0, ray.1) { Some((s, _)) => s.clamp(0., 1.), None => 0. };
    let p = lerp(a, b, s);
    (s, dot(sub(p, ray.0), ray.1))
}

/// Möller-Trumbore. Returns `(t, u, v)` with barycentrics `(1-u-v, u, v)`.
pub fn ray_triangle(ray: Ray, a: V3, b: V3, c: V3) -> Option<(f32, f32, f32)> {
    let (e1, e2) = (sub(b, a), sub(c, a));
    let p = cross(ray.1, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-12 { return None; }
    let inv = 1. / det;
    let s = sub(ray.0, a);
    let u = dot(s, p) * inv;
    if !(-1e-6..=1. + 1e-6).contains(&u) { return None; }
    let q = cross(s, e1);
    let v = dot(ray.1, q) * inv;
    if v < -1e-6 || u + v > 1. + 1e-6 { return None; }
    let t = dot(e2, q) * inv;
    (t > 0.).then_some((t, u, v))
}

/// Same convention as `ui::transform_gizmo::project`: column-major `m[col][row]`, the matrix
/// carries no aspect ratio, screen x is scaled by the viewport height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projector { pub matrix: [[f32; 4]; 4], pub rect_min: [f32; 2], pub rect_size: [f32; 2] }
impl Projector {
    fn center(&self) -> [f32; 2] { [self.rect_min[0] + self.rect_size[0] * 0.5, self.rect_min[1] + self.rect_size[1] * 0.5] }
    pub fn project(&self, p: V3) -> Option<[f32; 2]> {
        let m = self.matrix;
        let q = [p[0], p[1], p[2], 1.];
        let clip = [0, 1, 2, 3].map(|row| (0..4).map(|col| m[col][row] * q[col]).sum::<f32>());
        if clip[3] <= 1e-5 { return None; }
        let c = self.center();
        let aspect = self.rect_size[0] / self.rect_size[1].max(1.);
        Some([c[0] + clip[0] / clip[3] / aspect * self.rect_size[0] * 0.5, c[1] - clip[1] / clip[3] * self.rect_size[1] * 0.5])
    }
    /// Ray through a screen position by inverting the matrix (used by tests and tools without a ray).
    pub fn ray(&self, screen: [f32; 2]) -> Option<Ray> {
        let c = self.center(); let h = self.rect_size[1].max(1.);
        let (nx, ny) = ((screen[0] - c[0]) * 2. / h, (c[1] - screen[1]) * 2. / h);
        let inv = invert(self.matrix)?;
        let un = |z: f32| {
            let q = [nx, ny, z, 1.];
            let w = [0, 1, 2, 3].map(|row| (0..4).map(|col| inv[col][row] * q[col]).sum::<f32>());
            (w[3].abs() > 1e-12).then(|| [w[0] / w[3], w[1] / w[3], w[2] / w[3]])
        };
        let (a, b) = (un(-0.5)?, un(0.5)?);
        Some((a, norm(sub(b, a))?))
    }
}
pub fn dist2(a: [f32; 2], b: [f32; 2]) -> f32 { ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt() }

pub fn invert(m: [[f32; 4]; 4]) -> Option<[[f32; 4]; 4]> {
    let mut a = [[0f64; 8]; 4];
    for r in 0..4 { for c in 0..4 { a[r][c] = m[c][r] as f64; } a[r][4 + r] = 1.; }
    for col in 0..4 {
        let piv = (col..4).max_by(|&x, &y| a[x][col].abs().total_cmp(&a[y][col].abs()))?;
        if a[piv][col].abs() < 1e-14 { return None; }
        a.swap(col, piv);
        let d = a[col][col];
        for k in 0..8 { a[col][k] /= d; }
        for r in 0..4 { if r != col { let f = a[r][col]; for k in 0..8 { a[r][k] -= f * a[col][k]; } } }
    }
    let mut out = [[0f32; 4]; 4];
    for r in 0..4 { for c in 0..4 { out[c][r] = a[r][4 + c] as f32; } }
    Some(out)
}

/// Column-major look-at * perspective (no aspect, matching `Projector`), for tests and tools.
pub fn view_projection(eye: V3, target: V3, up: V3, fov_y: f32, ortho_half_height: Option<f32>) -> [[f32; 4]; 4] {
    let f = norm(sub(target, eye)).unwrap_or([0., 0., -1.]);
    let s = norm(cross(f, up)).unwrap_or([1., 0., 0.]);
    let u = cross(s, f);
    let view = [[s[0], u[0], -f[0], 0.], [s[1], u[1], -f[1], 0.], [s[2], u[2], -f[2], 0.], [-dot(s, eye), -dot(u, eye), dot(f, eye), 1.]];
    let (near, far) = (0.01f32, 1000f32);
    let proj = if let Some(h) = ortho_half_height {
        [[1. / h, 0., 0., 0.], [0., 1. / h, 0., 0.], [0., 0., -2. / (far - near), 0.], [0., 0., -(far + near) / (far - near), 1.]]
    } else {
        let t = 1. / (fov_y * 0.5).tan();
        [[t, 0., 0., 0.], [0., t, 0., 0.], [0., 0., (far + near) / (near - far), -1.], [0., 0., 2. * far * near / (near - far), 0.]]
    };
    let mut out = [[0f32; 4]; 4];
    for c in 0..4 { for r in 0..4 { out[c][r] = (0..4).map(|k| proj[k][r] * view[c][k]).sum(); } }
    out
}
