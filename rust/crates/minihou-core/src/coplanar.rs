//! Shared geometric plane matching for viewport selection and procedural groups.
use crate::{Geometry,Result,error::Error};
#[derive(Clone,Copy,Debug,PartialEq)]
pub struct Plane {pub normal:[f64;3],pub offset:f64}
pub fn primitive_plane(g:&Geometry,prim:usize)->Option<Plane> {
    if !g.topology.is_closed(prim) {return None;}
    let r=g.topology.prim_offsets.get(prim..prim+2)?;
    let ids=&g.topology.vertex_point[r[0] as usize..r[1] as usize];
    if ids.len()<3 {return None;}
    let mut normal=[0.;3];let mut center=[0.;3];
    for (i,&id) in ids.iter().enumerate() {
        let a=&g.p()[id as usize*3..id as usize*3+3];let next=ids[(i+1)%ids.len()] as usize*3;let b=&g.p()[next..next+3];
        for k in 0..3 {center[k]+=a[k] as f64;normal[k]+=(a[(k+1)%3] as f64-b[(k+1)%3] as f64)*(a[(k+2)%3] as f64+b[(k+2)%3] as f64);}
    }
    let length=normal.iter().map(|x|x*x).sum::<f64>().sqrt();if length<1e-15 {return None;}
    normal=normal.map(|x|x/length);center=center.map(|x|x/ids.len() as f64);
    // Opposite winding describes the same plane.
    let axis=(0..3).max_by(|&a,&b|normal[a].abs().total_cmp(&normal[b].abs())).unwrap();
    if normal[axis]<0. {normal=normal.map(|x|-x);}
    Some(Plane {normal,offset:(0..3).map(|k|normal[k]*center[k]).sum()})
}
pub fn encode(planes:&[Plane])->String {planes.iter().map(|p|format!("{} {} {} {}",p.normal[0],p.normal[1],p.normal[2],p.offset)).collect::<Vec<_>>().join("; ")}
pub fn parse(text:&str)->Result<Vec<Plane>> {
    text.split(';').filter(|s|!s.trim().is_empty()).map(|s| {
        let v=s.split_whitespace().map(str::parse::<f64>).collect::<std::result::Result<Vec<_>,_>>().map_err(|_|Error::new("invalid coplanar plane; use nx ny nz offset"))?;
        if v.len()!=4 || v.iter().any(|x|!x.is_finite()) {return Err(Error::new("invalid coplanar plane; use finite nx ny nz offset"));}
        let length=v[..3].iter().map(|x|x*x).sum::<f64>().sqrt();if !length.is_finite() || length<1e-15 {return Err(Error::new("coplanar plane normal must be finite and nonzero"));}
        Ok(Plane {normal:[v[0]/length,v[1]/length,v[2]/length],offset:v[3]/length})
    }).collect()
}
pub fn mask(g:&Geometry,planes:&[Plane],angle:f64,tolerance:f64)->Result<Vec<bool>> {
    if !angle.is_finite() || !(0.0..=180.0).contains(&angle) || !tolerance.is_finite() || tolerance<0. {return Err(Error::new("coplanar angle must be 0..180 degrees and distance tolerance nonnegative"));}
    let limit=angle.to_radians().cos()-1e-9;
    Ok((0..g.nprims()).map(|prim| {
        let Some(candidate)=primitive_plane(g,prim) else {return false;};
        planes.iter().any(|plane| {
            let dot=(0..3).map(|k|plane.normal[k]*candidate.normal[k]).sum::<f64>();
            let offset=if dot<0. {-candidate.offset}else{candidate.offset};
            dot.abs()>=limit && (offset-plane.offset).abs()<=tolerance+1e-9
        })
    }).collect())
}
/// Uses the exact full-resolution preview ordering, including its chunk sorting.
pub fn preview_layout(g:&Geometry)->crate::preview::Layout {crate::preview::layout(g.p(),&g.topology.vertex_point,&g.topology.prim_offsets,g.topology.closed.as_deref(),g.npoints,usize::MAX,usize::MAX,100_000,8192)}
pub fn seeds(g:&Geometry,triangles:&[u32])->Vec<Plane> {
    let layout=preview_layout(g);let mut seen=std::collections::HashSet::new();let mut planes=Vec::new();
    for &tri in triangles {if let Some(&prim)=layout.tri_prim.get(tri as usize) {if let Some(plane)=primitive_plane(g,prim as usize) {
        let key=[plane.normal[0],plane.normal[1],plane.normal[2],plane.offset].map(|v|(v*1e7).round() as i64);
        if seen.insert(key) {planes.push(plane);}
    }}}
    planes
}

