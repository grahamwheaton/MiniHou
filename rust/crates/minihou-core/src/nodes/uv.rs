//! UV projection with per-face seam correction (nodes/uv.py).
use std::sync::Arc;
use super::{Inputs,NodeSpec,GEO};
use crate::{error::{Error, Result},geometry::{Attr,Geometry},math,params::Params};
pub const UV_PROJECT:NodeSpec=NodeSpec{name:"uv_project",inputs:GEO,params:crate::params!(
    "method":"box";"center":[0.,0.,0.];"rotate":[0.,0.,0.];"scale":[1.,1.,1.];"rotate_uv":0.;"group":""),cook:project};
fn project(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=UV_PROJECT.one(inputs,0)?;let method=p.e("method",&["box","planar","cylindrical","spherical"])?;
    let center=p.v3("center")?;let r=math::transpose(&math::euler_xyz(p.v3("rotate")?));let scale=p.v3("scale")?.map(|v|if v.abs()<1e-12{1.}else{v});
    let local:Vec<[f64;3]>=g.topology.vertex_point.iter().map(|&i|{let q=std::array::from_fn(|k|g.p()[i as usize*3+k] as f64-center[k]);let q=math::apply(&r,q);std::array::from_fn(|k|q[k]/scale[k])}).collect();
    let max=local.iter().flatten().map(|v|v.abs()).fold(0f64,f64::max).max(1.);
    let normals=crate::normals::normalize_to_f32(&crate::normals::prim_normals(g.p(),&g.topology));
    let mut uv=vec![[0f64;2];g.nvertices()];let mut on_axis=vec![false;g.nvertices()];
    for i in 0..g.nprims(){
        let normal=math::apply(&r,std::array::from_fn(|k|normals[i*3+k] as f64));let mut axis=0;for k in 1..3{if normal[k].abs()>normal[axis].abs(){axis=k;}}
        let sign=if normal[axis]<0.{-1.}else{1.};let start=g.topology.prim_offsets[i] as usize;let end=g.topology.prim_offsets[i+1] as usize;
        for v in start..end{let q=local[v];uv[v]=match method.as_str(){
            "planar"=>[q[0]+0.5,q[1]+0.5],
            "box"=>match axis{0=>[q[1]*sign+0.5,q[2]+0.5],1=>[-q[0]*sign+0.5,q[2]+0.5],_=>[q[0]*sign+0.5,q[1]+0.5]},
            "cylindrical"=>{on_axis[v]=q[0].hypot(q[1])<1e-9*max;[(q[1].atan2(q[0])/(2.*std::f64::consts::PI)).rem_euclid(1.),q[2]+0.5]},
            _=>{let radius=q.iter().map(|x|x*x).sum::<f64>().sqrt().max(1e-12);on_axis[v]=q[0].hypot(q[1])<1e-6*radius;
                [(q[1].atan2(q[0])/(2.*std::f64::consts::PI)).rem_euclid(1.),1.-(q[2]/radius).clamp(-1.,1.).acos()/std::f64::consts::PI]},
        };}
        if method=="cylindrical" || method=="spherical"{
            let mut lo=f64::INFINITY;let mut hi=f64::NEG_INFINITY;
            for v in start..end{if !on_axis[v]{lo=lo.min(uv[v][0]);hi=hi.max(uv[v][0]);}}
            if hi-lo>0.5{for v in start..end{if uv[v][0]<0.5{uv[v][0]+=1.;}}}
            let mut sum=0.;let mut n=0;for v in start..end{if !on_axis[v]{sum+=uv[v][0];n+=1;}}
            for v in start..end{if on_axis[v]{uv[v][0]=sum/n.max(1) as f64;}}
        }
    }
    let(sn,cs)=p.f("rotate_uv")?.to_radians().sin_cos();let sel=super::group::mask(&g,"prim",&p.s("group")?)?;
    let old_vertex=g.vertex.get("uv").filter(|a|a.width==2).and_then(|a|a.as_f32());
    let old_point=g.point.get("uv").filter(|a|a.width==2).and_then(|a|a.as_f32());
    let restricted=p.s("group")?.split(|c:char|c==',' || c.is_whitespace()).any(|s|!s.is_empty());
    let mut values=Vec::with_capacity(g.nvertices()*2);
    for (i,&selected) in sel.iter().enumerate(){for v in g.topology.prim_offsets[i] as usize..g.topology.prim_offsets[i+1] as usize{
        if restricted && !selected{let old=if let Some(a)=old_vertex{[a[v*2],a[v*2+1]]}else if let Some(a)=old_point{let pt=g.topology.vertex_point[v] as usize;[a[pt*2],a[pt*2+1]]}else{[0.;2]};values.extend(old);}
        else{let[x,y]=uv[v].map(|v|v-0.5);values.extend([(x*cs-y*sn+0.5) as f32,(x*sn+y*cs+0.5) as f32]);}
    }}
    let mut out=(*g).clone();out.point.remove("uv");out.prim.remove("uv");out.vertex.insert("uv".into(),Arc::new(Attr::f32(2,values)));Ok(out)
}
pub const UV_TRANSFORM: NodeSpec = NodeSpec {
    name: "uv_transform",
    inputs: GEO,
    params: crate::params!(
        "uv": "uv";
        "scope": "whole";
        "translate": [0., 0., 0.];
        "rotate": 0.;
        "scale": [1., 1., 1.];
        "flip_u": false;
        "flip_v": false;
        "pivot": "center";
        "pivot_uv": [0.5, 0.5, 0.];
        "align": false;
        "direction": [0., 0., 1.];
        "align_axis": "v";
        "random_translate": [0., 0., 0.];
        "random_rotate": 0.;
        "random_scale": 0.;
        "seed": 0;
        "group": ""
    ),
    cook: transform_uv,
};

