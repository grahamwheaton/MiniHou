//! Per-point ambient occlusion and thickness, following minihou/nodes/occlusion.py.
use std::sync::Arc;
use super::{InputSpec, Inputs, NodeSpec, model::{add, cross, len, mul, positions, sub}, ray::RayMesh};
use crate::{error::{Error, Result}, geometry::{Attr, Geometry}, params::Params};

pub const OCCLUSION: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None, name: "occlusion", inputs: &[
    InputSpec{name:"geo",multi:false,optional:false}, InputSpec{name:"occluders",multi:false,optional:true}],
    params: crate::params!("mode":"ambient_occlusion";"name":"";"rays":32;"spread":90.;"max_distance":0.;"bias":0.0001;"seed":0;"self_occlusion":true;"normalize":false;"invert":false;"blur":0;"group":""), cook: occlusion };

fn unit(v:[f64;3])->[f64;3] { let n=len(v); if n==0. {[0.;3]} else {mul(v,1./n)} }
fn occlusion(inputs:&Inputs,p:&Params)->Result<Geometry> {
    let g=OCCLUSION.one(inputs,0)?;
    if g.nprims()==0 {return Err(Error::new("occlusion needs a polygon mesh"));}
    let mode=p.e("mode", &["ambient_occlusion","thickness"])?;
    let name=p.s("name")?; let name=if name.is_empty(){if mode=="thickness"{"thickness"}else{"ao"}.to_string()}else{name};
    super::group::valid_name(&name)?;
    let selected=super::group::mask(&g,"point",&p.s("group")?)?;
    let rays=p.i("rays")?; if rays<1{return Err(Error::new("rays must be positive"));}
    let rays=rays as usize;
    if selected.iter().filter(|&&x|x).count().saturating_mul(rays)>20_000_000{return Err(Error::new("occlusion rays exceeds the generated-element safety limit"));}
    let spread=p.f("spread")?.clamp(1.,90.);let limit=if p.f("max_distance")?>0.{p.f("max_distance")?}else{f64::INFINITY};
    let normals=if let Some(n)=g.point.get("N") {n.as_f32().ok_or_else(||Error::new("N must be float"))?.to_vec()}else{crate::normals::point_normals(g.p(),&g.topology,g.npoints)};
    let pos=positions(&g);let mut lo=[f64::INFINITY;3];let mut hi=[f64::NEG_INFINITY;3];
    for v in &pos {for k in 0..3{lo[k]=lo[k].min(v[k]);hi[k]=hi[k].max(v[k]);}}
    let size=if pos.is_empty(){1.}else{len(sub(hi,lo)).max(1e-30)};
    let target=match (p.b("self_occlusion")?,inputs[1].first()) {
        (true,Some(other)) if other.nprims()>0 => super::merge::merge(&[vec![g.clone(),other.clone()]],&Params::resolve(&[],None)?)?,
        (true,_) => (*g).clone(),
        (false,Some(other)) if other.nprims()>0 => (**other).clone(),
        _ => return Err(Error::new("occlusion has nothing to cast against: switch self occlusion on or connect occluders")),
    };
    let mesh=RayMesh::new(&target);
    let mut rng=super::np_random::Rng::new(&[(p.i("seed")? as u64)&0xffff_ffff]);
    let zmin=spread.to_radians().cos();let golden=std::f64::consts::PI*(3.-5f64.sqrt());
    let sign=if mode=="thickness"{-1.}else{1.};
    let default=if mode=="thickness"{0.}else if p.b("invert")?{0.}else{1.};
    let mut full=vec![default as f64;g.npoints];
    for i in 0..g.npoints {if !selected[i]{continue;}
        let n=unit([normals[3*i] as f64,normals[3*i+1] as f64,normals[3*i+2] as f64]);
        let axis=mul(n,sign);let reference=if axis[0].abs()<0.9{[1.,0.,0.]}else{[0.,1.,0.]};
        let tangent=unit(cross(axis,reference));let bitangent=cross(axis,tangent);
        let angle=rng.random()*2.*std::f64::consts::PI;let (sn,c)=angle.sin_cos();
        let origin=add(pos[i],mul(axis,p.f("bias")?*size));let mut escaped=0.;let mut weight=0.;let mut total=0.;let mut hits=0.;
        for j in 0..rays {let z=1.-((j as f64+0.5)/rays as f64)*(1.-zmin);let r=(1.-z*z).max(0.).sqrt();let phi=j as f64*golden;let (sp,cp)=phi.sin_cos();
            let x=r*(cp*c-sp*sn);let y=r*(cp*sn+sp*c);
            let d=add(add(mul(tangent,x),mul(bitangent,y)),mul(axis,z));
            let hit=mesh.first_hit_distance(origin,d,limit);
            if mode=="thickness" {if let Some(dist)=hit{total+=dist;hits+=1.;}}
            else {weight+=z;if hit.is_none(){escaped+=z;}}
        }
        full[i]=if mode=="thickness"{if hits>0.{total/hits}else{0.}}else{let v=escaped/weight;if p.b("invert")?{1.-v}else{v}};
    }
    if mode=="thickness" && p.b("normalize")? {let max=full.iter().copied().fold(0.,f64::max);if max>0.{for x in &mut full{*x/=max;}}}
    let blur=p.i("blur")?;if blur>0 {
        let mut neighbours=vec![Vec::<usize>::new();g.npoints];
        for prim in 0..g.nprims(){let a=g.topology.prim_offsets[prim] as usize;let b=g.topology.prim_offsets[prim+1] as usize;
            for v in a..b{let x=g.topology.vertex_point[v] as usize;let y=g.topology.vertex_point[if v+1==b{a}else{v+1}] as usize;if x!=y{neighbours[x].push(y);neighbours[y].push(x);}}}
        for row in &mut neighbours{row.sort_unstable();row.dedup();}
        for _ in 0..blur {let old=full.clone();for (i,ns) in neighbours.iter().enumerate(){if !ns.is_empty(){let mean=ns.iter().map(|&j|old[j]).sum::<f64>()/ns.len() as f64;full[i]=0.5*(old[i]+mean);}}}
    }
    let mut out=(*g).clone();out.point.insert(name,Arc::new(Attr::f32(1,full.into_iter().map(|v|v as f32).collect())));Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plate(size:f32,z:f32)->Arc<Geometry> {
        let mut g=Geometry::from_arrays(vec![-size,-size,z,size,-size,z,size,size,z,-size,size,z],vec![0,1,2,3],vec![0,4],None).unwrap();
        g.point.insert("N".into(),Arc::new(Attr::f32(3,[0.,0.,1.].repeat(4))));
        Arc::new(g)
    }
    #[test]
    fn python_roof_golden() {
        // Python: minihou.nodes.occlusion.occlusion, a unit plate under a roof at z=1, 8 rays, seed=3.
        let a=plate(1.,0.);let roof=plate(2.,1.);
        let p=Params::resolve(OCCLUSION.params,serde_json::json!({"rays":8,"seed":3,"self_occlusion":false}).as_object()).unwrap();
        let out=occlusion(&[vec![a.clone()],vec![roof]],&p).unwrap();
        let got=out.point["ao"].as_f32().unwrap();
        for (x,y) in got.iter().zip([0.390625,0.3125,0.28125,0.25]) {assert!((x-y).abs()<1e-6,"{got:?}");}
        let p=Params::resolve(OCCLUSION.params,serde_json::json!({"mode":"thickness","rays":8,"spread":5.,"seed":3,"self_occlusion":false}).as_object()).unwrap();
        assert_eq!(occlusion(&[vec![a],vec![plate(2.,1.)]],&p).unwrap().point["thickness"].as_f32().unwrap(), &[0.;4]);
    }

    #[test]
    fn test_occlusion_empty_and_single_element() {
        let empty = Geometry::points_only(vec![]).unwrap();
        let p = Params::resolve(OCCLUSION.params, None).unwrap();
        // Empty geometry without prims returns error
        assert!(occlusion(&[vec![Arc::new(empty)], vec![]], &p).is_err());

        // Single triangle
        let tri = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 0., 1., 0.],
            vec![0, 1, 2],
            vec![0, 3],
            None,
        ).unwrap();
        let p_tri = Params::resolve(OCCLUSION.params, serde_json::json!({"rays": 8, "self_occlusion": true}).as_object()).unwrap();
        let out_tri = occlusion(&[vec![Arc::new(tri)], vec![]], &p_tri).unwrap();
        assert_eq!(out_tri.npoints, 3);
        assert_eq!(out_tri.point["ao"].as_f32().unwrap(), &[1.0, 1.0, 1.0]);
    }

    #[test]
    fn test_occlusion_degenerate_and_params() {
        // Degenerate triangle (collinear)
        let degen = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 2., 0., 0.],
            vec![0, 1, 2],
            vec![0, 3],
            None,
        ).unwrap();
        let p = Params::resolve(OCCLUSION.params, serde_json::json!({"rays": 8, "self_occlusion": true}).as_object()).unwrap();
        let _ = occlusion(&[vec![Arc::new(degen)], vec![]], &p);

        // Rays <= 0 is rejected
        let p_zero_rays = Params::resolve(OCCLUSION.params, serde_json::json!({"rays": 0}).as_object()).unwrap();
        let a = plate(1., 0.);
        assert!(occlusion(&[vec![a.clone()], vec![]], &p_zero_rays).is_err());

        // Invalid mode enum
        let p_bad_mode = Params::resolve(OCCLUSION.params, serde_json::json!({"mode": "invalid"}).as_object()).unwrap();
        assert!(occlusion(&[vec![a.clone()], vec![]], &p_bad_mode).is_err());

        // Invalid output attribute name
        let p_bad_name = Params::resolve(OCCLUSION.params, serde_json::json!({"name": "123invalid"}).as_object()).unwrap();
        assert!(occlusion(&[vec![a.clone()], vec![]], &p_bad_name).is_err());

        // Self occlusion false without occluders
        let p_no_target = Params::resolve(OCCLUSION.params, serde_json::json!({"self_occlusion": false}).as_object()).unwrap();
        assert!(occlusion(&[vec![a], vec![]], &p_no_target).is_err());
    }

    #[test]
    fn test_occlusion_determinism_and_attribute_preservation() {
        let mut a = Geometry::from_arrays(vec![-1., -1., 0., 1., -1., 0., 1., 1., 0., -1., 1., 0.], vec![0, 1, 2, 3], vec![0, 4], None).unwrap();
        a.point.insert("N".into(), Arc::new(Attr::f32(3, [0., 0., 1.].repeat(4))));
        a.point.insert("Cd".into(), Arc::new(Attr::f32(3, [0.5, 0.5, 0.5].repeat(4))));
        a.vertex.insert("uv".into(), Arc::new(Attr::f32(2, vec![0.1; 8])));
        a.prim.insert("material_index".into(), Arc::new(Attr::i32(1, vec![10])));

        let p = Params::resolve(OCCLUSION.params, serde_json::json!({"rays": 16, "seed": 42, "self_occlusion": true}).as_object()).unwrap();
        let out1 = occlusion(&[vec![Arc::new(a.clone())], vec![]], &p).unwrap();
        let out2 = occlusion(&[vec![Arc::new(a.clone())], vec![]], &p).unwrap();

        // Determinism
        assert_eq!(out1.point["ao"].data, out2.point["ao"].data);

        // Attribute preservation
        assert_eq!(out1.point["Cd"].data, a.point["Cd"].data);
        assert_eq!(out1.vertex["uv"].data, a.vertex["uv"].data);
        assert_eq!(out1.prim["material_index"].data, a.prim["material_index"].data);
    }
}
