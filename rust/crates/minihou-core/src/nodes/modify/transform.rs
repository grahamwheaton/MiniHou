use super::*;

fn mat_f32(m: &math::M3) -> [[f32; 3]; 3] {
    m.map(|row| row.map(|x| x as f32))
}

pub(super) fn transform(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = TRANSFORM.one(inputs, 0)?;
    let selected = groups::point_mask(&geo, &p.s("group")?)?;
    if p.b("proportional_enabled")? {
        return proportional_transform(&geo, p, &selected);
    }
    let us = p.f("uniform_scale")?;
    let s = p.v3("scale")?.map(|x| x * us);
    let r = math::euler_xyz(p.v3("rotate")?);
    let b = math::euler_xyz(p.v3("axis_rotation")?);
    let pivot = p.v3("pivot")?;
    let translate = p.v3("translate")?;
    let scale_cols = |m: &math::M3, f: &dyn Fn(f64) -> f64| -> math::M3 {
        let mut o = *m;
        for row in o.iter_mut() {
            for (j, x) in row.iter_mut().enumerate() {
                *x *= f(s[j]);
            }
        }
        o
    };
    // M = B (R * s) B^T: scale, then rotate, in the axis_rotation frame
    let m = math::mul(&math::mul(&b, &scale_cols(&r, &|x| x)), &math::transpose(&b));
    let m32 = mat_f32(&m);
    let bt = math::apply(&b, translate);
    let shift: [f32; 3] = [0, 1, 2].map(|k| (pivot[k] + bt[k]) as f32);
    let pv: [f32; 3] = pivot.map(|x| x as f32);
    let mut out = (*geo).clone();
    let mut positions = xform::affine(geo.p(), m32, pv, shift);
    for (i, &use_point) in selected.iter().enumerate() {
        if !use_point { positions[3*i..3*i+3].copy_from_slice(&geo.p()[3*i..3*i+3]); }
    }
    out.set_p(positions);
    super::super::packed::transform_instances(&mut out, &m, &selected);
    if let Some(n) = geo.point.get("N") {
        let n = n.as_f32().filter(|_| n.width == 3).ok_or_else(|| Error::new("point attribute N must be 3 floats"))?;
        // normals take the inverse scale: B (R / s) B^T, a zero scale counting as 1
        let nm = math::mul(&math::mul(&b, &scale_cols(&r, &|x| 1.0 / if x.abs() < 1e-12 { 1.0 } else { x })), &math::transpose(&b));
        let mut nn = xform::linear_normalized(n, mat_f32(&nm));
        for (i, &use_point) in selected.iter().enumerate() {
            if !use_point { nn[3*i..3*i+3].copy_from_slice(&n[3*i..3*i+3]); }
        }
        out.point.insert("N".into(), Arc::new(Attr::f32(3, nn)));
    }
    Ok(out)
}

#[derive(Copy, Clone, PartialEq)]
struct Distance(f64, usize);
impl Eq for Distance {}
impl PartialOrd for Distance { fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) } }
impl Ord for Distance { fn cmp(&self, other: &Self) -> Ordering { self.0.total_cmp(&other.0).then(self.1.cmp(&other.1)) } }

