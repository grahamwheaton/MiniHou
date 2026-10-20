//! Python fracture.py's exploded_view (the fracture solvers are separate work).
use std::collections::HashMap;
use crate::{Error,Geometry,Result,params::Params};
use super::{Inputs,NodeSpec};
pub const EXPLODED_VIEW:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None, cook_sidecar: None,name:"exploded_view",inputs:super::GEO,params:crate::params!("piece_attrib":"piece";"scale":0.5;"center":[0.,0.,0.]),cook:exploded_view};
fn exploded_view(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=EXPLODED_VIEW.one(inputs,0)?;let name=p.s("piece_attrib")?;
    let a=g.prim.get(&name).ok_or_else(||Error::new(format!("no primitive attribute {name:?} (fracture first, or use connectivity)")))?;
    let labels=super::blender_input::numbers(a);
    let mut slots=HashMap::new();let mut keys=Vec::with_capacity(g.nprims());
    for row in labels.chunks_exact(a.width){let v=row[0];let key=if v==0.{0}else if v.is_nan(){f64::NAN.to_bits()}else{v.to_bits()};
        let next=slots.len();keys.push(*slots.entry(key).or_insert(next));
    }
    let mut centers=vec![[0f64;3];slots.len()];let mut counts=vec![0usize;slots.len()];
    let mut owner=vec![None;g.npoints];let positions=g.p();let t=&g.topology;
    for (prim,&key) in keys.iter().enumerate(){for &point in &t.vertex_point[t.prim_offsets[prim] as usize..t.prim_offsets[prim+1] as usize]{
        let point=point as usize;counts[key]+=1;
        for c in 0..3{centers[key][c]+=positions[point*3+c] as f64;}
        if owner[point].is_none(){owner[point]=Some(key);}
    }}
    // NumPy's centroid lookup also fails for an all-loose, nonempty point cloud.
    if centers.is_empty()&&g.npoints>0{return Err(Error::new("no primitive pieces to explode"));}
    for (center,count) in centers.iter_mut().zip(counts){for v in center{*v/=count.max(1) as f64;}}
    let scale=p.f("scale")?;let origin=p.v3("center")?;let mut positions=positions.to_vec();
    for (i,key) in owner.into_iter().enumerate(){if let Some(key)=key{for c in 0..3{positions[i*3+c]=(positions[i*3+c] as f64+(centers[key][c]-origin[c])*scale) as f32;}}}
    let mut out=(*g).clone();out.set_p(positions);Ok(out)
}

#[cfg(test)]mod tests{
    use super::*;use crate::Attr;use std::sync::Arc;
    #[test]fn corner_weighted_centers_and_first_face_owner(){
        let mut g=Geometry::from_arrays(vec![0.,0.,0.,2.,0.,0.,0.,2.,0.,4.,0.,0.,9.,9.,9.],vec![0,1,2,1,3],vec![0,3,5],Some(vec![true,false])).unwrap();
        g.prim.insert("piece".into(),Arc::new(Attr::i32(1,vec![10,-5])));
        let p=Params::resolve(EXPLODED_VIEW.params,None).unwrap();let out=exploded_view(&[vec![Arc::new(g.clone())]],&p).unwrap();
        assert!((out.p()[3]-(2.+1./3.)).abs()<1e-6);assert_eq!(out.p()[9],5.5);assert_eq!(&out.p()[12..],&[9.,9.,9.]);
        assert_eq!(g.p()[3],2.);assert!(Arc::ptr_eq(&g.topology,&out.topology));assert_eq!(g.prim,out.prim);
    }
    #[test]fn missing_piece_errors(){let g=Geometry::points_only(vec![]).unwrap();assert!(exploded_view(&[vec![Arc::new(g)]],&Params::resolve(EXPLODED_VIEW.params,None).unwrap()).is_err());}
}
