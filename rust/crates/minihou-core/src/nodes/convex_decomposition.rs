//! Approximate plane-cut decomposition of closed polygon solids.
//! Uses the existing pure-Rust hull, clip and cap operations; no native VHACD dependency.
use std::sync::Arc;
use super::{GEO,Inputs,NodeSpec,model::{self,add,cross,dot,len,mul,positions,range,sub}};
use crate::{error::{Error,Result},geometry::{Attr,Geometry},params::Params,topo};

pub const CONVEX_DECOMPOSITION:NodeSpec=NodeSpec{name:"convex_decomposition",inputs:GEO,
    params:crate::params!("max_concavity":0.02;"max_pieces":32;"min_volume":0.001;"search":16;"piece_attrib":"piece";
        "outgrp_inside":true;"outgrp_inside_name":"inside";"outgrp_outside":false;"outgrp_outside_name":"outside";
        "outgrp_cutpoints":false;"outgrp_cutpoints_name":"cutpoints";"outgrp_cutedges":false;"outgrp_cutedges_name":"cutedges"),cook:cook};

pub(super) fn signed_volume(g:&Geometry)->f64 {let pts=positions(g);let vp=&g.topology.vertex_point;let mut v=0.;
    for f in 0..g.nprims(){let r=range(g,f);if r.len()<3{continue;}let a=pts[vp[r.start] as usize];
        for k in r.start+1..r.end-1 {v+=dot(a,cross(pts[vp[k] as usize],pts[vp[k+1] as usize]))/6.;}}
    v}
fn hull_volume(g:&Geometry)->Result<f64>{let p=Params::resolve(super::surface::CONVEX_HULL.params,None)?;
    let used:std::collections::BTreeSet<_>=g.topology.vertex_point.iter().copied().collect();
    let xyz:Vec<f32>=used.into_iter().flat_map(|id|g.p()[id as usize*3..id as usize*3+3].to_vec()).collect();
    let cloud=Geometry::points_only(xyz)?;
    let h=(super::surface::CONVEX_HULL.cook)(&[vec![Arc::new(cloud)]],&p)?;
    Ok(signed_volume(&h).abs())}
