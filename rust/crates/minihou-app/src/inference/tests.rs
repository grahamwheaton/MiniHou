//! Headless acceptance tests (spec section 13) driving the engine with synthetic cameras/geometry.
//! Geometric checks use world tolerances (1e-5); acquisition uses pixel tolerances.
use super::*;

fn cam(eye: V3, target: V3, up: V3) -> Projector { Projector { matrix: math::view_projection(eye, target, up, 0.8, None), rect_min: [0., 0.], rect_size: [1000., 800.] } }
fn persp() -> Projector { cam([4., -9., 7.], [1.5, 1., 0.], [0., 0., 1.]) }
fn top() -> Projector { cam([0.3, 0.2, 12.], [0.3, 0.2, 0.], [0., 1., 0.]) }

struct Mesh { p: Vec<f32>, t: Vec<u32> }
impl Mesh {
    fn new() -> Self { Self { p: vec![], t: vec![] } }
    fn tri(mut self, a: V3, b: V3, c: V3) -> Self { let n = self.p.len() as u32 / 3; self.p.extend(a); self.p.extend(b); self.p.extend(c); self.t.extend([n, n + 1, n + 2]); self }
    fn quad(self, a: V3, b: V3, c: V3, d: V3) -> Self {
        let mut m = self; let n = m.p.len() as u32 / 3;
        for q in [a, b, c, d] { m.p.extend(q); }
        m.t.extend([n, n + 1, n + 2, n, n + 2, n + 3]); m
    }
    fn geo(&self) -> GeometryRef<'_> { GeometryRef { positions: &self.p, triangles: &self.t, curve_lines: &[], curve_centers: &[], revision: 0 } }
}
fn cube() -> Mesh {
    let v = |x: f32, y: f32, z: f32| [x, y, z];
    let c = [v(0., 0., 0.), v(1., 0., 0.), v(1., 1., 0.), v(0., 1., 0.), v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(0., 1., 1.)];
    let mut m = Mesh { p: c.iter().flatten().copied().collect(), t: vec![] };
    for f in [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]] { m.t.extend([f[0], f[1], f[2], f[0], f[2], f[3]]); }
    m
}

struct Rig { e: InferenceEngine, s: InferenceSettings, pr: Projector, tool: ToolContext, time: f64, shift: bool, typed: Option<Typed> }
impl Rig {
    fn new(pr: Projector) -> Self { Self { e: InferenceEngine::new(), s: InferenceSettings::default(), pr, tool: ToolContext::new(ToolSpec::LINE), time: 0., shift: false, typed: None } }
    fn at_screen(&mut self, screen: [f32; 2], geo: Option<GeometryRef>) -> InferenceResult {
        let ray = self.pr.ray(screen).unwrap();
        self.time += 0.016;
        self.e.update(&InferenceInput { ray, cursor: screen, projector: self.pr, time: self.time, geometry: geo, tool: self.tool.clone(), settings: &self.s, alt_held: self.shift, typed: self.typed })
    }
    fn near(&mut self, p: V3, offset: [f32; 2], geo: Option<GeometryRef>) -> InferenceResult {
        let s = self.pr.project(p).unwrap();
        self.at_screen([s[0] + offset[0], s[1] + offset[1]], geo)
    }
    fn dwell(&mut self, p: V3, geo: Option<GeometryRef>) { self.near(p, [0., 0.], geo); self.time += 0.5; self.near(p, [0., 0.], geo); }
}
fn close(a: V3, b: V3, eps: f32) -> bool { math::dist(a, b) <= eps }

#[test] fn evaluated_curve_segments_and_centres_share_mesh_snap_ranking() {
    let p = [0.,0.,0., 2.,0.,0., 3.,1.,0.];
    let lines = [0,1,1,2];
    let centers = [[5.,2.,0.]];
    let g = GeometryRef { positions: &p, triangles: &[], curve_lines: &lines, curve_centers: &centers, revision: 1 };
    let mut r = Rig::new(top());
    assert_eq!(r.near([0.,0.,0.],[1.,1.],Some(g)).winner.kind, Kind::Endpoint);
    assert_eq!(r.near([1.,0.,0.],[1.,1.],Some(g)).winner.kind, Kind::Midpoint);
    assert_eq!(r.near([1.35,0.,0.],[0.,0.],Some(g)).winner.kind, Kind::OnEdge);
    assert_eq!(r.near(centers[0],[0.,0.],Some(g)).winner.kind, Kind::Centre);
}

