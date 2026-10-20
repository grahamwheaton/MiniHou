type M4 = [[f32; 4]; 4];

fn mul(a: &M4, b: &M4) -> M4 {
    let mut r = [[0.0; 4]; 4];
    for c in 0..4 {
        for row in 0..4 {
            r[c][row] = (0..4).map(|k| a[k][row] * b[c][k]).sum();
        }
    }
    r
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt().max(1e-20);
    [a[0] / l, a[1] / l, a[2] / l]
}

fn look_at(eye: [f32; 3], at: [f32; 3], up: [f32; 3]) -> M4 {
    let f = norm(sub(at, eye));
    let s = norm(cross(f, up));
    let u = cross(s, f);
    [
        [s[0], u[0], -f[0], 0.0],
        [s[1], u[1], -f[1], 0.0],
        [s[2], u[2], -f[2], 0.0],
        [-dot(s, eye), -dot(u, eye), dot(f, eye), 1.0],
    ]
}

/// Right-handed, depth 0..1 (wgpu).
/// Orthographic projection that shows the same height as `perspective(fovy)` does at distance `dist`.
fn orthographic(fovy: f32, dist: f32, aspect: f32, near: f32, far: f32) -> M4 {
    let h = dist * (fovy / 2.0).tan();
    let w = h * aspect;
    [
        [1.0 / w, 0.0, 0.0, 0.0],
        [0.0, 1.0 / h, 0.0, 0.0],
        [0.0, 0.0, 1.0 / (near - far), 0.0],
        [0.0, 0.0, near / (near - far), 1.0],
    ]
}

fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> M4 {
    let t = 1.0 / (fovy / 2.0).tan();
    [
        [t / aspect, 0.0, 0.0, 0.0],
        [0.0, t, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, near * far / (near - far), 0.0],
    ]
}

/// The views on the numpad (perspective keeps the current angles).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StandardView { Perspective, Front, Back, Right, Left, Top, Bottom }

pub struct Camera {
    target: [f32; 3],
    dist: f32,
    yaw: f32,
    pitch: f32,
    radius: f32,
    ortho: bool,
    clip_start: f32,
    clip_end: f32,
}

/// Keep persisted and interactive clipping values finite, positive and ordered.
pub fn sanitize_clip_planes(near: f32, far: f32) -> (f32, f32) {
    let near = if near.is_finite() { near.clamp(1e-6, 1e8) } else { 0.01 };
    let far = if far.is_finite() { far } else { 1000.0 };
    (near, far.clamp(near * 1.001, 1e9))
}

#[cfg(test)] mod fly_tests {
    use super::*;
    fn close(a: [f32; 3], b: [f32; 3]) { for i in 0..3 { assert!((a[i]-b[i]).abs()<1e-4, "{a:?} != {b:?}"); } }
    #[test] fn look_keeps_eye_and_orbit_distance() {
        let mut c = Camera::frame([1., 2., 3.], 2.);
        let eye = c.eye(); let distance = c.distance();
        let right = [c.yaw.cos(), c.yaw.sin(), 0.];
        c.fly([100., -80.], [0.; 3], 0.);
        assert!(dot(c.forward(), right) > 0., "moving the mouse right must look right");
        close(c.eye(), eye); assert_eq!(c.distance(), distance);
        close(sub(c.target, eye), c.forward().map(|x| x * distance));
    }
    #[test] fn fly_uses_view_forward_world_z_and_normalized_diagonals() {
        let mut c = Camera::frame([0.; 3], 2.);
        let eye = c.eye(); let forward = c.forward();
        c.fly([0.; 2], [1., 0., 0.], 2.);
        close(c.eye(), [0,1,2].map(|i| eye[i]+forward[i]*2.));
        let eye = c.eye(); c.fly([0.;2], [0.,0.,1.], 3.);
        close(c.eye(), [eye[0],eye[1],eye[2]+3.]);
        let eye = c.eye(); c.fly([0.;2], [1.,1.,1.], 4.);
        let delta = sub(c.eye(), eye); assert!((dot(delta,delta).sqrt()-4.).abs()<1e-4);
        c.fly([0., 1e6], [0.;3], 0.);
        assert!(c.pitch < std::f32::consts::FRAC_PI_2);
        assert!(c.forward().iter().all(|v| v.is_finite()));
    }
}