#[cfg(test)] mod tests {
    use super::*;
    use serde_json::json;
    fn planes_mesh()->Geometry {
        let mut positions=Vec::new();let mut vp=Vec::new();let mut offsets=vec![0];
        for (i,(z,tilt)) in [(0.,0.),(0.,0.),(0.00005,0.),(1.,0.),(0.,0.001),(0.,1.)].into_iter().enumerate() {
            for p in [[0.,0.,z],[1.,0.,z],[1.,1.,z+tilt],[0.,1.,z+tilt]] {positions.extend(p);}
            vp.extend((0..4).map(|k|(i*4+k) as i32));offsets.push(vp.len() as i32);
        }
        Geometry::from_arrays(positions,vp,offsets,None).unwrap()
    }
    #[test] fn coplanar_requires_plane_distance_and_angle_and_roundtrips() {
        let g=planes_mesh();let plane=primitive_plane(&g,0).unwrap();
        assert_eq!(parse(&encode(&[plane])).unwrap(),vec![plane]);
        assert_eq!(mask(&g,&[plane],0.01,0.0001).unwrap(),vec![true,true,true,false,false,false]);
        assert_eq!(mask(&g,&[plane],1.,0.002).unwrap(),vec![true,true,true,false,true,false]);
        assert_eq!(mask(&g,&[],1.,0.0001).unwrap(),vec![false;6]);
        assert!(parse("0 0 0 1").is_err());assert!(parse("0 0 1 NaN").is_err());
        assert!(mask(&g,&[plane],1.,-1.).is_err());
        assert_eq!(seeds(&g,&[0,1,2,3]).len(),1,"duplicate same-plane seed faces should collapse");
    }
    #[test] fn a_seed_face_matches_its_own_estimated_plane() {
        let g=Geometry::from_arrays(vec![0.,0.,0.,1.,0.,0.,1.,1.,0.01,0.,1.,0.],vec![0,1,2,3],vec![0,4],None).unwrap();
        let plane=primitive_plane(&g,0).unwrap();assert_eq!(mask(&g,&[plane],0.,0.).unwrap(),vec![true]);
    }
    #[test] fn coplanar_group_recooks_conditions_instead_of_saved_face_numbers() {
        let mut value=json!({"nodes":{"grid":{"type":"grid","params":{"rows":2,"columns":2}},"move":{"type":"transform","inputs":{"geo":"grid"}},"group":{"type":"group_create","inputs":{"geo":"move"}}},"display":"group"});
        let initial=crate::Graph::from_value(&value).unwrap().cook(Some("move")).unwrap();
        let plane=primitive_plane(&initial,0).unwrap();
        value["nodes"]["group"]["params"]=json!({"name":"flat","domain":"primitives","method":"coplanar","planes":encode(&[plane]),"angle":0.1,"plane_tolerance":0.0001});
        let mut session=crate::session::Session::new(crate::Graph::from_value(&value).unwrap());
        let g=session.cook(Some("group")).unwrap();assert_eq!(g.prim["group_flat"].as_i32().unwrap(),&[1]);
        session.set_param("grid","rows",json!(3)).unwrap();session.set_param("grid","columns",json!(3)).unwrap();
        let g=session.cook(Some("group")).unwrap();assert_eq!(g.prim["group_flat"].as_i32().unwrap(),&[1;4]);
        session.set_param("move","translate",json!(plane.normal.map(|v|v*0.01))).unwrap();
        let g=session.cook(Some("group")).unwrap();assert_eq!(g.prim["group_flat"].as_i32().unwrap(),&[0;4]);
    }
}