#[test] fn t01_axis_snap_roughly_along_x() {
    let mut r = Rig::new(persp());
    r.tool.start = Some([0.; 3]);
    let res = r.near([2.5, 0., 0.], [2., 3.], None);
    assert_eq!(res.winner.kind, Kind::Axis(0), "{}", res.reason);
    let p = res.point.unwrap();
    assert!(p[1].abs() < 1e-5 && p[2].abs() < 1e-5 && p[0] > 2., "{p:?}");
}

#[test] fn t02_axis_lock_diagonal_cursor_commits_exact_x() {
    let mut r = Rig::new(persp());
    r.tool.start = Some([0.5, 0.5, 0.]);
    r.e.toggle_axis_lock(0);
    let res = r.near([2., 2., 0.], [0., 0.], None);
    assert_eq!(res.winner.kind, Kind::Locked(0));
    let p = res.point.unwrap();
    assert_eq!([p[1], p[2]], [0.5, 0.], "line must be exactly along X: {p:?}");
    r.e.toggle_axis_lock(0);
    assert!(r.e.active_lock().is_none());
}

#[test] fn t03_midpoint_of_arbitrary_edge() {
    let m = Mesh::new().tri([0., 0., 0.], [2., 1., 0.5], [0., 2., 0.2]);
    let mut r = Rig::new(persp());
    let res = r.near([1., 0.5, 0.25], [3., -2.], Some(m.geo()));
    assert_eq!(res.winner.kind, Kind::Midpoint, "{}", res.reason);
    assert!(close(res.point.unwrap(), [1., 0.5, 0.25], 1e-6));
    assert!(matches!(res.descriptor, Some(RefDescriptor::Edge { t, .. }) if (t - 0.5).abs() < 1e-6));
}

fn diag() -> Mesh { Mesh::new().tri([0., 0., 0.], [1., 1., 0.], [1., 0., 0.]) }

#[test] fn t04_parallel_to_referenced_diagonal_edge() {
    let m = diag();
    let mut r = Rig::new(persp());
    r.near([0.5, 0.5, 0.], [0., 0.], Some(m.geo()));
    r.tool.start = Some([3., -2., 0.]);
    let target = math::add([3., -2., 0.], [1.2, 1.2, 0.]);
    let res = r.near(target, [3., 1.], Some(m.geo()));
    assert_eq!(res.winner.kind, Kind::Parallel, "{}", res.reason);
    let d = math::sub(res.point.unwrap(), [3., -2., 0.]);
    assert!(math::len(math::cross(d, [1., 1., 0.])) < 1e-5, "{d:?}");
    assert!(res.winner.cause.is_some(), "causing edge is highlighted");
    // Down arrow locks it; any cursor stays parallel.
    r.e.lock_inferred();
    let res = r.near([5., 0., 0.], [0., 0.], Some(m.geo()));
    let d = math::sub(res.point.unwrap(), [3., -2., 0.]);
    assert!(math::len(math::cross(d, [1., 1., 0.])) < 1e-5 && res.winner.kind == Kind::Locked(3));
}

#[test] fn t05_perpendicular_in_drawing_plane() {
    let m = diag();
    let mut r = Rig::new(persp());
    r.near([0.5, 0.5, 0.], [0., 0.], Some(m.geo()));
    r.tool.start = Some([3., -2., 0.]);
    let res = r.near(math::add([3., -2., 0.], [-1.1, 1.1, 0.]), [2., 2.], Some(m.geo()));
    assert_eq!(res.winner.kind, Kind::Perpendicular, "{}", res.reason);
    let d = math::sub(res.point.unwrap(), [3., -2., 0.]);
    assert!(math::dot(d, [1., 1., 0.]).abs() < 1e-5 && d[2].abs() < 1e-6, "{d:?}");
    // Working plane from a tilted start face: perpendicular stays inside that plane.
    let n = math::norm([0., -0.5, 1.]).unwrap();
    r.tool.start_face = Some(([3., -2., 0.], n));
    let ed = [1., 1., 0.];
    let proj = math::sub(ed, math::mul(n, math::dot(ed, n)));
    let pd = math::norm(math::cross(n, proj)).unwrap();
    let res = r.near(math::add([3., -2., 0.], math::mul(pd, 1.3)), [1., 1.], Some(m.geo()));
    assert_eq!(res.winner.kind, Kind::Perpendicular, "{}", res.reason);
    let d = math::sub(res.point.unwrap(), [3., -2., 0.]);
    assert!(math::dot(d, n).abs() < 1e-5 && math::dot(d, proj).abs() < 1e-4, "{d:?}");
}