fn uv_corners(geo: &Geometry, name: &str) -> Result<Vec<[f64; 2]>> {
    let nv = geo.nvertices();
    if let Some(attr) = geo.vertex.get(name) {
        if attr.width >= 2 {
            if let Some(data) = attr.as_f32() {
                let w = attr.width;
                let mut uvc = Vec::with_capacity(nv);
                for i in 0..nv {
                    uvc.push([data[i * w] as f64, data[i * w + 1] as f64]);
                }
                return Ok(uvc);
            }
        }
    }
    if let Some(attr) = geo.point.get(name) {
        if attr.width >= 2 {
            if let Some(data) = attr.as_f32() {
                let w = attr.width;
                let mut uvc = Vec::with_capacity(nv);
                for &vp in &geo.topology.vertex_point {
                    let pt = vp as usize;
                    uvc.push([data[pt * w] as f64, data[pt * w + 1] as f64]);
                }
                return Ok(uvc);
            }
        }
    }
    Err(Error::new(format!(
        "uv_transform: no uv attribute {:?} (project or unwrap first, e.g. uv_project or uv_flatten)",
        name
    )))
}

pub fn uv_islands(geo: &Geometry, uv: &[[f64; 2]]) -> Vec<usize> {
    let nprims = geo.nprims();
    if nprims == 0 {
        return Vec::new();
    }
    let topo = &geo.topology;
    let nv = geo.nvertices();
    let mut pv = vec![0usize; nv];
    let mut nxt = vec![0usize; nv];
    let mut a = vec![0usize; nv];
    let mut b = vec![0usize; nv];

    for p in 0..nprims {
        let start = topo.prim_offsets[p] as usize;
        let end = topo.prim_offsets[p + 1] as usize;
        for v in start..end {
            pv[v] = p;
            let nx = if v + 1 < end { v + 1 } else { start };
            nxt[v] = nx;
            a[v] = topo.vertex_point[v] as usize;
            b[v] = topo.vertex_point[nx] as usize;
        }
    }

    let mut order: Vec<usize> = (0..nv).collect();
    order.sort_by_key(|&v| (a[v].min(b[v]), a[v].max(b[v])));

    let mut parent: Vec<usize> = (0..nprims).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        let mut root = i;
        while parent[root] != root {
            root = parent[root];
        }
        while parent[i] != root {
            let next = parent[i];
            parent[i] = root;
            i = next;
        }
        root
    }
    fn union(parent: &mut [usize], i: usize, j: usize) {
        let ri = find(parent, i);
        let rj = find(parent, j);
        if ri != rj {
            if ri < rj {
                parent[rj] = ri;
            } else {
                parent[ri] = rj;
            }
        }
    }

    for i in 0..order.len().saturating_sub(1) {
        let h1 = order[i];
        let h2 = order[i + 1];
        if (a[h1].min(b[h1]), a[h1].max(b[h1])) == (a[h2].min(b[h2]), a[h2].max(b[h2])) {
            let first = if a[h2] == a[h1] { h2 } else { nxt[h2] };
            let second = if a[h2] == a[h1] { nxt[h2] } else { h2 };
            let ua1 = uv[h1];
            let ub1 = uv[nxt[h1]];
            let u_first = uv[first];
            let u_second = uv[second];
            let close = (ua1[0] - u_first[0]).abs() < 1e-6
                && (ua1[1] - u_first[1]).abs() < 1e-6
                && (ub1[0] - u_second[0]).abs() < 1e-6
                && (ub1[1] - u_second[1]).abs() < 1e-6;
            if close {
                union(&mut parent, pv[h1], pv[h2]);
            }
        }
    }

    let mut root_to_label = vec![usize::MAX; nprims];
    let mut num_labels = 0;
    for i in 0..nprims {
        let r = find(&mut parent, i);
        if root_to_label[r] == usize::MAX {
            root_to_label[r] = num_labels;
            num_labels += 1;
        }
    }
    (0..nprims).map(|i| root_to_label[find(&mut parent, i)]).collect()
}