impl Camera {
    /// Distance from the eye to the orbit target (world units); sizes the viewport's edge fade.
    pub fn distance(&self) -> f32 {
        self.dist
    }
    pub fn clip_planes(&self) -> (f32, f32) {
        (self.clip_start, self.clip_end)
    }
    pub fn lod_camera(&self, size: [u32; 2]) -> minihou_core::lod::Camera {
        let (view_projection, _) = self.matrices(size[0].max(1) as f32 / size[1].max(1) as f32);
        let (near, _) = self.clip_planes();
        minihou_core::lod::Camera { view_projection, eye: self.eye(), viewport_height_px: size[1] as f32,
            near, orthographic: self.ortho }
    }
    pub fn frame(center: [f32; 3], radius: f32) -> Self {
        let radius = radius.max(1e-3); Camera { target: center, dist: radius * 2.6, yaw: 0.6, pitch: 0.5, radius, ortho: false, clip_start: 0.01, clip_end: 1000.0 }
    }
    /// Explicit eye-space clipping distances in world units, independent of framing and zoom.
    pub fn set_clip_planes(&mut self, near: f32, far: f32) {
        (self.clip_start, self.clip_end) = sanitize_clip_planes(near, far);
    }
    pub fn set_dist(&mut self, dist: f32) {
        self.dist = dist;
    }

    pub fn eye(&self) -> [f32; 3] {
        // +Z up (Blender); yaw 0 looks along +Y from -Y (Front view)
        let (cp, sp) = (self.pitch.cos(), self.pitch.sin());
        [
            self.target[0] + self.dist * cp * self.yaw.sin(),
            self.target[1] - self.dist * cp * self.yaw.cos(),
            self.target[2] + self.dist * sp,
        ]
    }

    pub fn forward(&self) -> [f32; 3] {
        norm(sub(self.target, self.eye()))
    }

    /// First-person look and motion, keeping the orbit distance and target attached to the eye.
    /// Motion is [forward, right, world Z], normalized to avoid faster diagonal flight.
    pub fn fly(&mut self, look: [f32; 2], motion: [f32; 3], distance: f32) {
        let eye = self.eye();
        self.ortho = false;
        self.yaw -= look[0] * 0.003;
        self.pitch = (self.pitch + look[1] * 0.003).clamp(-1.553343, 1.553343);
        let forward = [-self.pitch.cos() * self.yaw.sin(), self.pitch.cos() * self.yaw.cos(), -self.pitch.sin()];
        let right = [self.yaw.cos(), self.yaw.sin(), 0.];
        let delta = [0, 1, 2].map(|i| forward[i] * motion[0] + right[i] * motion[1] + if i == 2 { motion[2] } else { 0. });
        let length = dot(delta, delta).sqrt();
        self.target = [0, 1, 2].map(|i| eye[i] + if length > 0. { delta[i] / length * distance } else { 0. } + forward[i] * self.dist);
    }

