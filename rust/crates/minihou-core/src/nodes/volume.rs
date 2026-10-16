//! Mesh-in/mesh-out SDF nodes from minihou/nodes/volume.py.
use super::{GEO, InputSpec, Inputs, NodeSpec, model::*};
use crate::{error::{Error, Result}, geometry::Geometry, params::Params, math, normals, sdf_field, sdf_mesh, sdf_nets};
use std::sync::Arc;

const AB: &[InputSpec] = &[
    InputSpec { name: "a", multi: false, optional: false },
    InputSpec { name: "b", multi: false, optional: false },
];
const POINTS_IN: &[InputSpec] = &[
    InputSpec { name: "points", multi: false, optional: false },
];
pub const SDF_SHAPE: NodeSpec = NodeSpec {
    name: "sdf_shape", inputs: &[],
    params: crate::params!("shape":"sphere";"radius":0.5;"size":[1.,1.,1.];"depth":1.;"minor_radius":0.15;"radius2":0.2;"sides":6;"bevel":0.;"center":[0.,0.,0.];"rotate":[0.,0.,0.];"voxel_size":0.04;"smooth":0),
    cook: shape,
};
pub const SDF_BOOLEAN: NodeSpec = NodeSpec {
    name: "sdf_boolean", inputs: AB,
    params: crate::params!("operation":"union";"voxel_size":0.05;"smooth":0.;"transfer_attribs":true),
    cook: boolean,
};
pub const REMESH: NodeSpec = NodeSpec {
    name: "remesh", inputs: GEO,
    params: crate::params!("voxel_size": 0.05; "offset": 0.0; "smooth": 0; "transfer": ""),
    cook: remesh,
};
pub const POINTS_TO_SURFACE: NodeSpec = NodeSpec {
    name: "points_to_surface", inputs: POINTS_IN,
    params: crate::params!("radius": 0.2; "blend": 1.0; "voxel_size": 0.05; "smooth": 0; "transfer": "Cd"),
    cook: points_to_surface,
};