fn align_angles(geo: &Geometry, uv: &[[f64; 2]], label: &[usize], count: usize, direction: [f64; 3]) -> Vec<f64> {
    let t = &geo.topology;
    let nprims = geo.nprims();
    let p = geo.p();
    let d = direction;
    let mut acc = vec![[0f64; 2]; count];

    for i in 0..nprims {
        let s = t.prim_offsets[i] as usize;
        let e = t.prim_offsets[i + 1] as usize;
        if e - s < 3 {
            continue;
        }
        let (c0, c1, c2) = (s, s + 1, s + 2);
        let pt0 = t.vertex_point[c0] as usize;
        let pt1 = t.vertex_point[c1] as usize;
        let pt2 = t.vertex_point[c2] as usize;

        let p0 = [p[pt0 * 3] as f64, p[pt0 * 3 + 1] as f64, p[pt0 * 3 + 2] as f64];
        let p1 = [p[pt1 * 3] as f64, p[pt1 * 3 + 1] as f64, p[pt1 * 3 + 2] as f64];
        let p2 = [p[pt2 * 3] as f64, p[pt2 * 3 + 1] as f64, p[pt2 * 3 + 2] as f64];

        let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];

        let u1 = [uv[c1][0] - uv[c0][0], uv[c1][1] - uv[c0][1]];
        let u2 = [uv[c2][0] - uv[c0][0], uv[c2][1] - uv[c0][1]];

        let nrm = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let area = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2]).sqrt();
        let nn = if area > 0.0 {
            [nrm[0] / area, nrm[1] / area, nrm[2] / area]
        } else {
            nrm
        };

        let nn_dot_d = nn[0] * d[0] + nn[1] * d[1] + nn[2] * d[2];
        let dp = [
            d[0] - nn_dot_d * nn[0],
            d[1] - nn_dot_d * nn[1],
            d[2] - nn_dot_d * nn[2],
        ];

        let g11 = e1[0] * e1[0] + e1[1] * e1[1] + e1[2] * e1[2];
        let g12 = e1[0] * e2[0] + e1[1] * e2[1] + e1[2] * e2[2];
        let g22 = e2[0] * e2[0] + e2[1] * e2[1] + e2[2] * e2[2];

        let r1 = dp[0] * e1[0] + dp[1] * e1[1] + dp[2] * e1[2];
        let r2 = dp[0] * e2[0] + dp[1] * e2[1] + dp[2] * e2[2];

        let det = g11 * g22 - g12 * g12;
        let dp_norm = (dp[0] * dp[0] + dp[1] * dp[1] + dp[2] * dp[2]).sqrt();

        let good = det.abs() > 1e-18 && dp_norm > 1e-9;
        if good {
            let a = (r1 * g22 - r2 * g12) / det;
            let b = (g11 * r2 - g12 * r1) / det;
            let w = [a * u1[0] + b * u2[0], a * u1[1] + b * u2[1]];
            let ln = (w[0] * w[0] + w[1] * w[1]).sqrt();
            if ln > 1e-12 {
                let factor = (area / 2.0) / ln;
                let lbl = label[i];
                acc[lbl][0] += w[0] * factor;
                acc[lbl][1] += w[1] * factor;
            }
        }
    }

    acc.iter()
        .map(|a| {
            let ln = (a[0] * a[0] + a[1] * a[1]).sqrt();
            if ln > 1e-12 {
                a[1].atan2(a[0])
            } else {
                f64::NAN
            }
        })
        .collect()
}