    /// World up for the view: flips once the orbit has gone over the pole, so the camera rolls on through the zenith (turntable, as Blender).
    fn up(&self) -> [f32; 3] { [0.0, 0.0, if self.pitch.cos() < 0.0 { -1.0 } else { 1.0 }] }
    fn add_orbit(&mut self, dyaw: f32, dpitch: f32) {
        // upside down, horizontal drags turn the other way so the model still follows the mouse
        let sign = if self.pitch.cos() < 0.0 { -1.0 } else { 1.0 };
        self.yaw += sign * dyaw;
        let tau = std::f32::consts::TAU;
        self.pitch = (self.pitch + dpitch + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI;
    }
    pub fn matrices(&self, aspect: f32) -> (M4, M4) {
        let view = look_at(self.eye(), self.target, self.up());
        let (near, far) = self.clip_planes();
        let proj = if self.ortho { orthographic(0.8, self.dist, aspect, near, far) } else { perspective(0.8, aspect, near, far) };
        (mul(&proj, &view), view)
    }
}


impl Camera {
    /// Snap to one of Blender's standard views (+Z is up; Front looks along +Y). Top and bottom stop just short of the pole.
    pub fn set_view(&mut self, view: crate::StandardView) {
        use crate::StandardView as V;
        const POLE: f32 = 1.5698;
        let (yaw, pitch, ortho) = match view {
            V::Perspective => (self.yaw, self.pitch, false),
            V::Front => (0., 0., true),
            V::Back => (std::f32::consts::PI, 0., true),
            V::Right => (std::f32::consts::FRAC_PI_2, 0., true),
            V::Left => (-std::f32::consts::FRAC_PI_2, 0., true),
            V::Top => (0., POLE, true),
            V::Bottom => (0., -POLE, true),
        };
        self.yaw = yaw; self.pitch = pitch; self.ortho = ortho;
    }
    pub fn is_ortho(&self) -> bool { self.ortho }
    /// Blender's numpad 5: switch projection, keeping the current angles.
    pub fn toggle_ortho(&mut self) { self.ortho = !self.ortho; }
    /// The world-space ray through a point of the viewport: `nx`, `ny` in -1..1 (right, up), `aspect` = width / height.
    pub fn ray(&self, nx: f32, ny: f32, aspect: f32) -> ([f32; 3], [f32; 3]) {
        let eye = self.eye();
        let f = norm(sub(self.target, eye));
        let right = norm(cross(f, self.up()));
        let up = cross(right, f);
        let tan_half = (0.8_f32 / 2.0).tan();
        if self.ortho {
            let h = self.dist * tan_half;
            let o = [0, 1, 2].map(|k| eye[k] + right[k] * nx * aspect * h + up[k] * ny * h);
            (o, f)
        } else {
            let d = [0, 1, 2].map(|k| f[k] + right[k] * nx * aspect * tan_half + up[k] * ny * tan_half);
            (eye, norm(d))
        }
    }
    /// Orbit angles in radians (yaw about +Z, pitch up) for the view gizmo.
    pub fn angles(&self) -> (f32, f32) { (self.yaw, self.pitch) }
    pub fn orbit_degrees(&mut self, yaw: f32, pitch: f32) {
        self.ortho = false;
        self.add_orbit(-yaw.to_radians(), pitch.to_radians());
    }
    pub fn zoom_factor(&mut self, factor: f32) { if factor.is_finite() && factor > 0.0 { self.dist = (self.dist / factor).clamp(self.radius * 0.05, self.radius * 60.0); } }
    pub fn orbit(&mut self, dx: f32, dy: f32) { self.ortho = false; self.add_orbit(-dx * 0.008, dy * 0.008); }
    /// Orbit while keeping a world point at its original screen coordinates.
    pub fn orbit_at(&mut self, dx: f32, dy: f32, point: [f32; 3], nx: f32, ny: f32, aspect: f32) {
        self.orbit(dx, dy);
        let (origin, direction) = self.ray(nx, ny, aspect);
        let forward = self.forward();
        let depth = dot(sub(point, origin), forward) / dot(direction, forward).max(1e-6);
        self.target = [0, 1, 2].map(|k| self.target[k] + point[k] - origin[k] - direction[k] * depth);
    }
    pub fn pan(&mut self, dx: f32, dy: f32) {
        let f = norm(sub(self.target,self.eye())); let s = norm(cross(f,self.up())); let u = cross(s,f);
        for i in 0..3 { self.target[i] += (-s[i]*dx+u[i]*dy)*self.dist*0.0011; }
    }
    pub fn zoom(&mut self, delta: f32) { self.dist = (self.dist * (-delta * 0.0025).exp()).clamp(self.radius*0.002,self.radius*10000.); }