#[test] fn t06_extension_beyond_endpoint() {
    let m = diag();
    let mut r = Rig::new(persp());
    r.near([0.5, 0.5, 0.], [0., 0.], Some(m.geo()));
    let res = r.near([2.2, 2.2, 0.], [2., -1.], Some(m.geo()));
    assert_eq!(res.winner.kind, Kind::Extension, "{}", res.reason);
    let p = res.point.unwrap();
    assert!((p[0] - p[1]).abs() < 1e-5 && p[2].abs() < 1e-6 && p[0] > 1.5, "{p:?}");
}

#[test] fn t07_from_point_alignment_with_box_corner() {
    let m = cube();
    let mut r = Rig::new(persp());
    r.dwell([1., 1., 0.], Some(m.geo()));
    assert_eq!(r.e.references().len(), 1);
    let res = r.near([1., 2.6, 0.], [2., 1.], Some(m.geo()));
    assert_eq!(res.winner.kind, Kind::FromPoint(1), "{}", res.reason);
    let p = res.point.unwrap();
    assert!((p[0] - 1.).abs() < 1e-6 && p[2].abs() < 1e-6, "{p:?}");
    assert!(res.primitives.iter().any(|x| matches!(x, Primitive::Line { dashed: true, .. })), "temporary tracking line is dashed");
    assert!(res.primitives.iter().any(|x| matches!(x, Primitive::Marker { shape: Marker::Dot, .. })), "acquired reference dot");
}

#[test] fn t08_point_from_two_reference_alignments() {
    let m = Mesh::new().tri([0., 0., 0.], [1., 0., 0.], [0., 1., 0.]).tri([4., 4., 0.], [5., 4., 0.], [4., 5., 0.]);
    let mut r = Rig::new(cam([2.5, -6., 9.], [2.5, 2.5, 0.], [0., 0., 1.]));
    r.dwell([1., 0., 0.], Some(m.geo()));
    r.dwell([4., 5., 0.], Some(m.geo()));
    assert_eq!(r.e.references().len(), 2);
    let res = r.near([1., 5., 0.], [2., 2.], Some(m.geo()));
    assert_eq!(res.winner.kind, Kind::AlignIntersection, "{}", res.reason);
    assert!(close(res.point.unwrap(), [1., 5., 0.], 1e-5));
    // And start-point axis x reference alignment.
    r.tool.start = Some([3., 2., 0.]);
    let res = r.near([1., 2., 0.], [2., -2.], Some(m.geo()));
    assert_eq!(res.winner.kind, Kind::AlignIntersection, "{}", res.reason);
    assert!(close(res.point.unwrap(), [1., 2., 0.], 1e-5));
}

#[test] fn t09_angled_face_no_drift() {
    let (c, s) = (30f32.to_radians().cos(), 30f32.to_radians().sin());
    let m = Mesh::new().quad([0., 0., 0.], [4., 0., 0.], [4., 4. * c, 4. * s], [0., 4. * c, 4. * s]);
    let n = [0., -s, c];
    let mut r = Rig::new(persp());
    let first = r.near([1.3, 1.1 * c, 1.1 * s], [0., 0.], Some(m.geo()));
    assert_eq!(first.winner.kind, Kind::OnFace);
    let (start, face) = (first.point.unwrap(), first.face.expect("face plane"));
    r.tool.start = Some(start); r.tool.start_face = Some(face);
    for (u, v, off) in [(2.7, 2.2, [0., 0.]), (3.1, 1.4, [5., -3.]), (1.9, 3.3, [-4., 2.]), (6.0, 2.0, [0., 0.]), (2.0, 7.0, [3., 3.])] {
        let res = r.near([u, v * c, v * s], off, Some(m.geo()));
        let p = res.point.unwrap();
        assert!(math::dot(math::sub(p, start), n).abs() < 1e-4, "{:?} drifted off plane: {p:?} ({})", res.winner.kind, res.reason);
    }
    assert_eq!(r.at_screen([500., 400.], Some(m.geo())).plane.source, PlaneSource::Face);
}