fn transform_uv(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = UV_TRANSFORM.one(inputs, 0)?;
    let uv_name = p.s("uv")?;
    let scope = p.e("scope", &["whole", "island", "face"])?;
    let translate = p.v3("translate")?;
    let rotate = p.f("rotate")?;
    let scale = p.v3("scale")?;
    let flip_u = p.b("flip_u")?;
    let flip_v = p.b("flip_v")?;
    let pivot = p.e("pivot", &["center", "custom"])?;
    let pivot_uv = p.v3("pivot_uv")?;
    let align = p.b("align")?;
    let direction = p.v3("direction")?;
    let align_axis = p.e("align_axis", &["v", "u"])?;
    let random_translate = p.v3("random_translate")?;
    let random_rotate = p.f("random_rotate")?;
    let random_scale = p.f("random_scale")?;
    let seed = p.i("seed")?;
    let group = p.s("group")?;

    let uvc = uv_corners(&g, &uv_name)?;
    let nv = g.nvertices();
    let nprims = g.nprims();

    if align {
        let d_norm = (direction[0] * direction[0] + direction[1] * direction[1] + direction[2] * direction[2]).sqrt();
        if d_norm < 1e-12 {
            return Err(Error::new("uv_transform: the direction is zero"));
        }
    }

    let sel = super::group::mask(&g, "prim", &group)?;
    let mut pv = vec![0usize; nv];
    for prim_idx in 0..nprims {
        let s = g.topology.prim_offsets[prim_idx] as usize;
        let e = g.topology.prim_offsets[prim_idx + 1] as usize;
        for v in s..e {
            pv[v] = prim_idx;
        }
    }

    let on: Vec<bool> = (0..nv).map(|v| sel.get(pv[v]).copied().unwrap_or(false)).collect();
    if !on.iter().any(|&b| b) {
        let mut out = (*g).clone();
        out.point.remove(&uv_name);
        out.prim.remove(&uv_name);
        let mut flat = Vec::with_capacity(nv * 2);
        for v in 0..nv {
            flat.push(uvc[v][0] as f32);
            flat.push(uvc[v][1] as f32);
        }
        out.vertex.insert(uv_name, Arc::new(Attr::f32(2, flat)));
        return Ok(out);
    }

    let (label, count) = match scope.as_str() {
        "whole" => (vec![0usize; nprims], 1usize),
        "face" => ((0..nprims).collect(), nprims),
        _ => {
            let isl = uv_islands(&g, &uvc);
            let c = if nprims == 0 { 0 } else { isl.iter().copied().max().unwrap_or(0) + 1 };
            (isl, c)
        }
    };

    let corner_label: Vec<usize> = (0..nv).map(|v| label[pv[v]]).collect();

    let piv = if pivot == "custom" {
        vec![[pivot_uv[0], pivot_uv[1]]; count]
    } else {
        let mut lo = vec![[f64::INFINITY; 2]; count];
        let mut hi = vec![[f64::NEG_INFINITY; 2]; count];
        for v in 0..nv {
            if on[v] {
                let l = corner_label[v];
                lo[l][0] = lo[l][0].min(uvc[v][0]);
                lo[l][1] = lo[l][1].min(uvc[v][1]);
                hi[l][0] = hi[l][0].max(uvc[v][0]);
                hi[l][1] = hi[l][1].max(uvc[v][1]);
            }
        }
        let mut p = vec![[0f64; 2]; count];
        for l in 0..count {
            if lo[l][0].is_finite() && hi[l][0].is_finite() {
                p[l] = [(lo[l][0] + hi[l][0]) / 2.0, (lo[l][1] + hi[l][1]) / 2.0];
            }
        }
        p
    };

    let mut rot = vec![rotate.to_radians(); count];
    if align {
        let ang = align_angles(&g, &uvc, &label, count, direction);
        let target = if align_axis == "v" { std::f64::consts::FRAC_PI_2 } else { 0.0 };
        for l in 0..count {
            if !ang[l].is_nan() {
                rot[l] += target - ang[l];
            }
        }
    }

    let mut rng = super::np_random::Rng::new(&[(seed as u32) as u64]);
    let mut rnd = vec![[0f64; 4]; count];
    for item in rnd.iter_mut() {
        for k in 0..4 {
            item[k] = rng.random() * 2.0 - 1.0;
        }
    }

    let mut sc = vec![[scale[0], scale[1]]; count];
    let mut tr = vec![[translate[0], translate[1]]; count];
    for l in 0..count {
        rot[l] += random_rotate.to_radians() * rnd[l][0];
        let sc_rand = 1.0 + random_scale * rnd[l][1];
        sc[l][0] = scale[0] * sc_rand * (if flip_u { -1.0 } else { 1.0 });
        sc[l][1] = scale[1] * sc_rand * (if flip_v { -1.0 } else { 1.0 });
        tr[l][0] = translate[0] + random_translate[0] * rnd[l][2];
        tr[l][1] = translate[1] + random_translate[1] * rnd[l][3];
    }

    let mut new_uv = uvc.clone();
    for v in 0..nv {
        if on[v] {
            let l = corner_label[v];
            let p = piv[l];
            let d0 = (uvc[v][0] - p[0]) * sc[l][0];
            let d1 = (uvc[v][1] - p[1]) * sc[l][1];
            let (sn, cs) = rot[l].sin_cos();
            new_uv[v][0] = d0 * cs - d1 * sn + p[0] + tr[l][0];
            new_uv[v][1] = d0 * sn + d1 * cs + p[1] + tr[l][1];
        }
    }

    let mut out = (*g).clone();
    out.point.remove(&uv_name);
    out.prim.remove(&uv_name);
    let mut flat = Vec::with_capacity(nv * 2);
    for v in 0..nv {
        flat.push(new_uv[v][0] as f32);
        flat.push(new_uv[v][1] as f32);
    }
    out.vertex.insert(uv_name, Arc::new(Attr::f32(2, flat)));
    Ok(out)
}