fn proportional_weights(geo: &Geometry, selected: &[bool], p: &Params) -> Result<Vec<f64>> {
    let radius = p.f("proportional_radius")?;
    if !radius.is_finite() || radius <= 0.0 { return Err(Error::new("proportional radius must be positive and finite")); }
    let falloff = p.e("proportional_falloff", &["smooth", "sphere", "root", "inverse_square", "sharp", "linear", "constant", "random"])?;
    if falloff == "random" { return Err(Error::new("random proportional falloff is not supported by the Rust backend")); }
    let pos: Vec<[f64; 3]> = geo.p().as_chunks::<3>().0.iter().map(|q| [q[0] as f64, q[1] as f64, q[2] as f64]).collect();
    if pos.iter().flatten().any(|x| !x.is_finite()) { return Err(Error::new("proportional editing requires finite positions")); }
    let connected = p.b("proportional_connected")?;
    let projected = p.b("proportional_projected")? && !connected;
    let mut metric = pos.clone();
    if projected {
        let mut view = p.v3("proportional_view_direction")?;
        let len = view.iter().map(|v| v*v).sum::<f64>().sqrt();
        if !len.is_finite() || len < 1e-12 { return Err(Error::new("projected proportional editing requires a nonzero view direction")); }
        for v in &mut view { *v /= len; }
        for q in &mut metric {
            let dot: f64 = (0..3).map(|k| q[k]*view[k]).sum();
            for k in 0..3 { q[k] -= dot*view[k]; }
        }
    }
    let mut distance: Vec<f64> = selected.iter().map(|&b| if b { 0.0 } else { f64::INFINITY }).collect();
    if connected {
        let mut adj = vec![Vec::<(usize, f64)>::new(); geo.npoints];
        let edges = topo::edges(&geo.topology, geo.npoints);
        for pair in edges.points.as_chunks::<2>().0 {
            let (a,b) = (pair[0] as usize, pair[1] as usize);
            let len = (0..3).map(|k| (pos[a][k]-pos[b][k]).powi(2)).sum::<f64>().sqrt();
            adj[a].push((b, len)); adj[b].push((a, len));
        }
        let mut queue = BinaryHeap::new();
        for (i, &b) in selected.iter().enumerate() { if b { queue.push(Reverse(Distance(0.0, i))); } }
        while let Some(Reverse(Distance(d, i))) = queue.pop() {
            if d != distance[i] || d >= radius { continue; }
            for &(j, len) in &adj[i] {
                let candidate = d + len;
                if candidate < radius && candidate < distance[j] { distance[j] = candidate; queue.push(Reverse(Distance(candidate, j))); }
            }
        }
    } else {
        let seeds: Vec<_> = metric.iter().zip(selected).filter_map(|(q, &b)| b.then_some(q)).collect();
        if !seeds.is_empty() {
            for (i, q) in metric.iter().enumerate() {
                distance[i] = seeds.iter().map(|seed| (0..3).map(|k| (q[k]-seed[k]).powi(2)).sum::<f64>()).fold(f64::INFINITY, f64::min).sqrt();
            }
        }
    }
    Ok(distance.into_iter().zip(selected).map(|(d, &sel)| {
        if sel { return 1.0; }
        if d >= radius { return 0.0; }
        let x = (1.0 - d/radius).clamp(0.0, 1.0);
        match falloff.as_str() {
            "smooth" => x*x*(3.0-2.0*x), "sphere" => (2.0*x-x*x).sqrt(), "root" => x.sqrt(),
            "inverse_square" => x*x/(2.0-x*x), "sharp" => x*x, "constant" => 1.0,
            _ => x,
        }
    }).collect())
}

fn proportional_transform(geo: &Geometry, p: &Params, selected: &[bool]) -> Result<Geometry> {
    let weights = proportional_weights(geo, selected, p)?;
    let b = math::euler_xyz(p.v3("axis_rotation")?);
    let bt = math::transpose(&b);
    let translate = math::apply(&b, p.v3("translate")?);
    let pivot = p.v3("pivot")?;
    let angles = p.v3("rotate")?;
    let uniform_scale = p.f("uniform_scale")?;
    let scale = p.v3("scale")?.map(|x| x * uniform_scale);
    let mut out = geo.clone();
    let mut positions = geo.p().to_vec();
    for (i, &w) in weights.iter().enumerate() {
        if w == 0.0 { continue; }
        let q = [0,1,2].map(|k| geo.p()[3*i+k] as f64 - pivot[k]);
        let local = math::apply(&bt, q);
        let local_scale = scale.map(|s| 1.0 + w*(s-1.0));
        let scaled = [0,1,2].map(|k| local[k]*local_scale[k]);
        let rotated = math::apply(&math::euler_xyz(angles.map(|a| a*w)), scaled);
        let world = math::apply(&b, rotated);
        for k in 0..3 { positions[3*i+k] = (world[k] + pivot[k] + w*translate[k]) as f32; }
    }
    out.set_p(positions);
    if let Some(n) = geo.point.get("N") {
        let normal = n.as_f32().filter(|_| n.width == 3).ok_or_else(|| Error::new("point attribute N must be 3 floats"))?;
        let mut result = normal.to_vec();
        for (i, &w) in weights.iter().enumerate() {
            if w == 0.0 { continue; }
            let local = math::apply(&bt, [0,1,2].map(|k| normal[3*i+k] as f64));
            let divided = [0,1,2].map(|k| {
                let s = 1.0 + w*(scale[k]-1.0);
                local[k] / if s.abs() < 1e-12 { 1.0 } else { s }
            });
            let rotated = math::apply(&math::euler_xyz(angles.map(|a| a*w)), divided);
            let world = math::apply(&b, rotated);
            let len = world.iter().map(|x| x*x).sum::<f64>().sqrt();
            for k in 0..3 { result[3*i+k] = (world[k] / if len > 1e-12 { len } else { 1.0 }) as f32; }
        }
        out.point.insert("N".into(), Arc::new(Attr::f32(3, result)));
    }
    Ok(out)
}