#[test] fn t10_axis_lock_plus_typed_distance() {
    let mut r = Rig::new(persp());
    r.tool.start = Some([1., 1., 0.]);
    r.e.toggle_axis_lock(0);
    r.typed = constraints::parse_typed("2.5m", crate::units::UnitSettings::default());
    let res = r.near([3., 2., 0.], [0., 0.], None);
    assert_eq!(res.point.unwrap(), [3.5, 1., 0.]);
    assert!(res.typed_status.is_none());
    r.typed = constraints::parse_typed("0,3,0", crate::units::UnitSettings::default());
    assert!(matches!(r.typed, Some(Typed::Absolute(_))));
    let res = r.near([3., 2., 0.], [0., 0.], None);
    assert!(res.typed_status.as_deref().unwrap_or("").contains("conflict") || res.typed_status.as_deref().unwrap_or("").contains("off the locked"), "conflict must be reported");
    r.e.toggle_axis_lock(0);
    r.typed = constraints::parse_typed("@0m,0m,2m", crate::units::UnitSettings::default());
    assert_eq!(r.near([3., 2., 0.], [0., 0.], None).point.unwrap(), [1., 1., 2.]);
    r.typed = constraints::parse_typed("2m<90", crate::units::UnitSettings::default());
    assert!(close(r.near([3., 2., 0.], [0., 0.], None).point.unwrap(), [1., 3., 0.], 1e-5));
}

#[test] fn t11_no_flicker_between_overlapping_candidates() {
    let m = Mesh::new().tri([0., 0., 0.], [0.006, 0.0, 0.], [0., -2., 0.]);
    let mut r = Rig::new(persp());
    let (a, b) = (r.pr.project([0., 0., 0.]).unwrap(), r.pr.project([0.006, 0., 0.]).unwrap());
    assert!(math::dist2(a, b) < 6., "candidates overlap on screen");
    let first = r.at_screen(a, Some(m.geo()));
    let mut changes = 0; let mut last = first.winner.point;
    for i in 0..40 {
        let t = 0.5 + if i % 2 == 0 { 0.08 } else { -0.08 };
        let res = r.at_screen([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t], Some(m.geo()));
        if res.winner.point != last { changes += 1; last = res.winner.point; }
    }
    assert_eq!(changes, 0, "winner flickered");
    // Tab cycles to the other overlapping candidate.
    let before = r.at_screen(a, Some(m.geo())).winner.point;
    r.e.cycle_candidates();
    let after = r.at_screen(a, Some(m.geo()));
    assert_ne!(after.winner.point, before);
    assert!(after.alternatives.len() >= 2);
}

#[test] fn t12_projection_only_crossing_is_not_an_intersection() {
    let skew = Mesh::new().tri([-1., 0., 0.], [1., 0., 0.], [0., -0.5, 0.]).tri([0.3, -1., 1.], [0.3, 1., 1.], [0.8, 0., 1.]);
    let mut r = Rig::new(cam([0.3, 0.2, 12.], [0.3, 0.2, 0.], [0., 1., 0.]));
    r.s.include_hidden = true;
    let cross = r.pr.project([0.3, 0., 1.]).unwrap();
    let res = r.at_screen(cross, Some(skew.geo()));
    assert!(res.alternatives.iter().all(|c| c.kind != Kind::Intersection), "false intersection: {:?}", res.alternatives.iter().map(|c| c.kind).collect::<Vec<_>>());
    let real = Mesh::new().tri([-1., 0., 0.], [1., 0., 0.], [0., -0.5, 0.]).tri([0.3, -1., 0.], [0.3, 2., 0.], [0.8, 0., 0.]);
    let mut r = Rig::new(top());
    let res = r.near([0.3, 0., 0.], [1., 1.], Some(real.geo()));
    assert_eq!(res.winner.kind, Kind::Intersection, "{}", res.reason);
    assert!(close(res.point.unwrap(), [0.3, 0., 0.], 1e-5));
}