pub(super) fn missing(g:&Geometry)->Result<f64>{Ok((hull_volume(g)?-signed_volume(g).abs()).max(0.))}
fn closed(g:&Geometry)->bool{!g.topology.has_open()&&topo::edges(&g.topology,g.npoints).count.iter().all(|&v|v==2)}
fn plane_candidates(g:&Geometry,search:usize)->Vec<([f64;3],[f64;3])>{
    let pts=positions(g);let vp=&g.topology.vertex_point;let mut faces=Vec::new();
    for f in 0..g.nprims(){let r=range(g,f);if r.len()<3{continue;}let mut centre=[0.;3];
        for k in r.clone(){centre=add(centre,pts[vp[k] as usize]);}centre=mul(centre,1./r.len() as f64);
        let a=pts[vp[r.start] as usize];let mut n=[0.;3];
        for k in r.start+1..r.end-1{n=add(n,cross(sub(pts[vp[k] as usize],a),sub(pts[vp[k+1] as usize],a)));}
        if len(n)>1e-10{faces.push((centre,mul(n,1./len(n))));}}
    // Candidate faces most strongly next to concave geometry are preferable, but a
    // deterministic spread across faces avoids spending the entire search on one side.
    let count=faces.len().min(search);let mut planes=Vec::new();
    for k in 0..count{planes.push(faces[k*faces.len()/count]);}
    let centre=mul(pts.iter().copied().fold([0.;3],add),1./pts.len().max(1) as f64);
    for k in 0..3 {let mut axis=[0.;3];axis[k]=1.;
        planes.push((centre,axis));for sign in [-1.,1.]{let mut shifted=centre;shifted[k]+=sign*0.02*super::fracture_tools::extent(g);planes.push((shifted,axis));}}
    planes
}
fn split(g:&Geometry,o:[f64;3],n:[f64;3],min_volume:f64,eps:f64)->Result<Option<[Geometry;2]>>{
    let mut halves=Vec::new();
    for side in ["above","below"]{
        let p=Params::resolve(super::model::CLIP.params,serde_json::json!({"origin":o,"normal":n,"keep":side,"outgrp_above":false}).as_object())?;
        let clipped=(super::model::CLIP.cook)(&[vec![Arc::new(g.clone())]],&p)?;
        if clipped.nprims()==0{return Ok(None);}
        let cap=super::fracture_voronoi::cap_plane(clipped,o,n,eps)?;
        if !closed(&cap)||signed_volume(&cap).abs()<min_volume{return Ok(None);}
        halves.push(cap);
    }
    Ok(Some([halves.remove(0),halves.remove(0)]))
}
fn seam(g:&Geometry,inside:&[bool])->(Vec<bool>,Vec<bool>){
    let edges=topo::edges(&g.topology,g.npoints);let mut outside=vec![false;edges.len()];let mut inner=vec![false;edges.len()];
    for (face,&flag) in inside.iter().enumerate(){for v in range(g,face){let id=edges.of_vertex[v];if id>=0{if flag{inner[id as usize]=true;}else{outside[id as usize]=true;}}}}
    let seam:Vec<bool>=inner.iter().zip(&outside).map(|(&a,&b)|a&&b).collect();
    let mut points=vec![false;g.npoints];for (id,&yes) in seam.iter().enumerate(){if yes{points[edges.points[2*id] as usize]=true;points[edges.points[2*id+1] as usize]=true;}}
    (points,seam)
}
fn cook(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let source=CONVEX_DECOMPOSITION.one(inputs,0)?;model::polygons(&source,"convex_decomposition")?;
    if source.nprims()==0{return Ok((*source).clone());}
    if !closed(&source){return Err(Error::new("convex_decomposition needs a closed mesh (every edge shared by two faces)"));}
    let total=signed_volume(&source).abs();if total<=0.{return Err(Error::new("convex_decomposition: the mesh has no volume (faces pointing inwards?)"));}
    let threshold=p.f("max_concavity")?;let max_pieces=p.i("max_pieces")?;let min_volume=p.f("min_volume")?;
    let search=p.i("search")?;if !(0.0..=1.0).contains(&threshold)||!(1..=1000).contains(&max_pieces)
        ||!(0.0..=0.5).contains(&min_volume)||!(1..=200).contains(&search){return Err(Error::new("invalid convex_decomposition limits"));}
    let name=p.s("outgrp_inside_name")?;let prior=source.prim.get(&format!("group_{name}")).map(|a|super::group::flags(a)).unwrap_or_else(||vec![false;source.nprims()]);
    let mut start=(*source).clone();model::write_group(&mut start,"prim",super::fracture_voronoi::INSIDE,&prior)?;
    let size=super::fracture_tools::extent(&start);let eps=5e-7*size.max(1e-9)+1e-7;
    let mut pieces=vec![start];let mut stopped=std::collections::BTreeSet::new();
    while pieces.len()<max_pieces as usize{
        let mut ranked:Vec<_>=pieces.iter().enumerate().filter(|(i,_)|!stopped.contains(i))
            .map(|(i,g)|Ok((i,missing(g)?))).collect::<Result<Vec<_>>>()?;
        ranked.sort_by(|a,b|b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let Some((slot,deficit))=ranked.first().copied() else{break;};
        let parent=signed_volume(&pieces[slot]).abs();if deficit/parent.max(1e-300)<=threshold{stopped.insert(slot);continue;}
        let mut best:Option<(f64,[Geometry;2])>=None;
        for (o,n) in plane_candidates(&pieces[slot],search as usize){
            let Some(parts)=split(&pieces[slot],o,n,min_volume*total,eps)? else{continue;};
            let remaining=missing(&parts[0])?+missing(&parts[1])?;
            if best.as_ref().is_none_or(|(v,_)|remaining<*v){best=Some((remaining,parts));}
        }
        if let Some((remaining,[a,b]))=best.filter(|(v,_)|*v<deficit*(1.-1e-6)){
            let _=remaining;pieces[slot]=a;pieces.push(b);stopped.clear();
        }else{stopped.insert(slot);}
    }
    let piece_attrib=p.s("piece_attrib")?;let mut finished=Vec::new();
    for (id,mut g) in pieces.into_iter().enumerate(){
        let inside=g.prim.get(&format!("group_{}",super::fracture_voronoi::INSIDE)).map(|a|super::group::flags(a)).unwrap_or_else(||vec![false;g.nprims()]);
        g.prim.remove(&format!("group_{}",super::fracture_voronoi::INSIDE));
        if !piece_attrib.is_empty(){g.prim.insert(piece_attrib.clone(),Arc::new(Attr::i32(1,vec![id as i32;g.nprims()])));}
        let (points,edges)=seam(&g,&inside);
        model::outgroup(&mut g,p,"inside","prim",&inside)?;
        model::outgroup(&mut g,p,"outside","prim",&inside.iter().map(|&v|!v).collect::<Vec<_>>())?;
        model::outgroup(&mut g,p,"cutpoints","point",&points)?;
        model::outgroup(&mut g,p,"cutedges","edge",&edges)?;
        finished.push(Arc::new(g));
    }
    super::merge::merge(&[finished],&Params::resolve(&[],None)?)
}

#[cfg(test)] mod tests {use super::*;use crate::geometry::AttrData;
    #[test] fn convex_box_matches_python_golden(){
        // Python convex_decomposition(box()) returns 8 points, 6 faces, one piece, no inside faces.
        let bp=Params::resolve(super::super::generate::BOX.params,None).unwrap();
        let g=(super::super::generate::BOX.cook)(&[vec![]],&bp).unwrap();
        let p=Params::resolve(CONVEX_DECOMPOSITION.params,None).unwrap();let out=cook(&[vec![Arc::new(g)]],&p).unwrap();
        let oracle:serde_json::Value=serde_json::from_str(include_str!("test_data/last_nodes_parity.json")).unwrap();
        assert_eq!(out.npoints,oracle["convex_box"]["points"].as_u64().unwrap() as usize);
        assert_eq!(out.nprims(),oracle["convex_box"]["prims"].as_u64().unwrap() as usize);
        assert_eq!(out.prim["piece"].data,AttrData::I32(vec![0;6]));
        assert_eq!(out.prim["group_inside"].data,AttrData::I32(vec![0;6]));
    }
    #[test] fn l_prism_is_split_into_closed_pieces(){
        // Python oracle generated with Geometry.from_arrays on this six-corner L prism:
        // signed input volume 3; decomposition produces more than one piece.
        let xy=[[0.,0.],[2.,0.],[2.,1.],[1.,1.],[1.,2.],[0.,2.]];
        let pts:Vec<f32>=[0.,1.].into_iter().flat_map(|z|xy.into_iter().flat_map(move |p|[p[0],p[1],z])).collect();
        let mut faces=Vec::<Vec<i32>>::new();faces.push((0..6).rev().collect());faces.push((6..12).collect());
        for i in 0..6{faces.push(vec![i,(i+1)%6,(i+1)%6+6,i+6]);}
        let mut vp=Vec::new();let mut offsets=vec![0];for face in faces{vp.extend(face);offsets.push(vp.len() as i32);}
        let geo=Geometry::from_arrays(pts,vp,offsets,None).unwrap();
        let p=Params::resolve(CONVEX_DECOMPOSITION.params,serde_json::json!({"max_pieces":4}).as_object()).unwrap();
        let out=cook(&[vec![Arc::new(geo)]],&p).unwrap();
        let ids=out.prim["piece"].as_i32().unwrap();
        assert!(ids.iter().copied().max().unwrap_or(0)>0);
        assert!((signed_volume(&out)-3.).abs()<0.01,"volume {}",signed_volume(&out));
    }

    #[test]
    fn test_convex_decomposition_empty_and_errors() {
        // Empty geometry (0 prims) returns cloned geometry
        let empty = Geometry::points_only(vec![]).unwrap();
        let p = Params::resolve(CONVEX_DECOMPOSITION.params, None).unwrap();
        let out_empty = cook(&[vec![Arc::new(empty)]], &p).unwrap();
        assert_eq!(out_empty.npoints, 0);
        assert_eq!(out_empty.nprims(), 0);

        // Open mesh (e.g. single triangle) fails with clear error
        let tri = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 0., 1., 0.],
            vec![0, 1, 2],
            vec![0, 3],
            None,
        ).unwrap();
        let err_open = cook(&[vec![Arc::new(tri)]], &p).unwrap_err().to_string();
        assert!(err_open.contains("needs a closed mesh"), "{err_open}");

        // Invalid parameters
        let bp = Params::resolve(super::super::generate::BOX.params, None).unwrap();
        let cube = (super::super::generate::BOX.cook)(&[vec![]], &bp).unwrap();

        let p_bad_pieces = Params::resolve(CONVEX_DECOMPOSITION.params, serde_json::json!({"max_pieces": 0}).as_object()).unwrap();
        assert!(cook(&[vec![Arc::new(cube.clone())]], &p_bad_pieces).is_err());

        let p_bad_concavity = Params::resolve(CONVEX_DECOMPOSITION.params, serde_json::json!({"max_concavity": -0.1}).as_object()).unwrap();
        assert!(cook(&[vec![Arc::new(cube.clone())]], &p_bad_concavity).is_err());
    }

    #[test]
    fn test_convex_decomposition_determinism_and_attribute_preservation() {
        let bp = Params::resolve(super::super::generate::BOX.params, None).unwrap();
        let mut cube = (super::super::generate::BOX.cook)(&[vec![]], &bp).unwrap();
        cube.point.insert("Cd".into(), Arc::new(Attr::f32(3, vec![0.5; cube.npoints * 3])));
        cube.prim.insert("material_index".into(), Arc::new(Attr::i32(1, vec![10; cube.nprims()])));

        let p = Params::resolve(CONVEX_DECOMPOSITION.params, None).unwrap();
        let out1 = cook(&[vec![Arc::new(cube.clone())]], &p).unwrap();
        let out2 = cook(&[vec![Arc::new(cube.clone())]], &p).unwrap();

        // Deterministic output
        assert_eq!(out1.p(), out2.p());
        assert_eq!(out1.prim["piece"].data, out2.prim["piece"].data);

        // Attribute preservation
        assert_eq!(out1.point["Cd"].data, cube.point["Cd"].data);
        assert_eq!(out1.prim["material_index"].data, cube.prim["material_index"].data);
    }
}