/// Python model.triangulate ordering: convex fans, concave ears, then untouched triangles.
/// Kept here because both volume.py and ray.py use exactly this triangulation.
pub(super) fn triangulation(g: &Geometry) -> (Vec<[usize;3]>, Vec<usize>) {
    let pos = positions(g);
    let ns = normals::prim_normals(g.p(), &g.topology);
    let mut fan = Vec::new(); let mut ears = Vec::new(); let mut whole = Vec::new();
    for p in 0..g.nprims() {
        let r = range(g,p); let n = r.len();
        if n == 3 { whole.push(([r.start,r.start+1,r.start+2],p)); continue; }
        if n < 4 || !g.topology.is_closed(p) { continue; }
        let q: Vec<_> = r.clone().map(|v|pos[g.topology.vertex_point[v] as usize]).collect();
        let concave = (0..n).any(|i|dot(cross(sub(q[i],q[(i+n-1)%n]),sub(q[(i+1)%n],q[i])),ns[p]) < -1e-12);
        if !concave {
            for k in 1..n-1 { fan.push(([r.start,r.start+k,r.start+k+1],p)); }
        } else {
            let normal = mul(ns[p],1./len(ns[p]).max(1e-30));
            let u = unit(cross(normal,if normal[0].abs()<0.9 {[1.,0.,0.]} else {[0.,1.,0.]}));
            let w = cross(normal,u);
            let xy: Vec<_> = q.iter().map(|&v|[dot(v,u),dot(v,w)]).collect();
            for t in ear_clip(&xy) { ears.push((t.map(|v|r.start+v),p)); }
        }
    }
    fan.extend(ears); fan.extend(whole);
    fan.into_iter().unzip()
}
fn tris(g: &Geometry) -> Result<Vec<f64>> {
    polygons(g,"sdf_boolean")?;
    let (tv,_) = triangulation(g);
    Ok(tv.iter().flat_map(|t|t.iter().flat_map(|&v| {
        let i = g.topology.vertex_point[v] as usize*3;
        g.p()[i..i+3].iter().map(|&x|x as f64)
    })).collect())
}
fn spacing(p: &Params) -> Result<f64> {
    let h = p.f("voxel_size")?;
    if !h.is_finite() || h<=0. { return Err(Error::new("voxel_size must be positive")); }
    Ok(h)
}
fn grid(lo: [f64;3], hi: [f64;3], h:f64, pad:f64) -> Result<([f64;3],[usize;3])> {
    let lo = lo.map(|x|x-pad); let hi = hi.map(|x|x+pad);
    let mut dims = [0;3];
    for k in 0..3 {
        let d = ((hi[k]-lo[k])/h).floor()+2.;
        if !d.is_finite() || !(2. ..=24_000_000.).contains(&d) { return Err(Error::new("too many voxels; use a larger voxel size")); }
        dims[k] = d as usize;
    }
    if dims.iter().try_fold(1usize,|n,&d|n.checked_mul(d)).is_none_or(|n|n>24_000_000) { return Err(Error::new("too many voxels; use a larger voxel size")); }
    Ok((lo,dims))
}
fn mesh(values: &[f32], dims:[usize;3], origin:[f64;3], h:f64) -> Result<Geometry> {
    let values: Vec<_> = values.iter().map(|&x|x as f64).collect();
    let (p,q) = sdf_nets::surface_nets(&values,dims,origin,h);
    let offsets = (0..=q.len()/4).map(|i|(i*4) as i32).collect();
    Geometry::from_arrays(p.into_iter().map(|x|x as f32).collect(),q.into_iter().map(|v|v as i32).collect(),offsets,None)
}
struct Shape { radius:f64, size:[f64;3], depth:f64, minor:f64, radius2:f64, sides:usize, bevel:f64 }
fn extrude(d:f64,z:f64,h:f64)->f64 {
    let w = z.abs()-h;
    d.max(w).min(0.)+(d.max(0.).powi(2)+w.max(0.).powi(2)).sqrt()
}
fn distance(shape:&str,p:[f64;3],s:&Shape)->f64 {
    let Shape{radius:r,size,depth,minor,radius2:r2,sides,bevel:b} = *s;
    match shape {
        "sphere" => len(p)-r,
        "box" => {
            let q: [f64;3] = std::array::from_fn(|k|p[k].abs()-(size[k]/2.-b).max(1e-9));
            len(q.map(|x|x.max(0.)))+q.into_iter().fold(f64::NEG_INFINITY,f64::max).min(0.)-b
        },
        "cylinder" => extrude(p[0].hypot(p[1])-(r-b).max(1e-9),p[2],(depth/2.-b).max(1e-9))-b,
        "capsule" => len([p[0],p[1],p[2]-p[2].clamp(-depth/2.,depth/2.)])-r,
        "ellipsoid" => {
            let rr = size.map(|x|(x/2.).max(1e-9));
            let k0 = len(std::array::from_fn(|k|p[k]/rr[k]));
            let k1 = len(std::array::from_fn(|k|p[k]/(rr[k]*rr[k])));
            if k1>1e-12 {k0*(k0-1.)/k1.max(1e-12)} else {-rr.into_iter().fold(f64::INFINITY,f64::min)}
        },
        "torus" => ((p[0].hypot(p[1])-r).powi(2)+p[2]*p[2]).sqrt()-minor,
        "round_cone" => {
            let l2 = depth*depth; let rr = r-r2; let a2 = l2-rr*rr; let il2 = 1./l2;
            let pa = [p[0],p[1],p[2]+depth/2.]; let y = pa[2]*depth; let z = y-l2;
            let w = [pa[0]*l2,pa[1]*l2,pa[2]*l2-y*depth]; let x2 = dot(w,w);
            let y2 = y*y*l2; let z2 = z*z*l2;
            let sign = |v:f64|if v>0. {1.} else if v<0. {-1.} else {0.};
            let k = sign(rr)*rr*rr*x2;
            if sign(z)*a2*z2>k {(x2+z2).sqrt()*il2-r2}
            else if sign(y)*a2*y2<k {(x2+y2).sqrt()*il2-r}
            else {((x2*a2*il2).sqrt()+y*rr)*il2-r}
        },
        _ => {
            let an = std::f64::consts::PI/sides as f64;
            let bn = p[0].atan2(p[1]).rem_euclid(2.*an)-an;
            let ln = p[0].hypot(p[1]); let rad = (r-b).max(1e-9);
            let x = ln*bn.cos()-rad*an.cos(); let mut y = ln*bn.sin().abs()-rad*an.sin();
            y += (-y).clamp(0.,rad*an.sin());
            let sign = if x>0. {1.} else if x<0. {-1.} else {0.};
            extrude((x*x+y*y).sqrt()*sign,p[2],depth/2.)-b
        }
    }
}
fn shape(_: &Inputs,p:&Params)->Result<Geometry> {
    let name = p.e("shape", &["sphere","box","cylinder","capsule","ellipsoid","torus","round_cone","prism"])?;
    let h = spacing(p)?;
    let s = Shape {radius:p.f("radius")?,size:p.v3("size")?,depth:p.f("depth")?,minor:p.f("minor_radius")?,radius2:p.f("radius2")?,sides:p.i("sides")? as usize,bevel:p.f("bevel")?};
    if s.radius<1e-4 || s.depth<1e-4 || s.minor<1e-4 || s.radius2<1e-4 || s.size.iter().any(|x|*x<1e-4) || !(3..=64).contains(&s.sides) || s.bevel<0. { return Err(Error::new("invalid SDF shape dimensions")); }
    let smooth = p.i("smooth")?; if smooth<0 {return Err(Error::new("smooth must be nonnegative"));}
    let reach = match name.as_str() {
        "sphere"=>s.radius,"box"=>len(s.size)/2.,"cylinder"|"prism"=>s.radius.hypot(s.depth/2.),
        "capsule"=>s.radius+s.depth/2.,"ellipsoid"=>s.size.into_iter().fold(0.,f64::max)/2.,
        "torus"=>s.radius+s.minor,_=>s.radius.max(s.radius2)+s.depth/2.,
    }+2.*h;
    let c = p.v3("center")?; let r = math::transpose(&math::euler_xyz(p.v3("rotate")?));
    let (origin,dims) = grid(c.map(|x|x-reach),c.map(|x|x+reach),h,2.*h)?;
    let mut values = Vec::with_capacity(dims.iter().product());
    for i in 0..dims[0] {for j in 0..dims[1] {for k in 0..dims[2] {
        let ijk = [i,j,k]; let v = std::array::from_fn(|a|origin[a]+ijk[a] as f64*h-c[a]);
        values.push(distance(&name,math::apply(&r,v),&s) as f32);
    }}}
    if smooth>0 {values = sdf_field::blur(&values,dims,smooth as usize);}
    mesh(&values,dims,origin,h)
}
fn boolean(i:&Inputs,p:&Params)->Result<Geometry> {
    let a = SDF_BOOLEAN.one(i,0)?; let b = SDF_BOOLEAN.one(i,1)?;
    let h = spacing(p)?; let smooth = p.f("smooth")?;
    if !smooth.is_finite() || smooth<0. {return Err(Error::new("smooth must be nonnegative"));}
    let operation = p.e("operation",&["union","subtract","intersect"])?;
    let ta = tris(&a)?; let tb = tris(&b)?;
    if ta.is_empty() && tb.is_empty() {return Geometry::points_only(vec![]);}
    let band = 3f64.max(smooth/h+2.);
    for tris in [&ta,&tb] {
        let work: f64 = tris.chunks_exact(9).map(|t| (0..3).map(|k| {
            let lo = t[k].min(t[k+3]).min(t[k+6]); let hi = t[k].max(t[k+3]).max(t[k+6]);
            ((hi-lo+2.*band*h)/h).ceil()+1.
        }).product::<f64>()).sum();
        if work>15_000_000. {return Err(Error::new("this voxel size (with this smooth) needs too much work for the mesh: use a larger voxel size or a smaller smooth"));}
    }
    let mut lo = [f64::INFINITY;3]; let mut hi = [f64::NEG_INFINITY;3];
    for q in ta.chunks_exact(3).chain(tb.chunks_exact(3)) {for k in 0..3 {lo[k]=lo[k].min(q[k]);hi[k]=hi[k].max(q[k]);}}
    let (origin,dims) = grid(lo,hi,h,(band+2.)*h)?;
    let va = sdf_mesh::from_mesh(&ta,origin,dims,h,band*h).map_err(Error::new)?;
    let vb = sdf_mesh::from_mesh(&tb,origin,dims,h,band*h).map_err(Error::new)?;
    let (va,vb): (Vec<f32>,Vec<f32>) = va.into_iter().zip(vb).map(|(a,b)|match operation.as_str(){"intersect"=>(-a,-b),"subtract"=>(-a,b),_=>(a,b)}).unzip();
    let mut values = sdf_field::smooth_min_f32(&va,&vb,smooth as f32);
    if operation!="union" {for x in &mut values {*x = -*x;}}
    let mut out = mesh(&values,dims,origin,h)?;
    if p.b("transfer_attribs")? && out.npoints>0 {
        let source = if a.npoints>0 && b.npoints>0 {
            super::merge::merge(&[vec![a.clone(),b.clone()]],&Params::resolve(&[],None)?)?
        } else if a.npoints>0 {(*a).clone()} else {(*b).clone()};
        if source.npoints>0 {
            let sp = positions(&source);
            let ix: Vec<_> = positions(&out).into_iter().map(|p| {
                (0..sp.len()).min_by(|&a,&b|dot(sub(p,sp[a]),sub(p,sp[a])).total_cmp(&dot(sub(p,sp[b]),sub(p,sp[b])))).unwrap()
            }).collect();
            for (n,a) in gather(&source.point,&ix) {if n!="P" {out.point.insert(n,Arc::clone(&a));}}
        }
    }
    Ok(out)
}