#[test] fn t13_new_geometry_revision_is_used() {
    let a = diag();
    let mut b = diag(); b.p[3] = 1.5; // move vertex 1 to (1.5,1,0)
    let mut r = Rig::new(persp());
    r.near([1., 1., 0.], [0., 0.], Some(a.geo()));
    r.near([1., 1., 0.], [0., 0.], Some(a.geo()));
    assert_eq!(r.e.accel_builds(), 1, "cached per geometry");
    let res = r.near([1.5, 1., 0.], [0., 0.], Some(b.geo()));
    assert_eq!(r.e.accel_builds(), 2);
    assert_eq!(res.winner.kind, Kind::Endpoint);
    assert!(close(res.point.unwrap(), [1.5, 1., 0.], 1e-6));
    let res = r.near([1., 1., 0.], [0., 0.], Some(b.geo()));
    assert_ne!(res.point.unwrap(), [1., 1., 0.], "old vertex must be gone");
    // Same buffer edited in place with an explicit revision bump.
    let mut c = diag();
    let g = GeometryRef { positions: &c.p, triangles: &c.t, curve_lines: &[], curve_centers: &[], revision: 7 };
    r.near([0., 0., 0.], [0., 0.], Some(g));
    c.p[0] = -0.5;
    let g = GeometryRef { positions: &c.p, triangles: &c.t, curve_lines: &[], curve_centers: &[], revision: 8 };
    let res = r.near([-0.5, 0., 0.], [0., 0.], Some(g));
    assert!(close(res.point.unwrap(), [-0.5, 0., 0.], 1e-6));
}

#[test] fn t14_large_mesh_update_under_budget() {
    // ~1M triangles (708 x 708 quads), bumpy so no edge is a coplanar diagonal.
    let n = 708usize;
    let mut p = Vec::with_capacity((n + 1) * (n + 1) * 3);
    for j in 0..=n { for i in 0..=n { let (x, y) = (i as f32 / n as f32 * 10., j as f32 / n as f32 * 10.); p.extend([x, y, 0.05 * (x * 3.).sin() * (y * 2.).cos()]); } }
    let mut t = Vec::with_capacity(n * n * 6);
    for j in 0..n { for i in 0..n { let a = (j * (n + 1) + i) as u32; let b = a + 1; let c = a + n as u32 + 2; let d = a + n as u32 + 1; t.extend([a, b, c, a, c, d]); } }
    assert!(t.len() / 3 >= 1_000_000);
    let g = GeometryRef { positions: &p, triangles: &t, curve_lines: &[], curve_centers: &[], revision: 1 };
    let mut r = Rig::new(cam([5., -6., 8.], [5., 5., 0.], [0., 0., 1.]));
    r.tool.start = Some([5., 5., 0.]);
    let build = std::time::Instant::now();
    r.at_screen([500., 400.], Some(g));
    let build = build.elapsed();
    let mut times = Vec::new();
    for k in 0..60 {
        let s = [300. + (k * 7 % 400) as f32, 250. + (k * 13 % 300) as f32];
        let t0 = std::time::Instant::now();
        r.at_screen(s, Some(g));
        times.push(t0.elapsed().as_secs_f64() * 1e3);
    }
    assert_eq!(r.e.accel_builds(), 1);
    times.sort_by(f64::total_cmp);
    let median = times[times.len() / 2];
    eprintln!("1M-tri inference: build {build:?}, median update {median:.3} ms, max {:.3} ms", times.last().unwrap());
    // 3 ms is the optimised-build budget; unoptimised test builds (measured ~2 ms) get headroom against CI noise.
    let budget = if cfg!(debug_assertions) { 10. } else { 3. };
    assert!(median < budget, "median {median} ms");
}