    /// Zoom about a surface (or view-plane) point, keeping its projection fixed.
    pub fn zoom_at(&mut self, delta: f32, point: [f32; 3], nx: f32, ny: f32, aspect: f32) {
        if !delta.is_finite() || delta == 0. { return; }
        let old_dist = self.dist;
        let old_origin = self.ray(nx, ny, aspect).0;
        self.zoom(delta);
        if self.ortho {
            let new_origin = self.ray(nx, ny, aspect).0;
            self.target = [0, 1, 2].map(|k| self.target[k] + old_origin[k] - new_origin[k]);
        } else {
            let fraction = self.dist / old_dist;
            self.target = [0, 1, 2].map(|k| point[k] + (self.target[k] - point[k]) * fraction);
        }
    }

    /// The point at the current view-target depth when the cursor misses geometry.
    pub fn view_plane_point(&self, nx: f32, ny: f32, aspect: f32) -> [f32; 3] {
        let (origin, direction) = self.ray(nx, ny, aspect);
        let depth = dot(sub(self.target, origin), self.forward()) / dot(direction, self.forward()).max(1e-6);
        [0, 1, 2].map(|k| origin[k] + direction[k] * depth)
    }

}

#[cfg(test)] mod navigation_tests {
    use super::*;
    fn screen(c:&Camera,p:[f32;3])->[f32;2] {
        let m=c.matrices(1.5).0;
        let q=[p[0],p[1],p[2],1.];
        let clip=[0,1,3].map(|row|(0..4).map(|col|m[col][row]*q[col]).sum::<f32>());
        [clip[0]/clip[2],clip[1]/clip[2]]
    }
    fn close(a:[f32;2],b:[f32;2]) {assert!((a[0]-b[0]).abs()<2e-4 && (a[1]-b[1]).abs()<2e-4,"{a:?} {b:?}");}
    #[test] fn zoom_to_point_keeps_cursor_fixed_in_both_projections_and_clamps() {
        for ortho in [false,true] {
            let mut c=Camera::frame([0.;3],2.);
            if ortho {c.toggle_ortho();}
            let p=c.view_plane_point(0.4,-0.3,1.5);
            let before=screen(&c,p);
            c.zoom_at(130.,p,0.4,-0.3,1.5); close(before,screen(&c,p));
            c.zoom_at(-1e6,p,0.4,-0.3,1.5); close(before,screen(&c,p));
            assert!(c.distance().is_finite() && c.distance()<=c.radius*10000.);
        }
    }
    #[test] fn old_zoom_stays_centred_when_mouse_preference_is_off() {
        let mut c=Camera::frame([0.;3],2.); let target=c.target;
        c.zoom(100.);
        assert_eq!(c.target,target);
    }
    #[test] fn orbit_point_stays_on_screen_through_yaw_pitch_and_zenith() {
        let mut c=Camera::frame([0.;3],2.);
        let p=c.view_plane_point(0.42,-0.31,1.5);
        let before=screen(&c,p);
        for (dx,dy) in [(80.,20.),(-35.,150.),(14.,90.),(-110.,-30.)] {
            c.orbit_at(dx,dy,p,0.42,-0.31,1.5);
            close(before,screen(&c,p));
        }
        assert!(c.angles().1.cos()<0.);
    }
    #[test] fn orbit_fallback_target_stays_centred() {
        let mut c=Camera::frame([1.,2.,3.],2.);
        let target=c.target;
        c.orbit_at(80.,220.,target,0.,0.,1.5);
        close([0.,0.],screen(&c,target));
    }
}