fn transfer_point_attrs(out: &mut Geometry, source: &Geometry, transfer: &str) {
    if transfer.is_empty() || source.npoints == 0 || out.npoints == 0 {
        return;
    }
    let pats: Vec<&str> = transfer.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).collect();
    let matching: Vec<String> = source.point.keys().filter(|name| {
        *name != "P" && pats.iter().any(|pat| {
            if *pat == "*" { true }
            else if pat.ends_with('*') { name.starts_with(&pat[..pat.len() - 1]) }
            else { *name == *pat }
        })
    }).cloned().collect();
    if matching.is_empty() {
        return;
    }
    let sp = positions(source);
    let ix: Vec<_> = positions(out).into_iter().map(|p| {
        (0..sp.len()).min_by(|&a, &b| dot(sub(p, sp[a]), sub(p, sp[a])).total_cmp(&dot(sub(p, sp[b]), sub(p, sp[b])))).unwrap()
    }).collect();
    for (n, a) in gather(&source.point, &ix) {
        if matching.contains(&n) {
            out.point.insert(n, Arc::clone(&a));
        }
    }
}

fn remesh(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = REMESH.one(inputs, 0)?;
    let h = spacing(p)?;
    let offset = p.f("offset")?;
    let smooth = p.i("smooth")?;
    if smooth < 0 {
        return Err(Error::new("smooth must be nonnegative"));
    }
    let transfer = p.s("transfer")?;

    let ta = tris(&geo)?;
    if ta.is_empty() {
        return Geometry::points_only(vec![]);
    }

    let band = 3f64.max(offset.abs() / h + 2.0).max(smooth as f64 + 2.0);
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for q in ta.chunks_exact(3) {
        for k in 0..3 {
            lo[k] = lo[k].min(q[k]);
            hi[k] = hi[k].max(q[k]);
        }
    }
    let (origin, dims) = grid(lo, hi, h, (band + 2.0) * h)?;
    let mut values = sdf_mesh::from_mesh(&ta, origin, dims, h, band * h).map_err(Error::new)?;
    if offset.abs() > 1e-12 {
        let off32 = offset as f32;
        for v in &mut values {
            *v -= off32;
        }
    }
    if smooth > 0 {
        values = sdf_field::blur(&values, dims, smooth as usize);
    }
    let mut out = mesh(&values, dims, origin, h)?;
    transfer_point_attrs(&mut out, &geo, &transfer);
    Ok(out)
}