#[test] fn tracking_cap_fifo_escape_and_shift_hold() {
    let m = cube();
    let mut r = Rig::new(persp());
    for v in [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 0., 1.], [1., 0., 1.]] { r.dwell(v, Some(m.geo())); }
    let refs: Vec<V3> = r.e.references().iter().map(|x| x.anchor()).collect();
    assert_eq!(refs.len(), 4, "cap");
    assert!(!refs.contains(&[0., 0., 0.]), "oldest replaced first");
    r.e.toggle_axis_lock(2);
    assert!(r.e.escape(), "first Esc clears temporary state");
    assert!(r.e.references().is_empty() && r.e.active_lock().is_none());
    assert!(!r.e.escape(), "second Esc cancels the operation");
    // Shift held locks the current axis inference; release unlocks.
    r.tool.start = Some([3., 3., 0.]);
    let res = r.near([5., 3., 0.], [1., 2.], None);
    assert_eq!(res.winner.kind, Kind::Axis(0));
    r.shift = true;
    let res = r.near([5., 6., 0.], [0., 0.], None);
    assert_eq!(res.winner.kind, Kind::Locked(0));
    assert_eq!(res.point.unwrap()[1], 3.);
    r.shift = false;
    assert_ne!(r.near([5., 6., 0.], [0., 0.], None).winner.kind, Kind::Locked(0));
}

#[test] fn modes_tooltips_hidden_edges_and_descriptors() {
    let mut r = Rig::new(persp());
    r.tool.start = Some([0.; 3]);
    assert_eq!(r.near([2.5, 0., 0.], [2., 3.], None).winner.kind, Kind::Axis(0));
    assert_eq!(r.e.cycle_linear_mode(&r.s), LinearMode::PointsOnly);
    assert_eq!(r.near([2.5, 0., 0.], [2., 3.], None).winner.kind, Kind::Free);
    r.e.set_linear_mode(LinearMode::All);
    // Tooltip only after the delay.
    let m = cube();
    let mut r = Rig::new(persp());
    let tip = |res: &InferenceResult| res.primitives.iter().any(|p| matches!(p, Primitive::Tooltip { text, .. } if text == "Endpoint"));
    assert!(!tip(&r.near([1., 0., 0.], [1., 1.], Some(m.geo()))));
    r.time += 0.3;
    let res = r.near([1., 0., 0.], [1., 1.], Some(m.geo()));
    assert!(tip(&res));
    assert_eq!(res.descriptor, Some(RefDescriptor::Vertex { point: 1 }));
    assert_eq!(res.descriptor.unwrap().to_json("box1", 8)["kind"], "vertex");
    // The cube's face diagonals (0-2 on the bottom) are hidden: no midpoint at their centre.
    let res = r.near([0.5, 0.5, 1.], [0., 0.], Some(m.geo()));
    assert_eq!(res.winner.kind, Kind::OnFace, "{}", res.reason);
    // Degenerate edge (coincident vertices) is harmless.
    let d = Mesh::new().tri([0., 0., 0.], [0., 0., 0.], [1., 0., 0.]);
    let res = r.near([0.5, 0., 0.], [0., 0.], Some(d.geo()));
    assert!(res.point.unwrap().iter().all(|x| x.is_finite()));
    // Occluded vertex (behind the cube's top face from above) is not offered.
    let mut r = Rig::new(cam([0.5, 0.5, 6.], [0.5, 0.5, 0.], [0., 1., 0.]));
    let res = r.near([0.5, 0.5, 1.], [0., 0.], Some(m.geo()));
    assert!(res.alternatives.iter().all(|c| c.point[2] > 0.5), "{:?}", res.alternatives.iter().map(|c| (c.kind, c.point)).collect::<Vec<_>>());
}

#[test] fn settings_round_trip_and_key_conflicts() {
    let mut s = InferenceSettings::default();
    s.tolerance_px = 14.; s.linear_mode = LinearMode::ParallelPerpendicularOnly; s.colors.x = [1, 2, 3, 4]; s.keys.cycle_candidate = "ArrowUp".into();
    let back = InferenceSettings::from_json(&s.to_json());
    assert_eq!(back, s);
    assert_eq!(back.keys.conflicts(), vec!["ArrowUp".to_string()]);
    assert!(InferenceSettings::default().keys.conflicts().is_empty());
}