#[cfg(test)]
mod tests{
    use super::*;

    #[test]
    fn planar_and_group_preserve_point_uv(){
        let mut g=Geometry::from_arrays(vec![0.,0.,0.,1.,0.,0.,0.,1.,0.],vec![0,1,2,2,1,0],vec![0,3,6],None).unwrap();
        g.point.insert("uv".into(),Arc::new(Attr::f32(2,vec![2.;6])));
        let p=Params::resolve(UV_PROJECT.params,serde_json::json!({"method":"planar","group":"0"}).as_object()).unwrap();let out=project(&[vec![Arc::new(g)]],&p).unwrap();
        assert_eq!(out.vertex["uv"].as_f32().unwrap(),[0.5,0.5,1.5,0.5,0.5,1.5,2.,2.,2.,2.,2.,2.]);assert!(!out.point.contains_key("uv"));out.validate().unwrap();
    }

    #[test]
    fn seam_crossing_uses_continuous_u(){
        let g=Geometry::from_arrays(vec![1.,-0.1,0.,1.,0.1,0.,1.,0.2,1.],vec![0,1,2],vec![0,3],None).unwrap();let p=Params::resolve(UV_PROJECT.params,serde_json::json!({"method":"cylindrical"}).as_object()).unwrap();
        let out=project(&[vec![Arc::new(g)]],&p).unwrap();let u=out.vertex["uv"].as_f32().unwrap();assert!(u[2]>1.);assert!((u[2]-u[0]).abs()<0.1);
    }