fn points_to_surface(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let points = POINTS_TO_SURFACE.one(inputs, 0)?;
    let h = spacing(p)?;
    let radius = p.f("radius")?;
    let blend = p.f("blend")?;
    let smooth = p.i("smooth")?;
    if smooth < 0 {
        return Err(Error::new("smooth must be nonnegative"));
    }
    let transfer = p.s("transfer")?;

    if points.npoints == 0 {
        return Geometry::points_only(vec![]);
    }

    let n = points.npoints;
    let mut r = vec![radius; n];
    if let Some(pscale_attr) = points.point.get("pscale") {
        if let Some(src) = pscale_attr.as_f32() {
            let w = pscale_attr.width;
            for i in 0..n {
                r[i] *= src[i * w] as f64;
            }
        }
    }

    let p_f64: Vec<f64> = points.p().iter().map(|&x| x as f64).collect();
    let field = sdf_field::from_points(&p_f64, &r, h, blend).map_err(Error::new)?;
    let mut values = field.values;
    if smooth > 0 {
        values = sdf_field::blur(&values, field.dims, smooth as usize);
    }
    let mut out = mesh(&values, field.dims, field.origin, h)?;
    transfer_point_attrs(&mut out, &points, &transfer);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{Attr,AttrData,AttrTable};
    use serde_json::Value;

    fn table(v:&Value)->AttrTable {
        v.as_object().unwrap().iter().map(|(name,a)|{
            let width=a["width"].as_u64().unwrap() as usize;
            let values=a["data"].as_array().unwrap();
            let attr=if a["dtype"]=="float32" {Attr::f32(width,values.iter().map(|x|x.as_f64().unwrap() as f32).collect())}
                else {Attr::i32(width,values.iter().map(|x|x.as_i64().unwrap() as i32).collect())};
            (name.clone(),Arc::new(attr))
        }).collect()
    }
    fn geometry(v:&Value)->Geometry {
        let point=table(&v["point"]);
        let vp=v["vp"].as_array().unwrap().iter().map(|x|x.as_i64().unwrap() as i32).collect();
        let offsets=v["offsets"].as_array().unwrap().iter().map(|x|x.as_i64().unwrap() as i32).collect();
        let closed=v["closed"].as_array().unwrap().iter().map(|x|x.as_bool().unwrap()).collect();
        let mut g=Geometry::from_arrays(point["P"].as_f32().unwrap().to_vec(),vp,offsets,Some(closed)).unwrap();
        g.point=point;g.vertex=table(&v["vertex"]);g.prim=table(&v["prim"]);g.detail=v["detail"].as_object().unwrap().clone();g.validate().unwrap();g
    }
    fn compare(name:&str,a:&Geometry,b:&Geometry) {
        assert_eq!(a.npoints,b.npoints,"{name}: point count");
        assert_eq!(a.topology.vertex_point,b.topology.vertex_point,"{name}: topology");
        assert_eq!(a.topology.prim_offsets,b.topology.prim_offsets,"{name}: offsets");
        assert_eq!(a.topology.closed_mask(),b.topology.closed_mask(),"{name}: closed flags");
        // Python lacks neutral material provenance; all Python detail keys remain exact.
        super::super::oracle_material_detail::compare(name,a,b);
        let (original_a,original_b)=(a,b);
        for domain in ["point","vertex","prim"] {
            let (a,b)=(a.table(domain),b.table(domain));
            let neutral_index=domain=="prim" && super::super::oracle_material_detail::neutral_index_added(original_a,original_b);
            assert_eq!(a.keys().filter(|key|!neutral_index || key.as_str()!="material_index").collect::<Vec<_>>(),b.keys().collect::<Vec<_>>(),"{name}: {domain} names");
            for (key,a) in a {if neutral_index && key=="material_index"{continue;}let b=&b[key];assert_eq!(a.width,b.width,"{name}: {domain} {key} width");
                let invalid_uv=if domain=="vertex"{super::super::oracle_material_detail::invalid_uv_rows(original_a,original_b,key,&(0..a.rows()).collect::<Vec<_>>())}else{vec![false;a.rows()]};
                match (&a.data,&b.data) {
                    (AttrData::F32(a),AttrData::F32(b))=>{
                        assert_eq!(a.len(),b.len(),"{name}: {domain} {key} length");
                        for (i,(&a,&b)) in a.iter().zip(b).enumerate(){if invalid_uv[i / original_a.table(domain)[key].width] && a.is_nan() && b==0.{continue;}assert!((a-b).abs()<=2e-6+2e-6*b.abs(),"{name}: {domain} {key}[{i}] Rust={a} Python={b}");}
                    },
                    (AttrData::I32(a),AttrData::I32(b))=>assert_eq!(a,b,"{name}: {domain} {key}"),
                    _=>panic!("{name}: {domain} {key} dtype"),
                }
            }
        }
    }
    #[test]
    fn python_volume_voronoi_and_ray_oracles() {
        // Provenance can add neutral material metadata absent from Python's geometry detail.
        let cases:Value=serde_json::from_str(include_str!("test_data/volume_fracture_parity.json")).unwrap();
        for c in cases.as_array().unwrap(){
            let name=c["name"].as_str().unwrap();let spec=super::super::get(c["type"].as_str().unwrap()).unwrap();
            let defaults=c["defaults"].as_object().unwrap();
            // Ray adds Rust-only SOP controls; every Python parameter must still exist with its original default.
            if spec.name!="ray"{assert_eq!(spec.params.len(),defaults.len(),"{name}: default count");}
            for (name,value) in defaults{let p=spec.params.iter().find(|p|p.name==name).unwrap_or_else(||panic!("missing Python parameter {name}"));
                assert!(crate::params::same_value(value,&(p.default)()),"{name}: default {}",p.name);}
            let inputs:Vec<_>=c["inputs"].as_array().unwrap().iter().map(|v|if v.is_null(){vec![]}else{vec![Arc::new(geometry(v))]}).collect();
            let result=Params::resolve(spec.params,c["params"].as_object()).and_then(|p|(spec.cook)(&inputs,&p));
            if c.get("error").is_some(){assert!(result.is_err(),"{name}: expected error");continue;}
            let out=result.unwrap_or_else(|e|panic!("{name}: {e}"));out.validate().unwrap();compare(name,&out,&geometry(&c["expected"]));
            if c["manifold"].as_bool()==Some(true){
                let e=crate::topo::edges(&out.topology,out.npoints);assert!(e.count.iter().all(|&n|n==2),"{name}: nonmanifold edges");
                let (_,nx,_)=corners(&out);let mut directions=vec![0i32;e.len()];
                for v in 0..out.nvertices(){if e.of_vertex[v]>=0{directions[e.of_vertex[v] as usize]+=if out.topology.vertex_point[v]<out.topology.vertex_point[nx[v]]{1}else{-1};}}
                assert!(directions.iter().all(|&n|n==0),"{name}: inconsistent winding");
            }
        }
    }
    #[test]
    fn analytic_distances_at_landmarks() {
        let s=Shape{radius:0.5,size:[1.,2.,3.],depth:1.,minor:0.15,radius2:0.2,sides:6,bevel:0.};
        assert_eq!(distance("sphere",[0.;3],&s),-0.5);
        assert_eq!(distance("box",[0.;3],&s),-0.5);
        assert_eq!(distance("capsule",[0.,0.,1.],&s),0.);
        assert!(distance("torus",[0.5,0.,0.],&s)<0.);
        assert_eq!(distance("ellipsoid",[0.;3],&s),-0.5);
    }
}