    fn make_test_grid() -> Geometry {
        // 3x3 vertices = 2x2 quads (4 quads), points from x in [-1, 1], y in [-1, 1]
        // point UVs from 0 to 1
        let mut p = Vec::new();
        let mut uv = Vec::new();
        for row in 0..3 {
            let y = -1.0 + (row as f32);
            let v = (row as f32) / 2.0;
            for col in 0..3 {
                let x = -1.0 + (col as f32);
                let u = (col as f32) / 2.0;
                p.extend_from_slice(&[x, y, 0.0]);
                uv.extend_from_slice(&[u, v]);
            }
        }
        let mut vp = Vec::new();
        let mut offs = vec![0i32];
        for row in 0..2 {
            for col in 0..2 {
                let i0 = (row * 3 + col) as i32;
                let i1 = (row * 3 + col + 1) as i32;
                let i2 = ((row + 1) * 3 + col + 1) as i32;
                let i3 = ((row + 1) * 3 + col) as i32;
                vp.extend_from_slice(&[i0, i1, i2, i3]);
                offs.push(vp.len() as i32);
            }
        }
        let mut g = Geometry::from_arrays(p, vp, offs, None).unwrap();
        g.point.insert("uv".into(), Arc::new(Attr::f32(2, uv)));
        g
    }

    #[test]
    fn test_uv_transform_scale_translate_rotate_flip() {
        let grid = make_test_grid();
        let uvc = uv_corners(&grid, "uv").unwrap();

        // 1. scale and translate with custom pivot
        let p = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({
                "scale": [2., 3., 1.],
                "translate": [0.1, 0.2, 0.],
                "pivot": "custom",
                "pivot_uv": [0., 0., 0.]
            })
            .as_object(),
        )
        .unwrap();
        let out = transform_uv(&[vec![Arc::new(grid.clone())]], &p).unwrap();
        let out_uv = out.vertex["uv"].as_f32().unwrap();
        for (i, uv) in uvc.iter().enumerate() {
            let expected_u = uv[0] * 2.0 + 0.1;
            let expected_v = uv[1] * 3.0 + 0.2;
            assert!((out_uv[i * 2] as f64 - expected_u).abs() < 1e-6);
            assert!((out_uv[i * 2 + 1] as f64 - expected_v).abs() < 1e-6);
        }

        // 2. rotate 90 degrees about center
        let p_rot = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({"rotate": 90.0}).as_object(),
        )
        .unwrap();
        let out_rot = transform_uv(&[vec![Arc::new(grid.clone())]], &p_rot).unwrap();
        let rot_uv = out_rot.vertex["uv"].as_f32().unwrap();
        for (i, uv) in uvc.iter().enumerate() {
            let d0 = uv[0] - 0.5;
            let d1 = uv[1] - 0.5;
            let expected_u = -d1 + 0.5;
            let expected_v = d0 + 0.5;
            assert!((rot_uv[i * 2] as f64 - expected_u).abs() < 1e-6);
            assert!((rot_uv[i * 2 + 1] as f64 - expected_v).abs() < 1e-6);
        }

        // 3. flip_u
        let p_flip_u = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({"flip_u": true}).as_object(),
        )
        .unwrap();
        let out_flip_u = transform_uv(&[vec![Arc::new(grid.clone())]], &p_flip_u).unwrap();
        let flip_u_uv = out_flip_u.vertex["uv"].as_f32().unwrap();
        for (i, uv) in uvc.iter().enumerate() {
            let expected_u = 1.0 - uv[0];
            let expected_v = uv[1];
            assert!((flip_u_uv[i * 2] as f64 - expected_u).abs() < 1e-6);
            assert!((flip_u_uv[i * 2 + 1] as f64 - expected_v).abs() < 1e-6);
        }
        assert!(!out_flip_u.point.contains_key("uv"));

        // 4. flip_v
        let p_flip_v = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({"flip_v": true}).as_object(),
        )
        .unwrap();
        let out_flip_v = transform_uv(&[vec![Arc::new(grid.clone())]], &p_flip_v).unwrap();
        let flip_v_uv = out_flip_v.vertex["uv"].as_f32().unwrap();
        for (i, uv) in uvc.iter().enumerate() {
            let expected_u = uv[0];
            let expected_v = 1.0 - uv[1];
            assert!((flip_v_uv[i * 2] as f64 - expected_u).abs() < 1e-6);
            assert!((flip_v_uv[i * 2 + 1] as f64 - expected_v).abs() < 1e-6);
        }
    }

    #[test]
    fn test_uv_transform_islands_and_random() {
        let grid = make_test_grid();
        let uvc = uv_corners(&grid, "uv").unwrap();
        assert_eq!(uv_islands(&grid, &uvc), vec![0, 0, 0, 0]);

        // Random face transform is deterministic with same seed
        let p_rnd1 = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({
                "scope": "face",
                "random_translate": [0.3, 0.3, 0.],
                "random_rotate": 45.0,
                "random_scale": 0.3,
                "seed": 5
            })
            .as_object(),
        )
        .unwrap();
        let out_rnd1 = transform_uv(&[vec![Arc::new(grid.clone())]], &p_rnd1).unwrap();

        let p_rnd2 = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({
                "scope": "face",
                "random_translate": [0.3, 0.3, 0.],
                "random_rotate": 45.0,
                "random_scale": 0.3,
                "seed": 5
            })
            .as_object(),
        )
        .unwrap();
        let out_rnd2 = transform_uv(&[vec![Arc::new(grid.clone())]], &p_rnd2).unwrap();
        assert_eq!(
            out_rnd1.vertex["uv"].as_f32().unwrap(),
            out_rnd2.vertex["uv"].as_f32().unwrap()
        );

        // Different seed produces different results
        let p_rnd3 = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({
                "scope": "face",
                "random_translate": [0.3, 0.3, 0.],
                "random_rotate": 45.0,
                "random_scale": 0.3,
                "seed": 6
            })
            .as_object(),
        )
        .unwrap();
        let out_rnd3 = transform_uv(&[vec![Arc::new(grid.clone())]], &p_rnd3).unwrap();
        let uv1 = out_rnd1.vertex["uv"].as_f32().unwrap();
        let uv3 = out_rnd3.vertex["uv"].as_f32().unwrap();
        let max_diff = uv1
            .iter()
            .zip(uv3.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(max_diff > 0.01);

        // Group limitation
        let p_grp = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({
                "translate": [1., 0., 0.],
                "scope": "face",
                "group": "0"
            })
            .as_object(),
        )
        .unwrap();
        let out_grp = transform_uv(&[vec![Arc::new(grid.clone())]], &p_grp).unwrap();
        let grp_uv = out_grp.vertex["uv"].as_f32().unwrap();
        // Face 0 moved, faces 1, 2, 3 unmoved
        for v in 0..4 {
            assert!((grp_uv[v * 2] as f64 - (uvc[v][0] + 1.0)).abs() < 1e-6);
        }
        for v in 4..16 {
            assert!((grp_uv[v * 2] as f64 - uvc[v][0]).abs() < 1e-6);
        }
    }

    #[test]
    fn test_uv_transform_align_and_errors() {
        let grid = make_test_grid();
        let p_align_v = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({
                "align": true,
                "direction": [1., 0., 0.],
                "align_axis": "v"
            })
            .as_object(),
        )
        .unwrap();
        let out_align_v = transform_uv(&[vec![Arc::new(grid.clone())]], &p_align_v).unwrap();
        let uv_v = out_align_v.vertex["uv"].as_f32().unwrap();
        // Direction [1, 0, 0] aligned with v means v grows with x (90 degree ccw turn about center 0.5, 0.5)
        let uvc = uv_corners(&grid, "uv").unwrap();
        for (i, uv) in uvc.iter().enumerate() {
            let d0 = uv[0] - 0.5;
            let d1 = uv[1] - 0.5;
            let expected_u = -d1 + 0.5;
            let expected_v = d0 + 0.5;
            assert!((uv_v[i * 2] as f64 - expected_u).abs() < 1e-5);
            assert!((uv_v[i * 2 + 1] as f64 - expected_v).abs() < 1e-5);
        }

        // Missing UV attribute error
        let p_err_uv = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({"uv": "nonexistent"}).as_object(),
        )
        .unwrap();
        assert!(transform_uv(&[vec![Arc::new(grid.clone())]], &p_err_uv).is_err());

        // Zero direction error
        let p_err_dir = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({"align": true, "direction": [0., 0., 0.]}).as_object(),
        )
        .unwrap();
        assert!(transform_uv(&[vec![Arc::new(grid.clone())]], &p_err_dir).is_err());
    }
}

